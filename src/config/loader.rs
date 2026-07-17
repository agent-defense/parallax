use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tracing::{error, info, warn};

use crate::config::schema::{EvaluatorConfig, PlatformConfig};

/// Evaluator types whose rule loading from external files is handled here
/// (i.e. `rules_file` / `rules_dir` produce flat rule lists merged into `rules`).
/// Sigma is excluded because it uses multi-document YAML with its own loader.
const FILE_BACKED_TYPES: &[&str] = &["regex", "pattern", "cel", "sql"];

/// Engine types the rules-tree auto-discovery walks. Each one is expected
/// to live as a subdirectory under `<rules_dir>/<engine>/`.
const DISCOVERY_ENGINES: &[&str] = &["regex", "pattern", "cel", "sql", "sigma"];
use crate::engine::chain::EvaluatorChain;
use crate::evaluators::cel_eval::CELEvaluator;
use crate::evaluators::pattern_eval::PatternEvaluator;
use crate::evaluators::regex_eval::RegexEvaluator;
use crate::evaluators::sigma_eval::SigmaEvaluator;
use crate::evaluators::sql_eval::SQLEvaluator;

const DEFAULT_CONFIG_PATHS: &[&str] = &["parallax.yaml", "parallax.yml"];

/// Default stages for the synthesized Sigma evaluator when individual Sigma
/// documents do not declare stages (legacy). Prefer per-document `stages:`.
fn sigma_default_stages() -> Vec<String> {
    ["message.before", "tool.before", "tool.after"]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// Collect the union of per-rule `stages:` arrays from a list of rule values.
/// Rules missing a non-empty `stages:` are skipped (with a warning at the call site).
fn union_stages_from_rules(rules: &[serde_yaml::Value]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();
    for rule in rules {
        let Some(map) = rule.as_mapping() else {
            continue;
        };
        let Some(seq) = map
            .get(serde_yaml::Value::String("stages".into()))
            .and_then(|v| v.as_sequence())
        else {
            continue;
        };
        for v in seq {
            if let Some(s) = v.as_str() {
                if seen.insert(s.to_string()) {
                    ordered.push(s.to_string());
                }
            }
        }
    }
    ordered
}

/// Find a config file. If `path` is provided, use it directly.
/// Otherwise, search the default locations.
pub fn find_config(path: Option<&str>) -> Result<PathBuf, String> {
    if let Some(p) = path {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Ok(pb);
        }
        return Err(format!("Config file not found: {p}"));
    }

    for candidate in DEFAULT_CONFIG_PATHS {
        let pb = PathBuf::from(candidate);
        if pb.exists() {
            info!(path = %pb.display(), "Found config file");
            return Ok(pb);
        }
    }

    Err(format!(
        "No config file found in the current directory.\n\
         Searched for: {}\n\n\
         To get started, run from the parallax repo directory or specify a config path:\n  \
         parallax serve -c /path/to/parallax.yaml",
        DEFAULT_CONFIG_PATHS.join(", ")
    ))
}

/// Load evaluator configs from a directory of YAML files.
fn load_evaluator_dir(dir: &Path, config_root: &Path) -> Vec<EvaluatorConfig> {
    let dir = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        config_root.join(dir)
    };

    if !dir.is_dir() {
        return Vec::new();
    }

    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .map(|ext| ext == "yaml" || ext == "yml")
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort();

    let mut evaluators = Vec::new();
    for path in entries {
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_yaml::from_str::<serde_yaml::Value>(&content) {
                Ok(val) => {
                    if let Some(seq) = val.as_sequence() {
                        for item in seq {
                            match serde_yaml::from_value::<EvaluatorConfig>(item.clone()) {
                                Ok(ec) => evaluators.push(ec),
                                Err(e) => warn!(
                                    path = %path.display(),
                                    error = %e,
                                    "Failed to parse evaluator entry"
                                ),
                            }
                        }
                    } else {
                        match serde_yaml::from_value::<EvaluatorConfig>(val) {
                            Ok(ec) => {
                                info!(name = %ec.name, path = %path.display(), "Loaded evaluator from file");
                                evaluators.push(ec);
                            }
                            Err(e) => warn!(
                                path = %path.display(),
                                error = %e,
                                "Failed to parse evaluator file"
                            ),
                        }
                    }
                }
                Err(e) => warn!(path = %path.display(), error = %e, "Failed to parse YAML"),
            },
            Err(e) => warn!(path = %path.display(), error = %e, "Failed to read file"),
        }
    }

    evaluators
}

/// Load a YAML file expected to contain a sequence of rule entries.
/// Returns the sequence on success; warns and returns an empty vec on any failure.
fn load_rules_file(path: &Path) -> Vec<serde_yaml::Value> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Failed to read rules file");
            return Vec::new();
        }
    };
    let val: serde_yaml::Value = match serde_yaml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Failed to parse rules YAML");
            return Vec::new();
        }
    };
    match val {
        serde_yaml::Value::Sequence(seq) => seq,
        serde_yaml::Value::Null => Vec::new(),
        other => {
            warn!(
                path = %path.display(),
                kind = ?std::mem::discriminant(&other),
                "Rules file does not contain a YAML sequence, ignoring"
            );
            Vec::new()
        }
    }
}

/// Load all `.yaml` / `.yml` rule files from a directory, concatenating
/// their rule lists in sorted filename order.
fn load_rules_dir(dir: &Path) -> Vec<serde_yaml::Value> {
    if !dir.is_dir() {
        warn!(path = %dir.display(), "rules_dir not found, skipping");
        return Vec::new();
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .map(|ext| ext == "yaml" || ext == "yml")
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort();

    let mut rules = Vec::new();
    for path in entries {
        rules.extend(load_rules_file(&path));
    }
    rules
}

/// Resolve a possibly-relative path against the config root.
fn resolve_path(p: &str, config_root: &Path) -> PathBuf {
    let pb = PathBuf::from(p);
    if pb.is_absolute() {
        pb
    } else {
        config_root.join(pb)
    }
}

/// For evaluator types that support a flat rule list, expand `rules_file`
/// and `rules_dir` references (held in `extra`) into the evaluator's
/// inline `rules` list. Inline rules win; file-loaded rules are appended.
///
/// Sigma is intentionally skipped: it consumes `rules_dir` directly via its
/// own multi-document loader.
fn expand_rule_sources(evaluators: &mut [EvaluatorConfig], config_root: &Path) {
    for ec in evaluators.iter_mut() {
        if !FILE_BACKED_TYPES.contains(&ec.eval_type.as_str()) {
            continue;
        }

        let mut loaded: Vec<serde_yaml::Value> = Vec::new();

        if let Some(rules_file) = ec
            .extra
            .remove("rules_file")
            .and_then(|v| v.as_str().map(str::to_string))
        {
            let path = resolve_path(&rules_file, config_root);
            let entries = load_rules_file(&path);
            info!(
                name = %ec.name,
                path = %path.display(),
                count = entries.len(),
                "Loaded rules from rules_file"
            );
            loaded.extend(entries);
        }

        if let Some(rules_dir) = ec.rules_dir.clone() {
            let path = resolve_path(&rules_dir, config_root);
            let entries = load_rules_dir(&path);
            info!(
                name = %ec.name,
                path = %path.display(),
                count = entries.len(),
                "Loaded rules from rules_dir"
            );
            loaded.extend(entries);
            // Consumed by the generic loader; clear so evaluator construction
            // doesn't try to re-interpret it.
            ec.rules_dir = None;
        }

        if !loaded.is_empty() {
            ec.rules.extend(loaded);
        }
    }
}

/// Auto-discover a rules tree at `<rules_root>/<engine>/`, producing one
/// `EvaluatorConfig` per rule file (regex/pattern/cel/sql) and one combined
/// sigma evaluator that points the existing Sigma loader at `<rules_root>/sigma/`.
///
/// Each non-Sigma rule file is a flat YAML sequence of rule entries. Every
/// rule must declare its own `stages:` array. Evaluator name = filename stem;
/// evaluator-level stages = union of its rules' stages.
fn discover_rules_tree(rules_root: &Path) -> Vec<EvaluatorConfig> {
    if !rules_root.is_dir() {
        return Vec::new();
    }

    let mut discovered = Vec::new();

    for engine in DISCOVERY_ENGINES {
        let engine_dir = rules_root.join(engine);
        if !engine_dir.is_dir() {
            continue;
        }

        if *engine == "sigma" {
            // Single evaluator that delegates to the existing Sigma loader.
            discovered.push(EvaluatorConfig {
                name: "sigma-threats".into(),
                eval_type: "sigma".into(),
                enabled: true,
                stages: sigma_default_stages(),
                rules: Vec::new(),
                rules_dir: Some(engine_dir.to_string_lossy().into_owned()),
                extra: Default::default(),
            });
            info!(path = %engine_dir.display(), "Auto-discovered sigma evaluator");
            continue;
        }

        let mut files: Vec<PathBuf> = std::fs::read_dir(&engine_dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.extension()
                            .map(|ext| ext == "yaml" || ext == "yml")
                            .unwrap_or(false)
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort();

        for path in files {
            match parse_rule_file(&path, engine) {
                Some(ec) => {
                    info!(
                        name = %ec.name,
                        engine = engine,
                        rules = ec.rules.len(),
                        path = %path.display(),
                        "Auto-discovered evaluator from rules file"
                    );
                    discovered.push(ec);
                }
                None => warn!(
                    path = %path.display(),
                    engine = engine,
                    "Skipping unparseable rule file"
                ),
            }
        }
    }

    discovered
}

/// Parse a flat rule file (YAML sequence of rule entries) into an
/// `EvaluatorConfig`. Every rule must declare a non-empty `stages:` array;
/// rules missing it are skipped with a warning. Evaluator-level stages are
/// the union of surviving rules' stages.
///
/// ```yaml
/// - id: sec-001
///   title: AWS Access Key
///   description: ...
///   stages: [tool.before, tool.after]
///   pattern: "AKIA[0-9A-Z]{16}"
///   action: redact
///   priority: high
/// ```
fn parse_rule_file(path: &Path, engine: &str) -> Option<EvaluatorConfig> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Failed to read rule file");
            return None;
        }
    };
    let val: serde_yaml::Value = match serde_yaml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Failed to parse rule YAML");
            return None;
        }
    };

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unnamed")
        .to_string();

    let raw_rules = match val {
        serde_yaml::Value::Sequence(seq) => seq,
        serde_yaml::Value::Null => return None,
        other => {
            warn!(
                path = %path.display(),
                kind = ?std::mem::discriminant(&other),
                "Rule file must be a flat YAML list of rules (each with its own `stages:`)"
            );
            return None;
        }
    };

    let mut rules = Vec::new();
    for rule in raw_rules {
        let Some(map) = rule.as_mapping() else {
            warn!(path = %path.display(), "Skipping non-mapping rule entry");
            continue;
        };
        let id = map
            .get(serde_yaml::Value::String("id".into()))
            .and_then(|v| v.as_str())
            .unwrap_or("<unknown>");
        let stages_ok = map
            .get(serde_yaml::Value::String("stages".into()))
            .and_then(|v| v.as_sequence())
            .map(|seq| !seq.is_empty() && seq.iter().any(|v| v.as_str().is_some()))
            .unwrap_or(false);
        if !stages_ok {
            warn!(
                path = %path.display(),
                rule_id = id,
                "Rule is missing mandatory non-empty `stages:` array, skipping"
            );
            continue;
        }
        rules.push(rule);
    }

    if rules.is_empty() {
        warn!(
            path = %path.display(),
            "Rule file has no valid rules (each needs id + stages), skipping"
        );
        return None;
    }

    let stages = union_stages_from_rules(&rules);

    Some(EvaluatorConfig {
        name,
        eval_type: engine.to_string(),
        enabled: true,
        stages,
        rules,
        rules_dir: None,
        extra: Default::default(),
    })
}

/// Extract the set of rule ids from a discovered-evaluators list.
fn collect_rule_ids(evaluators: &[EvaluatorConfig]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for ec in evaluators {
        for rule in &ec.rules {
            if let Some(id) = rule
                .as_mapping()
                .and_then(|m| m.get(serde_yaml::Value::String("id".into())))
                .and_then(|v| v.as_str())
            {
                ids.insert(id.to_string());
            }
        }
    }
    ids
}

/// Apply "rules-tree wins" semantics: remove inline rules whose `id` also
/// appears in the discovered set. Operates on a slice of inline evaluators.
fn drop_overridden_inline_rules(
    inline: &mut [EvaluatorConfig],
    overridden_ids: &HashSet<String>,
) {
    if overridden_ids.is_empty() {
        return;
    }
    for ec in inline.iter_mut() {
        ec.rules.retain(|rule| {
            let id_opt = rule
                .as_mapping()
                .and_then(|m| m.get(serde_yaml::Value::String("id".into())))
                .and_then(|v| v.as_str());
            match id_opt {
                Some(id) if overridden_ids.contains(id) => {
                    info!(
                        rule_id = id,
                        evaluator = %ec.name,
                        "Inline rule overridden by rules/ tree"
                    );
                    false
                }
                _ => true,
            }
        });
    }
}

/// Load and validate the platform configuration from a YAML file.
///
/// Behaviour:
///   1. Parse `parallax.yaml` for server/proxy/reporting + any inline evaluators.
///   2. If `evaluators_dir` is set (or `./evaluators` exists), load full
///      evaluator definitions from there (legacy path; unchanged).
///   3. If `rules_dir` is set, or `./rules` exists next to the config file,
///      auto-discover evaluators from `<rules_dir>/<engine>/*.yaml`. Each rule
///      declares its own mandatory `stages:` array; evaluator stages are the
///      union. Skips files whose evaluator name is already declared inline.
///   4. Inline rules whose `id` matches a discovered rule are dropped (the
///      rules-tree version wins).
///   5. Any evaluator whose name is in `disabled:` is removed.
///   6. For evaluators using `rules_file` / `rules_dir` directly, expand
///      those references into inline `rules` (skip sigma — it loads on its own).
///
/// # Errors
///
/// Returns an error string if the file cannot be found, read, or parsed.
pub fn load_config(path: Option<&str>) -> Result<PlatformConfig, String> {
    let config_path = find_config(path)?;
    let config_root = config_path.parent().unwrap_or(Path::new("."));

    let content =
        std::fs::read_to_string(&config_path).map_err(|e| format!("Failed to read config: {e}"))?;
    let mut config: PlatformConfig =
        serde_yaml::from_str(&content).map_err(|e| format!("Failed to parse config: {e}"))?;

    // Legacy: full evaluator definitions from an evaluators/ directory.
    let eval_dir = config
        .evaluators_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| config_root.join("evaluators"));

    let dir_evaluators = load_evaluator_dir(&eval_dir, config_root);
    if !dir_evaluators.is_empty() {
        info!(
            count = dir_evaluators.len(),
            "Loaded evaluators from directory"
        );
        let mut merged: Vec<EvaluatorConfig> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();

        for ec in config.evaluators.drain(..).chain(dir_evaluators) {
            if let Some(&idx) = seen.get(&ec.name) {
                merged[idx] = ec.clone();
                info!(name = %ec.name, "Evaluator overwritten by later definition");
            } else {
                seen.insert(ec.name.clone(), merged.len());
                merged.push(ec);
            }
        }
        config.evaluators = merged;
    }

    // Auto-discover the rules tree (./rules or explicit rules_dir).
    let rules_root = config
        .rules_dir
        .as_deref()
        .map(|p| resolve_path(p, config_root))
        .or_else(|| {
            let default = config_root.join("rules");
            if default.is_dir() {
                Some(default)
            } else {
                None
            }
        });

    if let Some(rules_root) = rules_root {
        let discovered = discover_rules_tree(&rules_root);
        if !discovered.is_empty() {
            info!(
                path = %rules_root.display(),
                count = discovered.len(),
                "Auto-discovered evaluators from rules tree"
            );

            // Inline rules with ids that the rules-tree also defines: drop.
            let overridden = collect_rule_ids(&discovered);
            drop_overridden_inline_rules(&mut config.evaluators, &overridden);

            // Append discovered evaluators, skipping names already declared inline.
            let existing: HashSet<String> =
                config.evaluators.iter().map(|e| e.name.clone()).collect();
            for ec in discovered {
                if existing.contains(&ec.name) {
                    info!(
                        name = %ec.name,
                        "Skipping auto-discovered evaluator; inline definition in parallax.yaml takes precedence"
                    );
                    continue;
                }
                config.evaluators.push(ec);
            }
        }
    }

    // Apply `disabled:` filter.
    if !config.disabled.is_empty() {
        let drop: HashSet<&str> = config.disabled.iter().map(String::as_str).collect();
        let before = config.evaluators.len();
        config.evaluators.retain(|ec| {
            let keep = !drop.contains(ec.name.as_str());
            if !keep {
                info!(name = %ec.name, "Evaluator disabled by parallax.yaml `disabled:` list");
            }
            keep
        });
        if config.evaluators.len() != before {
            info!(
                removed = before - config.evaluators.len(),
                "Suppressed evaluators per `disabled:` list"
            );
        }
    }

    // Expand external rule references for file-backed evaluator types.
    expand_rule_sources(&mut config.evaluators, config_root);

    info!(
        evaluators = config.evaluators.len(),
        "Configuration loaded"
    );

    Ok(config)
}

/// Build an [`EvaluatorChain`] from the loaded configuration.
///
/// Disabled evaluators (`enabled: false`) are skipped. Unknown evaluator
/// types are logged and ignored.
pub fn build_chain(config: &PlatformConfig) -> EvaluatorChain {
    let mut chain = EvaluatorChain::new();

    for ec in &config.evaluators {
        if !ec.enabled {
            info!(name = %ec.name, "Evaluator disabled, skipping");
            continue;
        }

        let value = ec.to_evaluator_value();

        let evaluator: Option<Box<dyn crate::evaluators::Evaluator>> = match ec.eval_type.as_str() {
            "regex" => Some(Box::new(RegexEvaluator::new(ec.name.clone(), &value))),
            "pattern" => Some(Box::new(PatternEvaluator::new(ec.name.clone(), &value))),
            "sigma" => Some(Box::new(SigmaEvaluator::new(ec.name.clone(), &value))),
            "cel" => Some(Box::new(CELEvaluator::new(ec.name.clone(), &value))),
            "sql" => Some(Box::new(SQLEvaluator::new(ec.name.clone(), &value))),
            unknown => {
                error!(name = %ec.name, eval_type = unknown, "Unknown evaluator type, skipping");
                None
            }
        };

        if let Some(ev) = evaluator {
            info!(
                name = %ec.name,
                eval_type = %ec.eval_type,
                stages = ?ev.stages(),
                "Registered evaluator"
            );
            chain.add(ev);
        }
    }

    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_config_missing() {
        let result = find_config(Some("/nonexistent/path.yaml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_config_from_string() {
        let yaml = r#"
server:
  host: "0.0.0.0"
  port: 8080
evaluators:
  - name: test-regex
    type: regex
    stages: [tool.before]
    rules:
      - id: test-001
        title: "test"
        pattern: "foo"
        action: detect
"#;
        let config: PlatformConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);

        let chain = build_chain(&config);
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn test_load_rules_file_returns_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rules.yaml");
        std::fs::write(
            &path,
            r#"
- id: r-001
  title: one
  pattern: foo
  action: detect
- id: r-002
  title: two
  pattern: bar
  action: block
"#,
        )
        .unwrap();

        let rules = load_rules_file(&path);
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn test_load_rules_file_missing_returns_empty() {
        let rules = load_rules_file(Path::new("/nonexistent/rules.yaml"));
        assert!(rules.is_empty());
    }

    #[test]
    fn test_load_rules_dir_concatenates_sorted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("b.yaml"),
            "- {id: b-001, title: b, pattern: b, action: detect}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("a.yaml"),
            "- {id: a-001, title: a, pattern: a, action: detect}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("ignored.txt"),
            "not yaml, should be skipped",
        )
        .unwrap();

        let rules = load_rules_dir(dir.path());
        assert_eq!(rules.len(), 2);
        let first_id = rules[0]
            .as_mapping()
            .and_then(|m| m.get(serde_yaml::Value::String("id".into())))
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(first_id, "a-001");
    }

    #[test]
    fn test_expand_rule_sources_loads_rules_file() {
        let dir = tempfile::tempdir().unwrap();
        let rules_path = dir.path().join("cel-rules.yaml");
        std::fs::write(
            &rules_path,
            r#"
- id: cel-001
  title: t
  expr: 'tool_name == "exec"'
  action: block
"#,
        )
        .unwrap();

        let mut evals = vec![EvaluatorConfig {
            name: "cel-test".into(),
            eval_type: "cel".into(),
            enabled: true,
            stages: vec!["tool.before".into()],
            rules: vec![],
            rules_dir: None,
            extra: [(
                "rules_file".to_string(),
                serde_yaml::Value::String("cel-rules.yaml".into()),
            )]
            .into_iter()
            .collect(),
        }];

        expand_rule_sources(&mut evals, dir.path());
        assert_eq!(evals[0].rules.len(), 1);
        assert!(!evals[0].extra.contains_key("rules_file"));
    }

    #[test]
    fn test_expand_rule_sources_skips_sigma() {
        let dir = tempfile::tempdir().unwrap();
        let rules_dir = dir.path().join("sigma");
        std::fs::create_dir(&rules_dir).unwrap();

        let mut evals = vec![EvaluatorConfig {
            name: "sig".into(),
            eval_type: "sigma".into(),
            enabled: true,
            stages: vec!["tool.before".into()],
            rules: vec![],
            rules_dir: Some("sigma".into()),
            extra: Default::default(),
        }];

        expand_rule_sources(&mut evals, dir.path());
        // Sigma keeps its rules_dir intact so its own loader can use it.
        assert_eq!(evals[0].rules_dir.as_deref(), Some("sigma"));
        assert!(evals[0].rules.is_empty());
    }

    #[test]
    fn test_expand_rule_sources_merges_inline_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let rules_path = dir.path().join("regex.yaml");
        std::fs::write(
            &rules_path,
            r#"
- id: r-file-001
  title: from file
  pattern: foo
  action: detect
"#,
        )
        .unwrap();

        let inline: serde_yaml::Value = serde_yaml::from_str(
            r#"{id: r-inline-001, title: inline, pattern: bar, action: detect}"#,
        )
        .unwrap();

        let mut evals = vec![EvaluatorConfig {
            name: "regex-test".into(),
            eval_type: "regex".into(),
            enabled: true,
            stages: vec!["tool.before".into()],
            rules: vec![inline],
            rules_dir: None,
            extra: [(
                "rules_file".to_string(),
                serde_yaml::Value::String("regex.yaml".into()),
            )]
            .into_iter()
            .collect(),
        }];

        expand_rule_sources(&mut evals, dir.path());
        assert_eq!(evals[0].rules.len(), 2);
    }

    #[test]
    fn test_build_chain_skips_disabled() {
        let yaml = r#"
evaluators:
  - name: disabled-eval
    type: regex
    enabled: false
    rules: []
  - name: enabled-eval
    type: regex
    rules:
      - id: test-001
        title: x
        pattern: x
        action: detect
"#;
        let config: PlatformConfig = serde_yaml::from_str(yaml).unwrap();
        let chain = build_chain(&config);
        assert_eq!(chain.len(), 1);
    }

    /// Build a working rules tree on disk and verify auto-discovery unions
    /// per-rule `stages:` into the evaluator subscription.
    #[test]
    fn test_discover_rules_tree_reads_stages() {
        let dir = tempfile::tempdir().unwrap();
        let regex_dir = dir.path().join("regex");
        std::fs::create_dir(&regex_dir).unwrap();
        std::fs::write(
            regex_dir.join("secrets.yaml"),
            "- {id: sec-001, title: t, stages: [tool.before, tool.after], pattern: foo, action: redact}\n",
        )
        .unwrap();

        let cel_dir = dir.path().join("cel");
        std::fs::create_dir(&cel_dir).unwrap();
        std::fs::write(
            cel_dir.join("policies.yaml"),
            "- {id: pol-001, title: t, stages: [tool.before], expr: 'true', action: detect}\n",
        )
        .unwrap();

        let discovered = discover_rules_tree(dir.path());
        assert_eq!(discovered.len(), 2);

        let secrets = discovered.iter().find(|e| e.name == "secrets").unwrap();
        assert_eq!(secrets.eval_type, "regex");
        assert_eq!(
            secrets.stages,
            vec!["tool.before".to_string(), "tool.after".to_string()]
        );
        assert_eq!(secrets.rules.len(), 1);

        let policies = discovered.iter().find(|e| e.name == "policies").unwrap();
        assert_eq!(policies.eval_type, "cel");
        assert_eq!(policies.stages, vec!["tool.before".to_string()]);
    }

    /// Rules missing per-rule `stages:` are skipped; a file with no valid
    /// rules is rejected entirely.
    #[test]
    fn test_discover_rules_tree_rejects_missing_stages() {
        let dir = tempfile::tempdir().unwrap();
        let regex_dir = dir.path().join("regex");
        std::fs::create_dir(&regex_dir).unwrap();
        // Flat list but no per-rule stages.
        std::fs::write(
            regex_dir.join("flat.yaml"),
            "- {id: sec-001, title: t, pattern: foo, action: redact}\n",
        )
        .unwrap();
        // Old nested mapping shape (no longer accepted).
        std::fs::write(
            regex_dir.join("nested.yaml"),
            "stages: [tool.after]\nrules:\n  - {id: sec-002, title: t, pattern: bar, action: redact}\n",
        )
        .unwrap();

        let discovered = discover_rules_tree(dir.path());
        assert!(discovered.is_empty());
    }

    /// Sigma engine discovery emits a single evaluator that delegates to the
    /// existing sigma loader via `rules_dir`.
    #[test]
    fn test_discover_rules_tree_sigma_emits_combined_evaluator() {
        let dir = tempfile::tempdir().unwrap();
        let sigma_dir = dir.path().join("sigma");
        std::fs::create_dir(&sigma_dir).unwrap();
        // empty dir is fine — the sigma loader scans it on its own.
        let discovered = discover_rules_tree(dir.path());
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].eval_type, "sigma");
        assert_eq!(discovered[0].name, "sigma-threats");
        assert!(discovered[0].rules_dir.is_some());
    }

    /// When an inline rule id collides with a discovered one, drop the inline.
    #[test]
    fn test_drop_overridden_inline_rules() {
        let inline_rule: serde_yaml::Value = serde_yaml::from_str(
            r#"{id: sec-001, title: stub, pattern: foo, action: detect}"#,
        )
        .unwrap();
        let keep_rule: serde_yaml::Value = serde_yaml::from_str(
            r#"{id: sec-999, title: keep, pattern: bar, action: detect}"#,
        )
        .unwrap();

        let mut inline = vec![EvaluatorConfig {
            name: "starter".into(),
            eval_type: "regex".into(),
            enabled: true,
            stages: vec!["tool.before".into()],
            rules: vec![inline_rule, keep_rule],
            rules_dir: None,
            extra: Default::default(),
        }];

        let overridden: HashSet<String> = ["sec-001".to_string()].into_iter().collect();
        drop_overridden_inline_rules(&mut inline, &overridden);

        assert_eq!(inline[0].rules.len(), 1);
        let kept_id = inline[0].rules[0]
            .as_mapping()
            .and_then(|m| m.get(serde_yaml::Value::String("id".into())))
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(kept_id, "sec-999");
    }

    /// End-to-end: parallax.yaml + ./rules tree → loaded config has the
    /// discovered evaluators and the inline stub rule is dropped.
    #[test]
    fn test_load_config_auto_discovers_rules_tree() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("parallax.yaml");
        std::fs::write(
            &cfg_path,
            r#"
server:
  host: 127.0.0.1
  port: 9920
evaluators:
  - name: starter
    type: regex
    stages: [tool.before]
    rules:
      - id: sec-001
        title: inline stub
        pattern: STUB
        action: detect
"#,
        )
        .unwrap();

        let regex_dir = dir.path().join("rules").join("regex");
        std::fs::create_dir_all(&regex_dir).unwrap();
        std::fs::write(
            regex_dir.join("secrets.yaml"),
            "- {id: sec-001, title: real, stages: [tool.before, tool.after], pattern: AKIA, action: redact}\n",
        )
        .unwrap();

        let config = load_config(Some(cfg_path.to_str().unwrap())).unwrap();

        // We have the inline `starter` evaluator (now empty because its only
        // rule got overridden) plus the discovered `secrets` evaluator.
        let starter = config.evaluators.iter().find(|e| e.name == "starter").unwrap();
        assert!(starter.rules.is_empty(), "inline stub should be dropped");

        let secrets = config.evaluators.iter().find(|e| e.name == "secrets").unwrap();
        assert_eq!(secrets.rules.len(), 1);
    }

    /// `disabled:` removes both inline and discovered evaluators by name.
    #[test]
    fn test_load_config_disabled_filter() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("parallax.yaml");
        std::fs::write(
            &cfg_path,
            r#"
disabled: [pii]
"#,
        )
        .unwrap();

        let regex_dir = dir.path().join("rules").join("regex");
        std::fs::create_dir_all(&regex_dir).unwrap();
        std::fs::write(
            regex_dir.join("pii.yaml"),
            "- {id: pii-001, title: ssn, description: test, stages: [tool.after], pattern: \"\\\\d+\", action: redact, priority: high}\n",
        )
        .unwrap();
        std::fs::write(
            regex_dir.join("secrets.yaml"),
            "- {id: sec-001, title: t, stages: [tool.before, tool.after], pattern: AKIA, action: redact}\n",
        )
        .unwrap();

        let config = load_config(Some(cfg_path.to_str().unwrap())).unwrap();
        assert!(config.evaluators.iter().any(|e| e.name == "secrets"));
        assert!(!config.evaluators.iter().any(|e| e.name == "pii"));
    }

    /// Auto-discovered evaluators union per-rule `stages:`.
    #[test]
    fn test_load_config_reads_per_rule_stages() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("parallax.yaml");
        std::fs::write(&cfg_path, "server:\n  port: 9920\n").unwrap();

        let regex_dir = dir.path().join("rules").join("regex");
        std::fs::create_dir_all(&regex_dir).unwrap();
        std::fs::write(
            regex_dir.join("pii.yaml"),
            "- {id: pii-001, title: ssn, description: test, stages: [tool.after], pattern: \"\\\\d+\", action: redact, priority: high}\n",
        )
        .unwrap();

        let config = load_config(Some(cfg_path.to_str().unwrap())).unwrap();
        let pii = config.evaluators.iter().find(|e| e.name == "pii").unwrap();
        assert_eq!(pii.stages, vec!["tool.after".to_string()]);
    }

    /// When `./rules` is absent, only the inline evaluators survive.
    #[test]
    fn test_load_config_no_rules_dir_keeps_inline_only() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("parallax.yaml");
        std::fs::write(
            &cfg_path,
            r#"
evaluators:
  - name: starter
    type: regex
    stages: [tool.before]
    rules:
      - id: sec-001
        title: inline
        pattern: AKIA
        action: redact
"#,
        )
        .unwrap();

        let config = load_config(Some(cfg_path.to_str().unwrap())).unwrap();
        assert_eq!(config.evaluators.len(), 1);
        assert_eq!(config.evaluators[0].name, "starter");
        assert_eq!(config.evaluators[0].rules.len(), 1);
    }
}

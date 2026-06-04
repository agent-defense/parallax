use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tracing::{error, info, warn};

use crate::config::schema::{EvaluatorConfig, PlatformConfig};

/// Evaluator types whose rule loading from external files is handled here
/// (i.e. `rules_file` / `rules_dir` produce flat rule lists merged into `rules`).
/// Sigma is excluded because it uses multi-document YAML with its own loader.
const FILE_BACKED_TYPES: &[&str] = &["regex", "pattern", "cel", "sql"];
use crate::engine::chain::EvaluatorChain;
use crate::evaluators::cel_eval::CELEvaluator;
use crate::evaluators::pattern_eval::PatternEvaluator;
use crate::evaluators::regex_eval::RegexEvaluator;
use crate::evaluators::sigma_eval::SigmaEvaluator;
use crate::evaluators::sql_eval::SQLEvaluator;

const DEFAULT_CONFIG_PATHS: &[&str] = &[
    "parallax.yaml",
    "parallax.yml",
    "config.yaml",
    "config.yml",
];

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
         parallax serve -c /path/to/config.yaml",
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

/// Load and validate the platform configuration from a YAML file.
///
/// Merges inline evaluator definitions with any found in the `evaluators_dir`.
/// Duplicate evaluator names are resolved last-definition-wins.
///
/// For non-Sigma evaluators, also expands `rules_file` and `rules_dir`
/// references into the evaluator's inline `rules` list.
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

    // Load evaluators from directory
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
        // Merge: inline first, then directory. Deduplicate by name (last wins).
        let mut merged: Vec<EvaluatorConfig> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();

        for ec in config.evaluators.into_iter().chain(dir_evaluators) {
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
                stages = ?ec.stages,
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
}

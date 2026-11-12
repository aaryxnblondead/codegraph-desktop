//! Project configuration: modules, ignore globs, edge weights, rule thresholds.
//!
//! Loaded from JSON. Every field is optional and unknown fields are ignored.
//! A malformed file never crashes the app: the error carries the line/column
//! so the UI can point at it.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProjectKind {
    #[default]
    App,
    Library,
}

/// A committed suppression: a rule applied to a path glob.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct SuppressRule {
    pub rule: String,
    /// Glob over project-relative paths (`app/legacy/**`).
    pub path: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModuleDefConfig {
    pub name: String,
    pub paths: Vec<String>,
    /// Allowed dependency targets. `None` means no restriction.
    pub allowed_deps: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    pub project_kind: ProjectKind,
    pub ignore: Vec<String>,
    pub entry_points: Vec<String>,
    /// Edge-kind -> weight. `contains` defaults to 0 (structural).
    pub edge_weights: BTreeMap<String, f64>,
    pub min_confidence: f32,
    pub modules: Vec<ModuleDefConfig>,
    pub rules: BTreeMap<String, serde_json::Value>,
    /// Accepted debt, reviewed with the code.
    pub suppress: Vec<SuppressRule>,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        let mut edge_weights = BTreeMap::new();
        for (k, w) in [
            ("imports", 1.0),
            ("calls", 1.0),
            ("type_ref", 0.5),
            ("extends", 3.0),
            ("inherits", 3.0),
            ("implements", 2.0),
            ("references", 0.5),
            ("defines", 0.5),
            ("contains", 0.0),
        ] {
            edge_weights.insert(k.to_string(), w);
        }
        Self {
            project_kind: ProjectKind::App,
            ignore: Vec::new(),
            entry_points: vec![
                "**/main.rs".into(),
                "src/index.*".into(),
                "app/**/page.*".into(),
                "app/**/route.*".into(),
                "pages/**".into(),
            ],
            edge_weights,
            min_confidence: 0.5,
            modules: Vec::new(),
            rules: BTreeMap::new(),
            suppress: Vec::new(),
        }
    }
}

impl ProjectConfig {
    /// The file to read/write: a user-created `codegraph.config.json` in the
    /// project root wins; otherwise use the app config directory.
    pub fn config_path(project_root: &Path) -> PathBuf {
        let shared = project_root.join("codegraph.config.json");
        if shared.exists() {
            return shared;
        }
        if let Some(dirs) = crate::paths::app_dirs() {
            let id = crate::paths::project_root_id(project_root);
            return dirs
                .config_dir()
                .join("projects")
                .join(id)
                .join("config.json");
        }
        shared
    }

    /// Load from `path`; missing file yields defaults.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => {
                return Err(ConfigError {
                    line: 0,
                    col: 0,
                    message: e.to_string(),
                })
            }
        };
        Self::parse(&text)
    }

    /// Parse JSON text, mapping parse errors to a line/column.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        serde_json::from_str(text).map_err(|e| ConfigError {
            line: e.line(),
            col: e.column(),
            message: e.to_string(),
        })
    }

    /// Save pretty JSON, creating parent directories.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ConfigError {
                line: 0,
                col: 0,
                message: e.to_string(),
            })?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| ConfigError {
            line: 0,
            col: 0,
            message: e.to_string(),
        })?;
        std::fs::write(path, text).map_err(|e| ConfigError {
            line: 0,
            col: 0,
            message: e.to_string(),
        })
    }

    /// Weight for an edge kind (default 1.0).
    pub fn weight_for(&self, kind: &str) -> f64 {
        *self.edge_weights.get(kind).unwrap_or(&1.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line > 0 {
            write!(
                f,
                "config error at line {}, column {}: {}",
                self.line, self.col, self.message
            )
        } else {
            write!(f, "config error: {}", self.message)
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let cfg = ProjectConfig::default();
        assert_eq!(cfg.min_confidence, 0.5);
        assert_eq!(cfg.weight_for("imports"), 1.0);
        assert_eq!(cfg.weight_for("contains"), 0.0);
        assert!(!cfg.entry_points.is_empty());
    }

    #[test]
    fn empty_object_uses_defaults() {
        let cfg = ProjectConfig::parse("{}").expect("valid");
        assert_eq!(cfg.min_confidence, 0.5);
        assert_eq!(cfg.project_kind, ProjectKind::App);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let cfg = ProjectConfig::parse(r#"{"min_confidence": 0.7, "mystery": 42}"#).expect("valid");
        assert_eq!(cfg.min_confidence, 0.7);
    }

    #[test]
    fn bad_json_reports_position() {
        let err = ProjectConfig::parse("{ \"min_confidence\": }").unwrap_err();
        assert!(err.line >= 1);
        assert!(err.col >= 1);
    }

    #[test]
    fn editor_flow_validate_save_reload() {
        // What the module editor does: parse the text area, reject bad JSON with
        // a position, save, then load the file back.
        let dir = std::env::temp_dir().join("cg_config_editor");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");

        let err = ProjectConfig::parse(
            "{
  \"modules\": [ {
}",
        )
        .unwrap_err();
        assert!(err.line >= 2, "line should be reported: {err}");

        let mut cfg = ProjectConfig::default();
        cfg.modules.push(ModuleDefConfig {
            name: "UI".into(),
            paths: vec!["app/**".into()],
            allowed_deps: Some(vec!["Domain".into()]),
        });
        let text = serde_json::to_string_pretty(&cfg).unwrap();
        ProjectConfig::parse(&text).expect("generated text must validate");
        cfg.save(&path).unwrap();

        let back = ProjectConfig::load(&path).unwrap();
        assert_eq!(back.modules.len(), 1);
        assert_eq!(
            back.modules[0].allowed_deps.as_deref(),
            Some(&["Domain".to_string()][..])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suppress_rules_round_trip() {
        let cfg = ProjectConfig {
            suppress: vec![SuppressRule {
                rule: "complex_function".into(),
                path: "app/legacy/**".into(),
                reason: Some("generated".into()),
            }],
            ..Default::default()
        };
        let text = serde_json::to_string(&cfg).unwrap();
        let back = ProjectConfig::parse(&text).unwrap();
        assert_eq!(back.suppress.len(), 1);
        assert_eq!(back.suppress[0].rule, "complex_function");
        assert_eq!(back.suppress[0].path, "app/legacy/**");
    }

    #[test]
    fn round_trip() {
        let mut cfg = ProjectConfig {
            min_confidence: 0.9,
            ..Default::default()
        };
        cfg.modules.push(ModuleDefConfig {
            name: "UI".into(),
            paths: vec!["app/**".into()],
            allowed_deps: Some(vec!["Shared".into()]),
        });
        let text = serde_json::to_string(&cfg).unwrap();
        let back = ProjectConfig::parse(&text).unwrap();
        assert_eq!(back.min_confidence, 0.9);
        assert_eq!(back.modules[0].name, "UI");
    }
}

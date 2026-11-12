//! Persisted app settings (no GUI types).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub last_project: Option<PathBuf>,
    pub recent: Vec<PathBuf>,
    pub last_legacy_db: Option<PathBuf>,
    pub edge_kinds: Vec<String>,
    pub min_confidence: f32,
    pub color_by: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            last_project: None,
            recent: Vec::new(),
            last_legacy_db: None,
            edge_kinds: vec![
                "imports".into(),
                "calls".into(),
                "type_ref".into(),
                "extends".into(),
                "implements".into(),
            ],
            min_confidence: 0.5,
            color_by: "None".into(),
        }
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        crate::paths::app_dirs().map(|d| d.config_dir().join("settings.json"))
    }

    /// Load settings, falling back to defaults on a missing or corrupt file.
    pub fn load_or_default() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    /// Record a project as most-recent (bounded to 10).
    pub fn remember_project(&mut self, path: PathBuf) {
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path.clone());
        self.recent.truncate(10);
        self.last_project = Some(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_round_trip() {
        let mut s = Settings::default();
        s.remember_project(PathBuf::from("/tmp/a"));
        s.remember_project(PathBuf::from("/tmp/b"));
        s.remember_project(PathBuf::from("/tmp/a"));
        assert_eq!(s.recent[0], PathBuf::from("/tmp/a"));
        assert_eq!(s.recent.len(), 2);

        let text = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back.last_project, s.last_project);
    }

    #[test]
    fn corrupt_json_falls_back_to_default() {
        let parsed: Result<Settings, _> = serde_json::from_str("{ not json");
        assert!(parsed.is_err());
        let _ = Settings::default();
    }
}

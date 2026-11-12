//! Regression tracking: warning snapshots and diffs.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::metrics::UnitMetrics;
use crate::warnings::{Severity, Warning};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineWarning {
    pub id: String,
    pub rule: String,
    pub severity: String,
    /// Human-readable message, so a resolved finding can be shown by name.
    #[serde(default)]
    pub message: String,
    /// First location (`file::symbol:line–end`), when the rule had one.
    #[serde(default)]
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Baseline {
    pub created: String,
    pub warnings: Vec<BaselineWarning>,
    pub module_metrics: BTreeMap<String, f32>,
}

/// One side of a baseline diff, ready to display.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffEntry {
    pub id: String,
    pub rule: String,
    pub severity: String,
    pub message: String,
    pub location: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct BaselineDiff {
    pub new: Vec<String>,
    pub resolved: Vec<String>,
    pub unchanged: usize,
    /// Full details for the "what changed" list.
    pub new_findings: Vec<DiffEntry>,
    pub fixed_findings: Vec<DiffEntry>,
    /// Change in error / warning counts versus the baseline.
    pub error_delta: i64,
    pub warn_delta: i64,
}

impl BaselineDiff {
    pub fn is_empty(&self) -> bool {
        self.new.is_empty() && self.resolved.is_empty()
    }
}

pub fn snapshot(warnings: &[Warning], modules: &[UnitMetrics]) -> Baseline {
    let mut module_metrics = BTreeMap::new();
    for m in modules {
        if let Some(i) = m.instability {
            module_metrics.insert(m.unit.clone(), i);
        }
    }
    Baseline {
        created: unix_now().to_string(),
        warnings: warnings
            .iter()
            .map(|w| BaselineWarning {
                id: w.id.clone(),
                rule: w.rule.clone(),
                severity: w.severity.label().to_string(),
                message: w.message.clone(),
                location: w.locations.first().map(|l| l.display()),
            })
            .collect(),
        module_metrics,
    }
}

/// Compare current warnings against a baseline.
pub fn diff(baseline: &Baseline, current: &[Warning]) -> BaselineDiff {
    let old: std::collections::HashSet<&str> =
        baseline.warnings.iter().map(|w| w.id.as_str()).collect();
    let now: std::collections::HashSet<&str> = current.iter().map(|w| w.id.as_str()).collect();

    let new_findings: Vec<DiffEntry> = current
        .iter()
        .filter(|w| !old.contains(w.id.as_str()))
        .map(|w| DiffEntry {
            id: w.id.clone(),
            rule: w.rule.clone(),
            severity: w.severity.label().to_string(),
            message: w.message.clone(),
            location: w.locations.first().map(|l| l.display()),
        })
        .collect();
    let fixed_findings: Vec<DiffEntry> = baseline
        .warnings
        .iter()
        .filter(|w| !now.contains(w.id.as_str()))
        .map(|w| DiffEntry {
            id: w.id.clone(),
            rule: w.rule.clone(),
            severity: w.severity.clone(),
            message: w.message.clone(),
            location: w.location.clone(),
        })
        .collect();

    let count = |list: &[BaselineWarning], sev: &str| {
        list.iter().filter(|w| w.severity == sev).count() as i64
    };
    let current_count = |sev: Severity| current.iter().filter(|w| w.severity == sev).count() as i64;

    BaselineDiff {
        new: new_findings.iter().map(|e| e.id.clone()).collect(),
        resolved: fixed_findings.iter().map(|e| e.id.clone()).collect(),
        unchanged: current
            .iter()
            .filter(|w| old.contains(w.id.as_str()))
            .count(),
        new_findings,
        fixed_findings,
        error_delta: current_count(Severity::Error) - count(&baseline.warnings, "ERROR"),
        warn_delta: current_count(Severity::Warn) - count(&baseline.warnings, "WARN"),
    }
}

impl Baseline {
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    /// Default per-project baseline path.
    pub fn path_for(project_root: &Path) -> std::path::PathBuf {
        if let Some(dirs) = crate::paths::app_dirs() {
            let id = crate::paths::project_root_id(project_root);
            dirs.config_dir()
                .join("projects")
                .join(id)
                .join("baseline.json")
        } else {
            project_root.join("codegraph-baseline.json")
        }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::warnings::Severity;

    fn warning(id: &str, rule: &str) -> Warning {
        Warning {
            id: id.into(),
            severity: Severity::Warn,
            rule: rule.into(),
            message: String::new(),
            subjects: Vec::new(),
            count: 1,
            metrics: BTreeMap::new(),
            explanation: String::new(),
            suggestion: String::new(),
            locations: vec![crate::warnings::Location::file("src/a.rs")],
        }
    }

    #[test]
    fn snapshot_and_diff() {
        let old = snapshot(&[warning("a", "cycle"), warning("b", "god_file")], &[]);
        let diff = diff(
            &old,
            &[warning("b", "god_file"), warning("c", "fan_out_module")],
        );
        assert_eq!(diff.new, vec!["c".to_string()]);
        assert_eq!(diff.resolved, vec!["a".to_string()]);
        assert_eq!(diff.unchanged, 1);
    }

    #[test]
    fn diff_carries_details_and_count_deltas() {
        let old = snapshot(&[warning("a", "cycle"), warning("b", "god_file")], &[]);
        let current = vec![warning("b", "god_file"), warning("c", "fan_out_module")];
        let d = diff(&old, &current);
        assert_eq!(d.new, vec!["c".to_string()]);
        assert_eq!(d.resolved, vec!["a".to_string()]);
        assert_eq!(d.unchanged, 1);
        assert_eq!(d.new_findings[0].location.as_deref(), Some("src/a.rs"));
        assert_eq!(d.fixed_findings[0].rule, "cycle");
        // Two warnings before, two now: the counts did not move, only the set.
        assert_eq!(d.error_delta, 0);
        assert_eq!(d.warn_delta, 0);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join("cg_baseline_test");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("baseline.json");
        let b = snapshot(&[warning("a", "cycle")], &[]);
        b.save(&path).unwrap();
        let back = Baseline::load(&path).expect("load");
        assert_eq!(back.warnings.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

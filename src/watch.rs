//! Live file watching for the indexed project.
//!
//! Emits the set of project-relative paths that changed, debounced by the
//! caller. The watcher only observes; it never writes inside the project.
//!
//! `.git` is ignored on purpose: branch switches are handled by the git layer,
//! and a checkpoint flood would otherwise re-index on every commit.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use notify::{
    event::{EventKind, ModifyKind},
    recommended_watcher, RecursiveMode, Watcher,
};

use crate::paths::normalize;

/// Handle to a running watcher. Dropping it stops watching.
pub struct ProjectWatcher {
    _watcher: notify::RecommendedWatcher,
    rx: Receiver<PathBuf>,
    root: PathBuf,
}

impl ProjectWatcher {
    /// Watch `root` recursively. Returns an error when the platform watcher
    /// cannot be created (for example, out of inotify handles).
    pub fn start(root: &Path) -> notify::Result<Self> {
        let (tx, rx) = channel();
        let mut watcher = recommended_watcher(move |result: notify::Result<notify::Event>| {
            let Ok(event) = result else { return };
            if !matches!(
                event.kind,
                EventKind::Create(_)
                    | EventKind::Remove(_)
                    | EventKind::Modify(ModifyKind::Data(_))
                    | EventKind::Modify(ModifyKind::Name(_))
                    | EventKind::Modify(ModifyKind::Any)
                    | EventKind::Any
            ) {
                return;
            }
            for path in event.paths {
                if path.components().any(|c| c.as_os_str() == ".git") {
                    continue;
                }
                let _ = tx.send(path);
            }
        })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self {
            _watcher: watcher,
            rx,
            root: root.to_path_buf(),
        })
    }

    /// Drain pending events into project-relative paths (`a/b.rs`).
    ///
    /// Waits up to `timeout` for the first event, then drains whatever is
    /// already queued so one save produces one batch.
    pub fn drain(&self, timeout: Duration) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(first) = self.rx.recv_timeout(timeout) else {
            return out;
        };
        out.push(normalize(&self.root, &first));
        let deadline = std::time::Instant::now() + Duration::from_millis(120);
        while std::time::Instant::now() < deadline {
            match self.rx.recv_timeout(Duration::from_millis(20)) {
                Ok(path) => out.push(normalize(&self.root, &path)),
                Err(_) => break,
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Non-blocking drain, for the UI thread.
    pub fn try_drain(&self) -> Vec<String> {
        self.drain(Duration::from_millis(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reports_project_relative_paths_on_write() {
        let dir = temp_dir("cg_watch_write");
        let watcher = ProjectWatcher::start(&dir).expect("watcher starts");
        std::fs::write(dir.join("a.ts"), "export const a = 1;").unwrap();

        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !seen.iter().any(|p| p == "a.ts") {
            seen.extend(watcher.drain(Duration::from_millis(250)));
        }
        assert!(
            seen.iter().any(|p| p == "a.ts"),
            "expected a.ts in {seen:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignores_git_internals() {
        let dir = temp_dir("cg_watch_git");
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let watcher = ProjectWatcher::start(&dir).expect("watcher starts");
        std::fs::write(dir.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            seen.extend(watcher.drain(Duration::from_millis(150)));
        }
        assert!(
            !seen.iter().any(|p| p.contains(".git")),
            ".git events must be filtered: {seen:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

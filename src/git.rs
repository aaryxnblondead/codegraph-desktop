//! Git analytics: churn, hotspots, and logical (co-change) coupling.
//!
//! Shells out to the `git` CLI (no `git2` C dependency). Any failure disables
//! git features quietly.

use std::collections::HashMap;
use std::process::Command;

use crate::error::CodegraphError;

#[derive(Debug, Clone)]
pub struct Commit {
    pub hash: String,
    pub ts: i64,
    /// (path, added, deleted)
    pub files: Vec<(String, u64, u64)>,
}

#[derive(Debug, Clone, Default)]
pub struct Churn {
    pub file: String,
    pub commits: usize,
    pub added: u64,
    pub deleted: u64,
    pub last_ts: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoChange {
    pub a: String,
    pub b: String,
    pub support: usize,
    pub confidence: f64,
}

/// Parse `git log --numstat --pretty=format:%x1e%H%x1f%ct` output.
pub fn parse_git_log(text: &str) -> Vec<Commit> {
    let mut out = Vec::new();
    for record in text.split('\u{1e}') {
        let record = record.trim_start_matches(['\r', '\n']);
        if record.trim().is_empty() {
            continue;
        }
        let (header, body) = match record.split_once('\n') {
            Some((h, b)) => (h, b),
            None => (record, ""),
        };
        let mut parts = header.split('\u{1f}');
        let hash = parts.next().unwrap_or("").trim().to_string();
        let ts = parts
            .next()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0);
        if hash.is_empty() {
            continue;
        }
        let mut files = Vec::new();
        for line in body.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let mut cols = line.splitn(3, '\t');
            let added = cols.next().unwrap_or("");
            let deleted = cols.next().unwrap_or("");
            let path = cols.next().unwrap_or("");
            if path.is_empty() {
                continue;
            }
            let added = added.parse::<u64>().unwrap_or(0);
            let deleted = deleted.parse::<u64>().unwrap_or(0);
            files.push((path.to_string(), added, deleted));
        }
        out.push(Commit { hash, ts, files });
    }
    out
}

/// Run `git log` for the last `days` days and parse it.
pub fn run_log(root: &std::path::Path, days: u64) -> Result<Vec<Commit>, CodegraphError> {
    let since = format!("--since={days} days ago");
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).args([
        "log",
        &since,
        "--no-renames",
        "--numstat",
        "--pretty=format:%x1e%H%x1f%ct",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = cmd
        .output()
        .map_err(|e| CodegraphError::Git(e.to_string()))?;
    if !output.status.success() {
        return Err(CodegraphError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(parse_git_log(&String::from_utf8_lossy(&output.stdout)))
}

/// Aggregate churn per file.
pub fn churn(commits: &[Commit]) -> HashMap<String, Churn> {
    let mut out: HashMap<String, Churn> = HashMap::new();
    for commit in commits {
        for (path, added, deleted) in &commit.files {
            let entry = out.entry(path.clone()).or_insert_with(|| Churn {
                file: path.clone(),
                ..Default::default()
            });
            entry.commits += 1;
            entry.added += added;
            entry.deleted += deleted;
            entry.last_ts = entry.last_ts.max(commit.ts);
        }
    }
    out
}

/// Logical coupling: files that change together.
pub fn co_change(
    commits: &[Commit],
    max_files_per_commit: usize,
    min_support: usize,
    min_confidence: f64,
) -> Vec<CoChange> {
    let mut file_commits: HashMap<String, usize> = HashMap::new();
    let mut pair_support: HashMap<(String, String), usize> = HashMap::new();

    for commit in commits {
        let mut files: Vec<&String> = commit.files.iter().map(|(p, _, _)| p).collect();
        files.sort();
        files.dedup();
        if files.len() > max_files_per_commit {
            continue;
        }
        for f in &files {
            *file_commits.entry((*f).clone()).or_default() += 1;
        }
        for i in 0..files.len() {
            for j in (i + 1)..files.len() {
                let key = (files[i].clone(), files[j].clone());
                *pair_support.entry(key).or_default() += 1;
            }
        }
    }

    let mut out = Vec::new();
    for ((a, b), support) in pair_support {
        if support < min_support {
            continue;
        }
        let ca = *file_commits.get(&a).unwrap_or(&1) as f64;
        let cb = *file_commits.get(&b).unwrap_or(&1) as f64;
        let conf_ab = support as f64 / ca;
        let conf_ba = support as f64 / cb;
        let confidence = conf_ab.max(conf_ba);
        if confidence >= min_confidence {
            out.push(CoChange {
                a,
                b,
                support,
                confidence,
            });
        }
    }
    out.sort_by(|x, y| {
        y.confidence
            .partial_cmp(&x.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| y.support.cmp(&x.support))
    });
    out
}

// ---- repository state: branch, HEAD, worktrees -------------------------

/// One worktree of the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// Absolute path of the worktree root.
    pub path: String,
    /// Short branch name, or `None` for a detached worktree.
    pub branch: Option<String>,
    /// Commit currently checked out.
    pub head: String,
    /// True for the main worktree (the one `git worktree list` prints first).
    pub is_main: bool,
}

/// Branch and HEAD of the working tree at `root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoState {
    pub branch: Option<String>,
    pub head: String,
}

fn run_git(root: &std::path::Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Current branch and HEAD commit, or `None` when `root` is not a repository.
pub fn repo_state(root: &std::path::Path) -> Option<RepoState> {
    let head = run_git(root, &["rev-parse", "HEAD"])?.trim().to_string();
    let branch = run_git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|b| b.trim().to_string())
        .filter(|b| b != "HEAD");
    Some(RepoState { branch, head })
}

/// Parse `git worktree list --porcelain`.
pub fn parse_worktrees(text: &str) -> Vec<Worktree> {
    let mut out: Vec<Worktree> = Vec::new();
    let mut path: Option<String> = None;
    let mut head = String::new();
    let mut branch: Option<String> = None;
    let mut is_main = true;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            if let Some(previous) = path.take() {
                out.push(Worktree {
                    path: previous,
                    branch: branch.take(),
                    head: std::mem::take(&mut head),
                    is_main,
                });
                is_main = false;
            }
            path = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            head = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("branch ") {
            branch = Some(rest.trim().trim_start_matches("refs/heads/").to_string());
        } else if line.trim() == "detached" {
            branch = None;
        }
    }
    if let Some(previous) = path {
        out.push(Worktree {
            path: previous,
            branch,
            head,
            is_main,
        });
    }
    out
}

/// All worktrees of the repository containing `root`.
pub fn worktrees(root: &std::path::Path) -> Vec<Worktree> {
    run_git(root, &["worktree", "list", "--porcelain"])
        .map(|t| parse_worktrees(&t))
        .unwrap_or_default()
}

/// Local branches, most recent first.
pub fn branches(root: &std::path::Path) -> Vec<String> {
    run_git(
        root,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)",
            "refs/heads",
        ],
    )
    .map(|t| {
        t.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

/// Does `root` have uncommitted changes?
pub fn is_dirty(root: &std::path::Path) -> bool {
    run_git(root, &["status", "--porcelain"])
        .map(|t| !t.trim().is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{1e}abc\u{1f}1000\n3\t1\tsrc/a.ts\n2\t0\tsrc/b.ts\n\u{1e}def\u{1f}2000\n5\t0\tsrc/a.ts\n1\t1\tsrc/c.ts\n";

    #[test]
    fn parses_records_and_numstat() {
        let commits = parse_git_log(SAMPLE);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "abc");
        assert_eq!(commits[0].files.len(), 2);
        assert_eq!(commits[0].files[0], ("src/a.ts".into(), 3, 1));
    }

    #[test]
    fn churn_counts() {
        let commits = parse_git_log(SAMPLE);
        let c = churn(&commits);
        assert_eq!(c["src/a.ts"].commits, 2);
        assert_eq!(c["src/a.ts"].added, 8);
    }

    #[test]
    fn co_change_needs_support() {
        let commits = parse_git_log(SAMPLE);
        let pairs = co_change(&commits, 30, 1, 0.5);
        assert!(pairs
            .iter()
            .any(|p| (p.a == "src/a.ts" && p.b == "src/b.ts")
                || (p.a == "src/b.ts" && p.b == "src/a.ts")));
    }

    #[test]
    fn parses_worktree_list() {
        let text = "worktree C:/repo\nHEAD 1111111111111111111111111111111111111111\nbranch refs/heads/main\n\nworktree C:/repo-feature\nHEAD 2222222222222222222222222222222222222222\nbranch refs/heads/feature/x\n\nworktree C:/repo-detached\nHEAD 3333333333333333333333333333333333333333\ndetached\n";
        let trees = parse_worktrees(text);
        assert_eq!(trees.len(), 3);
        assert!(trees[0].is_main);
        assert_eq!(trees[0].branch.as_deref(), Some("main"));
        assert_eq!(trees[1].branch.as_deref(), Some("feature/x"));
        assert!(!trees[1].is_main);
        assert_eq!(trees[2].branch, None);
        assert_eq!(trees[2].path, "C:/repo-detached");
    }

    #[test]
    fn repo_state_reads_a_real_repository() {
        let dir = std::env::temp_dir().join("cg_git_state");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["init", "-b", "main"])
            .output();
        let Ok(status) = status else {
            let _ = std::fs::remove_dir_all(&dir);
            return; // git not installed: the feature is inert anyway
        };
        if !status.status.success() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        std::fs::write(dir.join("a.txt"), "hi").unwrap();
        for args in [
            vec!["add", "a.txt"],
            vec![
                "-c",
                "user.email=t@example.com",
                "-c",
                "user.name=T",
                "commit",
                "-m",
                "init",
            ],
        ] {
            let _ = std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output();
        }

        let state = repo_state(&dir).expect("repo state");
        assert_eq!(state.branch.as_deref(), Some("main"));
        assert_eq!(state.head.len(), 40);
        assert!(branches(&dir).iter().any(|b| b == "main"));
        let trees = worktrees(&dir);
        assert_eq!(trees.len(), 1);
        assert!(trees[0].is_main);
        assert!(!is_dirty(&dir));
        std::fs::write(dir.join("a.txt"), "changed").unwrap();
        assert!(is_dirty(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn binary_and_empty_lines_are_tolerated() {
        let text = "\u{1e}h\u{1f}1\n-\t-\tsrc/img.png\n\n";
        let commits = parse_git_log(text);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].files[0].1, 0);
    }
}

//! Headless indexer and report generator.
//!
//! ```text
//! cg_index <project_root> [--json out.json] [--markdown out.md]
//!          [--baseline baseline.json] [--fail-on-new error|warn]
//!          [--no-git] [--min-confidence 0.5]
//! ```
//!
//! Exit codes: 0 ok, 1 failure, 2 new warnings at or above `--fail-on-new`.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use clap::Parser;

use codegraph_desktop::baseline::{self, Baseline};
use codegraph_desktop::config::ProjectConfig;
use codegraph_desktop::indexer::cache::cache_path_for;
use codegraph_desktop::indexer::index_project_cached;
use codegraph_desktop::metrics;
use codegraph_desktop::modules::ModuleAssigner;
use codegraph_desktop::report::Report;
use codegraph_desktop::warnings::{self, Severity};

#[derive(Debug, Parser)]
#[command(
    name = "cg_index",
    about = "Index a project and report design problems"
)]
struct Args {
    /// Project folder to index.
    project_root: PathBuf,

    /// Write the JSON report here.
    #[arg(long)]
    json: Option<PathBuf>,

    /// Write the Markdown report here.
    #[arg(long)]
    markdown: Option<PathBuf>,

    /// Compare against this baseline and report new warnings.
    #[arg(long)]
    baseline: Option<PathBuf>,

    /// Write the current findings as a baseline (accept today's debt).
    #[arg(long)]
    save_baseline: Option<PathBuf>,

    /// Exit with code 2 when new warnings at or above this level appear.
    #[arg(long, value_parser = ["error", "warn"])]
    fail_on_new: Option<String>,

    /// Skip git analytics.
    #[arg(long)]
    no_git: bool,

    /// Minimum edge confidence to count.
    #[arg(long, default_value_t = 0.5)]
    min_confidence: f32,
}

fn main() {
    let args = Args::parse();
    match run(&args) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("cg_index: {err}");
            std::process::exit(1);
        }
    }
}

fn run(args: &Args) -> Result<i32, Box<dyn std::error::Error>> {
    let root = args
        .project_root
        .canonicalize()
        .unwrap_or_else(|_| args.project_root.clone());

    let mut config = ProjectConfig::load(&ProjectConfig::config_path(&root))?;
    config.min_confidence = args.min_confidence;

    let cache_path = cache_path_for(&root);
    let outcome = index_project_cached(
        &root,
        &config,
        None,
        Arc::new(AtomicBool::new(false)),
        Some(&cache_path),
    )?;
    let graph = &outcome.graph;

    let assigner = ModuleAssigner::new(&config.modules);
    let metrics = metrics::compute(graph, &assigner, &config);
    let mut found = warnings::run_warnings(graph, &assigner, &metrics, &config);

    if args.no_git {
        found.retain(|w| w.rule != "hotspot" && w.rule != "hidden_coupling");
    } else {
        add_git_warnings(&root, graph, &mut found);
    }

    // Baseline diff first: it is the most useful thing in the output.
    let baseline_data = args
        .baseline
        .as_ref()
        .and_then(|path| Baseline::load(path).map(|b| (b, path.clone())));
    let diff = baseline_data
        .as_ref()
        .map(|(b, _)| baseline::diff(b, &found));

    let mut report = Report::build_with_delta(graph, &metrics, &found, diff.as_ref());
    report.summary.refs_total = outcome.summary.refs_total;
    report.summary.refs_resolved = outcome.summary.refs_resolved;
    report.summary.resolved_ratio = outcome.summary.resolved_ratio();

    println!(
        "{} files · {} symbols · {} edges · {:.0}% refs resolved · {} ms",
        outcome.summary.files,
        outcome.summary.symbols,
        outcome.summary.edges,
        outcome.summary.resolved_ratio() * 100.0,
        outcome.summary.duration_ms
    );
    for note in &outcome.summary.notes {
        println!("note: {note}");
    }
    // Budgeted summary: counts, delta, top rules, then at most 10 findings.
    println!("{}", summarize(graph, &found, diff.as_ref()));

    if let Some(path) = &args.save_baseline {
        let modules = metrics.modules.as_slice();
        let snapshot = baseline::snapshot(&found, modules);
        snapshot
            .save(path)
            .map_err(|e| format!("cannot write baseline {}: {e}", path.display()))?;
        println!(
            "baseline: wrote {} findings to {}",
            found.len(),
            path.display()
        );
    }

    if let Some(path) = &args.json {
        write_file(path, &report.to_json())?;
        println!("wrote {}", path.display());
    }
    if let Some(path) = &args.markdown {
        write_file(path, &report.to_markdown())?;
        println!("wrote {}", path.display());
    }

    // Baseline diff (new vs resolved warnings).
    let mut exit = 0;
    if let Some(path) = &args.baseline {
        let old = Baseline::load(path)
            .ok_or_else(|| format!("baseline not found or unreadable: {}", path.display()))?;
        let diff = baseline::diff(&old, &found);
        println!(
            "baseline: {} new · {} resolved · {} unchanged",
            diff.new.len(),
            diff.resolved.len(),
            diff.unchanged
        );
        if let Some(level) = &args.fail_on_new {
            let threshold = if level == "error" {
                Severity::Error
            } else {
                Severity::Warn
            };
            let new_ids: std::collections::HashSet<&str> =
                diff.new.iter().map(String::as_str).collect();
            let blocking = found
                .iter()
                .filter(|w| new_ids.contains(w.id.as_str()) && w.severity >= threshold)
                .count();
            if blocking > 0 {
                eprintln!("{blocking} new warning(s) at or above {level}");
                exit = 2;
            }
        }
    }

    Ok(exit)
}

/// Human summary: counts, baseline delta, top rules, then the 10 worst findings.
fn summarize(
    graph: &codegraph_desktop::CodeGraph,
    warnings: &[warnings::Warning],
    diff: Option<&codegraph_desktop::baseline::BaselineDiff>,
) -> String {
    use std::collections::BTreeMap;
    let (mut e, mut w, mut i) = (0, 0, 0);
    for x in warnings {
        match x.severity {
            Severity::Error => e += 1,
            Severity::Warn => w += 1,
            Severity::Info => i += 1,
        }
    }
    let mut out = String::new();
    match diff {
        Some(d) => out.push_str(&format!(
            "findings: {e} error · {w} warn · {i} info  ({} new, {} fixed; errors {:+}, warnings {:+})\n",
            d.new.len(),
            d.resolved.len(),
            d.error_delta,
            d.warn_delta
        )),
        None => out.push_str(&format!("findings: {e} error · {w} warn · {i} info\n")),
    }

    if let Some(d) = diff {
        let show =
            |label: &str, entries: &[codegraph_desktop::baseline::DiffEntry], limit: usize| {
                if entries.is_empty() {
                    return String::new();
                }
                let mut s = format!("{label}:\n");
                for entry in entries.iter().take(limit) {
                    let at = entry
                        .location
                        .as_deref()
                        .map(|l| format!("  [{l}]"))
                        .unwrap_or_default();
                    s.push_str(&format!(
                        "  {:<5} {:<18} {}{}\n",
                        entry.severity, entry.rule, entry.message, at
                    ));
                }
                if entries.len() > limit {
                    s.push_str(&format!("  … {} more\n", entries.len() - limit));
                }
                s
            };
        out.push_str(&show("NEW", &d.new_findings, 10));
        out.push_str(&show("FIXED", &d.fixed_findings, 5));
    }

    // Rule histogram, so a 900-finding report is one line per rule.
    let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
    for x in warnings {
        *by_rule.entry(x.rule.as_str()).or_default() += 1;
    }
    if !by_rule.is_empty() {
        let mut rules: Vec<(&str, usize)> = by_rule.into_iter().collect();
        rules.sort_by_key(|r| std::cmp::Reverse(r.1));
        out.push_str("by rule: ");
        out.push_str(
            &rules
                .iter()
                .map(|(r, n)| format!("{r} {n}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push('\n');
    }

    if diff.is_none() {
        out.push_str("top findings:\n");
        for x in warnings.iter().take(10) {
            let at = x
                .locations
                .first()
                .filter(|l| !x.message.contains(&l.display()))
                .map(|l| format!("  [{}]", l.display()))
                .unwrap_or_default();
            out.push_str(&format!(
                "  {:<5} {:<18} {}{}\n",
                x.severity.label(),
                x.rule,
                x.message,
                at
            ));
        }
        if warnings.len() > 10 {
            out.push_str(&format!("  … {} more\n", warnings.len() - 10));
        }
    }
    let _ = graph;
    out.trim_end().to_string()
}

fn add_git_warnings(
    root: &std::path::Path,
    graph: &codegraph_desktop::CodeGraph,
    warnings: &mut Vec<warnings::Warning>,
) {
    let Ok(commits) = codegraph_desktop::git::run_log(root, 180) else {
        return; // not a repo / git missing: features disable quietly
    };
    let churn = codegraph_desktop::git::churn(&commits);

    // Hotspots: complexity percentile × commits percentile.
    let mut ranked: Vec<(String, f64)> = graph
        .files()
        .iter()
        .filter_map(|(path, info)| {
            let commits = churn.get(path).map(|c| c.commits).unwrap_or(0);
            if commits == 0 {
                return None;
            }
            let complexity = if info.function_count == 0 {
                info.loc as u32
            } else {
                info.complexity_sum
            };
            Some((path.clone(), complexity as f64 * commits as f64))
        })
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (path, score) in ranked.iter().take(10) {
        let mut m = std::collections::BTreeMap::new();
        m.insert("score".to_string(), *score);
        warnings.push(warnings::Warning {
            id: warnings::warning_id("hotspot", std::slice::from_ref(path)),
            severity: Severity::Warn,
            rule: "hotspot".to_string(),
            message: format!("`{path}` is a churn hotspot"),
            subjects: vec![path.clone()],
            count: 1,
            metrics: m,
            explanation: "This file changes often and carries a lot of complexity.".to_string(),
            suggestion:
                "Add tests around it, or split the parts that change for different reasons."
                    .to_string(),
            locations: vec![warnings::Location::file(path.clone())],
        });
    }

    // Hidden coupling: strong co-change with no static path.
    let pairs = codegraph_desktop::git::co_change(&commits, 30, 5, 0.6);
    let cross = warnings::cross_edges(graph, &ProjectConfig::default());
    for pair in &pairs {
        let direct = cross.contains_key(&(pair.a.clone(), pair.b.clone()))
            || cross.contains_key(&(pair.b.clone(), pair.a.clone()));
        if direct {
            continue;
        }
        let mut m = std::collections::BTreeMap::new();
        m.insert("support".to_string(), pair.support as f64);
        m.insert("confidence".to_string(), pair.confidence);
        warnings.push(warnings::Warning {
            id: warnings::warning_id("hidden_coupling", &[pair.a.clone(), pair.b.clone()]),
            severity: Severity::Info,
            rule: "hidden_coupling".to_string(),
            message: format!(
                "`{}` and `{}` change together but are not linked",
                pair.a, pair.b
            ),
            subjects: vec![pair.a.clone(), pair.b.clone()],
            count: 1,
            metrics: m,
            explanation:
                "Files that change together usually share a concept the code does not name."
                    .to_string(),
            suggestion: "Check whether they share a hidden rule; if so, extract it.".to_string(),
            locations: vec![
                warnings::Location::file(pair.a.clone()),
                warnings::Location::file(pair.b.clone()),
            ],
        });
    }
}

fn write_file(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, content)
}

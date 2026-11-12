//! CodeGraph desktop explorer — egui/eframe UI.
//!
//! Immediate-mode UI with a custom graph painter:
//! - Left: searchable symbol list.
//! - Right: hierarchical graph (module overview → file view → symbol
//!   ego-network) drawn with directed arrows.
//! - Bottom: audit (cycles + coupling) in overview modes, symbol detail in ego.
//!
//! Database loading runs on a worker thread so big graphs never freeze the UI.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use eframe::egui;
use egui::{Color32, RichText, Vec2};

use codegraph_desktop::ui::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use codegraph_desktop::analysis::{self, Cycle, FileMetrics};
use codegraph_desktop::baseline::{self, Baseline};
use codegraph_desktop::config::ProjectConfig;
use codegraph_desktop::git::{self, Churn, CoChange};
use codegraph_desktop::graph_view::{
    build_ego_view, build_file_overview, build_module_overview, GraphView,
};
use codegraph_desktop::impact;
use codegraph_desktop::indexer::cache::{cache_path_for, load_cached_graph};
use codegraph_desktop::indexer::{index_project_cached, IndexOutcome, IndexProgress};
use codegraph_desktop::metrics::{self, MetricsBundle};
use codegraph_desktop::modules::{self as module_defs, ModuleAssigner};
use codegraph_desktop::report::{
    context_text, cycle_text, impact_text, module_text, symbol_ref, warning_text, warnings_text,
};
use codegraph_desktop::settings::Settings;
use codegraph_desktop::structure::{self, CutEdge};
use codegraph_desktop::warnings::{self, Severity, Warning};
use codegraph_desktop::{
    model::{EdgeDirection, RelationshipKind, SymbolId},
    storage::Storage,
    CodeGraph, CodegraphError, MemoryStorage,
};

use codegraph_desktop::SqliteStorage;

const MAX_MODULES: usize = 40;
const MAX_FILE_NODES: usize = 160;
const MAX_VISUALIZED_NODES: usize = 80;
const MAX_LIST: usize = 500;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DbSource {
    #[default]
    Demo,
    File,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Modules,
    Files(String),
    Ego,
}

#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub summary: String,
    pub cycles: Vec<Cycle>,
    pub report: Vec<FileMetrics>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Audit,
    Modules,
    Warnings,
    Cycles,
    Impact,
    Hotspots,
    Hidden,
}

/// Git state shown in the header (branch, HEAD, worktrees, branches).
#[derive(Debug, Clone, Default)]
pub struct RepoInfo {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub worktrees: Vec<codegraph_desktop::git::Worktree>,
    pub branches: Vec<String>,
}

/// One file-level dependency cycle and the edges that would break it.
#[derive(Debug, Clone, Default)]
pub struct CycleGroup {
    pub files: Vec<String>,
    pub cuts: Vec<CutEdge>,
}

/// A database load running on a worker thread.
type LoadResult = (PathBuf, Result<(CodeGraph, String), String>);

/// Result of a git analytics run: (churn, co-change pairs, note).
type GitResult = (Vec<Churn>, Vec<CoChange>, Option<String>);

/// A project index running on a worker thread.
enum IndexEvent {
    Progress(IndexProgress),
    Finished(Result<Box<IndexOutcome>, String>),
}

pub struct CodegraphApp {
    graph: CodeGraph,
    symbols: Vec<codegraph_desktop::model::Symbol>,
    selected: Option<SymbolId>,
    search: String,
    status: String,
    error: Option<String>,
    db_source: DbSource,
    db_path: Option<PathBuf>,
    project_root: Option<PathBuf>,
    visualized: Option<GraphView>,
    view_mode: ViewMode,
    analysis: Option<AnalysisResult>,

    // camera
    pan: Vec2,
    zoom: f32,

    // async load
    load_rx: Option<Receiver<LoadResult>>,
    loading: bool,

    // async indexing
    index_rx: Option<Receiver<IndexEvent>>,
    index_cancel: Option<Arc<AtomicBool>>,
    index_progress: String,
    indexing: bool,

    // analysis
    tab: Tab,
    metrics: Option<MetricsBundle>,
    warnings: Vec<Warning>,
    assigner: ModuleAssigner,
    churn: Vec<Churn>,
    cochange: Vec<CoChange>,
    git_note: Option<String>,
    git_rx: Option<Receiver<GitResult>>,
    git_loading: bool,
    baseline: Option<Baseline>,
    /// Project config in use (modules, rules, thresholds).
    config: ProjectConfig,
    /// Where `config` was loaded from / is saved to.
    config_path: Option<PathBuf>,
    /// Cycle groups with the edges that would break them.
    cycles: Vec<CycleGroup>,
    /// Live file watching and recently touched files.
    watcher: Option<codegraph_desktop::watch::ProjectWatcher>,
    touched: Vec<(String, std::time::Instant)>,
    last_watch_event: Option<std::time::Instant>,
    watch_enabled: bool,
    /// Git state for the header and the worktree picker.
    repo: Option<crate::RepoInfo>,
    git_state_rx: Option<Receiver<RepoInfo>>,
    git_state_loading: bool,
    /// Transient "copied" acknowledgement.
    copied: Option<(String, std::time::Instant)>,
    /// Warning filter state.
    warning_filter: String,
    show_error: bool,
    show_warn: bool,
    show_info: bool,
    /// Module editor window state.
    editor_open: bool,
    editor_text: String,
    editor_error: Option<String>,
}

impl Default for CodegraphApp {
    fn default() -> Self {
        let storage: Box<dyn Storage<Error = CodegraphError>> =
            Box::new(MemoryStorage::new(demo_graph()));
        let (graph, symbols, status) = load_graph(&*storage).expect("demo graph is valid");
        Self::from_graph(graph, symbols, status, DbSource::Demo, None)
    }
}

impl CodegraphApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(16, 16, 16);
        visuals.window_fill = Color32::from_rgb(16, 16, 16);
        visuals.extreme_bg_color = BG;
        visuals.window_stroke = egui::Stroke::new(1.0, HAIRLINE);
        visuals.selection.bg_fill = ACCENT.linear_multiply(0.35);
        visuals.hyperlink_color = ACCENT;
        cc.egui_ctx.set_visuals(visuals);

        cc.egui_ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(12.0, 7.0);
            style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(7);
            style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(7);
            style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(7);
            style.visuals.widgets.inactive.bg_fill = CARD;
            style.visuals.widgets.hovered.bg_fill = CARD_HOVER;
            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT_DIM);
            style.text_styles = [
                (egui::TextStyle::Body, egui::FontId::proportional(13.0)),
                (egui::TextStyle::Button, egui::FontId::proportional(12.5)),
                (egui::TextStyle::Small, egui::FontId::proportional(11.0)),
                (egui::TextStyle::Heading, egui::FontId::proportional(17.0)),
                (egui::TextStyle::Monospace, egui::FontId::monospace(11.5)),
            ]
            .into();
        });

        // Show the last project's cached graph immediately (no waiting).
        let settings = Settings::load_or_default();
        if let Some(last) = settings.last_project.clone() {
            let config_path = ProjectConfig::config_path(&last);
            let config = ProjectConfig::load(&config_path).unwrap_or_default();
            let mut app = Self {
                project_root: Some(last.clone()),
                config,
                config_path: Some(config_path),
                ..Self::default()
            };
            // A cache makes the first frame instant; without one we still open
            // the project (instead of dropping back to the demo) and index it.
            if let Some(graph) = load_cached_graph(&last) {
                let symbols = graph.symbols().to_vec();
                let status = format!(
                    "showing cached index · {} symbols",
                    fmt_num(graph.symbol_count())
                );
                app.adopt_graph(graph, symbols, status, DbSource::File, Some(last.clone()));
                app.start_indexing(last.clone(), true);
            } else {
                app.status = "indexing project…".into();
                app.start_indexing(last.clone(), false);
            }
            app.baseline = Baseline::load(&Baseline::path_for(&last));
            app.start_watching(&last);
            return app;
        }
        Self::default()
    }

    fn from_graph(
        graph: CodeGraph,
        symbols: Vec<codegraph_desktop::model::Symbol>,
        status: String,
        db_source: DbSource,
        db_path: Option<PathBuf>,
    ) -> Self {
        let mut app = Self {
            graph,
            symbols,
            selected: None,
            search: String::new(),
            status,
            error: None,
            db_source,
            db_path,
            project_root: None,
            visualized: None,
            view_mode: ViewMode::Modules,
            analysis: None,
            pan: Vec2::ZERO,
            zoom: 1.0,
            load_rx: None,
            loading: false,
            index_rx: None,
            index_cancel: None,
            index_progress: String::new(),
            indexing: false,
            tab: Tab::Audit,
            metrics: None,
            warnings: Vec::new(),
            assigner: ModuleAssigner::default(),
            churn: Vec::new(),
            cochange: Vec::new(),
            git_note: None,
            git_rx: None,
            git_loading: false,
            baseline: None,
            config: ProjectConfig::default(),
            config_path: None,
            cycles: Vec::new(),
            watcher: None,
            touched: Vec::new(),
            last_watch_event: None,
            watch_enabled: true,
            repo: None,
            git_state_rx: None,
            git_state_loading: false,
            copied: None,
            warning_filter: String::new(),
            show_error: true,
            show_warn: true,
            show_info: false,
            editor_open: false,
            editor_text: String::new(),
            editor_error: None,
        };
        app.rebuild_all();
        app
    }

    fn rebuild_all(&mut self) {
        let config = self.config.clone();
        self.assigner = ModuleAssigner::new(&config.modules);
        let metrics = metrics::compute(&self.graph, &self.assigner, &config);
        self.warnings = warnings::run_warnings(&self.graph, &self.assigner, &metrics, &config);
        self.metrics = Some(metrics);

        let cross = warnings::cross_edges(&self.graph, &config);
        let cross_list: Vec<(String, String)> = cross.keys().cloned().collect();
        self.cycles = analysis::find_file_cycles(&self.graph, config.min_confidence)
            .into_iter()
            .map(|files| CycleGroup {
                cuts: structure::break_cycle(&files, &cross_list, &cross),
                files,
            })
            .collect();

        self.visualized = build_module_overview(&self.graph, MAX_MODULES);
        self.analysis = Some(run_analysis(&self.graph));
        self.view_mode = ViewMode::Modules;
        self.selected = None;
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;

        self.churn.clear();
        self.cochange.clear();
        self.git_note = None;
        if let Some(root) = self.project_root.clone() {
            self.load_git(root);
        }
    }

    fn load_git(&mut self, root: PathBuf) {
        if self.git_loading {
            return;
        }
        let (tx, rx) = channel();
        self.git_rx = Some(rx);
        self.git_loading = true;
        std::thread::spawn(move || {
            let result = match git::run_log(&root, 180) {
                Ok(commits) => {
                    let mut churn: Vec<Churn> = git::churn(&commits).into_values().collect();
                    churn.sort_by(|a, b| {
                        b.commits.cmp(&a.commits).then_with(|| a.file.cmp(&b.file))
                    });
                    let pairs = git::co_change(&commits, 30, 5, 0.6);
                    (churn, pairs, None)
                }
                Err(e) => (Vec::new(), Vec::new(), Some(e.to_string())),
            };
            let _ = tx.send(result);
        });
    }

    fn poll_git(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.git_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok((churn, pairs, note)) => {
                self.churn = churn;
                self.cochange = pairs;
                self.git_note = note;
                self.git_loading = false;
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                self.git_rx = Some(rx);
            }
            Err(TryRecvError::Disconnected) => {
                self.git_loading = false;
            }
        }
    }

    /// Focus the graph on a warning subject (module, file, or symbol).
    fn focus_subject(&mut self, subject: &str) {
        if let Some((a, _b)) = subject.split_once(" -> ") {
            self.show_module_files(a.trim());
            return;
        }
        if self.graph.file(subject).is_some() {
            self.show_file(subject);
            return;
        }
        if let Some(sym) = self
            .graph
            .symbols()
            .iter()
            .find(|s| s.qualified_name == subject || s.name == subject)
        {
            let id = sym.id.clone();
            self.select_symbol(&id);
        }
    }

    fn adopt_graph(
        &mut self,
        graph: CodeGraph,
        symbols: Vec<codegraph_desktop::model::Symbol>,
        status: String,
        source: DbSource,
        path: Option<PathBuf>,
    ) {
        self.graph = graph;
        self.symbols = symbols;
        self.status = status;
        self.error = None;
        self.db_source = source;
        self.db_path = path;
        self.rebuild_all();
    }

    fn refresh_symbols(&mut self) {
        self.symbols = self.graph.find(&self.search).into_iter().cloned().collect();
    }

    fn show_overview(&mut self) {
        self.visualized = build_module_overview(&self.graph, MAX_MODULES);
        self.view_mode = ViewMode::Modules;
        self.selected = None;
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
    }

    fn show_module_files(&mut self, module: &str) {
        if let Some(view) = build_file_overview(&self.graph, MAX_FILE_NODES, Some(module)) {
            self.visualized = Some(view);
            self.view_mode = ViewMode::Files(module.to_string());
            self.selected = None;
            self.pan = Vec2::ZERO;
            self.zoom = 1.0;
        }
    }

    fn show_file(&mut self, file: &str) {
        let mut best: Option<(SymbolId, usize)> = None;
        for s in self.graph.symbols() {
            if s.file != file {
                continue;
            }
            let deg = self
                .graph
                .relationships_for(s.id.clone(), EdgeDirection::Outgoing)
                .len()
                + self
                    .graph
                    .relationships_for(s.id.clone(), EdgeDirection::Incoming)
                    .len();
            if best.as_ref().map(|(_, d)| deg > *d).unwrap_or(true) {
                best = Some((s.id.clone(), deg));
            }
        }
        if let Some((id, _)) = best {
            self.select_symbol(&id);
        }
    }

    fn select_symbol(&mut self, id: &SymbolId) {
        self.selected = Some(id.clone());
        self.status = self
            .graph
            .symbol(id.clone())
            .map(|s| format!("{id} — {}", s.name))
            .unwrap_or_else(|| format!("selected {id}"));
        self.error = None;
        self.visualized = build_ego_view(&self.graph, id, MAX_VISUALIZED_NODES);
        self.view_mode = ViewMode::Ego;
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
    }

    fn handle_click(&mut self, id: SymbolId) {
        match self.view_mode.clone() {
            ViewMode::Modules => self.show_module_files(&id.0),
            ViewMode::Files(_) => self.show_file(&id.0),
            ViewMode::Ego => self.select_symbol(&id),
        }
    }

    fn use_demo(&mut self) {
        self.project_root = None;
        let storage: Box<dyn Storage<Error = CodegraphError>> =
            Box::new(MemoryStorage::new(demo_graph()));
        match load_graph(&*storage) {
            Ok((graph, symbols, status)) => {
                self.adopt_graph(graph, symbols, status, DbSource::Demo, None)
            }
            Err(e) => self.error = Some(format!("demo failed: {e}")),
        }
    }

    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("CodeGraph database", &["db"])
            .set_title("Open codegraph.db")
            .pick_file()
        {
            let (tx, rx) = channel();
            let p = path.clone();
            std::thread::spawn(move || {
                let res = load_database(&p).map_err(|e| e.to_string());
                let _ = tx.send((p, res));
            });
            self.load_rx = Some(rx);
            self.loading = true;
            self.project_root = None;
            self.status = "loading database…".into();
            self.error = None;
        }
    }

    fn poll_load(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.load_rx else { return };
        match rx.try_recv() {
            Ok((path, Ok((graph, status)))) => {
                let symbols = graph.symbols().to_vec();
                self.adopt_graph(graph, symbols, status, DbSource::File, Some(path));
                self.load_rx = None;
                self.loading = false;
            }
            Ok((path, Err(e))) => {
                self.error = Some(format!("failed to open {path:?}: {e}"));
                self.status = "load failed".into();
                self.load_rx = None;
                self.loading = false;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint(),
            Err(TryRecvError::Disconnected) => {
                self.load_rx = None;
                self.loading = false;
            }
        }
    }

    fn open_project(&mut self) {
        let Some(dir) = rfd::FileDialog::new()
            .set_title("Open Project folder")
            .pick_folder()
        else {
            return;
        };
        self.adopt_project(dir);
    }

    /// Load a project (config, cache, watch, index) and show it immediately.
    fn adopt_project(&mut self, dir: PathBuf) {
        let config_path = ProjectConfig::config_path(&dir);
        let config = ProjectConfig::load(&config_path).unwrap_or_default();
        self.config = config;
        self.config_path = Some(config_path);
        self.project_root = Some(dir.clone());
        self.selected = None;
        self.error = None;

        // Show the cached graph immediately, then refresh in the background.
        let mut had_cache = false;
        if let Some(graph) = load_cached_graph(&dir) {
            let symbols = graph.symbols().to_vec();
            let status = format!(
                "showing cached index · {} symbols",
                fmt_num(graph.symbol_count())
            );
            self.adopt_graph(graph, symbols, status, DbSource::File, Some(dir.clone()));
            had_cache = true;
        }

        let mut settings = Settings::load_or_default();
        settings.remember_project(dir.clone());
        let _ = settings.save();

        self.baseline = Baseline::load(&Baseline::path_for(&dir));
        self.start_indexing(dir.clone(), had_cache);
        self.start_watching(&dir);
    }

    /// Watch the project for edits, and start polling git state.
    fn start_watching(&mut self, root: &std::path::Path) {
        self.watcher = codegraph_desktop::watch::ProjectWatcher::start(root).ok();
        self.touched.clear();
        self.last_watch_event = None;
        self.repo = None;
        self.poll_repo_state(root.to_path_buf());
    }

    /// Read branch / HEAD / worktrees off the UI thread.
    fn poll_repo_state(&mut self, root: PathBuf) {
        if self.git_state_loading {
            return;
        }
        let (tx, rx) = channel();
        self.git_state_rx = Some(rx);
        self.git_state_loading = true;
        std::thread::spawn(move || {
            let info = RepoInfo {
                branch: git::repo_state(&root).and_then(|s| s.branch),
                head: git::repo_state(&root).map(|s| s.head),
                worktrees: git::worktrees(&root),
                branches: git::branches(&root),
            };
            let _ = tx.send(info);
        });
    }

    fn poll_repo(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.git_state_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(info) => {
                let head_changed =
                    self.repo.as_ref().and_then(|old| old.head.clone()) != info.head.clone();
                let first = self.repo.is_none();
                if head_changed && !first && !info.head.is_none() {
                    self.status = "branch moved — re-indexing".into();
                    if let Some(root) = self.project_root.clone() {
                        self.start_indexing(root, true);
                    }
                }
                self.repo = Some(info);
                self.git_state_loading = false;
            }
            Err(TryRecvError::Empty) => {
                self.git_state_rx = Some(rx);
                ctx.request_repaint_after(std::time::Duration::from_millis(400));
            }
            Err(TryRecvError::Disconnected) => self.git_state_loading = false,
        }
    }

    /// Drain watcher events, mark files as touched, and re-index after a pause.
    fn poll_watch(&mut self, ctx: &egui::Context) {
        if !self.watch_enabled {
            return;
        }
        let Some(watcher) = &self.watcher else {
            return;
        };
        let changed = watcher.try_drain();
        if !changed.is_empty() {
            let now = std::time::Instant::now();
            for path in changed {
                if !is_source_path(&path) {
                    continue;
                }
                self.touched.retain(|(p, _)| p != &path);
                self.touched.insert(0, (path, now));
            }
            self.touched.truncate(50);
            self.last_watch_event = Some(now);
        }

        // Drop highlights after a while so the canvas settles down.
        self.touched
            .retain(|(_, at)| at.elapsed() < std::time::Duration::from_secs(20));

        // Re-index once typing pauses for 600 ms.
        let due = self
            .last_watch_event
            .map(|t| t.elapsed() >= std::time::Duration::from_millis(600))
            .unwrap_or(false);
        if due && !self.indexing {
            let root = self.project_root.clone();
            if let Some(root) = root {
                self.last_watch_event = None;
                self.start_indexing(root, true);
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn touched_set(&self) -> codegraph_desktop::ui::Touched {
        codegraph_desktop::ui::Touched {
            paths: self.touched.iter().map(|(p, _)| p.clone()).collect(),
        }
    }

    /// Copy text and show a short acknowledgement.
    fn copy(&mut self, ctx: &egui::Context, what: &str, text: String) {
        ctx.copy_text(text);
        self.copied = Some((what.to_string(), std::time::Instant::now()));
    }

    fn copy_ack(&self, ui: &mut egui::Ui, what: &str) {
        if let Some((label, at)) = &self.copied {
            if label == what && at.elapsed() < std::time::Duration::from_secs(2) {
                chip(ui, "copied", OK);
            }
        }
    }

    /// Spawn the indexing worker for `root`. With `cached` the graph on screen
    /// stays visible while the refresh runs.
    fn start_indexing(&mut self, root: PathBuf, cached: bool) {
        let config = self.config.clone();
        let cache_path = cache_path_for(&root);
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let cancel_for_worker = cancel.clone();
        std::thread::spawn(move || {
            let progress = {
                let tx = tx.clone();
                let (wrapped_tx, wrapped_rx) = channel();
                std::thread::spawn(move || {
                    while let Ok(ev) = wrapped_rx.recv() {
                        if tx.send(IndexEvent::Progress(ev)).is_err() {
                            break;
                        }
                    }
                });
                Some(wrapped_tx)
            };
            let result = index_project_cached(
                &root,
                &config,
                progress,
                cancel_for_worker,
                Some(&cache_path),
            )
            .map(Box::new)
            .map_err(|e| e.to_string());
            let _ = tx.send(IndexEvent::Finished(result));
        });
        self.index_rx = Some(rx);
        self.index_cancel = Some(cancel);
        self.indexing = true;
        self.index_progress = "walking…".into();
        if !cached {
            self.status = "indexing project…".into();
        }
    }

    fn poll_index(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.index_rx.take() else {
            return;
        };
        let mut keep = true;
        loop {
            match rx.try_recv() {
                Ok(IndexEvent::Progress(p)) => {
                    self.index_progress = progress_text(&p);
                }
                Ok(IndexEvent::Finished(Ok(outcome))) => {
                    let summary = outcome.summary.clone();
                    let symbols = outcome.graph.symbols().to_vec();
                    let status = format!(
                        "indexed {} files · {} symbols · {} edges · {:.0}% refs resolved · {} ms",
                        summary.files,
                        summary.symbols,
                        summary.edges,
                        summary.resolved_ratio() * 100.0,
                        summary.duration_ms
                    );
                    self.adopt_graph(
                        outcome.graph,
                        symbols,
                        status,
                        DbSource::File,
                        self.project_root.clone(),
                    );
                    self.indexing = false;
                    self.index_progress.clear();
                    self.index_cancel = None;
                    keep = false;
                    break;
                }
                Ok(IndexEvent::Finished(Err(e))) => {
                    self.error = Some(format!("index failed: {e}"));
                    self.status = "index failed".into();
                    self.indexing = false;
                    self.index_progress.clear();
                    self.index_cancel = None;
                    keep = false;
                    break;
                }
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint();
                    break;
                }
                Err(TryRecvError::Disconnected) => {
                    self.indexing = false;
                    keep = false;
                    break;
                }
            }
        }
        if keep {
            self.index_rx = Some(rx);
        }
        // Keep frames coming while work is in flight: a finished worker can
        // otherwise leave the last "done · N files" frame on screen until the
        // user moves the mouse.
        if self.indexing {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn cancel_index(&mut self) {
        if let Some(cancel) = &self.index_cancel {
            cancel.store(true, Ordering::Relaxed);
            self.index_progress = "cancelling…".into();
        }
    }

    fn modules_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Modules").size(14.0).strong().color(TEXT));
            ui.label(
                RichText::new("coupling and cohesion per module")
                    .size(11.0)
                    .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(RichText::new("Edit modules…").size(12.0)))
                    .clicked()
                {
                    self.open_editor();
                }
                let table = metrics_text(self.metrics.as_ref());
                if copy_button(ui, &table, "copy table") {
                    self.copy(ui.ctx(), "modules", table.clone());
                }
                self.copy_ack(ui, "modules");
                let unassigned = self.unassigned_files();
                if unassigned > 0 {
                    chip(ui, &format!("{unassigned} unassigned"), WARN);
                } else {
                    chip(ui, "all files assigned", OK);
                }
            });
        });

        let Some(metrics) = self.metrics.as_ref() else {
            return;
        };
        let mut modules: Vec<_> = metrics
            .modules
            .iter()
            .filter(|m| m.file_count > 0)
            .collect();
        modules.sort_by(|a, b| b.ce.cmp(&a.ce).then_with(|| a.unit.cmp(&b.unit)));
        let shown = modules.len().min(40);

        ui.add_space(2.0);
        ui.label(
            RichText::new(
                "I = instability (0 stable · 1 unstable) · A = abstractness ·                  D = distance from the main sequence · cohesion = largest group share",
            )
            .size(10.5)
            .color(MUTED),
        );
        ui.add_space(4.0);

        let mut clicked = None;
        card(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("modules_table")
                        .num_columns(8)
                        .spacing([14.0, 3.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for h in ["module", "files", "Ca", "Ce", "I", "A", "cohesion", "cx"] {
                                column_header(ui, h, false);
                            }
                            ui.end_row();

                            for m in modules.iter().take(shown) {
                                let resp = ui.add(
                                    egui::Label::new(
                                        RichText::new(truncate(&m.unit, 34)).size(12.0).color(TEXT),
                                    )
                                    .sense(egui::Sense::click()),
                                );
                                if resp.clicked() {
                                    clicked = Some(m.unit.clone());
                                }
                                ui.label(
                                    RichText::new(m.file_count.to_string())
                                        .monospace()
                                        .size(11.0)
                                        .color(TEXT_DIM),
                                );
                                ui.label(
                                    RichText::new(m.ca.to_string())
                                        .monospace()
                                        .size(11.0)
                                        .color(TEXT_DIM),
                                );
                                ui.label(
                                    RichText::new(m.ce.to_string())
                                        .monospace()
                                        .size(11.0)
                                        .color(if m.ce > 8 { WARN } else { TEXT_DIM }),
                                );
                                let i = m
                                    .instability
                                    .map(|v| format!("{v:.2}"))
                                    .unwrap_or_else(|| "–".into());
                                ui.label(RichText::new(i).monospace().size(11.0).color(TEXT_DIM));
                                let a = m
                                    .abstractness
                                    .map(|v| format!("{v:.2}"))
                                    .unwrap_or_else(|| "–".into());
                                ui.label(RichText::new(a).monospace().size(11.0).color(TEXT_DIM));
                                let coh = format!(
                                    "{}/{} · {:.2}",
                                    m.cohesion_components, m.file_count, m.cohesion_largest_ratio
                                );
                                ui.label(RichText::new(coh).monospace().size(11.0).color(
                                    if m.cohesion_largest_ratio < 0.5 {
                                        WARN
                                    } else {
                                        TEXT_DIM
                                    },
                                ));
                                ui.label(
                                    RichText::new(m.complexity_sum.to_string())
                                        .monospace()
                                        .size(11.0)
                                        .color(TEXT_DIM),
                                );
                                ui.end_row();
                            }
                        });
                });
        });
        if modules.len() > shown {
            ui.label(
                RichText::new(format!(
                    "showing the {shown} most coupled of {} modules",
                    modules.len()
                ))
                .size(10.5)
                .color(MUTED),
            );
        }
        if let Some(unit) = clicked {
            self.show_module_files(&unit);
        }
    }

    /// Open the module editor with the current config as pretty JSON.
    fn open_editor(&mut self) {
        self.editor_text =
            serde_json::to_string_pretty(&self.config).unwrap_or_else(|_| "{}".to_string());
        self.editor_error = None;
        self.editor_open = true;
    }

    /// Seed `modules` from the project's top-level folders.
    fn generate_starter_modules(&mut self) {
        let Some(root) = &self.project_root else {
            return;
        };
        let mut folders: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if matches!(
                        name.as_str(),
                        "node_modules" | "target" | ".git" | "dist" | "build" | ".next"
                    ) {
                        continue;
                    }
                    folders.push(name);
                }
            }
        }
        folders.sort();
        folders.truncate(24);
        let modules = module_defs::starter_modules(&folders);
        self.config.modules = modules;
        self.editor_text =
            serde_json::to_string_pretty(&self.config).unwrap_or_else(|_| "{}".to_string());
        self.editor_error = None;
    }

    fn save_editor(&mut self) {
        let parsed = match ProjectConfig::parse(&self.editor_text) {
            Ok(cfg) => cfg,
            Err(err) => {
                self.editor_error = Some(err.to_string());
                return;
            }
        };
        let path = match &self.config_path {
            Some(p) => p.clone(),
            None => {
                self.editor_error = Some("no project is open".into());
                return;
            }
        };
        if let Err(err) = parsed.save(&path) {
            self.editor_error = Some(err.to_string());
            return;
        }
        self.config = parsed;
        self.editor_error = None;
        self.rebuild_all();
    }

    fn module_editor_window(&mut self, ctx: &egui::Context) {
        if !self.editor_open {
            return;
        }
        let mut open = self.editor_open;
        egui::Window::new("Modules")
            .open(&mut open)
            .resizable(true)
            .default_size([620.0, 460.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Define modules with glob patterns. First match wins; files that match                          nothing go to `Unassigned`. `allowed_deps` turns a dependency into a                          layer violation.",
                    )
                    .monospace()
                    .size(10.0)
                    .color(MUTED),
                );
                ui.horizontal(|ui| {
                    if ui.button("Generate from folders").clicked() {
                        self.generate_starter_modules();
                    }
                    if ui.button("Validate").clicked() {
                        self.editor_error = match ProjectConfig::parse(&self.editor_text) {
                            Ok(_) => Some("valid".to_string()),
                            Err(e) => Some(e.to_string()),
                        };
                    }
                    if ui.button("Save & re-analyze").clicked() {
                        self.save_editor();
                    }
                    if ui.button("Reload from disk").clicked() {
                        if let Some(path) = &self.config_path {
                            match ProjectConfig::load(path) {
                                Ok(cfg) => {
                                    self.config = cfg;
                                    self.editor_text = serde_json::to_string_pretty(&self.config)
                                        .unwrap_or_else(|_| "{}".to_string());
                                    self.editor_error = None;
                                    self.rebuild_all();
                                }
                                Err(e) => self.editor_error = Some(e.to_string()),
                            }
                        }
                    }
                });
                if let Some(err) = &self.editor_error {
                    let color = if err == "valid" {
                        Color32::from_rgb(120, 200, 120)
                    } else {
                        ACCENT
                    };
                    ui.label(RichText::new(err).monospace().size(10.0).color(color));
                }
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.editor_text)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(18),
                        );
                    });
                ui.separator();
                ui.label(
                    RichText::new("Live preview")
                        .monospace()
                        .strong()
                        .size(10.0)
                        .color(TEXT),
                );
                if let Ok(cfg) = ProjectConfig::parse(&self.editor_text) {
                    let assigner = ModuleAssigner::new(&cfg.modules);
                    let mut counts: std::collections::BTreeMap<String, usize> =
                        std::collections::BTreeMap::new();
                    for file in self.graph.files().keys() {
                        *counts.entry(assigner.module_of(file)).or_default() += 1;
                    }
                    for (module, count) in counts.iter().take(20) {
                        ui.label(
                            RichText::new(format!("  {module}: {count} file(s)"))
                                .monospace()
                                .size(10.0)
                                .color(MUTED),
                        );
                    }
                    let overlaps = assigner.overlaps(
                        &self.graph.files().keys().cloned().collect::<Vec<_>>(),
                    );
                    if !overlaps.is_empty() {
                        ui.label(
                            RichText::new(format!(
                                "  ⚠ {} file(s) match more than one module",
                                overlaps.len()
                            ))
                            .monospace()
                            .size(10.0)
                            .color(Color32::from_rgb(230, 170, 30)),
                        );
                    }
                }
            });
        self.editor_open = open;
    }

    fn unassigned_files(&self) -> usize {
        self.graph
            .files()
            .keys()
            .filter(|f| self.assigner.module_of(f) == "Unassigned")
            .count()
    }

    fn cycles_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Cycles").size(14.0).strong().color(TEXT));
            ui.label(
                RichText::new("each group with the edges that break it")
                    .size(11.0)
                    .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.cycles.is_empty() {
                    chip(ui, "acyclic", OK);
                } else {
                    chip(ui, &format!("{} groups", self.cycles.len()), WARN);
                }
            });
        });

        if self.cycles.is_empty() {
            card(ui, |ui| {
                ui.label(
                    RichText::new("No circular dependencies.")
                        .size(12.5)
                        .color(OK),
                );
                ui.label(
                    RichText::new(
                        "Dependencies flow in one direction, so a change cannot                          boomerang through a loop.",
                    )
                    .size(11.0)
                    .color(MUTED),
                );
            });
            return;
        }

        let mut clicked = None;
        let mut to_copy: Option<(&str, String)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for (idx, group) in self.cycles.iter().take(40).enumerate() {
                    card(ui, |ui| {
                        ui.horizontal(|ui| {
                            chip(
                                ui,
                                &format!("GROUP {}", idx + 1),
                                if group.files.len() >= 10 {
                                    DANGER
                                } else {
                                    WARN
                                },
                            );
                            ui.label(
                                RichText::new(format!("{} files", group.files.len()))
                                    .size(11.5)
                                    .color(TEXT_DIM),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if let Some(first) = group.files.first() {
                                        if ui.small_button("open").clicked() {
                                            clicked = Some(first.clone());
                                        }
                                    }
                                    if copy_button(
                                        ui,
                                        &cycle_text(&group.files, &group.cuts),
                                        "copy",
                                    ) {
                                        to_copy =
                                            Some(("cycle", cycle_text(&group.files, &group.cuts)));
                                    }
                                },
                            );
                        });
                        ui.add_space(2.0);
                        for file in group.files.iter().take(5) {
                            let r = ui.add(
                                egui::Label::new(
                                    RichText::new(format!("  {file}"))
                                        .monospace()
                                        .size(10.5)
                                        .color(TEXT_DIM),
                                )
                                .sense(egui::Sense::click()),
                            );
                            if r.clicked() {
                                clicked = Some(file.clone());
                            }
                        }
                        if group.files.len() > 5 {
                            ui.label(
                                RichText::new(format!("  +{} more", group.files.len() - 5))
                                    .size(10.5)
                                    .color(MUTED),
                            );
                        }
                        ui.add_space(4.0);
                        if group.cuts.is_empty() {
                            ui.label(
                                RichText::new(
                                    "All edges in this group are low-confidence guesses. \
                                     Confirm the dependency by hand before cutting anything.",
                                )
                                .size(10.5)
                                .color(MUTED),
                            );
                        } else {
                            ui.label(
                                RichText::new("CUT THESE (cheapest first)")
                                    .size(9.5)
                                    .strong()
                                    .color(MUTED),
                            );
                            for cut in group.cuts.iter().take(3) {
                                ui.label(
                                    RichText::new(format!(
                                        "  {} → {}   weight {}",
                                        cut.from, cut.to, cut.weight
                                    ))
                                    .monospace()
                                    .size(10.5)
                                    .color(OK),
                                );
                            }
                        }
                    });
                }
            });
        if let Some(file) = clicked {
            self.show_file(&file);
        }
        if let Some((what, text)) = to_copy {
            self.copy(ui.ctx(), what, text);
        }
    }

    fn warnings_panel(&mut self, ui: &mut egui::Ui) {
        let errors = self
            .warnings
            .iter()
            .filter(|w| w.severity == Severity::Error)
            .count();
        let warns = self
            .warnings
            .iter()
            .filter(|w| w.severity == Severity::Warn)
            .count();
        let infos = self.warnings.len() - errors - warns;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Warnings").size(14.0).strong().color(TEXT));
            ui.label(
                RichText::new("click a rule to see the reasoning")
                    .size(11.0)
                    .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if copy_button(ui, &warnings_text(&self.warnings), "copy all") {
                    self.copy(ui.ctx(), "warnings", warnings_text(&self.warnings));
                }
                self.copy_ack(ui, "warnings");
                ui.add(
                    egui::TextEdit::singleline(&mut self.warning_filter)
                        .hint_text("filter…")
                        .desired_width(180.0)
                        .margin(egui::Margin::symmetric(8, 5)),
                );
                filter_toggle(ui, &mut self.show_info, "notes", infos, MUTED);
                filter_toggle(ui, &mut self.show_warn, "warn", warns, WARN);
                filter_toggle(ui, &mut self.show_error, "error", errors, DANGER);
            });
        });

        let query = self.warning_filter.to_lowercase();
        let visible: Vec<Warning> = self
            .warnings
            .iter()
            .filter(|w| match w.severity {
                Severity::Error => self.show_error,
                Severity::Warn => self.show_warn,
                Severity::Info => self.show_info,
            })
            .filter(|w| {
                query.is_empty()
                    || w.message.to_lowercase().contains(&query)
                    || w.rule.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();

        if visible.is_empty() {
            card(ui, |ui| {
                ui.label(
                    RichText::new("Nothing to show with these filters.")
                        .size(12.0)
                        .color(TEXT_DIM),
                );
            });
            return;
        }

        let mut clicked = None;
        let mut to_copy: Option<(&str, String)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for w in visible.iter().take(80) {
                    let color = severity_color(w.severity);
                    card(ui, |ui| {
                        ui.horizontal(|ui| {
                            chip(ui, w.severity.label(), color);
                            ui.label(RichText::new(&w.rule).monospace().size(10.5).color(MUTED));
                            if w.count > 1 {
                                chip(ui, &format!("×{}", w.count), color);
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if let Some(subject) = w.subjects.first() {
                                        if ui.small_button("focus").clicked() {
                                            clicked = Some(subject.clone());
                                        }
                                    }
                                    if copy_button(ui, &warning_text(w), "copy") {
                                        to_copy = Some(("warning", warning_text(w)));
                                    }
                                },
                            );
                        });
                        ui.label(RichText::new(&w.message).size(12.5).color(TEXT));
                        if let Some(loc) = w.locations.first() {
                            let resp = ui.add(
                                egui::Label::new(
                                    RichText::new(loc.display())
                                        .monospace()
                                        .size(10.5)
                                        .color(ACCENT),
                                )
                                .sense(egui::Sense::click()),
                            );
                            if resp.clicked() {
                                clicked = Some(loc.file.clone());
                            }
                        }
                        ui.collapsing(
                            RichText::new("why, and what to try")
                                .size(10.5)
                                .color(TEXT_DIM),
                            |ui| {
                                ui.label(RichText::new(&w.explanation).size(11.0).color(TEXT_DIM));
                                ui.add_space(2.0);
                                ui.label(
                                    RichText::new(format!("→ {}", w.suggestion))
                                        .size(11.0)
                                        .color(OK),
                                );
                                if w.subjects.len() > 1 {
                                    ui.add_space(2.0);
                                    ui.label(
                                        RichText::new(w.subjects.join("  ·  "))
                                            .monospace()
                                            .size(10.0)
                                            .color(MUTED),
                                    );
                                }
                            },
                        );
                    });
                }
                if visible.len() > 80 {
                    ui.label(
                        RichText::new(format!("{} more — narrow the filter", visible.len() - 80))
                            .size(10.5)
                            .color(MUTED),
                    );
                }
            });
        if let Some(subject) = clicked {
            self.focus_subject(&subject);
        }
        if let Some((what, text)) = to_copy {
            self.copy(ui.ctx(), what, text);
        }
    }

    fn hotspots_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Hotspots").size(14.0).strong().color(TEXT));
            ui.label(
                RichText::new("files that change often · last 180 days")
                    .size(11.0)
                    .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.git_loading {
                    ui.add(egui::Spinner::new().size(12.0));
                    ui.label(
                        RichText::new("reading git history…")
                            .size(11.0)
                            .color(MUTED),
                    );
                }
            });
        });
        if let Some(note) = &self.git_note {
            card(ui, |ui| {
                ui.label(
                    RichText::new(format!("Git unavailable: {note}"))
                        .size(11.0)
                        .color(TEXT_DIM),
                );
            });
        }
        if self.churn.is_empty() && !self.git_loading {
            card(ui, |ui| {
                ui.label(
                    RichText::new("No git history for this project.")
                        .size(12.0)
                        .color(TEXT_DIM),
                );
                ui.label(
                    RichText::new("Hotspots need a git repository with commits in the window.")
                        .size(11.0)
                        .color(MUTED),
                );
            });
            return;
        }

        let mut clicked = None;
        card(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("hotspots_table")
                        .num_columns(4)
                        .spacing([18.0, 3.0])
                        .striped(true)
                        .show(ui, |ui| {
                            column_header(ui, "file", false);
                            column_header(ui, "commits", false);
                            column_header(ui, "added", false);
                            column_header(ui, "deleted", false);
                            ui.end_row();
                            for c in self.churn.iter().take(60) {
                                let r = ui.add(
                                    egui::Label::new(
                                        RichText::new(truncate(&c.file, 46)).size(11.5).color(TEXT),
                                    )
                                    .sense(egui::Sense::click()),
                                );
                                if r.clicked() {
                                    clicked = Some(c.file.clone());
                                }
                                ui.label(
                                    RichText::new(c.commits.to_string())
                                        .monospace()
                                        .size(11.0)
                                        .color(TEXT_DIM),
                                );
                                ui.label(
                                    RichText::new(format!("+{}", c.added))
                                        .monospace()
                                        .size(11.0)
                                        .color(OK),
                                );
                                ui.label(
                                    RichText::new(format!("-{}", c.deleted))
                                        .monospace()
                                        .size(11.0)
                                        .color(DANGER),
                                );
                                ui.end_row();
                            }
                        });
                });
        });
        if let Some(file) = clicked {
            self.show_file(&file);
        }
    }

    fn hidden_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Hidden coupling")
                    .size(14.0)
                    .strong()
                    .color(TEXT),
            );
            ui.label(
                RichText::new("files that change together with no dependency")
                    .size(11.0)
                    .color(MUTED),
            );
        });
        if self.cochange.is_empty() {
            card(ui, |ui| {
                ui.label(
                    RichText::new("No co-change pairs found.")
                        .size(12.0)
                        .color(TEXT_DIM),
                );
                ui.label(
                    RichText::new("Pairs need at least 5 shared commits and 60% confidence.")
                        .size(11.0)
                        .color(MUTED),
                );
            });
            return;
        }
        let cross = warnings::cross_edges(&self.graph, &self.config);
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let mut shown = 0;
                for pair in &self.cochange {
                    if shown >= 40 {
                        break;
                    }
                    let direct = cross.contains_key(&(pair.a.clone(), pair.b.clone()))
                        || cross.contains_key(&(pair.b.clone(), pair.a.clone()));
                    if direct {
                        continue;
                    }
                    shown += 1;
                    card(ui, |ui| {
                        ui.horizontal(|ui| {
                            chip(ui, "HIDDEN", WARN);
                            chip(ui, &format!("support {}", pair.support), TEXT_DIM);
                            chip(
                                ui,
                                &format!("confidence {:.0}%", pair.confidence * 100.0),
                                TEXT_DIM,
                            );
                        });
                        ui.label(
                            RichText::new(format!("{}  <->  {}", pair.a, pair.b))
                                .monospace()
                                .size(11.0)
                                .color(TEXT),
                        );
                        ui.label(
                            RichText::new(
                                "They change together but nothing links them. Look for a shared rule or a missing abstraction.",
                            )
                            .size(10.5)
                            .color(MUTED),
                        );
                        if ui.small_button("open first file").clicked() {
                            clicked = Some(pair.a.clone());
                        }
                    });
                }
                if shown == 0 {
                    ui.label(
                        RichText::new(
                            "Every frequently co-changing pair also has a static edge.",
                        )
                        .size(11.0)
                        .color(OK),
                    );
                }
            });
        if let Some(file) = clicked {
            self.show_file(&file);
        }
    }

    fn impact_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Impact").size(14.0).strong().color(TEXT));
            ui.label(
                RichText::new("what breaks if this changes")
                    .size(11.0)
                    .color(MUTED),
            );
        });
        let Some(id) = self.selected.clone() else {
            card(ui, |ui| {
                ui.label(
                    RichText::new("Select a symbol first.")
                        .size(12.0)
                        .color(TEXT_DIM),
                );
                ui.label(
                    RichText::new("Pick one in the left list or click a node in the graph.")
                        .size(11.0)
                        .color(MUTED),
                );
            });
            return;
        };
        let kinds = [
            RelationshipKind::Calls,
            RelationshipKind::Imports,
            RelationshipKind::TypeRef,
            RelationshipKind::Extends,
            RelationshipKind::Implements,
        ];
        let rev = impact::reverse_closure(&self.graph, &id, &kinds, 0.5, 6, 5000);
        let fwd = impact::forward_closure(&self.graph, &id, &kinds, 0.5, 6, 5000);
        let name = self
            .graph
            .symbol(id.clone())
            .map(|s| format!("{}  ({}:{})", s.name, s.file, s.line.unwrap_or(0)))
            .unwrap_or_default();
        let summary = impact_text(
            &name,
            &rev.by_depth
                .iter()
                .map(|(d, c)| (*d, *c))
                .collect::<Vec<_>>(),
            &fwd.by_depth
                .iter()
                .map(|(d, c)| (*d, *c))
                .collect::<Vec<_>>(),
        );

        ui.horizontal(|ui| {
            if copy_button(ui, &summary, "copy impact") {
                self.copy(ui.ctx(), "impact", summary.clone());
            }
            self.copy_ack(ui, "impact");
        });
        ui.horizontal_wrapped(|ui| {
            stat_tile(ui, &rev.nodes.len().to_string(), "dependents", ACCENT);
            stat_tile(ui, &fwd.nodes.len().to_string(), "dependencies", TEXT);
            stat_tile(ui, &rev.by_depth.len().to_string(), "levels deep", TEXT_DIM);
        });
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            for (title, data) in [("DEPENDENTS", &rev), ("DEPENDENCIES", &fwd)] {
                ui.vertical(|ui| {
                    section_title(ui, title, "");
                    card(ui, |ui| {
                        if data.by_depth.is_empty() {
                            ui.label(RichText::new("none").size(11.5).color(TEXT_DIM));
                        }
                        for (depth, count) in &data.by_depth {
                            ui.horizontal(|ui| {
                                chip(ui, &format!("depth {depth}"), TEXT_DIM);
                                ui.label(
                                    RichText::new(format!("{count} node(s)"))
                                        .size(11.0)
                                        .color(TEXT),
                                );
                            });
                        }
                    });
                });
            }
        });
        if rev.truncated || fwd.truncated {
            ui.label(
                RichText::new("Truncated at 5000 nodes.")
                    .size(10.5)
                    .color(MUTED),
            );
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            // Brand mark: a small accent square + wordmark.
            let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(6),
                ACCENT.linear_multiply(0.9),
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "C",
                egui::FontId::proportional(12.0),
                Color32::BLACK,
            );
            ui.add_space(2.0);
            ui.label(RichText::new("CodeGraph").size(17.0).strong().color(TEXT));
            chip(
                ui,
                match self.db_source {
                    DbSource::Demo => "DEMO",
                    DbSource::File => "PROJECT",
                },
                if matches!(self.db_source, DbSource::Demo) {
                    WARN
                } else {
                    OK
                },
            );

            ui.add_space(10.0);
            ui.label(RichText::new(&self.status).size(11.5).color(TEXT_DIM));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let primary = egui::Button::new(
                    RichText::new("Open project")
                        .size(12.5)
                        .color(Color32::BLACK)
                        .strong(),
                )
                .fill(ACCENT)
                .corner_radius(egui::CornerRadius::same(8));
                if ui
                    .add(primary)
                    .on_hover_text("Index a folder (Ctrl+O)")
                    .clicked()
                {
                    self.open_project();
                }
                if ui
                    .add(egui::Button::new(RichText::new("Open .db").size(12.0)))
                    .clicked()
                {
                    self.open_dialog();
                }
                if ui
                    .add(egui::Button::new(RichText::new("Demo").size(12.0)))
                    .clicked()
                {
                    self.use_demo();
                }
                ui.separator();
                if ui
                    .add(egui::Button::new(RichText::new("Overview").size(12.0)))
                    .clicked()
                {
                    self.show_overview();
                }
                if self.indexing {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(DANGER))
                                .fill(CARD),
                        )
                        .clicked()
                    {
                        self.cancel_index();
                    }
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label(
                        RichText::new(&self.index_progress)
                            .size(11.0)
                            .color(TEXT_DIM),
                    );
                }
            });
        });

        let path = self
            .project_root
            .as_ref()
            .or(self.db_path.as_ref())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "no project open".to_string());
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            ui.label(RichText::new(path).size(10.5).color(MUTED));
            if let Some(repo) = self.repo.clone() {
                ui.label(RichText::new("·").size(10.5).color(MUTED));
                chip(ui, repo.branch.as_deref().unwrap_or("detached"), ACCENT);
                if let Some(head) = &repo.head {
                    ui.label(
                        RichText::new(&head[..head.len().min(8)])
                            .monospace()
                            .size(10.0)
                            .color(MUTED),
                    );
                }
                let label = format!(
                    "worktrees {}{}",
                    repo.worktrees.len(),
                    if repo.branches.len() > 1 {
                        format!(" · branches {}", repo.branches.len())
                    } else {
                        String::new()
                    }
                );
                ui.menu_button(RichText::new(label).size(10.5), |ui| {
                    ui.label(RichText::new("WORKTREES").size(9.5).strong().color(MUTED));
                    let current = self
                        .project_root
                        .as_ref()
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    let mut switch: Option<PathBuf> = None;
                    for tree in &repo.worktrees {
                        let normalized = tree.path.replace('\\', "/");
                        let selected = normalized.eq_ignore_ascii_case(&current);
                        let text = format!(
                            "{}  {}",
                            tree.branch.as_deref().unwrap_or("detached"),
                            truncate(&normalized, 52)
                        );
                        if ui
                            .selectable_label(selected, RichText::new(text).size(11.0))
                            .clicked()
                        {
                            switch = Some(PathBuf::from(&tree.path));
                        }
                    }
                    ui.separator();
                    ui.label(
                        RichText::new("BRANCHES (read-only)")
                            .size(9.5)
                            .strong()
                            .color(MUTED),
                    );
                    for branch in repo.branches.iter().take(12) {
                        let selected = repo.branch.as_deref() == Some(branch.as_str());
                        ui.label(
                            RichText::new(branch)
                                .monospace()
                                .size(11.0)
                                .color(if selected { ACCENT } else { TEXT_DIM }),
                        );
                    }
                    ui.separator();
                    if ui.button("Refresh git state").clicked() {
                        if let Some(root) = self.project_root.clone() {
                            self.poll_repo_state(root);
                        }
                    }
                    if let Some(dir) = switch {
                        ui.close();
                        self.adopt_project(dir);
                    }
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let live = self.touched.len();
                if live > 0 {
                    chip(ui, &format!("● {live} touched"), WARN);
                }
                if self.project_root.is_none() {
                    return;
                }
                let label = if self.watch_enabled {
                    "watching"
                } else {
                    "watch off"
                };
                let color = if self.watch_enabled { OK } else { MUTED };
                if ui
                    .add(
                        egui::Button::new(RichText::new(label).size(10.5).color(color))
                            .fill(Color32::TRANSPARENT),
                    )
                    .on_hover_text("Watch the project for edits and re-index automatically")
                    .clicked()
                {
                    self.watch_enabled = !self.watch_enabled;
                    if self.watch_enabled {
                        let root = self
                            .project_root
                            .clone()
                            .or(self.db_path.clone())
                            .unwrap_or_default();
                        self.start_watching(&root);
                    } else {
                        self.watcher = None;
                    }
                }
            });
        });
        if let Some(err) = &self.error {
            ui.add_space(2.0);
            card(ui, |ui| {
                ui.label(RichText::new(err).size(11.5).color(DANGER));
            });
        }
        ui.add_space(6.0);
        let sep = ui.available_rect_before_wrap();
        ui.painter()
            .hline(sep.x_range(), sep.top(), egui::Stroke::new(1.0, HAIRLINE));
        ui.add_space(4.0);
    }

    fn symbol_list(&mut self, ui: &mut egui::Ui) -> Option<SymbolId> {
        ui.add_space(2.0);
        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search symbols, files, signatures…")
                .desired_width(f32::INFINITY)
                .margin(egui::Margin::symmetric(10, 7)),
        );
        if resp.changed() {
            self.refresh_symbols();
        }
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} matching", fmt_num(self.symbols.len())))
                    .size(10.5)
                    .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.search.is_empty() && ui.small_button("clear").clicked() {
                    self.search.clear();
                    self.refresh_symbols();
                }
            });
        });
        ui.add_space(2.0);

        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let touched = self.touched_set();
                let mut ordered: Vec<&codegraph_desktop::model::Symbol> =
                    self.symbols.iter().collect();
                ordered.sort_by_key(|s| !touched.contains_file(&s.file));
                for sym in ordered.into_iter().take(MAX_LIST) {
                    let selected = self.selected.as_ref() == Some(&sym.id);
                    let live = touched.contains_file(&sym.file);
                    let subtitle = match sym.line {
                        Some(line) => format!("{}:{line}", truncate(&sym.file, 34)),
                        None => truncate(&sym.file, 40),
                    };
                    // Files are stored as `Module` symbols; show their language
                    // extension instead of a generic "mod" badge.
                    let (badge, color) =
                        if matches!(sym.kind, codegraph_desktop::model::SymbolKind::Module) {
                            let ext = sym.file.rsplit('.').next().unwrap_or("file").to_uppercase();
                            (ext, kind_color(&sym.kind))
                        } else {
                            (sym.kind.short().to_string(), kind_color(&sym.kind))
                        };
                    if list_row(ui, selected, live, &badge, color, &sym.name, &subtitle) {
                        clicked = Some(sym.id.clone());
                    }
                }
                if self.symbols.len() > MAX_LIST {
                    ui.label(
                        RichText::new(format!(
                            "{} more — narrow the search",
                            self.symbols.len() - MAX_LIST
                        ))
                        .size(10.5)
                        .color(MUTED),
                    );
                }
                if self.symbols.is_empty() {
                    ui.add_space(16.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Nothing matches").size(13.0).color(TEXT_DIM));
                        ui.label(
                            RichText::new("Try a shorter query, or open a project.")
                                .size(11.0)
                                .color(MUTED),
                        );
                    });
                }
            });
        clicked
    }

    fn audit_panel(&mut self, ui: &mut egui::Ui) {
        let Some(a) = self.analysis.clone() else {
            return;
        };
        let errors = self
            .warnings
            .iter()
            .filter(|w| w.severity == Severity::Error)
            .count();
        let warns = self
            .warnings
            .iter()
            .filter(|w| w.severity == Severity::Warn)
            .count();
        let infos = self.warnings.len() - errors - warns;
        let files = self.graph.files().len();

        let context = context_text(
            &self
                .project_root
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "demo".into()),
            self.repo.as_ref().and_then(|r| r.branch.as_deref()),
            self.repo.as_ref().and_then(|r| r.head.as_deref()),
            (
                files,
                self.graph.symbol_count(),
                self.graph.relationship_count(),
            ),
        );
        ui.horizontal(|ui| {
            if copy_button(ui, &context, "copy summary") {
                self.copy(ui.ctx(), "summary", context.clone());
            }
            if copy_button(ui, &warnings_text(&self.warnings), "copy warnings") {
                self.copy(ui.ctx(), "summary", warnings_text(&self.warnings));
            }
            self.copy_ack(ui, "summary");
        });
        ui.horizontal_wrapped(|ui| {
            stat_tile(ui, &fmt_num(files), "files indexed", TEXT);
            stat_tile(ui, &fmt_num(self.graph.symbol_count()), "symbols", TEXT);
            stat_tile(
                ui,
                &fmt_num(self.cycles.len()),
                "cycle groups",
                if self.cycles.is_empty() { OK } else { WARN },
            );
            stat_tile(
                ui,
                &errors.to_string(),
                "errors",
                if errors == 0 { OK } else { DANGER },
            );
            stat_tile(
                ui,
                &warns.to_string(),
                "warnings",
                if warns == 0 { OK } else { WARN },
            );
            stat_tile(ui, &infos.to_string(), "notes", TEXT_DIM);
        });
        ui.add_space(6.0);

        ui.horizontal_top(|ui| {
            // Left: coupling hot spots table.
            ui.vertical(|ui| {
                ui.set_min_width(ui.available_width() * 0.58);
                section_title(ui, "MOST COUPLED FILES", "efferent coupling ≥ 2");
                card(ui, |ui| {
                    egui::Grid::new("audit_coupling")
                        .num_columns(5)
                        .spacing([16.0, 4.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for h in ["file", "Ca", "Ce", "I", "cohesion"] {
                                column_header(ui, h, false);
                            }
                            ui.end_row();
                            let mut rows: Vec<_> = a.report.iter().filter(|m| m.ce >= 2).collect();
                            rows.sort_by_key(|m| std::cmp::Reverse(m.ce));
                            for m in rows.into_iter().take(8) {
                                let base = m.file.rsplit(['/', '\\']).next().unwrap_or(&m.file);
                                ui.label(RichText::new(truncate(base, 30)).size(11.5).color(TEXT));
                                ui.label(
                                    RichText::new(m.ca.to_string())
                                        .monospace()
                                        .size(11.0)
                                        .color(TEXT_DIM),
                                );
                                ui.label(
                                    RichText::new(m.ce.to_string())
                                        .monospace()
                                        .size(11.0)
                                        .color(TEXT_DIM),
                                );
                                let i = m.instability;
                                ui.label(
                                    RichText::new(format!("{i:.2}"))
                                        .monospace()
                                        .size(11.0)
                                        .color(if i >= 0.8 { WARN } else { TEXT_DIM }),
                                );
                                ui.label(
                                    RichText::new(format!("{:.2}", m.cohesion))
                                        .monospace()
                                        .size(11.0)
                                        .color(if m.cohesion < 0.5 { WARN } else { TEXT_DIM }),
                                );
                                ui.end_row();
                            }
                        });
                });
            });

            // Right: what to do next.
            ui.vertical(|ui| {
                section_title(ui, "NEXT STEPS", "ranked by severity");
                card(ui, |ui| {
                    if self.warnings.is_empty() {
                        ui.label(
                            RichText::new("No problems found in this project.")
                                .size(11.5)
                                .color(OK),
                        );
                        return;
                    }
                    let top: Vec<(Severity, String, Option<String>)> = self
                        .warnings
                        .iter()
                        .take(5)
                        .map(|w| (w.severity, w.message.clone(), w.subjects.first().cloned()))
                        .collect();
                    for (severity, message, subject) in top {
                        ui.horizontal(|ui| {
                            chip(ui, severity.label(), severity_color(severity));
                            let label = ui.add(
                                egui::Label::new(
                                    RichText::new(truncate(&message, 54)).size(11.5).color(TEXT),
                                )
                                .sense(egui::Sense::click()),
                            );
                            if label.clicked() {
                                if let Some(subject) = subject {
                                    self.focus_subject(&subject);
                                }
                            }
                        });
                    }
                    if self.warnings.len() > 5 {
                        ui.label(
                            RichText::new(format!(
                                "+{} more in the Warnings tab",
                                self.warnings.len() - 5
                            ))
                            .size(10.5)
                            .color(MUTED),
                        );
                    }
                });
            });
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(RichText::new("Set baseline").size(12.0)))
                .clicked()
            {
                self.set_baseline();
            }
            if self.baseline.is_some() {
                ui.label(
                    RichText::new("Save again to accept the current findings as the new baseline.")
                        .size(10.5)
                        .color(MUTED),
                );
            } else {
                ui.label(
                    RichText::new("Save a baseline to see new vs fixed findings on every run.")
                        .size(10.5)
                        .color(MUTED),
                );
            }
        });

        // "What changed" — the most useful block in the report.
        if let Some(old) = &self.baseline {
            let diff = baseline::diff(old, &self.warnings);
            let mut clicked: Option<String> = None;
            ui.add_space(4.0);
            section_title(
                ui,
                "CHANGED SINCE BASELINE",
                "new findings first, then fixed",
            );
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    chip(
                        ui,
                        &format!("{} new", diff.new.len()),
                        if diff.new.is_empty() { OK } else { DANGER },
                    );
                    chip(
                        ui,
                        &format!("{} fixed", diff.resolved.len()),
                        if diff.resolved.is_empty() { MUTED } else { OK },
                    );
                    ui.label(
                        RichText::new(format!(
                            "errors {:+} · warnings {:+} · {} unchanged",
                            diff.error_delta, diff.warn_delta, diff.unchanged
                        ))
                        .size(10.5)
                        .color(MUTED),
                    );
                });
                for entry in diff.new_findings.iter().take(6) {
                    let label = match &entry.location {
                        Some(loc) => format!("{}  [{}]", entry.message, loc),
                        None => entry.message.clone(),
                    };
                    let r = ui.add(
                        egui::Label::new(
                            RichText::new(format!("NEW  {label}"))
                                .size(11.0)
                                .color(DANGER),
                        )
                        .sense(egui::Sense::click()),
                    );
                    if r.clicked() {
                        clicked = entry.location.clone();
                    }
                }
                for entry in diff.fixed_findings.iter().take(4) {
                    let label = match &entry.location {
                        Some(loc) => format!("{}  [{}]", entry.message, loc),
                        None => entry.message.clone(),
                    };
                    let r = ui.add(
                        egui::Label::new(
                            RichText::new(format!("FIXED  {label}"))
                                .size(11.0)
                                .color(OK),
                        )
                        .sense(egui::Sense::click()),
                    );
                    if r.clicked() {
                        clicked = entry.location.clone();
                    }
                }
                if diff.new_findings.len() > 6 || diff.fixed_findings.len() > 4 {
                    ui.label(
                        RichText::new("See the Warnings tab for the full list.")
                            .size(10.0)
                            .color(MUTED),
                    );
                }
            });
            if let Some(loc) = clicked {
                // `location` is `file::symbol:line`; focus the file part.
                let file = loc
                    .split("::")
                    .next()
                    .unwrap_or(&loc)
                    .split(':')
                    .next()
                    .unwrap_or(&loc);
                if self.graph.file(file).is_some() {
                    self.show_file(file);
                } else if let Some(sym) = self
                    .graph
                    .symbols()
                    .iter()
                    .find(|s| s.qualified_name == loc || loc.contains(&s.name))
                {
                    let id = sym.id.clone();
                    self.select_symbol(&id);
                }
            }
        }
    }

    fn set_baseline(&mut self) {
        let modules = self
            .metrics
            .as_ref()
            .map(|m| m.modules.as_slice())
            .unwrap_or(&[]);
        let snapshot = baseline::snapshot(&self.warnings, modules);
        if let Some(root) = &self.project_root {
            let path = Baseline::path_for(root);
            let _ = snapshot.save(&path);
        }
        self.baseline = Some(snapshot);
    }

    fn detail_panel(&mut self, ui: &mut egui::Ui) {
        let Some(id) = &self.selected else {
            card(ui, |ui| {
                ui.label(RichText::new("Nothing selected").size(12.5).color(TEXT_DIM));
                ui.label(
                    RichText::new("Pick a symbol in the list, or click a node in the graph.")
                        .size(11.0)
                        .color(MUTED),
                );
            });
            return;
        };
        let Some(sym) = self.graph.symbol(id.clone()) else {
            ui.label(RichText::new("Symbol not found").size(12.0).color(DANGER));
            return;
        };
        let out = self
            .graph
            .relationships_for(id.clone(), EdgeDirection::Outgoing)
            .len();
        let inc = self
            .graph
            .relationships_for(id.clone(), EdgeDirection::Incoming)
            .len();

        let mut to_copy: Option<String> = None;
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let is_file = matches!(sym.kind, codegraph_desktop::model::SymbolKind::Module)
                    && sym.line.is_none();
                chip(
                    ui,
                    if is_file { "file" } else { sym.kind.short() },
                    kind_color(&sym.kind),
                );
                ui.label(RichText::new(&sym.name).size(16.0).strong().color(TEXT));
                if sym.exported {
                    chip(ui, "exported", OK);
                }
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!(
                    "{}:{}",
                    sym.file,
                    sym.line.map_or("-".to_string(), |l| l.to_string())
                ))
                .monospace()
                .size(11.0)
                .color(MUTED),
            );
            if let Some(sig) = &sym.signature {
                ui.label(RichText::new(sig).monospace().size(11.0).color(TEXT_DIM));
            }
            if let Some(doc) = &sym.doc {
                ui.add_space(2.0);
                ui.label(RichText::new(doc).size(11.5).color(TEXT_DIM));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                chip(ui, &format!("out {out}"), TEXT_DIM);
                chip(ui, &format!("in {inc}"), TEXT_DIM);
                if let Some(lang) = &sym.lang {
                    chip(ui, lang, TEXT_DIM);
                }
                if copy_button(ui, &symbol_ref(&sym.file, sym.line, &sym.name), "copy ref") {
                    to_copy = Some(symbol_ref(&sym.file, sym.line, &sym.name));
                }
            });
        });
        if let Some(text) = to_copy {
            self.copy(ui.ctx(), "symbol", text);
        }
        self.copy_ack(ui, "symbol");
    }
}

impl eframe::App for CodegraphApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_load(ui.ctx());
        self.poll_index(ui.ctx());
        self.poll_git(ui.ctx());
        self.poll_watch(ui.ctx());
        self.poll_repo(ui.ctx());
        self.module_editor_window(ui.ctx());

        // Keyboard shortcuts: Ctrl+O opens a project, Esc clears selection.
        let (open_shortcut, escape) = ui.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::O),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if open_shortcut {
            self.open_project();
        }
        if escape {
            self.selected = None;
        }

        // Arrow keys walk the visible symbol list; Enter jumps to its impact.
        let (down, up, enter) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::Enter),
            )
        });
        if (down || up) && !self.symbols.is_empty() {
            let current = self
                .selected
                .as_ref()
                .and_then(|id| self.symbols.iter().position(|s| &s.id == id));
            let next = match (current, down) {
                (Some(i), true) => (i + 1).min(self.symbols.len() - 1),
                (Some(i), false) => i.saturating_sub(1),
                (None, _) => 0,
            };
            let id = self.symbols[next].id.clone();
            self.select_symbol(&id);
        }
        if enter {
            if let Some(id) = self.selected.clone() {
                self.select_symbol(&id);
                self.tab = Tab::Impact;
            }
        }

        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ui, |ui| self.header(ui));

        let mode = self.view_mode.clone();
        // Fixed height keeps the layout predictable: switching tabs never
        // resizes the panel. Each tab scrolls inside it.
        egui::Panel::bottom("audit")
            .exact_size(336.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(ui, |ui| {
                let errors = self
                    .warnings
                    .iter()
                    .filter(|w| w.severity == Severity::Error)
                    .count();
                let cycle_count = self.cycles.len();
                ui.horizontal(|ui| {
                    tab_click(ui, &mut self.tab, Tab::Audit, "Overview", None);
                    tab_click(ui, &mut self.tab, Tab::Modules, "Modules", None);
                    tab_click(
                        ui,
                        &mut self.tab,
                        Tab::Warnings,
                        "Warnings",
                        Some(self.warnings.len()),
                    );
                    tab_click(ui, &mut self.tab, Tab::Cycles, "Cycles", Some(cycle_count));
                    tab_click(ui, &mut self.tab, Tab::Impact, "Impact", None);
                    tab_click(ui, &mut self.tab, Tab::Hotspots, "Hotspots", None);
                    tab_click(ui, &mut self.tab, Tab::Hidden, "Hidden coupling", None);
                    if errors > 0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            chip(ui, &format!("{errors} errors"), DANGER);
                        });
                    }
                });
                ui.add_space(2.0);
                match self.tab {
                    Tab::Audit => {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| match mode {
                                ViewMode::Ego => self.detail_panel(ui),
                                _ => self.audit_panel(ui),
                            });
                    }
                    Tab::Modules => self.modules_panel(ui),
                    Tab::Warnings => self.warnings_panel(ui),
                    Tab::Cycles => self.cycles_panel(ui),
                    Tab::Impact => {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| self.impact_panel(ui));
                    }
                    Tab::Hotspots => self.hotspots_panel(ui),
                    Tab::Hidden => self.hidden_panel(ui),
                }
            });

        let mut clicked_list = None;
        egui::Panel::left("symbols")
            .resizable(true)
            .default_size(330.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(10, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("SYMBOLS").size(10.0).strong().color(MUTED));
                });
                clicked_list = self.symbol_list(ui);
            });
        if let Some(id) = clicked_list {
            self.select_symbol(&id);
        }

        // Breadcrumb + first-run hint above the canvas.
        egui::Panel::top("crumbs")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let at_overview = matches!(self.view_mode, ViewMode::Modules);
                    if ui
                        .add_enabled(
                            !at_overview,
                            egui::Button::new(RichText::new("Overview").size(11.5))
                                .fill(Color32::TRANSPARENT),
                        )
                        .clicked()
                    {
                        self.show_overview();
                    }
                    if let ViewMode::Files(module) = self.view_mode.clone() {
                        ui.label(RichText::new("/").size(11.0).color(MUTED));
                        ui.label(RichText::new(module).size(11.5).color(TEXT));
                    }
                    if matches!(self.view_mode, ViewMode::Ego) {
                        ui.label(RichText::new("/").size(11.0).color(MUTED));
                        if let Some(sym) = self
                            .selected
                            .as_ref()
                            .and_then(|id| self.graph.symbol(id.clone()))
                        {
                            ui.label(
                                RichText::new(truncate(&sym.file, 40))
                                    .size(11.5)
                                    .color(TEXT_DIM),
                            );
                            ui.label(RichText::new("/").size(11.0).color(MUTED));
                            ui.label(RichText::new(&sym.name).size(11.5).color(TEXT));
                        }
                    }
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.label(
                                RichText::new(match self.view_mode {
                                    ViewMode::Modules => "module overview",
                                    ViewMode::Files(_) => "files in module",
                                    ViewMode::Ego => "symbol neighbourhood",
                                })
                                .size(10.5)
                                .color(MUTED),
                            );
                        },
                    );
                });
                if matches!(self.db_source, DbSource::Demo) {
                    card(ui, |ui| {
                        ui.horizontal(|ui| {
                            chip(ui, "SAMPLE", WARN);
                            ui.label(
                                RichText::new(
                                    "This is a demo graph. Index a folder to explore your own code.",
                                )
                                .size(11.5)
                                .color(TEXT_DIM),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("Open project")
                                                    .size(11.5)
                                                    .color(Color32::BLACK),
                                            )
                                            .fill(ACCENT),
                                        )
                                        .clicked()
                                    {
                                        self.open_project();
                                    }
                                },
                            );
                        });
                    });
                }
            });

        let mut pan = self.pan;
        let mut zoom = self.zoom;
        let mut clicked_node = None;
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(view) = &self.visualized {
                clicked_node = draw_graph(ui, view, &mut pan, &mut zoom, &self.touched_set());
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("no graph loaded").monospace().color(MUTED));
                });
            }
        });
        self.pan = pan;
        self.zoom = zoom;
        if let Some(id) = clicked_node {
            self.handle_click(id);
        }
    }
}

// ---- data loading ----

fn run_analysis(graph: &CodeGraph) -> AnalysisResult {
    let cycles = analysis::find_cycles(graph);
    let report = analysis::coupling_report(graph);
    let summary = analysis::summarize(&cycles, &report);
    AnalysisResult {
        summary,
        cycles,
        report,
    }
}

fn load_graph(
    storage: &dyn Storage<Error = CodegraphError>,
) -> Result<(CodeGraph, Vec<codegraph_desktop::model::Symbol>, String), CodegraphError> {
    let symbols = storage.symbols()?;
    let relationships = storage.relationships()?;
    let files = storage.files().unwrap_or_default();
    let graph = CodeGraph::new_with_files(symbols.clone(), relationships, files)?;
    let status = format!(
        "{} symbols, {} relationships loaded",
        graph.symbol_count(),
        graph.relationship_count()
    );
    Ok((graph, symbols, status))
}

/// Adapter: the pill widget lives in the ui module; this keeps call sites terse.
fn tab_click(ui: &mut egui::Ui, current: &mut Tab, tab: Tab, label: &str, count: Option<usize>) {
    if tab_button(ui, *current == tab, label, count) {
        *current = tab;
    }
}

/// Plain-text module table for the clipboard.
fn metrics_text(metrics: Option<&MetricsBundle>) -> String {
    let Some(metrics) = metrics else {
        return "no module metrics".to_string();
    };
    let rows: Vec<codegraph_desktop::report::ModuleRow> = metrics
        .modules
        .iter()
        .filter(|m| m.file_count > 0)
        .map(|m| codegraph_desktop::report::ModuleRow {
            module: m.unit.clone(),
            ca: m.ca,
            ce: m.ce,
            instability: m.instability,
            abstractness: m.abstractness,
            distance: m.distance,
            cohesion_components: m.cohesion_components,
            cohesion_largest_ratio: m.cohesion_largest_ratio,
            complexity_sum: m.complexity_sum,
            files: m.file_count,
        })
        .collect();
    rows.iter().map(module_text).collect::<Vec<_>>().join("\n")
}

/// Extensions the indexer understands; other writes should not trigger reindex.
fn is_source_path(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("");
    matches!(
        ext,
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "rs"
    )
}

fn progress_text(p: &IndexProgress) -> String {
    match p {
        IndexProgress::Walking { found } => format!("walking · {found} files"),
        IndexProgress::Parsing { done, total } => format!("parsing {done}/{total}"),
        IndexProgress::Resolving => "resolving references…".into(),
        IndexProgress::Writing => "writing graph…".into(),
        IndexProgress::Done(s) => format!("done · {} files", s.files),
        IndexProgress::Failed(e) => format!("failed: {e}"),
        IndexProgress::Cancelled => "cancelled".into(),
    }
}

fn load_database(path: &PathBuf) -> Result<(CodeGraph, String), CodegraphError> {
    let storage: Box<dyn Storage<Error = CodegraphError>> = Box::new(SqliteStorage::open(path)?);
    let (graph, _symbols, status) = load_graph(&*storage)?;
    Ok((graph, status))
}

fn demo_graph() -> CodeGraph {
    use codegraph_desktop::model::{Relationship, RelationshipKind, Symbol, SymbolKind};

    let mut parse = Symbol::new(
        SymbolId("1".into()),
        "parse_input",
        SymbolKind::Function,
        "src/parser.rs",
    );
    parse.line = Some(12);
    parse.signature = Some("(s: &str) -> Result<Vec<Token>, ParseError>".into());
    parse.doc = Some("Tokenize a source string into a token stream.".into());

    let mut lexer = Symbol::new(
        SymbolId("2".into()),
        "lexer",
        SymbolKind::Struct,
        "src/lexer.rs",
    );
    lexer.line = Some(4);
    lexer.signature = Some("struct Lexer { .. }".into());

    let mut module = Symbol::new(
        SymbolId("3".into()),
        "codegraph_desktop",
        SymbolKind::Module,
        "lib.rs",
    );
    module.line = Some(1);

    let mut render = Symbol::new(
        SymbolId("4".into()),
        "render",
        SymbolKind::Function,
        "src/ui.rs",
    );
    render.line = Some(33);
    render.signature = Some("(g: &CodeGraph) -> Element".into());
    render.doc = Some("Render the knowledge graph to the screen.".into());

    let symbols = vec![parse, lexer, module, render];

    let relationships = vec![
        Relationship::new(
            1,
            SymbolId("1".into()),
            SymbolId("2".into()),
            RelationshipKind::References,
        ),
        Relationship::new(
            2,
            SymbolId("3".into()),
            SymbolId("1".into()),
            RelationshipKind::Contains,
        ),
        Relationship::new(
            3,
            SymbolId("3".into()),
            SymbolId("4".into()),
            RelationshipKind::Contains,
        ),
        Relationship::new(
            4,
            SymbolId("4".into()),
            SymbolId("2".into()),
            RelationshipKind::References,
        ),
    ];

    CodeGraph::new(symbols, relationships).expect("demo graph is valid")
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]),
        ..Default::default()
    };
    eframe::run_native(
        "CODEGRAPH — knowledge graph explorer",
        options,
        Box::new(|cc| Ok(Box::new(CodegraphApp::new(cc)))),
    )
}

#[cfg(not(target_arch = "wasm32"))]
use super::git::{self};
use super::git::{CommitInfo, GitState};
#[cfg(target_arch = "wasm32")]
use super::git_http;
#[cfg(not(target_arch = "wasm32"))]
use super::models::BenchmarkProblem;
use super::models::{BenchmarkDataset, SolverBenchmark};
use super::AppError;
use crate::types::{
    AggregationKind, DataFormat, DataMode, MetricType, PlotType, ProfileFilter, XaxisType,
};
use crate::visualization::formula::CompiledFormula;
use crate::visualization::outliers::{CommitOutlierReport, OutlierDetectionConfig};
use crate::visualization::plotting::PlotData;
use crate::visualization::stacked_bar::StackedBarData;
use crate::visualization::timeseries::TimeseriesData;
#[cfg(not(target_arch = "wasm32"))]
use egui_file_dialog::FileDialog;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

/// Web-build stub for `egui_file_dialog::FileDialog`.
///
/// `egui-file-dialog` uses `rfd` under the hood, which pulls native OS
/// picker APIs and doesn't compile for `wasm32-unknown-unknown`. Rather
/// than sprinkle `#[cfg]` across every field and UI call site, we expose
/// a ZST with the same inherent-method surface the sidebar uses. On wasm
/// the buttons are visible but `take_picked()` always returns `None` —
/// i.e. a visual-only no-op until W2 wires in the HTML file-drop path.
///
/// W2 will replace this with a real wasm file-drop integration.
#[cfg(target_arch = "wasm32")]
pub struct FileDialog;

#[cfg(target_arch = "wasm32")]
impl FileDialog {
    pub fn new() -> Self {
        Self
    }
    pub fn update(&mut self, _ctx: &egui::Context) {}
    pub fn pick_file(&mut self) {}
    pub fn pick_directory(&mut self) {}
    pub fn take_picked(&mut self) -> Option<PathBuf> {
        None
    }
}

#[cfg(target_arch = "wasm32")]
impl Default for FileDialog {
    fn default() -> Self {
        Self::new()
    }
}

/// Maximum number of commits to walk when loading a git repository.
/// Large repos (Linux kernel, etc.) have millions of commits — this bound
/// keeps the UI responsive and memory usage bounded.
pub const MAX_GIT_COMMITS: usize = 1000;

/// Upper bound on messages drained from the load channel per frame.
///
/// A fast producer (streaming parse of a huge file with tight progress
/// throttling) could otherwise starve the UI. Drain at most this many in a
/// single `check_loading_status` pass and defer the rest to the next frame.
const MAX_MESSAGES_PER_FRAME: usize = 256;

/// Emit a streaming-parse progress update every N items. The UI redraws and
/// `ctx.request_repaint()` fire at this cadence — ~once per batch, not per
/// item — which keeps overhead negligible without making progress feel jerky.
pub const PROGRESS_BATCH_SIZE: usize = 16;

/// Per-item progress reported from a worker thread.
///
/// `total = None` signals a streaming source whose size is not known up
/// front (e.g. streaming JSON — arrays don't advertise their length); the
/// UI should show a spinner + item count rather than a filled bar in that
/// case.
#[derive(Debug, Clone)]
pub struct LoadProgress {
    pub current: usize,
    pub total: Option<usize>,
    pub phase: &'static str,
}

/// Messages sent from a worker thread to the UI thread during a load.
///
/// `Progress` updates flow continuously; exactly one terminal message is
/// expected: either `Done(...)` on success or a top-level `Err` on failure.
pub enum LoadUpdate {
    Progress(LoadProgress),
    Done(LoadResult),
}

type LoadUpdateMessage = Result<LoadUpdate, AppError>;
type LoadChannel = (Sender<LoadUpdateMessage>, Receiver<LoadUpdateMessage>);

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum ViewMode {
    Benchmark,
    Solver,
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum SolverXAxis {
    Iteration,
    Time,
}

pub struct PlotConfig {
    pub plot_type: PlotType,
    pub x_axis: Option<XaxisType>,
    pub data_metric: Option<MetricType>,
    pub baseline_format: DataFormat,
    pub normalize: bool,
    pub profile_filter: ProfileFilter,
    pub filter_outliers: bool,
    pub log_scale_x: bool,
    pub show_percentile_bands: bool,
    /// Raw text from the custom-formula TextEdit.
    pub custom_formula_text: String,
    /// Compiled formula; `Some` iff `custom_formula_text` parses successfully.
    pub custom_formula: Option<CompiledFormula>,
    /// Inline error string displayed beneath the formula TextEdit.
    pub custom_formula_error: Option<String>,
}

impl Default for PlotConfig {
    fn default() -> Self {
        Self {
            plot_type: PlotType::Scatter,
            x_axis: Some(XaxisType::NonZeros),
            data_metric: Some(MetricType::Time),
            baseline_format: DataFormat::CSR,
            normalize: false,
            profile_filter: ProfileFilter::None,
            filter_outliers: false,
            log_scale_x: false,
            show_percentile_bands: false,
            custom_formula_text: String::new(),
            custom_formula: None,
            custom_formula_error: None,
        }
    }
}

#[derive(Default)]
pub struct DataSelection {
    pub mode: Option<DataMode>,
    pub dataset: HashMap<String, BenchmarkDataset>,
    pub active_dataset: Vec<String>,
    pub sorted_dataset_keys: Vec<String>,
    pub active_formats: HashMap<DataFormat, bool>,
}

pub struct LoadingState {
    pub is_loading: bool,
    pub picked_file: Option<PathBuf>,
    pub file_dialog: FileDialog,
    /// Separate dialog for picking a git repository directory. Using a distinct
    /// dialog instance (vs. toggling modes on one) keeps the `take_picked()`
    /// plumbing unambiguous in `render_side_panel`.
    pub git_file_dialog: FileDialog,
    pub rx_load: Option<Receiver<LoadUpdateMessage>>,
    pub last_error: Option<String>,
    /// Latest progress snapshot from the active worker. `None` between loads,
    /// and before the worker has emitted its first update.
    pub progress: Option<LoadProgress>,
    /// Label (file stem / repo name) for the in-flight load; held here so the
    /// UI-thread `Done` handler can tag the result without re-deriving it.
    pub current_label: Option<String>,
}

// On wasm `FileDialog` is a ZST whose `Default` impl is equivalent to
// `FileDialog::new()`, which tempts clippy into suggesting `#[derive(Default)]`.
// On native that's not true — `egui_file_dialog::FileDialog::new()` is the
// canonical constructor and we want to keep calling it explicitly. Silence
// the wasm-only false positive.
#[cfg_attr(target_arch = "wasm32", allow(clippy::derivable_impls))]
impl Default for LoadingState {
    fn default() -> Self {
        Self {
            is_loading: false,
            picked_file: None,
            file_dialog: FileDialog::new(),
            git_file_dialog: FileDialog::new(),
            rx_load: None,
            last_error: None,
            progress: None,
            current_label: None,
        }
    }
}

pub struct SolverState {
    pub data: Option<Vec<SolverBenchmark>>,
    pub selected_idx: usize,
    pub selected_methods: HashSet<String>,
    pub selected_detail_method: Option<String>,
    pub x_axis: SolverXAxis,
    pub log_scale: bool,
    pub show_recurrent: bool,
    pub show_true: bool,
    pub show_implicit: bool,
    pub show_timestamp: bool,
}

impl Default for SolverState {
    fn default() -> Self {
        Self {
            data: None,
            selected_idx: 0,
            selected_methods: HashSet::new(),
            selected_detail_method: None,
            x_axis: SolverXAxis::Iteration,
            log_scale: true,
            show_recurrent: true,
            show_true: true,
            show_implicit: true,
            show_timestamp: false,
        }
    }
}

#[derive(Default)]
pub struct PlotCache {
    pub data: Option<Rc<PlotData>>,
    pub key: u64,
}

/// Which side is the "before" reference in an A-vs-B comparison.
///
/// Speedup ratios and normalization baselines are computed relative to this
/// side; swapping it flips every ratio in the diff table and histogram.
#[derive(Default, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CompareSide {
    #[default]
    A,
    B,
}

/// User-facing controls for the Comparison plot type.
///
/// `commit_a`/`commit_b` are keys into [`DataSelection::dataset`]; a pair
/// must be pre-loaded via the existing Git panel / file loader before the
/// comparison UI is enabled. All other fields tune how ratios are computed
/// and filtered.
#[derive(Clone)]
pub struct ComparisonState {
    pub commit_a: Option<String>,
    pub commit_b: Option<String>,
    pub baseline_side: CompareSide,
    pub shared_y_range: bool,
    pub lower_is_better: bool,
    /// Percent threshold for diff-table row suppression. Rows with
    /// `|delta| / |value_a|` below this value are hidden from the table
    /// (but still contribute to histogram / summary counts).
    pub diff_threshold: f64,
}

impl Default for ComparisonState {
    fn default() -> Self {
        Self {
            commit_a: None,
            commit_b: None,
            baseline_side: CompareSide::A,
            shared_y_range: true,
            lower_is_better: true,
            diff_threshold: 0.0,
        }
    }
}

/// Memoization slot for the comparison-view computation.
///
/// Kept separate from [`PlotCache`] so switching the plot-type ComboBox
/// back and forth between Scatter and Comparison doesn't repeatedly
/// invalidate the single-plot cache.
#[derive(Default)]
pub struct ComparisonCache {
    pub data: Option<Rc<crate::visualization::compare::ComparisonPlotData>>,
    pub key: u64,
}

/// User-facing controls for the line-timeseries plot type (Task 6).
///
/// `problem_filter == Some(name)` degenerates the aggregate to a single
/// problem's value per dataset — useful for "how did problem X evolve
/// across every commit?" queries.
#[derive(Clone)]
pub struct TimeseriesState {
    pub aggregation: AggregationKind,
    pub format: DataFormat,
    pub problem_filter: Option<String>,
}

impl Default for TimeseriesState {
    fn default() -> Self {
        Self {
            aggregation: AggregationKind::Median,
            format: DataFormat::CSR,
            problem_filter: None,
        }
    }
}

/// User-facing controls for the stacked-bar plot type (Task 6).
///
/// `dataset == None` is a pre-selection gate: the UI shows a hint until
/// the user picks one of the loaded datasets. `top_n` is clamped by the
/// sidebar `DragValue` to `5..=500`.
#[derive(Clone)]
pub struct StackedBarState {
    pub dataset: Option<String>,
    pub sort_by_total: bool,
    pub top_n: usize,
}

impl Default for StackedBarState {
    fn default() -> Self {
        Self {
            dataset: None,
            sort_by_total: true,
            top_n: 30,
        }
    }
}

#[derive(Default)]
pub struct TimeseriesCache {
    pub data: Option<Rc<TimeseriesData>>,
    pub key: u64,
}

#[derive(Default)]
pub struct StackedBarCache {
    pub data: Option<Rc<StackedBarData>>,
    pub key: u64,
}

/// Memoization slot for the cross-commit outlier detector (Task 8).
///
/// Keyed on every input that can change the report vec so the sidebar /
/// timeseries view don't recompute per-frame — outlier math iterates every
/// (commit, problem, format) and would be wasteful on the hot path.
#[derive(Default)]
pub struct OutlierCache {
    pub reports: Option<Rc<Vec<CommitOutlierReport>>>,
    pub key: u64,
}

/// Sidebar status slot for the vector-export buttons (Task 10).
///
/// Holds the most recent success message (the absolute path on native, or
/// a size summary on wasm) or error string. Cleared whenever the user
/// clicks the matching export button again so stale output doesn't linger.
#[derive(Default)]
pub struct ExportState {
    pub last_message: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub enum ExportKind {
    Svg,
    #[cfg(not(target_arch = "wasm32"))]
    Pdf,
}

pub struct Dashboard {
    pub view_mode: ViewMode,
    pub plot_config: PlotConfig,
    pub data_selection: DataSelection,
    pub loading: LoadingState,
    pub solver: SolverState,
    pub plot_cache: PlotCache,
    pub comparison: ComparisonState,
    pub comparison_cache: ComparisonCache,
    pub timeseries: TimeseriesState,
    pub timeseries_cache: TimeseriesCache,
    pub stacked_bar: StackedBarState,
    pub stacked_bar_cache: StackedBarCache,
    pub git: GitState,
    pub outlier_config: OutlierDetectionConfig,
    pub outlier_cache: OutlierCache,
    pub export: ExportState,
}

pub enum LoadResult {
    Benchmark(BenchmarkDataset),
    Solver(Vec<SolverBenchmark>),
    Git(Vec<CommitInfo>),
}

impl Default for Dashboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Dashboard {
    pub fn new() -> Self {
        Self {
            view_mode: ViewMode::Benchmark,
            plot_config: PlotConfig::default(),
            data_selection: DataSelection::default(),
            loading: LoadingState::default(),
            solver: SolverState::default(),
            plot_cache: PlotCache::default(),
            comparison: ComparisonState::default(),
            comparison_cache: ComparisonCache::default(),
            timeseries: TimeseriesState::default(),
            timeseries_cache: TimeseriesCache::default(),
            stacked_bar: StackedBarState::default(),
            stacked_bar_cache: StackedBarCache::default(),
            git: GitState::default(),
            outlier_config: OutlierDetectionConfig::default(),
            outlier_cache: OutlierCache::default(),
            export: ExportState::default(),
        }
    }

    pub fn encode_file(&self) -> String {
        let file_stem = self
            .loading
            .picked_file
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("unknown_file");

        let time = chrono::Utc::now().format("%H-%M-%S").to_string();
        format!("{}#{}", file_stem, time)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn process_file(&mut self, ctx: &egui::Context) {
        // Guard against clobbering an in-flight load's channel.
        if self.loading.is_loading {
            return;
        }
        if let Some(file_path) = self.loading.picked_file.clone() {
            let file_name = self.encode_file();
            let view_mode = self.view_mode;

            let (tx, rx): LoadChannel = channel();

            self.loading.is_loading = true;
            self.loading.rx_load = Some(rx);
            self.loading.last_error = None;
            self.loading.progress = None;
            self.loading.current_label = Some(file_name);

            // Clone so the worker can request repaints without holding a
            // borrow on the Dashboard.
            let ctx = ctx.clone();

            thread::spawn(move || {
                let result = (|| -> Result<LoadResult, AppError> {
                    let file = std::fs::File::open(&file_path)?;
                    let reader = std::io::BufReader::new(file);

                    match view_mode {
                        ViewMode::Benchmark => {
                            let problems = stream_parse_vec::<BenchmarkProblem, _>(
                                reader,
                                "Parsing benchmark",
                                &tx,
                                &ctx,
                            )?;

                            // Post-processing phase: emit a progress update so
                            // the UI reflects that we've moved past parsing.
                            let total = problems.len();
                            let _ = tx.send(Ok(LoadUpdate::Progress(LoadProgress {
                                current: 0,
                                total: Some(total),
                                phase: "Computing metrics",
                            })));
                            ctx.request_repaint();

                            let processed_data = super::loader::process_benchmark_data(problems)?;

                            // Emit a final "computed" update so the UI shows
                            // 100% momentarily before `Done` swaps it out.
                            let _ = tx.send(Ok(LoadUpdate::Progress(LoadProgress {
                                current: total,
                                total: Some(total),
                                phase: "Computing metrics",
                            })));

                            Ok(LoadResult::Benchmark(processed_data))
                        }
                        ViewMode::Solver => {
                            let solvers = stream_parse_vec::<SolverBenchmark, _>(
                                reader,
                                "Parsing solver",
                                &tx,
                                &ctx,
                            )?;
                            Ok(LoadResult::Solver(solvers))
                        }
                    }
                })();

                // Terminal message: Done on success, top-level Err on failure.
                match result {
                    Ok(data) => {
                        let _ = tx.send(Ok(LoadUpdate::Done(data)));
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                    }
                }
                ctx.request_repaint();
            });
        }
    }

    /// Web-build stub for [`process_file`]. Surfaces a user-visible error
    /// explaining that local-file parsing is not yet supported; W2 will
    /// replace this with a wasm file-drop implementation that hands the
    /// reader to `stream_parse_vec` on a `wasm-bindgen-futures` task.
    #[cfg(target_arch = "wasm32")]
    pub fn process_file(&mut self, _ctx: &egui::Context) {
        if self.loading.is_loading {
            return;
        }
        self.loading.last_error =
            Some("Local file loading is not yet supported on the web build (see W2).".to_string());
        // Clear the picked file so the sidebar doesn't re-trigger on next frame.
        self.loading.picked_file = None;
    }

    /// Spawn a background thread to load the commit history of `path`.
    ///
    /// Result is delivered via the existing loading channel; the UI picks it
    /// up in [`check_loading_status`]. The `Dashboard::git.repo_path` is set
    /// synchronously so the sidebar can display the chosen path immediately.
    ///
    /// `gix::Repository` is `Send` but not `Sync`; we open it inside the
    /// spawned thread to avoid blocking the UI during potentially expensive
    /// repo discovery and history walks on large repos.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_git_repo(&mut self, ctx: &egui::Context, path: PathBuf) {
        // Guard against clobbering an in-flight load's channel.
        if self.loading.is_loading {
            return;
        }

        let (tx, rx): LoadChannel = channel();

        self.loading.is_loading = true;
        self.loading.rx_load = Some(rx);
        self.loading.last_error = None;
        self.loading.progress = None;

        self.git.repo_path = Some(path.clone());
        self.git.last_error = None;
        // Clear previous selection/commits so the UI reflects the new load.
        self.git.commits.clear();
        self.git.selected_commit_idx = None;
        self.git.last_prefetch_idx = None;
        self.git.prefetch_in_flight = false;

        let label = path
            .file_name()
            .and_then(|s| s.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        self.loading.current_label = Some(label);

        let ctx = ctx.clone();

        thread::spawn(move || {
            let tx_progress = tx.clone();
            let ctx_progress = ctx.clone();
            let result = git::load_commits(&path, MAX_GIT_COMMITS, |progress| {
                let _ = tx_progress.send(Ok(LoadUpdate::Progress(progress)));
                ctx_progress.request_repaint();
            })
            .map(LoadResult::Git)
            .map_err(AppError::from);

            match result {
                Ok(data) => {
                    let _ = tx.send(Ok(LoadUpdate::Done(data)));
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
            ctx.request_repaint();
        });
    }

    /// Web-build implementation of [`load_git_repo`].
    ///
    /// Unlike native (which walks a local repo via `gix` on a worker
    /// thread), this path HTTP-fetches a prebuilt `benchmarks/commits.json`
    /// manifest from the same origin. `_path` is ignored — the web build
    /// has no concept of a "local repo directory" — so the sidebar calls
    /// this with an empty `PathBuf`. The shared signature is kept so
    /// cross-target call sites don't need their own `#[cfg]` dance.
    ///
    /// Control flow: set up a channel, mutate UI state synchronously, then
    /// hand the sender to `git_http::load_commits_http`. That function
    /// calls `ehttp::fetch` which resolves on a JS microtask — no thread
    /// spawn, because wasm32-unknown-unknown has no threads. The terminal
    /// `Done`/`Err` message arrives via the same `check_loading_status`
    /// poll used by every other load.
    #[cfg(target_arch = "wasm32")]
    pub fn load_git_repo(&mut self, ctx: &egui::Context, _path: PathBuf) {
        // Guard against clobbering an in-flight load's channel.
        if self.loading.is_loading {
            return;
        }

        let (tx, rx): LoadChannel = channel();

        self.loading.is_loading = true;
        self.loading.rx_load = Some(rx);
        self.loading.last_error = None;
        self.loading.progress = None;

        // No local repo path on wasm; clear it along with any prior state.
        self.git.repo_path = None;
        self.git.last_error = None;
        self.git.commits.clear();
        self.git.selected_commit_idx = None;
        self.git.last_prefetch_idx = None;
        self.git.prefetch_in_flight = false;

        self.loading.current_label = Some("benchmarks".to_string());

        git_http::load_commits_http(ctx.clone(), tx);
    }

    pub fn check_loading_status(&mut self) {
        // Take the receiver out so we can mutate `self` freely during
        // message processing; put it back if we didn't finish.
        let Some(rx) = self.loading.rx_load.take() else {
            return;
        };

        let mut finished = false;
        let mut drained = 0;

        while drained < MAX_MESSAGES_PER_FRAME {
            match rx.try_recv() {
                Ok(Ok(LoadUpdate::Progress(progress))) => {
                    self.loading.progress = Some(progress);
                }
                Ok(Ok(LoadUpdate::Done(data))) => {
                    let file_name = self.loading.current_label.take().unwrap_or_default();
                    self.apply_load_result(file_name, data);
                    self.loading.is_loading = false;
                    self.loading.progress = None;
                    finished = true;
                    break;
                }
                Ok(Err(e)) => {
                    self.apply_load_error(e);
                    self.loading.is_loading = false;
                    self.loading.progress = None;
                    self.loading.current_label = None;
                    finished = true;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Worker dropped the sender without a terminal message.
                    // Treat as a silent failure so the UI recovers gracefully.
                    self.loading.is_loading = false;
                    self.loading.progress = None;
                    self.loading.current_label = None;
                    finished = true;
                    break;
                }
            }
            drained += 1;
        }

        if !finished {
            // Either the channel was empty or we hit the per-frame cap;
            // re-install the receiver so the next frame keeps draining.
            // Edge case: if a terminal message sits behind exactly
            // MAX_MESSAGES_PER_FRAME progress messages, it's processed on the
            // next frame — the spinner can linger one frame after completion.
            self.loading.rx_load = Some(rx);
        }
    }

    fn apply_load_result(&mut self, file_name: String, data: LoadResult) {
        match data {
            LoadResult::Benchmark(dataset) => {
                // A prefetch load enters the dataset map but does NOT push
                // onto `active_dataset` — the whole point of prefetch is to
                // warm the cache without changing what the user is looking
                // at. Only interactive / explicit loads toggle the active
                // set; the commit-switcher's `select_commit` path handles
                // activation when the user lands on a prefetched commit.
                let was_prefetch = self.git.prefetch_in_flight;
                if !was_prefetch && !self.data_selection.active_dataset.contains(&file_name) {
                    self.data_selection.active_dataset.push(file_name.clone());
                }
                self.data_selection.dataset.insert(file_name, dataset);
                self.data_selection.sorted_dataset_keys =
                    self.data_selection.dataset.keys().cloned().collect();
                self.data_selection.sorted_dataset_keys.sort();
                if was_prefetch {
                    self.git.prefetch_in_flight = false;
                }
            }
            LoadResult::Solver(solver_bench) => {
                self.solver.data = Some(solver_bench);
            }
            LoadResult::Git(commits) => {
                self.git.commits = commits;
                self.git.selected_commit_idx = None;
                self.git.last_prefetch_idx = None;
                self.git.prefetch_in_flight = false;
            }
        }
    }

    fn apply_load_error(&mut self, e: AppError) {
        // Prefetch failures shouldn't surface noise — they are opportunistic
        // background loads, and a missing bench file is a perfectly
        // legitimate state for a commit with no benchmark artifacts.
        if self.git.prefetch_in_flight {
            self.git.prefetch_in_flight = false;
            log::debug!("commit prefetch failed silently: {}", e);
            return;
        }
        match e {
            // Git errors belong on the git panel only — surfacing them
            // in the Data Sources banner would duplicate.
            AppError::Git(git_err) => {
                self.git.last_error = Some(git_err.to_string());
            }
            _ => {
                self.loading.last_error = Some(format!("Error loading file: {}", e));
            }
        }
    }

    /// Build the oldest-first commit-key ordering used by the outlier
    /// detector and the LineTimeseries badge lookup (Task 8).
    ///
    /// When a git repo is loaded, this returns one `bench_<short_sha>` key
    /// per commit in chronological order (the git walk itself is newest-
    /// first, so we reverse). When no commits are loaded, falls back to
    /// the alphabetical `sorted_dataset_keys` list so the detector still
    /// produces a stable — if less meaningful — output.
    pub fn outlier_ordered_keys(&self) -> Vec<String> {
        if self.git.commits.is_empty() {
            return self.data_selection.sorted_dataset_keys.clone();
        }
        // git.commits is newest-first; emit bench keys oldest-first so the
        // rolling baseline window walks forward in time.
        self.git
            .commits
            .iter()
            .rev()
            .map(Self::commit_dataset_key)
            .filter(|k| self.data_selection.dataset.contains_key(k))
            .collect()
    }

    /// Compute or fetch the cached outlier reports for the current commit
    /// ordering + configuration (Task 8).
    ///
    /// Returns an empty vec when detection is disabled or no commits are
    /// loaded. `ordered_keys` is the timeline-ordered dataset key list used
    /// by the LineTimeseries view — oldest-first so the rolling baseline
    /// window makes physical sense.
    pub fn outlier_reports(
        &mut self,
        ordered_keys: &[String],
    ) -> Rc<Vec<crate::visualization::outliers::CommitOutlierReport>> {
        use crate::visualization::outliers::{
            build_commit_stats, detect_outliers, outlier_cache_key,
        };

        if !self.outlier_config.enabled || ordered_keys.is_empty() {
            return Rc::new(Vec::new());
        }

        let mut sorted_keys = self.data_selection.sorted_dataset_keys.clone();
        sorted_keys.sort();

        let custom_src = self
            .plot_config
            .custom_formula
            .as_ref()
            .map(|f| f.source.as_str());

        let key = outlier_cache_key(&self.outlier_config, ordered_keys, &sorted_keys, custom_src);

        if self.outlier_cache.key == key {
            if let Some(rc) = self.outlier_cache.reports.as_ref() {
                return Rc::clone(rc);
            }
        }

        let stats = build_commit_stats(
            &self.data_selection.dataset,
            ordered_keys,
            self.outlier_config.metric,
            self.plot_config.custom_formula.as_ref(),
        );
        let reports = detect_outliers(&stats, &self.outlier_config);
        let rc = Rc::new(reports);
        self.outlier_cache.key = key;
        self.outlier_cache.reports = Some(Rc::clone(&rc));
        rc
    }

    /// Dataset key used when a commit's bench file is loaded via the
    /// commit-switcher (Task 7).
    ///
    /// Deterministic and cross-target: both native and wasm derive the
    /// same `"bench_<short_sha>"` string so the `DataSelection::dataset`
    /// map key matches regardless of load path. This differs from
    /// [`Dashboard::encode_file`], which embeds a wall-clock timestamp
    /// and is used only for ad-hoc user file picks (where collisions
    /// between re-imports of the same file are wanted-by-design).
    pub fn commit_dataset_key(commit: &CommitInfo) -> String {
        format!("bench_{}", commit.short_sha)
    }

    /// Step the commit selection by `delta` (positive = forward in the
    /// commit list = later in history, since the walk is newest-first).
    ///
    /// Semantics:
    /// - Empty commit list is a no-op.
    /// - `None` + positive delta picks index 0; `None` + negative delta picks
    ///   the last index.
    /// - Out-of-range results saturate at 0 / `last_idx`.
    ///
    /// Does **not** kick off a load here; callers should invoke
    /// [`Dashboard::on_commit_selected`] after stepping to propagate the
    /// new selection to the active dataset and prefetch queue.
    pub fn step_commit(&mut self, delta: i32) {
        let len = self.git.commits.len();
        if len == 0 {
            return;
        }
        let last = len - 1;
        let new_idx = match self.git.selected_commit_idx {
            None => {
                if delta > 0 {
                    0
                } else {
                    last
                }
            }
            Some(cur) => {
                let cur_i = cur as i64;
                let raw = cur_i.saturating_add(delta as i64);
                raw.clamp(0, last as i64) as usize
            }
        };
        self.git.selected_commit_idx = Some(new_idx);
    }

    /// Handler for "the user just picked commit `idx`".
    ///
    /// If the associated dataset is already loaded, mutate
    /// [`DataSelection::active_dataset`] so the render path switches to it.
    /// If not, kick off a single-file load (native: worker thread; wasm:
    /// `ehttp::fetch`). Also updates [`ComparisonState::commit_b`] so the
    /// Comparison view follows the scrubber for the "right-hand side".
    ///
    /// Returns without action if `idx` is out of range.
    pub fn on_commit_selected(&mut self, ctx: &egui::Context, idx: usize) {
        if idx >= self.git.commits.len() {
            return;
        }
        self.git.selected_commit_idx = Some(idx);

        let key = {
            let commit = &self.git.commits[idx];
            Self::commit_dataset_key(commit)
        };

        // Comparison view: follow the scrubber on side B; leave A alone so
        // the user can anchor a reference and scrub the other side.
        self.comparison.commit_b = Some(key.clone());

        if self.data_selection.dataset.contains_key(&key) {
            // Already loaded: swap active to exactly this key.
            self.data_selection.active_dataset.clear();
            self.data_selection.active_dataset.push(key);
        } else {
            // Not loaded: kick off a foreground load. `is_prefetch=false`
            // means the `Done` handler will push the key onto active_dataset
            // automatically via `apply_load_result`.
            self.load_commit_bench(ctx, idx, false);
        }
    }

    /// Kick off background loads for the datasets at `idx - 1` and `idx + 1`
    /// if their bench files are not yet in [`DataSelection::dataset`].
    ///
    /// Guards:
    /// - skips when there's already an in-flight load (the load channel is
    ///   single-slot; a second fire would clobber it);
    /// - skips when the selection hasn't moved since the last prefetch;
    /// - runs at most one prefetch at a time (there are two neighbours but
    ///   only one active channel).
    pub fn prefetch_adjacent_commits(&mut self, ctx: &egui::Context) {
        let Some(idx) = self.git.selected_commit_idx else {
            return;
        };
        if self.loading.is_loading {
            return;
        }
        if self.git.last_prefetch_idx == Some(idx) {
            return;
        }

        // Candidates: older (idx + 1) first, then newer (idx - 1). The walk
        // is newest-first, so stepping "right" in the UI tends to move older,
        // which is the more common scrub direction when surveying history.
        let mut candidates: Vec<usize> = Vec::new();
        if idx + 1 < self.git.commits.len() {
            candidates.push(idx + 1);
        }
        if idx > 0 {
            candidates.push(idx - 1);
        }

        for cand in candidates {
            let key = Self::commit_dataset_key(&self.git.commits[cand]);
            if self.data_selection.dataset.contains_key(&key) {
                continue;
            }
            if self.git.commits[cand].bench_file.is_none() {
                continue;
            }
            self.load_commit_bench(ctx, cand, true);
            // Only one prefetch at a time (single load channel).
            break;
        }

        self.git.last_prefetch_idx = Some(idx);
    }

    /// Load the bench file for commit `idx` via the standard load channel.
    ///
    /// `is_prefetch` toggles which apply-path the terminal `Done` message
    /// takes: a prefetch drops the dataset into the cache without pushing
    /// onto `active_dataset`, while a foreground load activates it (and
    /// surfaces any error on the Data Sources banner).
    ///
    /// Native: spawns a worker thread that reads the file and streams the
    /// same `BenchmarkProblem` parser used for interactive file picks.
    /// Wasm: issues an `ehttp::fetch` against the bench file's relative URL
    /// (as stored in `CommitInfo::bench_file` by the HTTP commit loader).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_commit_bench(&mut self, ctx: &egui::Context, idx: usize, is_prefetch: bool) {
        if self.loading.is_loading {
            return;
        }
        let Some(commit) = self.git.commits.get(idx) else {
            return;
        };
        let Some(bench_path) = commit.bench_file.clone() else {
            if !is_prefetch {
                self.loading.last_error = Some(format!(
                    "No benchmark file registered for commit {} (short {}).",
                    commit.sha, commit.short_sha
                ));
            }
            return;
        };
        let key = Self::commit_dataset_key(commit);

        let (tx, rx): LoadChannel = channel();

        self.loading.is_loading = true;
        self.loading.rx_load = Some(rx);
        self.loading.last_error = None;
        self.loading.progress = None;
        self.loading.current_label = Some(key);
        self.git.prefetch_in_flight = is_prefetch;

        let ctx_worker = ctx.clone();
        thread::spawn(move || {
            let result = (|| -> Result<LoadResult, AppError> {
                let file = std::fs::File::open(&bench_path)?;
                let reader = std::io::BufReader::new(file);
                let problems = stream_parse_vec::<BenchmarkProblem, _>(
                    reader,
                    "Parsing benchmark",
                    &tx,
                    &ctx_worker,
                )?;
                let processed = super::loader::process_benchmark_data(problems)?;
                Ok(LoadResult::Benchmark(processed))
            })();
            match result {
                Ok(data) => {
                    let _ = tx.send(Ok(LoadUpdate::Done(data)));
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
            ctx_worker.request_repaint();
        });
    }

    /// Web-build counterpart to `load_commit_bench`.
    ///
    /// Fetches the commit's bench file over HTTP using the path stored in
    /// `CommitInfo::bench_file` (populated by the HTTP commit loader as a
    /// relative URL like `benchmarks/bench_<sha>.json`). On success the
    /// parsed dataset is delivered through the same load channel the UI
    /// already polls in `check_loading_status`.
    #[cfg(target_arch = "wasm32")]
    pub fn load_commit_bench(&mut self, ctx: &egui::Context, idx: usize, is_prefetch: bool) {
        if self.loading.is_loading {
            return;
        }
        let Some(commit) = self.git.commits.get(idx) else {
            return;
        };
        let Some(bench_path) = commit.bench_file.clone() else {
            if !is_prefetch {
                self.loading.last_error = Some(format!(
                    "No benchmark file registered for commit {} (short {}).",
                    commit.sha, commit.short_sha
                ));
            }
            return;
        };
        let key = Self::commit_dataset_key(commit);
        let url = bench_path.to_string_lossy().into_owned();

        let (tx, rx): LoadChannel = channel();

        self.loading.is_loading = true;
        self.loading.rx_load = Some(rx);
        self.loading.last_error = None;
        self.loading.progress = None;
        self.loading.current_label = Some(key);
        self.git.prefetch_in_flight = is_prefetch;

        super::git_http::load_bench_http(ctx.clone(), tx, url);
    }
}

/// Stream-parse a top-level JSON array into `Vec<T>`, sending a
/// `LoadProgress` update every `PROGRESS_BATCH_SIZE` items.
///
/// Uses a custom `Visitor` that drives `SeqAccess::next_element` in a loop,
/// so it never materialises the full `Vec<Value>` before decoding — each
/// `T` is deserialized directly from the token stream. This matters for
/// 10MB+ benchmark files where holding the JSON tree and the decoded
/// `Vec<T>` at the same time would double peak memory.
///
/// Returns the fully-materialised vector on success. Errors propagate as
/// `AppError::Json` (via the `From<serde_json::Error>` impl). This function
/// does **not** send a terminal `Done`/`Err` message over `tx` — the caller
/// is responsible for that. Progress messages emitted here use `total = None`
/// since JSON arrays don't advertise length.
///
/// Made `pub(crate)` rather than private so the streaming path is reachable
/// from tests in this crate without going through the full `process_file`
/// UI plumbing.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn stream_parse_vec<T, R>(
    reader: R,
    phase: &'static str,
    tx: &Sender<LoadUpdateMessage>,
    ctx: &egui::Context,
) -> Result<Vec<T>, AppError>
where
    T: serde::de::DeserializeOwned,
    R: std::io::Read,
{
    use serde::de::{Deserializer, SeqAccess, Visitor};
    use std::marker::PhantomData;

    struct ProgressVisitor<'a, T> {
        phase: &'static str,
        tx: &'a Sender<LoadUpdateMessage>,
        ctx: &'a egui::Context,
        _marker: PhantomData<fn() -> T>,
    }

    impl<'de, T> Visitor<'de> for ProgressVisitor<'_, T>
    where
        T: serde::de::DeserializeOwned,
    {
        type Value = Vec<T>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a JSON array")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut items: Vec<T> = Vec::with_capacity(seq.size_hint().unwrap_or(0));
            while let Some(item) = seq.next_element::<T>()? {
                items.push(item);

                // Throttle progress + repaints: once per batch, not per item.
                if items.len().is_multiple_of(PROGRESS_BATCH_SIZE) {
                    let _ = self.tx.send(Ok(LoadUpdate::Progress(LoadProgress {
                        current: items.len(),
                        total: None,
                        phase: self.phase,
                    })));
                    self.ctx.request_repaint();
                }
            }
            Ok(items)
        }
    }

    let mut de = serde_json::Deserializer::from_reader(reader);
    let items = de
        .deserialize_seq(ProgressVisitor::<T> {
            phase,
            tx,
            ctx,
            _marker: PhantomData,
        })
        .map_err(AppError::Json)?;

    // Final progress for this phase so the UI sees the exact count.
    let _ = tx.send(Ok(LoadUpdate::Progress(LoadProgress {
        current: items.len(),
        total: None,
        phase,
    })));

    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    /// Streaming round-trip: a well-formed JSON array of numbers should
    /// parse via `stream_parse_vec` and preserve every element.
    #[test]
    fn test_stream_parse_roundtrip() {
        let ctx = egui::Context::default();
        let (tx, rx) = channel::<LoadUpdateMessage>();

        let input: Vec<u64> = (0..100).collect();
        let json = serde_json::to_vec(&input).expect("serialize");

        let parsed: Vec<u64> =
            stream_parse_vec(std::io::Cursor::new(json), "Parsing", &tx, &ctx).expect("parse");

        assert_eq!(parsed.len(), input.len());
        assert_eq!(parsed, input);

        // Drain channel; shouldn't panic.
        drop(tx);
        while rx.try_recv().is_ok() {}
    }

    /// Progress emission: a 100-item input with `PROGRESS_BATCH_SIZE = 16`
    /// must emit at least `100 / 16 = 6` batched updates plus the final
    /// catch-up update, for >= 7 total.
    #[test]
    fn test_stream_parse_emits_progress() {
        let ctx = egui::Context::default();
        let (tx, rx) = channel::<LoadUpdateMessage>();

        let n = 100usize;
        let input: Vec<u64> = (0..n as u64).collect();
        let json = serde_json::to_vec(&input).expect("serialize");

        let _ = stream_parse_vec::<u64, _>(std::io::Cursor::new(json), "Parsing", &tx, &ctx)
            .expect("parse");
        drop(tx);

        let mut progress_count = 0usize;
        let mut last_current = 0usize;
        while let Ok(msg) = rx.try_recv() {
            if let Ok(LoadUpdate::Progress(p)) = msg {
                progress_count += 1;
                last_current = p.current;
                assert_eq!(p.total, None, "streaming phase reports unknown total");
                assert_eq!(p.phase, "Parsing");
            }
        }

        let expected_min = n / PROGRESS_BATCH_SIZE + 1;
        assert!(
            progress_count >= expected_min,
            "expected at least {} progress updates, got {}",
            expected_min,
            progress_count
        );
        assert_eq!(last_current, n, "final progress reports total count");
    }

    /// Malformed JSON must surface as `AppError::Json` rather than
    /// panicking inside the streaming iterator.
    #[test]
    fn test_stream_parse_error_propagates() {
        let ctx = egui::Context::default();
        let (tx, _rx) = channel::<LoadUpdateMessage>();

        let bad = b"[1, 2, not-a-number]";
        let result: Result<Vec<u64>, _> =
            stream_parse_vec(std::io::Cursor::new(&bad[..]), "Parsing", &tx, &ctx);
        match result {
            Err(AppError::Json(_)) => {}
            other => panic!("expected AppError::Json, got {:?}", other.map(|v| v.len())),
        }
    }

    /// Full-pipeline round-trip on real `BenchmarkProblem` data: generated
    /// deterministically via `datagen`, serialized, then stream-parsed.
    /// Verifies that the streaming visitor handles the production-shaped
    /// JSON (nested `HashMap<String, BenchmarkEntry>`, `MatrixMetadata`,
    /// etc.) without drift from the `serde_json::from_reader` baseline.
    #[cfg(feature = "datagen")]
    #[test]
    fn test_stream_parse_benchmark_problem() {
        use crate::data::models::BenchmarkProblem;
        use crate::datagen::{generate_benchmark_file, rng_from_seed, GenConfig};
        use std::path::PathBuf;

        let ctx = egui::Context::default();
        let (tx, _rx) = channel::<LoadUpdateMessage>();

        let config = GenConfig {
            matrices: 40, // > PROGRESS_BATCH_SIZE * 2 so we see multiple updates
            commits: 0,
            seed: 42,
            noise_stddev: 0.08,
            out_dir: PathBuf::from("."),
            commit_sha: None,
            commit_author: None,
            commit_date: None,
            commit_message: None,
            anomaly_rate: 0.10,
        };
        let mut rng = rng_from_seed(config.seed);
        let problems = generate_benchmark_file(&config, &mut rng);

        let json = serde_json::to_vec(&problems).expect("serialize");

        let parsed: Vec<BenchmarkProblem> =
            stream_parse_vec(std::io::Cursor::new(json), "Parsing", &tx, &ctx)
                .expect("stream parse");

        assert_eq!(parsed.len(), problems.len());
        for (orig, rt) in problems.iter().zip(parsed.iter()) {
            assert_eq!(orig.problem.rows, rt.problem.rows);
            assert_eq!(orig.problem.nonzeros, rt.problem.nonzeros);
            assert_eq!(orig.spmv.len(), rt.spmv.len());
        }
    }

    // ---------------------------------------------------------------------
    // Commit-switcher tests (Task 7).
    //
    // `step_commit` and `prefetch_adjacent_commits` are exercised in
    // isolation: neither needs the full load plumbing, so we populate
    // `git.commits` directly with cheap fixtures and assert on the fields
    // that matter.
    // ---------------------------------------------------------------------

    use super::super::git::CommitInfo;

    fn make_commits(n: usize) -> Vec<CommitInfo> {
        (0..n)
            .map(|i| {
                let short = format!("{:07x}", i);
                CommitInfo {
                    sha: format!("{short}0000000000000000000000000000000000"),
                    short_sha: short,
                    author: "Test <t@example.com>".to_string(),
                    date: "2026-01-01T00:00:00Z".to_string(),
                    message: format!("commit {i}"),
                    bench_file: None,
                }
            })
            .collect()
    }

    #[test]
    fn step_commit_clamps_at_zero() {
        let mut dash = Dashboard::new();
        dash.git.commits = make_commits(3);
        dash.git.selected_commit_idx = Some(0);
        dash.step_commit(-1);
        assert_eq!(dash.git.selected_commit_idx, Some(0));
        dash.step_commit(-5);
        assert_eq!(dash.git.selected_commit_idx, Some(0));
    }

    #[test]
    fn step_commit_clamps_at_end() {
        let mut dash = Dashboard::new();
        dash.git.commits = make_commits(3);
        dash.git.selected_commit_idx = Some(2);
        dash.step_commit(1);
        assert_eq!(dash.git.selected_commit_idx, Some(2));
        dash.step_commit(999);
        assert_eq!(dash.git.selected_commit_idx, Some(2));
    }

    #[test]
    fn step_commit_from_none_selects_zero_or_last() {
        let mut dash = Dashboard::new();
        dash.git.commits = make_commits(4);
        dash.git.selected_commit_idx = None;
        dash.step_commit(1);
        assert_eq!(dash.git.selected_commit_idx, Some(0));

        dash.git.selected_commit_idx = None;
        dash.step_commit(-1);
        assert_eq!(dash.git.selected_commit_idx, Some(3));
    }

    #[test]
    fn step_commit_on_empty_is_noop() {
        let mut dash = Dashboard::new();
        assert!(dash.git.commits.is_empty());
        dash.step_commit(1);
        assert_eq!(dash.git.selected_commit_idx, None);
        dash.step_commit(-1);
        assert_eq!(dash.git.selected_commit_idx, None);
    }

    #[test]
    fn prefetch_skips_when_loading() {
        let ctx = egui::Context::default();
        let mut dash = Dashboard::new();
        dash.git.commits = make_commits(3);
        // Register bench files so the prefetch path would otherwise fire.
        for c in dash.git.commits.iter_mut() {
            c.bench_file = Some(PathBuf::from(format!(
                "benchmarks/bench_{}.json",
                c.short_sha
            )));
        }
        dash.git.selected_commit_idx = Some(1);
        dash.loading.is_loading = true;

        dash.prefetch_adjacent_commits(&ctx);
        // is_loading guard must short-circuit before state mutation.
        assert!(!dash.git.prefetch_in_flight);
        assert_eq!(dash.git.last_prefetch_idx, None);
    }

    #[test]
    fn prefetch_respects_last_idx() {
        let ctx = egui::Context::default();
        let mut dash = Dashboard::new();
        dash.git.commits = make_commits(3);
        // All commits already "loaded" → prefetch will find nothing to do
        // but still stamp `last_prefetch_idx`.
        for c in &dash.git.commits {
            let key = Dashboard::commit_dataset_key(c);
            dash.data_selection
                .dataset
                .insert(key, BenchmarkDataset { benchmark: vec![] });
        }
        dash.git.selected_commit_idx = Some(1);

        dash.prefetch_adjacent_commits(&ctx);
        assert_eq!(dash.git.last_prefetch_idx, Some(1));
        assert!(!dash.git.prefetch_in_flight);

        // Second call with the same idx: still a no-op; `last_prefetch_idx`
        // stays at Some(1).
        dash.prefetch_adjacent_commits(&ctx);
        assert_eq!(dash.git.last_prefetch_idx, Some(1));
    }

    #[test]
    fn commit_dataset_key_is_deterministic() {
        let commits = make_commits(1);
        assert_eq!(Dashboard::commit_dataset_key(&commits[0]), "bench_0000000");
    }

    /// `on_commit_selected` on an already-loaded commit key activates the
    /// dataset without touching the loading channel.
    #[test]
    fn on_commit_selected_activates_preloaded_dataset() {
        let ctx = egui::Context::default();
        let mut dash = Dashboard::new();
        dash.git.commits = make_commits(2);
        let key0 = Dashboard::commit_dataset_key(&dash.git.commits[0]);
        dash.data_selection
            .dataset
            .insert(key0.clone(), BenchmarkDataset { benchmark: vec![] });
        // Pretend some other dataset was active before the switch.
        dash.data_selection.active_dataset.push("stale".to_string());

        dash.on_commit_selected(&ctx, 0);

        assert_eq!(dash.git.selected_commit_idx, Some(0));
        assert_eq!(dash.data_selection.active_dataset, vec![key0.clone()]);
        // Comparison view's commit_b tracks the selection.
        assert_eq!(dash.comparison.commit_b.as_deref(), Some(key0.as_str()));
        // No load was kicked off.
        assert!(!dash.loading.is_loading);
    }
}

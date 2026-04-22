#[cfg(not(target_arch = "wasm32"))]
use super::git::{self};
use super::git::{CommitInfo, GitState};
#[cfg(target_arch = "wasm32")]
use super::git_http;
#[cfg(not(target_arch = "wasm32"))]
use super::models::BenchmarkProblem;
use super::models::{BenchmarkDataset, SolverBenchmark};
use super::AppError;
use crate::types::{DataFormat, DataMode, MetricType, PlotType, ProfileFilter, XaxisType};
use crate::visualization::formula::CompiledFormula;
use crate::visualization::plotting::PlotData;
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

pub struct Dashboard {
    pub view_mode: ViewMode,
    pub plot_config: PlotConfig,
    pub data_selection: DataSelection,
    pub loading: LoadingState,
    pub solver: SolverState,
    pub plot_cache: PlotCache,
    pub git: GitState,
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
            git: GitState::default(),
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
                self.data_selection.active_dataset.push(file_name.clone());
                self.data_selection.dataset.insert(file_name, dataset);
                self.data_selection.sorted_dataset_keys =
                    self.data_selection.dataset.keys().cloned().collect();
                self.data_selection.sorted_dataset_keys.sort();
            }
            LoadResult::Solver(solver_bench) => {
                self.solver.data = Some(solver_bench);
            }
            LoadResult::Git(commits) => {
                self.git.commits = commits;
                self.git.selected_commit_idx = None;
            }
        }
    }

    fn apply_load_error(&mut self, e: AppError) {
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
}

use super::models::{BenchmarkDataset, BenchmarkProblem, SolverBenchmark};
use super::AppError;
use crate::types::{DataFormat, DataMode, MetricType, PlotType, ProfileFilter, XaxisType};
use crate::visualization::plotting::PlotData;
use egui_file_dialog::FileDialog;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

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

pub struct Dashboard {
    pub view_mode: ViewMode,
    pub solver_data: Option<Vec<SolverBenchmark>>,

    pub solver_selected_idx: usize,
    pub solver_selected_methods: HashSet<String>,
    pub solver_selected_detail_method: Option<String>,
    pub show_percentile_bands: bool,
    pub solver_x_axis: SolverXAxis,
    pub solver_log_scale: bool,

    pub solver_show_recurrent: bool,
    pub solver_show_true: bool,
    pub solver_show_implicit: bool,
    pub solver_show_timestamp: bool,
    pub mode: Option<DataMode>,
    pub dataset: HashMap<String, BenchmarkDataset>,
    pub active_dataset: Vec<String>,
    pub x_axis: Option<XaxisType>,
    pub data_metric: Option<MetricType>,
    pub active_formats: HashMap<DataFormat, bool>,
    pub baseline_format: DataFormat,

    pub normalize: bool,
    pub file_dialog: FileDialog,
    pub picked_file: Option<PathBuf>,
    pub plot_type: PlotType,
    pub profile_filter: ProfileFilter,
    pub last_error: Option<String>,
    pub cached_plot_data: Option<Rc<PlotData>>,
    pub plot_cache_key: String,
    pub filter_outliers: bool,
    pub log_scale_x: bool,

    pub sorted_dataset_keys: Vec<String>,

    pub is_loading: bool,

    pub rx_load: Option<Receiver<Result<(String, LoadResult), AppError>>>,
}

pub enum LoadResult {
    Benchmark(BenchmarkDataset),
    Solver(Vec<SolverBenchmark>),
}

impl Dashboard {
    pub fn new() -> Self {
        Self {
            view_mode: ViewMode::Benchmark,
            solver_data: None,
            solver_selected_idx: 0,
            solver_selected_methods: HashSet::new(),
            solver_selected_detail_method: None,
            solver_x_axis: SolverXAxis::Iteration,
            solver_log_scale: true,

            solver_show_recurrent: true,
            solver_show_true: true,
            solver_show_implicit: true,
            solver_show_timestamp: false,
            mode: None,
            show_percentile_bands: false,
            dataset: HashMap::new(),
            active_dataset: Vec::new(),
            x_axis: Some(XaxisType::NonZeros),
            active_formats: HashMap::new(),
            baseline_format: DataFormat::CSR,

            normalize: false,
            data_metric: Some(MetricType::Time),
            file_dialog: FileDialog::new(),
            picked_file: None,
            plot_type: PlotType::Scatter,
            profile_filter: ProfileFilter::None,
            last_error: None,
            cached_plot_data: None,
            plot_cache_key: String::new(),
            filter_outliers: false,
            log_scale_x: false,
            sorted_dataset_keys: Vec::new(),
            is_loading: false,
            rx_load: None,
        }
    }

    pub fn encode_file(&self) -> String {
        let file_stem = self
            .picked_file
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("unknown_file");

        let time = chrono::Utc::now().format("%H-%M-%S").to_string();
        format!("{}#{}", file_stem, time)
    }

    pub fn process_file(&mut self) {
        if let Some(file_path) = self.picked_file.clone() {
            let file_name = self.encode_file();
            let view_mode = self.view_mode;

            let (tx, rx): (
                std::sync::mpsc::Sender<Result<(String, LoadResult), AppError>>,
                std::sync::mpsc::Receiver<Result<(String, LoadResult), AppError>>,
            ) = channel();

            self.is_loading = true;
            self.rx_load = Some(rx);
            self.last_error = None;

            thread::spawn(move || {
                let result = (|| -> Result<LoadResult, AppError> {
                    let file = std::fs::File::open(&file_path)?;
                    let reader = std::io::BufReader::new(file);

                    match view_mode {
                        ViewMode::Benchmark => {
                            let raw_data: Vec<BenchmarkProblem> = serde_json::from_reader(reader)?;
                            let processed_data = super::loader::process_benchmark_data(raw_data)?;
                            Ok(LoadResult::Benchmark(processed_data))
                        }
                        ViewMode::Solver => {
                            let solver_data: Vec<SolverBenchmark> =
                                serde_json::from_reader(reader)?;
                            Ok(LoadResult::Solver(solver_data))
                        }
                    }
                })();

                let _ = tx.send(result.map(|data| (file_name, data)));
            });
        }
    }

    pub fn check_loading_status(&mut self) {
        if let Some(rx) = &self.rx_load {
            if let Ok(result) = rx.try_recv() {
                self.is_loading = false;
                self.is_loading = false;
                self.rx_load = None;

                match result {
                    Ok((file_name, data)) => match data {
                        LoadResult::Benchmark(dataset) => {
                            self.active_dataset.push(file_name.clone());
                            self.dataset.insert(file_name, dataset);
                            self.sorted_dataset_keys = self.dataset.keys().cloned().collect();
                            self.sorted_dataset_keys.sort();
                        }
                        LoadResult::Solver(solver_bench) => {
                            self.solver_data = Some(solver_bench);
                        }
                    },
                    Err(e) => {
                        self.last_error = Some(format!("Error loading file: {}", e));
                    }
                }
            }
        }
    }
}

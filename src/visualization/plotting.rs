use crate::data::models::BenchmarkDataset;
use crate::types::{DataFormat, DataMode, MetricType, PlotType, ProfileFilter, XaxisType};
use crate::visualization::formula::CompiledFormula;
use egui::Color32;
use egui_plot::MarkerShape;
use smol_str::SmolStr;
use std::collections::HashMap;

// Re-export specific items for other modules to use
pub use self::filter::FilterStats;

use super::filter;
use super::profile::generate_performance_profile_data;
use super::scatter;

#[derive(Clone, Debug)]
pub enum PointMetadata {
    Scatter {
        problem_name: SmolStr,
        rows: u64,
        cols: u64,
        nonzeros: u64,
        sparsity: f64,
        format: DataFormat,
        label: SmolStr,
    },
    Profile {
        ratio: f64,
        probability: f64,
    },
}

#[derive(Clone)]
pub struct PlotSeries {
    pub name: String,
    pub points: Vec<[f64; 2]>,
    pub metadata: Vec<PointMetadata>,
    pub marker: MarkerShape,
    pub color: Color32,
    pub is_auxiliary: bool,
}

/// Contains all data necessary to render a plot.
#[derive(Clone)]
pub struct PlotData {
    pub series: Vec<PlotSeries>,
    pub filter_stats: FilterStats,
}

// TODO(task-6): collapse these args into a config struct when the PlotKind
// abstraction lands (additional plot types task).
#[allow(clippy::too_many_arguments)]
pub fn generate_plot_data(
    dataset: &HashMap<String, BenchmarkDataset>,
    active_dataset: &[String],
    active_formats: &HashMap<DataFormat, bool>,
    mode: Option<DataMode>,
    x_axis: Option<XaxisType>,
    data_metric: Option<MetricType>,
    baseline_format: DataFormat,
    normalize: bool,
    filter_outliers: bool,
    plot_type: PlotType,
    profile_filter: ProfileFilter,
    log_scale_x: bool,
    show_percentile_bands: bool,
    formula: Option<&CompiledFormula>,
) -> PlotData {
    if matches!(plot_type, PlotType::PerformanceProfile) {
        return generate_performance_profile_data(
            dataset,
            active_dataset,
            active_formats,
            mode,
            data_metric,
            profile_filter,
            log_scale_x,
            formula,
        );
    }

    scatter::generate_scatter_plot_data(
        dataset,
        active_dataset,
        active_formats,
        mode,
        x_axis,
        data_metric,
        baseline_format,
        normalize,
        filter_outliers,
        show_percentile_bands,
        formula,
    )
}

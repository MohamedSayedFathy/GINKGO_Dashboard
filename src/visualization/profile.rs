use super::filter::{filter_ratios, get_formats_to_plot, FilterStats};
use super::formula::CompiledFormula;
use super::math::{calculate_method_ratios, generate_step_function};
use super::plotting::{PlotData, PlotSeries};
use super::utils::{adjust_color_for_dataset, get_format_color, get_y_value};

use crate::data::models::BenchmarkDataset;
use crate::types::{DataFormat, DataMode, MetricType, ProfileFilter};
use egui_plot::MarkerShape;
use smol_str::SmolStr;
use std::collections::{HashMap, HashSet};

// TODO(task-6): collapse these args into a config struct when the PlotKind
// abstraction lands (additional plot types task).
#[allow(clippy::too_many_arguments)]
pub fn generate_performance_profile_data(
    dataset: &HashMap<String, BenchmarkDataset>,
    active_dataset: &[String],
    active_formats: &HashMap<DataFormat, bool>,
    mode: Option<DataMode>,
    data_metric: Option<MetricType>,
    profile_filter: ProfileFilter,
    log_scale_x: bool,
    formula: Option<&CompiledFormula>,
) -> PlotData {
    let mut filter_stats = FilterStats::default();

    let metric_type = match data_metric {
        Some(m) => m,
        None => {
            return PlotData {
                series: vec![],
                filter_stats,
            };
        }
    };

    // Custom formulas default to higher-is-better. Users expressing a cost
    // metric (e.g. raw time) should wrap it as a ratio like `baseline_time / time`
    // to get the expected orientation on the performance profile.
    let lower_is_better = matches!(
        metric_type,
        MetricType::Time | MetricType::Storage | MetricType::Repetitions
    );

    let formats_to_plot = get_formats_to_plot(mode, active_formats);

    let problem_results = aggregate_problem_results(
        dataset,
        active_dataset,
        &formats_to_plot,
        &metric_type,
        &mut filter_stats,
        formula,
    );

    let mut unique_problems = HashSet::new();
    for ds_name in active_dataset {
        if let Some(ds) = dataset.get(ds_name) {
            for prob in &ds.benchmark {
                unique_problems.insert(&prob.problem.name);
            }
        }
    }
    let total_problems = unique_problems.len() as f64;
    if total_problems == 0.0 {
        return PlotData {
            series: vec![],
            filter_stats,
        };
    }

    let mut method_ratios = calculate_method_ratios(&problem_results, lower_is_better);

    filter_ratios(&mut method_ratios, profile_filter, &mut filter_stats);

    let final_total_problems = total_problems;

    let mut series_data = Vec::new();

    let mut all_dataset_names: Vec<_> = dataset.keys().collect();
    all_dataset_names.sort();

    for dataset_name in active_dataset.iter() {
        let stable_idx = all_dataset_names
            .iter()
            .position(|&n| n == dataset_name)
            .unwrap_or(0);

        for format in &formats_to_plot {
            let key = (dataset_name.clone(), *format);
            if let Some(ratios) = method_ratios.get_mut(&key) {
                let (points, metadata) =
                    generate_step_function(ratios, final_total_problems, log_scale_x);

                let name = format!("{} - {:?}", dataset_name, format);
                let base_color = get_format_color(format);
                let color = adjust_color_for_dataset(base_color, stable_idx);

                let band_color = color;

                series_data.push(PlotSeries {
                    name,
                    points,
                    marker: MarkerShape::Circle,
                    color: band_color,
                    metadata,
                    is_auxiliary: false,
                });
            }
        }
    }

    filter_stats.shown_matrices = method_ratios.values().map(|v| v.len()).sum();

    PlotData {
        series: series_data,
        filter_stats,
    }
}

fn aggregate_problem_results(
    dataset: &HashMap<String, BenchmarkDataset>,
    active_dataset: &[String],
    formats_to_plot: &[DataFormat],
    metric_type: &MetricType,
    stats: &mut FilterStats,
    formula: Option<&CompiledFormula>,
) -> HashMap<SmolStr, HashMap<(String, DataFormat), f64>> {
    let mut problem_results: HashMap<SmolStr, HashMap<(String, DataFormat), f64>> = HashMap::new();

    for dataset_name in active_dataset {
        if let Some(ds) = dataset.get(dataset_name) {
            for problem in &ds.benchmark {
                let problem_name = problem.problem.name.clone();
                for format in formats_to_plot {
                    stats.total_matrices += 1;
                    // Performance profile has no concept of a "baseline format" for Custom
                    // formulas — pass None for baseline so baseline_* vars return None.
                    if let Some(val) = get_y_value(
                        &problem.spmv,
                        format,
                        metric_type,
                        &problem.problem,
                        None,
                        formula,
                    ) {
                        if val.is_finite() && val > 0.0 {
                            problem_results
                                .entry(problem_name.clone())
                                .or_default()
                                .insert((dataset_name.clone(), *format), val);
                        } else {
                            stats.filtered_invalid_values += 1;
                        }
                    } else {
                        stats.filtered_no_format_data += 1;
                    }
                }
            }
        }
    }
    problem_results
}

use super::filter::{filter_outliers, get_formats_to_plot, FilterStats};
use super::formula::CompiledFormula;
use super::math::{calculate_normalized_value, calculate_percentile_bands};
use super::plotting::{PlotData, PlotSeries, PointMetadata};
use super::utils::{adjust_color_for_dataset, get_format_color, get_x_value, get_y_value};
use crate::data::models::BenchmarkDataset;
use crate::types::{DataFormat, DataMode, MetricType, XaxisType};
use egui_plot::MarkerShape;
use smol_str::SmolStr;
use std::collections::HashMap;

// TODO(task-6): collapse these args into a config struct when the PlotKind
// abstraction lands (additional plot types task).
#[allow(clippy::too_many_arguments)]
pub fn generate_scatter_plot_data(
    dataset: &HashMap<String, BenchmarkDataset>,
    active_dataset: &[String],
    active_formats: &HashMap<DataFormat, bool>,
    mode: Option<DataMode>,
    x_axis: Option<XaxisType>,
    data_metric: Option<MetricType>,
    baseline_format: DataFormat,
    normalize: bool,
    should_filter_outliers: bool,
    show_percentile_bands: bool,
    formula: Option<&CompiledFormula>,
) -> PlotData {
    let mut series_data = Vec::new();

    let mut filter_stats = FilterStats::default();

    let (x_axis_type, metric_type) = match (x_axis, data_metric) {
        (Some(x), Some(m)) => (x, m),
        _ => {
            return PlotData {
                series: vec![],
                filter_stats,
            };
        }
    };

    let mut baseline_values: HashMap<String, HashMap<SmolStr, f64>> = HashMap::new();
    if normalize {
        for dataset_name in active_dataset {
            if let Some(ds) = dataset.get(dataset_name) {
                let mut problem_baselines = HashMap::new();
                for problem in &ds.benchmark {
                    // Baseline entry for normalization is the baseline_format row of this problem.
                    let bl_entry = problem.spmv.get(baseline_format.as_key());
                    if let Some(y_base) = get_y_value(
                        &problem.spmv,
                        &baseline_format,
                        &metric_type,
                        &problem.problem,
                        bl_entry,
                        formula,
                    ) {
                        problem_baselines.insert(problem.problem.name.clone(), y_base);
                    }
                }
                baseline_values.insert(dataset_name.clone(), problem_baselines);
            }
        }
    }

    let formats_to_plot = get_formats_to_plot(mode, active_formats);

    // Create a stable ordering of all available datasets to ensure consistent colors
    let mut all_dataset_names: Vec<_> = dataset.keys().collect();
    all_dataset_names.sort();

    for dataset_name in active_dataset.iter() {
        if let Some(ds) = dataset.get(dataset_name) {
            let stable_idx = all_dataset_names
                .iter()
                .position(|&n| n == dataset_name)
                .unwrap_or(0);

            for format in &formats_to_plot {
                let mut points = Vec::new();
                let mut metadata = Vec::new();
                for problem in &ds.benchmark {
                    filter_stats.total_matrices += 1;

                    let x = get_x_value(&problem.problem, x_axis_type);
                    // Baseline entry for Custom formula evaluation (not for normalization).
                    let baseline_entry = problem.spmv.get(baseline_format.as_key());
                    let raw_y = get_y_value(
                        &problem.spmv,
                        format,
                        &metric_type,
                        &problem.problem,
                        baseline_entry,
                        formula,
                    );

                    if let Some(mut y_val) = raw_y {
                        if normalize {
                            let base_y_opt = baseline_values
                                .get(dataset_name)
                                .and_then(|m| m.get(&problem.problem.name));

                            if let Some(&base_y) = base_y_opt {
                                if base_y != 0.0
                                    && y_val != 0.0
                                    && base_y.is_finite()
                                    && y_val.is_finite()
                                {
                                    y_val = calculate_normalized_value(y_val, base_y, &metric_type);
                                } else {
                                    filter_stats.filtered_invalid_values += 1;
                                    continue;
                                }
                            } else {
                                filter_stats.filtered_no_format_data += 1;
                                continue;
                            }
                        }

                        if y_val.is_finite() {
                            points.push([x, y_val]);
                            filter_stats.shown_matrices += 1;

                            let _name = format!("{} - {:?}", dataset_name, format);

                            let label = if normalize {
                                if matches!(metric_type, MetricType::Time | MetricType::Repetitions)
                                {
                                    "Speedup"
                                } else {
                                    "Ratio"
                                }
                            } else {
                                ""
                            };

                            metadata.push(PointMetadata::Scatter {
                                problem_name: problem.problem.name.clone(),
                                rows: problem.problem.rows,
                                cols: problem.problem.cols,
                                nonzeros: problem.problem.nonzeros,
                                sparsity: problem.problem.sparsity,
                                format: *format,
                                label: label.into(),
                            });
                        } else {
                            filter_stats.filtered_invalid_values += 1;
                        }
                    } else {
                        filter_stats.filtered_no_format_data += 1;
                    }
                }

                points.sort_by(|a, b| a[0].total_cmp(&b[0]));

                let name = format!("{} - {:?}", dataset_name, format);
                let base_color = get_format_color(format);
                let color = adjust_color_for_dataset(base_color, stable_idx);

                if !points.is_empty() {
                    let bands = if show_percentile_bands && points.len() >= 5 {
                        let (p25, p75) = calculate_percentile_bands(&points, 10);
                        Some((p25, p75))
                    } else {
                        None
                    };

                    let metadata = metadata;

                    series_data.push(PlotSeries {
                        name: name.clone(),
                        points,
                        metadata,
                        marker: MarkerShape::Circle,
                        color,
                        is_auxiliary: false,
                    });

                    if let Some((p25_band, p75_band)) = bands {
                        if !p25_band.is_empty() {
                            let band_color = color;

                            series_data.push(PlotSeries {
                                name: name.clone(),
                                points: p25_band,
                                metadata: vec![],
                                marker: MarkerShape::Circle,
                                color: band_color,
                                is_auxiliary: true,
                            });

                            series_data.push(PlotSeries {
                                name: name.clone(),
                                points: p75_band,
                                metadata: vec![],
                                marker: MarkerShape::Circle,
                                color: band_color,
                                is_auxiliary: true,
                            });
                        }
                    }
                }
            }
        }
    }

    if should_filter_outliers {
        filter_outliers(&mut series_data, &mut filter_stats);
    }

    PlotData {
        series: series_data,
        filter_stats,
    }
}

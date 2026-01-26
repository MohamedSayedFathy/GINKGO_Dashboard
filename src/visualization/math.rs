use crate::types::{DataFormat, MetricType};
use crate::visualization::plotting::PointMetadata;
use smol_str::SmolStr;
use std::collections::HashMap;

/// Helper to calculate normalized value.
pub fn calculate_normalized_value(current: f64, baseline: f64, metric: &MetricType) -> f64 {
    match metric {
        MetricType::Time | MetricType::Repetitions => baseline / current,
        _ => current / baseline,
    }
}

/// Calculates 25th and 75th percentiles using sliding window
/// Optimized to reuse buffer and avoid full sort.
pub fn calculate_percentile_bands(
    points: &[[f64; 2]],
    window_size: usize,
) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
    if points.is_empty() {
        return (vec![], vec![]);
    }

    let sorted_points = points;

    let mut p25_band = Vec::with_capacity(sorted_points.len());
    let mut p75_band = Vec::with_capacity(sorted_points.len());

    // reusable buffer to avoid allocations in loop
    let mut window_y = Vec::with_capacity(window_size * 2);

    for i in 0..sorted_points.len() {
        let start = i.saturating_sub(window_size);
        let end = (i + window_size + 1).min(sorted_points.len());

        window_y.clear();
        window_y.extend(sorted_points[start..end].iter().map(|p| p[1]));

        if window_y.is_empty() {
            continue;
        }

        let len = window_y.len();
        let idx_25 = ((len as f64 * 0.25) as usize).min(len - 1);
        let idx_75 = ((len as f64 * 0.75) as usize).min(len - 1);

        let (_, p25_val, _) = window_y.select_nth_unstable_by(idx_25, |a, b| a.total_cmp(b));
        let val_25 = *p25_val;

        let (_, p75_val, _) = window_y.select_nth_unstable_by(idx_75, |a, b| a.total_cmp(b));
        let val_75 = *p75_val;

        let x = sorted_points[i][0];
        p25_band.push([x, val_25]);
        p75_band.push([x, val_75]);
    }

    (p25_band, p75_band)
}

pub fn generate_step_function(
    ratios: &[f64],
    total_problems: f64,
    log_scale: bool,
) -> (Vec<[f64; 2]>, Vec<PointMetadata>) {
    let mut points = Vec::new();
    let mut metadata = Vec::new();

    if ratios.is_empty() {
        return (points, metadata);
    }

    let transform_x = |ratio: f64| -> f64 {
        if log_scale {
            if ratio >= 1.0 {
                ratio.log10()
            } else {
                0.0
            }
        } else {
            ratio
        }
    };

    let start_x = transform_x(1.0);
    points.push([start_x, 0.0]);
    metadata.push(PointMetadata::Profile {
        ratio: 1.0,
        probability: 0.0,
    });

    let mut current_y = 0.0;

    for (i, &ratio) in ratios.iter().enumerate() {
        let next_y = (i as f64 + 1.0) / total_problems;
        let x = transform_x(ratio);

        points.push([x, current_y]);
        metadata.push(PointMetadata::Profile {
            ratio,
            probability: current_y,
        });

        points.push([x, next_y]);
        metadata.push(PointMetadata::Profile {
            ratio,
            probability: next_y,
        });

        current_y = next_y;
    }

    (points, metadata)
}

/// Calculates ratios for performance profile
pub fn calculate_method_ratios(
    problem_results: &HashMap<SmolStr, HashMap<(String, DataFormat), f64>>,
    lower_is_better: bool,
) -> HashMap<(String, DataFormat), Vec<f64>> {
    let mut method_ratios: HashMap<(String, DataFormat), Vec<f64>> = HashMap::new();

    for results in problem_results.values() {
        let best_val = if lower_is_better {
            results.values().fold(f64::INFINITY, |a, &b| a.min(b))
        } else {
            results.values().fold(f64::NEG_INFINITY, |a, &b| a.max(b))
        };

        for ((ds_name, format), val) in results {
            let ratio = if lower_is_better {
                // Time: val / best (>= 1.0)
                val / best_val
            } else {
                // GFlops: best / val (>= 1.0)
                best_val / val
            };

            if ratio.is_finite() && ratio >= 1.0 {
                method_ratios
                    .entry((ds_name.clone(), *format))
                    .or_default()
                    .push(ratio);
            }
        }
    }
    for ratios in method_ratios.values_mut() {
        ratios.sort_by(|a, b| a.total_cmp(b));
    }
    method_ratios
}

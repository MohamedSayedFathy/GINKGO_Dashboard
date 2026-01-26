use crate::types::{DataFormat, DataMode, ProfileFilter};
use crate::visualization::plotting::PlotSeries;
use std::collections::HashMap;

/// Statistics about data filtering
#[derive(Clone, Debug)]
pub struct FilterStats {
    pub total_matrices: usize,
    pub shown_matrices: usize,
    pub filtered_missing_time: usize,
    pub filtered_no_format_data: usize,
    pub filtered_invalid_values: usize,
    pub filtered_outliers: usize,
}

impl Default for FilterStats {
    fn default() -> Self {
        Self {
            total_matrices: 0,
            shown_matrices: 0,
            filtered_missing_time: 0,
            filtered_no_format_data: 0,
            filtered_invalid_values: 0,
            filtered_outliers: 0,
        }
    }
}

/// Determines which data formats to include in the plot based on mode and active selection.
pub fn get_formats_to_plot(
    mode: Option<DataMode>,
    active_formats: &HashMap<DataFormat, bool>,
) -> Vec<DataFormat> {
    if let Some(DataMode::Single) = mode {
        vec![
            DataFormat::CSR,
            DataFormat::COO,
            DataFormat::ELL,
            DataFormat::HYBRID,
            DataFormat::SELLP,
        ]
    } else if let Some(DataMode::Multi) = mode {
        active_formats
            .iter()
            .filter(|(_, active)| **active)
            .map(|(k, _)| *k)
            .collect()
    } else {
        Vec::new()
    }
}

/// Applies outlier filtering to a list of series based on Y-axis values.
pub fn filter_outliers(series_data: &mut [PlotSeries], stats: &mut FilterStats) {
    let total_points: usize = series_data
        .iter()
        .filter(|s| !s.is_auxiliary)
        .map(|s| s.points.len())
        .sum();

    if total_points == 0 {
        return;
    }

    let mut all_y_values = Vec::with_capacity(total_points);
    for s in series_data.iter() {
        if !s.is_auxiliary {
            all_y_values.extend(s.points.iter().map(|p| p[1]));
        }
    }

    // HPC Optimization: Use O(N) selection instead of O(N log N) sort
    let k5 = (total_points as f64 * 0.05) as usize;
    let k95 = (total_points as f64 * 0.95) as usize;

    let k5 = k5.min(total_points.saturating_sub(1));
    let k95 = k95.min(total_points.saturating_sub(1));

    all_y_values.select_nth_unstable_by(k5, |a, b| a.total_cmp(b));
    let y_min = all_y_values[k5];

    if k95 > k5 {
        all_y_values[k5..].select_nth_unstable_by(k95 - k5, |a, b| a.total_cmp(b));
    }
    let y_max = all_y_values[k95];

    for series in series_data.iter_mut() {
        if series.is_auxiliary {
            continue;
        }

        let original_len = series.points.len();
        series.points.retain(|p| p[1] >= y_min && p[1] <= y_max);
        let removed = original_len - series.points.len();

        stats.shown_matrices = stats.shown_matrices.saturating_sub(removed);
        stats.filtered_outliers += removed;
    }
}

pub fn filter_ratios(
    method_ratios: &mut HashMap<(String, DataFormat), Vec<f64>>,
    profile_filter: ProfileFilter,
    stats: &mut FilterStats,
) {
    match profile_filter {
        ProfileFilter::TrimPercent(p) => {
            if p > 0.0 && p < 100.0 {
                for ratios in method_ratios.values_mut() {
                    if ratios.is_empty() {
                        continue;
                    }
                    // Ensure sorted for correct percentile trimming (removing worst/largest values)
                    ratios.sort_by(|a, b| a.total_cmp(b));
                    let n = ratios.len();
                    let remove_count = ((n as f64) * (p / 100.0)).round() as usize;

                    if remove_count < n {
                        stats.filtered_outliers += remove_count;
                        ratios.truncate(n - remove_count);
                    }
                }
            }
        }
        ProfileFilter::MaxTau(t_max) => {
            for ratios in method_ratios.values_mut() {
                let initial_len = ratios.len();
                ratios.retain(|&r| r <= t_max);
                stats.filtered_outliers += initial_len - ratios.len();
            }
        }
        _ => {}
    }
}

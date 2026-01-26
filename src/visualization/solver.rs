use crate::data::models::SolverResult;
use crate::data::state::SolverXAxis;
use egui::Color32;
use egui_plot::PlotPoints;
use std::collections::{HashMap, HashSet};

#[allow(dead_code)]
/// Configuration options for solver plots.
///
/// Controls which data series are displayed and how axes are scaled.
pub struct SolverPlotOptions {
    pub show_recurrent: bool,
    pub show_true: bool,
    pub show_implicit: bool,
    pub show_timestamp: bool,
    pub log_scale: bool,
    pub x_axis: SolverXAxis,
}

/// Statistics comparing the current solver against others.
///
/// Used to display relative performance metrics in the UI.
pub struct ComparisonStats {
    pub total_time: f64,
    pub final_residual: Option<f64>,
    pub is_fastest: bool,
    pub slowdown_factor: f64,
    pub fastest_solver_name: String,
}

/// Returns the color palette for a given solver index.
///
/// Returns a tuple of (Recurrent Color, True Color, Implicit Color).
/// Colors are cycled through a predefined palette of 6 variations.
pub fn get_solver_colors(index: usize) -> (Color32, Color32, Color32) {
    let recurrent_colors = [
        Color32::from_rgb(30, 144, 255),  // Dodger Blue
        Color32::from_rgb(65, 105, 225),  // Royal Blue
        Color32::from_rgb(100, 149, 237), // Cornflower Blue
        Color32::from_rgb(70, 130, 180),  // Steel Blue
        Color32::from_rgb(95, 158, 160),  // Cadet Blue
        Color32::from_rgb(0, 191, 255),   // Deep Sky Blue
    ];
    let true_colors = [
        Color32::from_rgb(220, 20, 60), // Crimson
        Color32::from_rgb(255, 69, 0),  // Orange Red
        Color32::from_rgb(255, 99, 71), // Tomato
        Color32::from_rgb(255, 140, 0), // Dark Orange
        Color32::from_rgb(255, 165, 0), // Orange
        Color32::from_rgb(205, 92, 92), // Indian Red
    ];
    let implicit_colors = [
        Color32::from_rgb(34, 139, 34),  // Forest Green
        Color32::from_rgb(50, 205, 50),  // Lime Green
        Color32::from_rgb(60, 179, 113), // Medium Sea Green
        Color32::from_rgb(46, 139, 87),  // Sea Green
        Color32::from_rgb(0, 128, 0),    // Green
        Color32::from_rgb(124, 252, 0),  // Lawn Green
    ];

    let i = index % 6;
    (recurrent_colors[i], true_colors[i], implicit_colors[i])
}

/// Generates plotting points from raw data series.
///
/// Handles X-axis mapping (Iteration vs Time) and Y-axis scaling (Linear vs Log).
pub fn get_plot_points(
    data: &[f64],
    times: Option<&Vec<f64>>,
    x_axis: SolverXAxis,
    log_scale: bool,
) -> PlotPoints<'static> {
    data.iter()
        .enumerate()
        .map(|(i, &val)| {
            let x = match x_axis {
                SolverXAxis::Iteration => i as f64,
                SolverXAxis::Time => {
                    if let Some(ts) = times {
                        *ts.get(i).unwrap_or(&(i as f64))
                    } else {
                        i as f64
                    }
                }
            };

            // Determine Y coordinate, applying log scale if requested
            // Note: We clamp <= 0 values to -16.0 in log mode to avoid NaN/Inf issues
            let y = if log_scale {
                if val > 0.0 {
                    val.log10()
                } else {
                    -16.0
                }
            } else {
                val
            };
            [x, y]
        })
        .collect()
}

/// Calculates performance statistics for a specific method compared to all other selected methods.
///
/// metrics include:
/// - Total execution time (Generate + Apply)
/// - Final residual value
/// - Relative slowdown factor compared to the fastest selected method
pub fn calculate_comparison_stats(
    method_name: &str,
    current_result: &SolverResult,
    all_results: &HashMap<String, SolverResult>,
    selected_methods: &HashSet<String>,
) -> ComparisonStats {
    let current_total_time = current_result.generate.time + current_result.apply.time;

    let mut fastest_time = current_total_time;
    let mut fastest_name = method_name.to_string();

    for method in selected_methods {
        if let Some(other_result) = all_results.get(method) {
            let other_time = other_result.generate.time + other_result.apply.time;
            if other_time < fastest_time {
                fastest_time = other_time;
                fastest_name = method.clone();
            }
        }
    }

    let final_residual = current_result
        .recurrent_residuals
        .as_ref()
        .and_then(|r| r.last())
        .or_else(|| {
            current_result
                .true_residuals
                .as_ref()
                .and_then(|r| r.last())
        })
        .cloned();

    ComparisonStats {
        total_time: current_total_time,
        final_residual,
        is_fastest: fastest_name == method_name,
        slowdown_factor: if fastest_time > 0.0 {
            current_total_time / fastest_time
        } else {
            1.0
        },
        fastest_solver_name: fastest_name,
    }
}

use crate::types::PlotType;
use crate::visualization::plotting::PointMetadata;
use egui_plot::PlotPoint;

/// Generates tooltip text for a given point on a plot.
///
/// This function handles two types of plots:
/// 1. **Performance Profile**: Uses step-function logic to find the active step at the cursor's X position.
/// 2. **Scatter Plot**: Uses nearest-neighbor logic to find the closest data point.
pub fn generate_tooltip_text(
    name: &str,
    point: &PlotPoint,
    series_points: &[PlotPoint],
    metadata: &[PointMetadata],
    plot_type: PlotType,
) -> String {
    let mut best_idx = None;
    let mut min_dist = f64::MAX;
    let is_profile = plot_type == PlotType::PerformanceProfile;

    if is_profile {
        // Find the index of the point defining the current step (the last point with x <= cursor_x).
        // Since `partition_point` returns the first index where the predicate is false (i.e., x > cursor_x),
        // we subtract 1 to get the point to the left.
        let idx = series_points.partition_point(|p| p.x <= point.x);
        let step_idx = idx.saturating_sub(1);

        if step_idx < series_points.len() {
            let p = series_points[step_idx];

            // In a step function, the Y value is constant from the current point until the next point.
            // We check the vertical distance from the cursor to this constant Y level.
            let dist_y = (p.y - point.y).abs();

            // Use a relaxed tolerance (0.05) to make hovering over lines easier for the user.
            if dist_y < 0.05 {
                best_idx = Some(step_idx);
            }
        }
    } else {
        let idx = series_points.partition_point(|p| p.x < point.x);
        for i in [idx.saturating_sub(1), idx] {
            if i < series_points.len() {
                let p = series_points[i];
                let dist_x = (p.x - point.x).abs();
                let dist_y = (p.y - point.y).abs();

                // Use a strict tolerance (1e-5) to ensure we only show tooltips when hovering exactly over a point.
                if dist_x < 1e-5 && dist_y < 1e-5 {
                    if dist_x < min_dist {
                        min_dist = dist_x;
                        best_idx = Some(i);
                    }
                }
            }
        }
    }

    if let Some(i) = best_idx {
        if i < metadata.len() {
            let meta = &metadata[i];
            match meta {
                PointMetadata::Scatter {
                    problem_name,
                    rows,
                    cols,
                    nonzeros,
                    sparsity,
                    format,
                    label,
                } => {
                    return format!(
                        "Dataset: {}\nProblem: {}\nRows: {}\nCols: {}\nNNZ: {}\nSparsity: {:.4}\n\nFormat: {:?}\nX: {:.4}\nY: {:.4} {}",
                        name.split(" - ").next().unwrap_or("?"), 
                        problem_name,
                        rows,
                        cols,
                        nonzeros,
                        sparsity,
                        format,
                        point.x,
                        point.y,
                        label
                    );
                }
                PointMetadata::Profile { ratio, probability } => {
                    return format!(
                        "Method: {}\nPerformance Ratio (t): {:.2}\nProbability (p): {:.2}",
                        name, ratio, probability
                    );
                }
            }
        }
    }

    // Fallback
    format!("{}\nX: {:.2}\nY: {:.2}", name, point.x, point.y)
}

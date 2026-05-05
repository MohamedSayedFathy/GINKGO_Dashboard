//! Dashboard-facing entry points for vector export (Task 10).
//!
//! Keeps the heavy SVG-building / PDF-conversion logic in dedicated files
//! (see `svg.rs`, `pdf.rs`) and restricts this module to the glue code the
//! UI layer actually calls: "build bytes for the currently-visible plot,
//! pop the save dialog, stash the payload until the user picks a path."
//!
//! Splitting this from `state.rs` keeps the state module free of
//! plot-type-specific knowledge — the rest of `Dashboard` doesn't need to
//! know that exports exist, only the export buttons in the sidebar do.

use std::rc::Rc;

use crate::data::state::Dashboard;
#[cfg(not(target_arch = "wasm32"))]
use crate::data::state::ExportKind;
use crate::export::svg::{
    build_plot_data_svg, build_stacked_bar_svg, build_timeseries_svg, render_comparison_pair,
    ExportConfig,
};
use crate::types::{MetricType, PlotType, XaxisType};
use crate::visualization::compare::ComparisonPlotData;
use crate::visualization::plotting::PlotData;
use crate::visualization::stacked_bar::StackedBarData;
use crate::visualization::timeseries::TimeseriesData;

/// Compose an `ExportConfig` with axis labels derived from the current
/// plot-config state. Titles default to the plot-type name so empty
/// exports still carry enough context to identify them at a glance.
fn current_config(app: &Dashboard) -> ExportConfig {
    let plot_type = app.plot_config.plot_type;
    let x_label = match plot_type {
        PlotType::PerformanceProfile => Some("Performance Ratio (tau)".to_string()),
        PlotType::LineTimeseries => Some("Dataset".to_string()),
        PlotType::StackedBar => Some("Problem".to_string()),
        _ => app
            .plot_config
            .x_axis
            .map(|x| format!("{:?}", x))
            .or_else(|| Some(format!("{:?}", XaxisType::NonZeros))),
    };
    let y_label = match plot_type {
        PlotType::PerformanceProfile => Some("Probability (rho)".to_string()),
        _ => app
            .plot_config
            .data_metric
            .map(|m| format!("{:?}", m))
            .or_else(|| Some(format!("{:?}", MetricType::Time))),
    };
    let title = Some(match plot_type {
        PlotType::Scatter => "Scatter".to_string(),
        PlotType::PerformanceProfile => "Performance Profile".to_string(),
        PlotType::Comparison => "Comparison (A vs B)".to_string(),
        PlotType::LineTimeseries => "Line Timeseries".to_string(),
        PlotType::StackedBar => "Stacked Bar".to_string(),
    });
    ExportConfig {
        title,
        x_axis_label: x_label,
        y_axis_label: y_label,
        log_scale_x: app.plot_config.log_scale_x,
    }
}

/// Resolve the SVG string for the currently-active plot type.
///
/// Returns `Err(&str)` when the relevant cache slot is empty — happens
/// before the plot has rendered once, or when a pre-condition is missing
/// (no datasets loaded, no A/B picked, etc.). The error string is
/// user-facing.
fn build_current_svg(app: &Dashboard) -> Result<String, String> {
    let cfg = current_config(app);
    match app.plot_config.plot_type {
        PlotType::Scatter => build_scatter_like_svg(app, &cfg, false),
        PlotType::PerformanceProfile => build_scatter_like_svg(app, &cfg, true),
        PlotType::LineTimeseries => {
            let data: &Rc<TimeseriesData> =
                app.timeseries_cache.data.as_ref().ok_or_else(|| {
                    "Timeseries not rendered yet — open the view first.".to_string()
                })?;
            Ok(build_timeseries_svg(data, &cfg).render())
        }
        PlotType::StackedBar => {
            let data: &Rc<StackedBarData> =
                app.stacked_bar_cache.data.as_ref().ok_or_else(|| {
                    "Stacked bar not rendered yet — open the view first.".to_string()
                })?;
            Ok(build_stacked_bar_svg(data, &cfg).render())
        }
        PlotType::Comparison => {
            let data: &Rc<ComparisonPlotData> =
                app.comparison_cache.data.as_ref().ok_or_else(|| {
                    "Comparison not rendered yet — pick A and B first.".to_string()
                })?;
            Ok(render_comparison_pair(data, &cfg))
        }
    }
}

fn build_scatter_like_svg(
    app: &Dashboard,
    cfg: &ExportConfig,
    is_profile: bool,
) -> Result<String, String> {
    let data: &Rc<PlotData> = app
        .plot_cache
        .data
        .as_ref()
        .ok_or_else(|| "Plot not rendered yet — load data first.".to_string())?;
    Ok(build_plot_data_svg(data, cfg, is_profile).render())
}

/// Build a default output filename: `plot_<plot_type>_<HHMMSS>.<ext>`.
///
/// The HH:MM:SS suffix avoids collisions when the user clicks Save several
/// times in a session. Date is omitted to keep the name short — adjacent
/// timestamps are enough to disambiguate within a working day.
#[cfg(not(target_arch = "wasm32"))]
fn default_export_filename(plot_type: PlotType, kind: ExportKind) -> String {
    let stem = match plot_type {
        PlotType::Scatter => "scatter",
        PlotType::PerformanceProfile => "profile",
        PlotType::Comparison => "comparison",
        PlotType::LineTimeseries => "timeseries",
        PlotType::StackedBar => "stacked_bar",
    };
    let ts = chrono::Local::now().format("%H%M%S");
    let ext = match kind {
        ExportKind::Svg => "svg",
        ExportKind::Pdf => "pdf",
    };
    format!("plot_{stem}_{ts}.{ext}")
}

impl Dashboard {
    /// Save the currently-visible plot as an SVG file.
    ///
    /// Native: writes to `./exports/plot_<plot_type>_<timestamp>.svg` next
    /// to the running binary's working directory. The directory is created
    /// on demand. The absolute saved path is shown in `export.last_message`
    /// so the user always knows where the file landed.
    ///
    /// We deliberately skip the egui save-dialog flow: it added two extra
    /// frames of state-machine plumbing and one click could fail silently
    /// when the cache slot was empty. Direct write is bulletproof and the
    /// user can move/rename afterwards.
    pub fn export_current_svg(&mut self) {
        match build_current_svg(self) {
            Ok(svg) => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let bytes = svg.into_bytes();
                    let filename =
                        default_export_filename(self.plot_config.plot_type, ExportKind::Svg);
                    self.export.last_message = Some(write_export(&filename, &bytes, "SVG"));
                }
                #[cfg(target_arch = "wasm32")]
                {
                    self.export.last_message = Some(format!(
                        "Download not yet supported on the web build — SVG emitted to console log ({} bytes).",
                        svg.len()
                    ));
                    log::info!("exported SVG ({} bytes):\n{}", svg.len(), svg);
                }
            }
            Err(msg) => {
                self.export.last_message = Some(format!("SVG export failed: {msg}"));
            }
        }
    }

    /// Save the currently-visible plot as a PDF. Native-only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_current_pdf(&mut self) {
        match build_current_svg(self) {
            Ok(svg) => match crate::export::pdf::svg_to_pdf(&svg) {
                Ok(bytes) => {
                    let filename =
                        default_export_filename(self.plot_config.plot_type, ExportKind::Pdf);
                    self.export.last_message = Some(write_export(&filename, &bytes, "PDF"));
                }
                Err(e) => {
                    self.export.last_message = Some(format!("PDF conversion failed: {e}"));
                }
            },
            Err(msg) => {
                self.export.last_message = Some(format!("PDF export failed: {msg}"));
            }
        }
    }
}

/// Write `bytes` to `./exports/<filename>`, creating the directory on
/// demand. Returns the user-facing status string (success path or error).
#[cfg(not(target_arch = "wasm32"))]
fn write_export(filename: &str, bytes: &[u8], label: &str) -> String {
    use std::path::PathBuf;

    let dir = PathBuf::from("exports");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return format!("{label} write failed: could not create exports/ ({e})");
    }
    let path = dir.join(filename);
    match std::fs::write(&path, bytes) {
        Ok(()) => {
            let abs = std::fs::canonicalize(&path).unwrap_or(path);
            format!("Saved {label} ({} bytes) to {}", bytes.len(), abs.display())
        }
        Err(e) => format!("{label} write failed: {e}"),
    }
}

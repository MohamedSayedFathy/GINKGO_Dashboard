//! Vector export pipeline (Task 10).
//!
//! Everything here operates on already-computed plot payloads (
//! [`crate::visualization::plotting::PlotData`],
//! [`crate::visualization::timeseries::TimeseriesData`],
//! [`crate::visualization::stacked_bar::StackedBarData`],
//! [`crate::visualization::compare::ComparisonPlotData`]
//! ) so the exported document matches exactly what's rendered on screen —
//! the same cached series, colors, and ordering flow through the writer.
//!
//! The two entry points are:
//! - [`svg::SvgCanvas::render`] — produces an SVG document string.
//! - [`pdf::svg_to_pdf`] (native only) — converts that string to a PDF
//!   buffer via `svg2pdf` / `usvg`.

pub mod dashboard;
pub mod svg;

#[cfg(not(target_arch = "wasm32"))]
pub mod pdf;

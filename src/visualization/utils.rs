use crate::data::models::{BenchmarkEntry, MatrixMetadata};
use crate::types::{DataFormat, MetricType, XaxisType};
use egui::Color32;
use std::collections::HashMap;

pub const COLOR_CSR: Color32 = Color32::from_rgb(255, 0, 0); // Red
pub const COLOR_COO: Color32 = Color32::from_rgb(0, 255, 0); // Green
pub const COLOR_ELL: Color32 = Color32::from_rgb(0, 0, 255); // Blue
pub const COLOR_HYBRID: Color32 = Color32::from_rgb(255, 165, 0); // Orange
pub const COLOR_SELLP: Color32 = Color32::from_rgb(128, 0, 128); // Purple

pub fn get_x_value(metadata: &MatrixMetadata, x_type: XaxisType) -> f64 {
    match x_type {
        XaxisType::Cols => metadata.cols as f64,
        XaxisType::ColCv => metadata.col_distribution.col_cv,
        XaxisType::Rows => metadata.rows as f64,
        XaxisType::RowCv => metadata.row_distribution.row_cv,
        XaxisType::NonZeros => metadata.nonzeros as f64,
        XaxisType::Sparsity => metadata.sparsity,
        XaxisType::AvgNnzPerRow => metadata.avg_nnz_per_row,
        XaxisType::AvgNnzPerCol => metadata.avg_nnz_per_col,
        XaxisType::MatrixShapeRatio => metadata.matrix_shape_ratio,
    }
}

pub fn get_y_value(
    spmv: &HashMap<String, BenchmarkEntry>,
    format: &DataFormat,
    metric: &MetricType,
) -> Option<f64> {
    let key = format.as_key();

    if let Some(entry) = spmv.get(key) {
        match metric {
            MetricType::Storage => entry.storage.map(|x| x as f64),
            MetricType::Time => entry.time,
            MetricType::Repetitions => entry.repetitions.map(|x| x as f64),
            MetricType::GflopsPerSecond => Some(entry.gflops_per_second),
            MetricType::OperationalIntensity => Some(entry.operational_intensity),
            MetricType::EffectiveMemoryBandwidth => Some(entry.effective_memory_bandwidth),
        }
    } else {
        None
    }
}

pub fn get_format_color(format: &DataFormat) -> Color32 {
    match format {
        DataFormat::CSR => COLOR_CSR,
        DataFormat::COO => COLOR_COO,
        DataFormat::ELL => COLOR_ELL,
        DataFormat::HYBRID => COLOR_HYBRID,
        DataFormat::SELLP => COLOR_SELLP,
    }
}

/// Adjusts the color based on dataset index to distinguish multiple files
/// while keeping the same fundamental hue for the format.
pub fn adjust_color_for_dataset(base: Color32, idx: usize) -> Color32 {
    if idx == 0 {
        return base;
    }
    let (r, g, b, a) = (base.r(), base.g(), base.b(), base.a());

    match idx % 3 {
        1 => Color32::from_rgba_premultiplied(
            (r as f32 * 0.6) as u8,
            (g as f32 * 0.6) as u8,
            (b as f32 * 0.6) as u8,
            a,
        ),
        2 => Color32::from_rgba_premultiplied(
            (r as f32 * 1.3).min(255.0) as u8,
            (g as f32 * 1.3).min(255.0) as u8,
            (b as f32 * 1.3).min(255.0) as u8,
            a,
        ),
        _ => Color32::from_rgba_premultiplied(
            (r as f32 * 0.4) as u8,
            (g as f32 * 0.4) as u8,
            (b as f32 * 0.4) as u8,
            a,
        ),
    }
}

pub fn format_metric_suffix(val: f64) -> String {
    let abs = val.abs();
    if abs >= 1_000_000_000.0 {
        format!("{:.1}G", val / 1_000_000_000.0)
    } else if abs >= 1_000_000.0 {
        format!("{:.1}M", val / 1_000_000.0)
    } else if abs >= 1_000.0 {
        format!("{:.1}K", val / 1_000.0)
    } else if abs < 1.0 && abs > 0.0 {
        if abs < 0.001 {
            format!("{:.1e}", val)
        } else {
            format!("{:.4}", val)
        }
    } else {
        format!("{:.1}", val)
    }
}

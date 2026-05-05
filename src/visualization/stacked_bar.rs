//! Stacked-bar builder (Task 6).
//!
//! For one selected dataset, emits a per-problem stack: each bar is one
//! problem, segments are one per active format, heights are the selected
//! metric. The UI stacks visually via `egui_plot::BarChart::stack_on`.

use crate::data::models::BenchmarkDataset;
use crate::types::{DataFormat, MetricType};
use crate::visualization::formula::CompiledFormula;
use crate::visualization::utils::{get_format_color, get_y_value};
use egui::Color32;
use std::collections::HashMap;

/// Stable format iteration order. Anchored in a single place so the UI
/// stacks segments bottom-up in the same sequence every render.
const FORMAT_ORDER: &[DataFormat] = &[
    DataFormat::CSR,
    DataFormat::COO,
    DataFormat::ELL,
    DataFormat::HYBRID,
    DataFormat::SELLP,
];

/// One horizontal layer of the stacked bar: same format across every
/// visible problem. `values[i]` corresponds to `problem_labels[i]`.
#[derive(Clone, Debug)]
pub struct StackedBarSeries {
    pub format: DataFormat,
    pub values: Vec<f64>,
    pub color: Color32,
}

/// Full payload rendered by the stacked-bar view.
#[derive(Clone, Debug, Default)]
pub struct StackedBarData {
    pub problem_labels: Vec<String>,
    pub series: Vec<StackedBarSeries>,
    /// Sum across every series for each problem, in the same order as
    /// `problem_labels`. Useful for axis autoscaling and sort-by-total.
    pub totals: Vec<f64>,
}

/// Build per-problem stacked-bar data from a single dataset.
///
/// Missing/non-finite metric values contribute zero rather than skipping
/// the whole bar — otherwise the visual alignment of stacks would drift
/// across formats. `top_n == 0` is treated as "show nothing" (the UI
/// `DragValue` clamps to `>= 5`, but we guard here to stay panic-free).
#[must_use]
pub fn build_stacked_bar(
    dataset: &BenchmarkDataset,
    active_formats: &HashMap<DataFormat, bool>,
    metric: MetricType,
    custom_formula: Option<&CompiledFormula>,
    sort_by_total: bool,
    top_n: usize,
) -> StackedBarData {
    if top_n == 0 || dataset.benchmark.is_empty() {
        return StackedBarData::default();
    }

    // Active set honors checkbox order; preserve `FORMAT_ORDER` so the
    // legend reads CSR→COO→ELL→HYBRID→SELLP every frame.
    let formats: Vec<DataFormat> = FORMAT_ORDER
        .iter()
        .copied()
        .filter(|f| active_formats.get(f).copied().unwrap_or(false))
        .collect();

    if formats.is_empty() {
        return StackedBarData::default();
    }

    // One row per problem; one column per active format + a totals column.
    let mut rows: Vec<(String, Vec<f64>, f64)> = dataset
        .benchmark
        .iter()
        .map(|problem| {
            let baseline_entry = problem.spmv.get(DataFormat::CSR.as_key());
            let per_format: Vec<f64> = formats
                .iter()
                .map(|fmt| {
                    get_y_value(
                        &problem.spmv,
                        fmt,
                        &metric,
                        &problem.problem,
                        baseline_entry,
                        custom_formula,
                    )
                    .filter(|v| v.is_finite())
                    .unwrap_or(0.0)
                })
                .collect();
            let total: f64 = per_format.iter().sum();
            (problem.problem.name.to_string(), per_format, total)
        })
        .collect();

    if sort_by_total {
        rows.sort_by(|a, b| b.2.total_cmp(&a.2));
    }
    rows.truncate(top_n);

    let problem_labels: Vec<String> = rows.iter().map(|(name, _, _)| name.clone()).collect();
    let totals: Vec<f64> = rows.iter().map(|(_, _, t)| *t).collect();

    let series: Vec<StackedBarSeries> = formats
        .iter()
        .enumerate()
        .map(|(col, &format)| {
            let values: Vec<f64> = rows.iter().map(|(_, per, _)| per[col]).collect();
            StackedBarSeries {
                format,
                values,
                color: get_format_color(&format),
            }
        })
        .collect();

    StackedBarData {
        problem_labels,
        series,
        totals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::models::{
        BenchmarkEntry, BenchmarkOptimal, BenchmarkProblem, MatrixColumnsMetadata, MatrixMetadata,
        MatrixRowsMetadata,
    };

    /// Per-problem totals column equals the sum of each bar segment.
    #[test]
    fn totals_match_per_format_sum() {
        let ds = mini_dataset(&[("p1", &[("csr", 100), ("coo", 50), ("ell", 25)])]);
        let mut active = HashMap::new();
        active.insert(DataFormat::CSR, true);
        active.insert(DataFormat::COO, true);
        active.insert(DataFormat::ELL, true);

        let data = build_stacked_bar(&ds, &active, MetricType::Storage, None, false, 30);
        assert_eq!(data.totals.len(), 1);
        let sum: f64 = data.series.iter().map(|s| s.values[0]).sum();
        assert!((data.totals[0] - sum).abs() < 1e-9);
        assert!((data.totals[0] - 175.0).abs() < 1e-9);
    }

    /// `sort_by_total = true` puts the biggest problems first.
    #[test]
    fn sort_by_total_descending() {
        let ds = mini_dataset(&[
            ("small", &[("csr", 10)]),
            ("big", &[("csr", 100)]),
            ("mid", &[("csr", 50)]),
        ]);
        let mut active = HashMap::new();
        active.insert(DataFormat::CSR, true);

        let data = build_stacked_bar(&ds, &active, MetricType::Storage, None, true, 30);
        assert_eq!(data.problem_labels, vec!["big", "mid", "small"]);
    }

    /// `top_n = 5` over 20 problems keeps only the 5 largest (by total).
    #[test]
    fn top_n_limits_to_largest() {
        let specs: Vec<(String, Vec<(&str, u64)>)> = (0..20)
            .map(|i| (format!("p{i:02}"), vec![("csr", (i as u64) * 10 + 1)]))
            .collect();
        // Re-borrow into &str-friendly form for mini_dataset.
        let specs_ref: Vec<(&str, &[(&str, u64)])> = specs
            .iter()
            .map(|(name, v)| (name.as_str(), v.as_slice()))
            .collect();
        let ds = mini_dataset(&specs_ref);

        let mut active = HashMap::new();
        active.insert(DataFormat::CSR, true);

        let data = build_stacked_bar(&ds, &active, MetricType::Storage, None, true, 5);
        assert_eq!(data.problem_labels.len(), 5);
        // Five largest values in a 0..20 → i*10+1 progression are i=19,18,17,16,15.
        assert_eq!(data.problem_labels[0], "p19");
        assert_eq!(data.problem_labels[4], "p15");
    }

    /// Formats not enabled in the active map must not appear in the output.
    #[test]
    fn disabled_format_excluded() {
        let ds = mini_dataset(&[("p1", &[("csr", 100), ("coo", 50)])]);
        let mut active = HashMap::new();
        active.insert(DataFormat::CSR, true);
        active.insert(DataFormat::COO, false);

        let data = build_stacked_bar(&ds, &active, MetricType::Storage, None, false, 30);
        assert_eq!(data.series.len(), 1);
        assert_eq!(data.series[0].format, DataFormat::CSR);
    }

    /// Empty dataset → empty output, no crash.
    #[test]
    fn empty_dataset_returns_default() {
        let ds = BenchmarkDataset { benchmark: vec![] };
        let mut active = HashMap::new();
        active.insert(DataFormat::CSR, true);
        let data = build_stacked_bar(&ds, &active, MetricType::Storage, None, true, 30);
        assert!(data.problem_labels.is_empty());
        assert!(data.series.is_empty());
        assert!(data.totals.is_empty());
    }

    /// No active formats → empty output.
    #[test]
    fn no_active_formats_returns_default() {
        let ds = mini_dataset(&[("p1", &[("csr", 100)])]);
        let active: HashMap<DataFormat, bool> = HashMap::new();
        let data = build_stacked_bar(&ds, &active, MetricType::Storage, None, true, 30);
        assert!(data.series.is_empty());
    }

    // ------------------------------------------------------------------
    // Fixture helpers
    // ------------------------------------------------------------------

    fn mini_dataset(problems: &[(&str, &[(&str, u64)])]) -> BenchmarkDataset {
        let benchmark = problems
            .iter()
            .map(|(name, fmts)| {
                let mut spmv = HashMap::new();
                for (fmt_key, storage) in *fmts {
                    spmv.insert(
                        (*fmt_key).to_string(),
                        BenchmarkEntry {
                            storage: Some(*storage),
                            max_relative_norm2: None,
                            time: Some(1.0),
                            repetitions: Some(1),
                            completed: true,
                            gflops_per_second: 0.0,
                            bytes_per_nnz: 0.0,
                            operational_intensity: 0.0,
                            effective_memory_bandwidth: 0.0,
                        },
                    );
                }
                BenchmarkProblem {
                    filename: String::new(),
                    problem: MatrixMetadata {
                        id: 0,
                        group: "g".into(),
                        name: (*name).into(),
                        rows: 10,
                        cols: 10,
                        nonzeros: 100,
                        real: true,
                        binary: false,
                        is_2d3d: false,
                        posdef: false,
                        psym: 0.0,
                        nsym: 0.0,
                        kind: "k".into(),
                        row_distribution: zero_rows(),
                        col_distribution: zero_cols(),
                        sparsity: 0.1,
                        avg_nnz_per_row: 10.0,
                        avg_nnz_per_col: 10.0,
                        matrix_shape_ratio: 1.0,
                    },
                    spmv,
                    optimal: BenchmarkOptimal {
                        spmv: "csr".to_string(),
                    },
                }
            })
            .collect();
        BenchmarkDataset { benchmark }
    }

    fn zero_rows() -> MatrixRowsMetadata {
        MatrixRowsMetadata {
            min: 0.0,
            q1: 0.0,
            median: 0.0,
            q3: 0.0,
            max: 0.0,
            mean: 0.0,
            variance: 0.0,
            skewness: 0.0,
            kurtosis: 0.0,
            hyperskewness: 0.0,
            hyperflatness: 0.0,
            row_irregularity: 0.0,
            row_cv: 0.0,
        }
    }

    fn zero_cols() -> MatrixColumnsMetadata {
        MatrixColumnsMetadata {
            min: 0.0,
            q1: 0.0,
            median: 0.0,
            q3: 0.0,
            max: 0.0,
            mean: 0.0,
            variance: 0.0,
            skewness: 0.0,
            kurtosis: 0.0,
            hyperskewness: 0.0,
            hyperflatness: 0.0,
            col_irregularity: 0.0,
            col_cv: 0.0,
        }
    }
}

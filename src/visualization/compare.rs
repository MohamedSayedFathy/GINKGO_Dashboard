//! Commit A vs B benchmark comparison (Task 5).
//!
//! Produces a side-by-side scatter pair, a per-problem diff table, and a
//! log2-binned speedup histogram. The UI wrapper lives in
//! [`crate::ui::charts`]; everything here is pure data.

use crate::data::models::BenchmarkDataset;
use crate::data::state::CompareSide;
use crate::types::{DataFormat, DataMode, MetricType, XaxisType};
use crate::visualization::formula::CompiledFormula;
use crate::visualization::plotting::PlotData;
use crate::visualization::scatter::generate_scatter_plot_data;
use crate::visualization::utils::get_y_value;
use smol_str::SmolStr;
use std::collections::{HashMap, HashSet};

/// Number of histogram bins across `[-4, 4]` in log2(ratio) space.
pub const HIST_BINS: usize = 25;

/// Clamp bound (in log2 space) for the histogram range.
///
/// Ratios above `2^4 = 16x` or below `2^-4 = 1/16x` are lumped into the
/// outermost bins. In practice benchmarks rarely shift by that much between
/// adjacent commits, so the clamp simply prevents one pathological outlier
/// from blowing out the x-axis.
pub const HIST_CLAMP_ABS: f64 = 4.0;

/// One row in the diff table: a single (problem, format) pair present in at
/// least one of the two compared datasets.
#[derive(Clone, Debug)]
pub struct DiffRow {
    pub problem_name: SmolStr,
    pub format: DataFormat,
    /// NaN when the problem is missing on side A.
    pub value_a: f64,
    /// NaN when the problem is missing on side B.
    pub value_b: f64,
    /// `b - a`; NaN when either side is missing.
    pub delta: f64,
    /// Speedup ratio (see [`speedup_ratio`]); NaN when either side is missing.
    pub ratio: f64,
}

/// Log2-spaced speedup histogram. `bin_edges_log2.len() == bin_counts.len() + 1`.
#[derive(Clone, Debug, Default)]
pub struct SpeedupHistogram {
    pub bin_edges_log2: Vec<f64>,
    pub bin_counts: Vec<usize>,
}

/// Aggregate counts + geometric-mean summary over every finite ratio.
#[derive(Clone, Debug, Default)]
pub struct ComparisonSummary {
    pub geometric_mean_speedup: f64,
    pub regressions: usize,
    pub improvements: usize,
    pub unchanged: usize,
    pub missing_in_a: usize,
    pub missing_in_b: usize,
}

/// Full payload rendered by the comparison view.
pub struct ComparisonPlotData {
    pub pane_a: PlotData,
    pub pane_b: PlotData,
    pub diff_rows: Vec<DiffRow>,
    pub histogram: SpeedupHistogram,
    pub summary: ComparisonSummary,
}

/// Speedup ratio where `ratio > 1` always means "B is better than A".
///
/// - `lower_is_better` (time/repetitions): ratio = `a / b` — if B's time is
///   smaller, the ratio exceeds 1 and we report an improvement.
/// - `!lower_is_better` (gflops, bandwidth): ratio = `b / a`.
///
/// Non-finite or non-positive inputs yield `f64::NAN`. The caller filters
/// these out of the histogram and geometric-mean computation.
#[must_use]
pub fn speedup_ratio(a: f64, b: f64, lower_is_better: bool) -> f64 {
    if !a.is_finite() || !b.is_finite() || a <= 0.0 || b <= 0.0 {
        return f64::NAN;
    }
    if lower_is_better {
        a / b
    } else {
        b / a
    }
}

/// Build a `HIST_BINS`-wide log2-spaced histogram over `[-HIST_CLAMP_ABS, HIST_CLAMP_ABS]`.
#[must_use]
pub fn histogram_bins(ratios: &[f64]) -> SpeedupHistogram {
    let edges: Vec<f64> = (0..=HIST_BINS)
        .map(|i| -HIST_CLAMP_ABS + (i as f64) * (2.0 * HIST_CLAMP_ABS) / (HIST_BINS as f64))
        .collect();
    let mut counts = vec![0usize; HIST_BINS];

    for &r in ratios {
        if !r.is_finite() || r <= 0.0 {
            continue;
        }
        let log2 = r.log2().clamp(-HIST_CLAMP_ABS, HIST_CLAMP_ABS);
        // Map log2 to a bin index in [0, HIST_BINS-1].
        let bin_width = (2.0 * HIST_CLAMP_ABS) / (HIST_BINS as f64);
        let raw = ((log2 + HIST_CLAMP_ABS) / bin_width).floor() as isize;
        let idx = raw.clamp(0, (HIST_BINS as isize) - 1) as usize;
        counts[idx] += 1;
    }

    SpeedupHistogram {
        bin_edges_log2: edges,
        bin_counts: counts,
    }
}

/// Build the comparison payload for two already-loaded datasets.
///
/// The per-pane scatters delegate to [`generate_scatter_plot_data`] with a
/// one-element `active_dataset`, so normalization / outlier / formula
/// behavior matches the single-view scatter exactly.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn generate_comparison_data(
    dataset_a: &BenchmarkDataset,
    dataset_b: &BenchmarkDataset,
    active_formats: &HashMap<DataFormat, bool>,
    x_axis: Option<XaxisType>,
    metric: Option<MetricType>,
    baseline_format: DataFormat,
    normalize: bool,
    filter_outliers: bool,
    custom_formula: Option<&CompiledFormula>,
    baseline_side: CompareSide,
    lower_is_better: bool,
    diff_threshold: f64,
    name_a: &str,
    name_b: &str,
) -> ComparisonPlotData {
    // Build each pane as a Multi-mode scatter with exactly one dataset. The
    // baseline side's pane is passed `normalize` as the user requested; the
    // opposite pane uses the same `normalize` flag but reads its own values
    // — normalization is always intra-dataset in the scatter path, so both
    // panes are comparable as long as `baseline_format` matches.
    let (pane_a, pane_b) = {
        let mut ds_map: HashMap<String, BenchmarkDataset> = HashMap::with_capacity(2);
        ds_map.insert(name_a.to_string(), dataset_a.clone());
        ds_map.insert(name_b.to_string(), dataset_b.clone());

        // `DataMode::Multi` so `active_formats` is honored as the user sees
        // it in the sidebar.
        let mode = Some(DataMode::Multi);

        let a = generate_scatter_plot_data(
            &ds_map,
            &[name_a.to_string()],
            active_formats,
            mode,
            x_axis,
            metric,
            baseline_format,
            normalize && matches!(baseline_side, CompareSide::A),
            filter_outliers,
            false,
            custom_formula,
        );
        let b = generate_scatter_plot_data(
            &ds_map,
            &[name_b.to_string()],
            active_formats,
            mode,
            x_axis,
            metric,
            baseline_format,
            normalize && matches!(baseline_side, CompareSide::B),
            filter_outliers,
            false,
            custom_formula,
        );
        (a, b)
    };

    // ------------------------------------------------------------------
    // Diff table + histogram + summary
    // ------------------------------------------------------------------
    let metric = match metric {
        Some(m) => m,
        None => {
            return ComparisonPlotData {
                pane_a,
                pane_b,
                diff_rows: Vec::new(),
                histogram: SpeedupHistogram::default(),
                summary: ComparisonSummary::default(),
            };
        }
    };

    // Active format set honors user checkboxes; fall back to "all formats
    // checked" if the map is empty (matches default after first load).
    let formats: Vec<DataFormat> = if active_formats.is_empty() {
        vec![
            DataFormat::CSR,
            DataFormat::COO,
            DataFormat::ELL,
            DataFormat::HYBRID,
            DataFormat::SELLP,
        ]
    } else {
        active_formats
            .iter()
            .filter_map(|(k, v)| if *v { Some(*k) } else { None })
            .collect()
    };

    // Orient `a`/`b` so that `ratio > 1` always reads as "B better than A",
    // regardless of which side the user picked as baseline. When the user
    // flips baseline_side, the pane labels swap in the UI but the ratio
    // direction stays fixed relative to the A/B panes they see.
    let a_problems: HashMap<&SmolStr, &_> = dataset_a
        .benchmark
        .iter()
        .map(|p| (&p.problem.name, p))
        .collect();
    let b_problems: HashMap<&SmolStr, &_> = dataset_b
        .benchmark
        .iter()
        .map(|p| (&p.problem.name, p))
        .collect();

    let all_names: HashSet<&SmolStr> = a_problems
        .keys()
        .copied()
        .chain(b_problems.keys().copied())
        .collect();

    let mut diff_rows: Vec<DiffRow> = Vec::new();
    let mut ratios_for_hist: Vec<f64> = Vec::new();
    let mut summary = ComparisonSummary::default();

    for name in all_names {
        let a_problem = a_problems.get(name).copied();
        let b_problem = b_problems.get(name).copied();

        for &format in &formats {
            let a_val = a_problem.and_then(|p| {
                let bl = p.spmv.get(baseline_format.as_key());
                get_y_value(&p.spmv, &format, &metric, &p.problem, bl, custom_formula)
            });
            let b_val = b_problem.and_then(|p| {
                let bl = p.spmv.get(baseline_format.as_key());
                get_y_value(&p.spmv, &format, &metric, &p.problem, bl, custom_formula)
            });

            match (a_val, b_val) {
                (Some(a), Some(b)) if a.is_finite() && b.is_finite() => {
                    let ratio = speedup_ratio(a, b, lower_is_better);
                    let delta = b - a;
                    if ratio.is_finite() {
                        ratios_for_hist.push(ratio);
                        // ~1% tolerance band counts as "unchanged" — stops
                        // floating-point drift inflating regression counts.
                        if (ratio - 1.0).abs() < 0.01 {
                            summary.unchanged += 1;
                        } else if ratio > 1.0 {
                            summary.improvements += 1;
                        } else {
                            summary.regressions += 1;
                        }
                    }
                    // Threshold is a percentage of `|value_a|`; skip when a
                    // is zero to avoid a 0/0 that'd drop every row.
                    let suppress = if a == 0.0 {
                        false
                    } else {
                        (delta / a).abs() * 100.0 < diff_threshold
                    };
                    if !suppress {
                        diff_rows.push(DiffRow {
                            problem_name: name.clone(),
                            format,
                            value_a: a,
                            value_b: b,
                            delta,
                            ratio,
                        });
                    }
                }
                (Some(a), None) => {
                    summary.missing_in_b += 1;
                    diff_rows.push(DiffRow {
                        problem_name: name.clone(),
                        format,
                        value_a: a,
                        value_b: f64::NAN,
                        delta: f64::NAN,
                        ratio: f64::NAN,
                    });
                }
                (None, Some(b)) => {
                    summary.missing_in_a += 1;
                    diff_rows.push(DiffRow {
                        problem_name: name.clone(),
                        format,
                        value_a: f64::NAN,
                        value_b: b,
                        delta: f64::NAN,
                        ratio: f64::NAN,
                    });
                }
                _ => {
                    // Both missing or non-finite — nothing to show.
                }
            }
        }
    }

    // Sort diff rows by "how different this is" descending; rows with NaN
    // ratio (missing one side) sink to the bottom via `total_cmp` on the
    // absolute deviation from 1.0.
    diff_rows.sort_by(|l, r| {
        let lm = if l.ratio.is_finite() {
            (l.ratio - 1.0).abs()
        } else {
            f64::NEG_INFINITY
        };
        let rm = if r.ratio.is_finite() {
            (r.ratio - 1.0).abs()
        } else {
            f64::NEG_INFINITY
        };
        rm.total_cmp(&lm)
    });

    // Geometric mean = exp2(mean of log2(ratio)).
    summary.geometric_mean_speedup = if ratios_for_hist.is_empty() {
        f64::NAN
    } else {
        let sum: f64 = ratios_for_hist.iter().map(|r| r.log2()).sum();
        (sum / ratios_for_hist.len() as f64).exp2()
    };

    let histogram = histogram_bins(&ratios_for_hist);

    ComparisonPlotData {
        pane_a,
        pane_b,
        diff_rows,
        histogram,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speedup_ratio_direction() {
        // lower_is_better: A=2s, B=1s → B is 2x faster.
        assert!((speedup_ratio(2.0, 1.0, true) - 2.0).abs() < 1e-12);
        // higher_is_better: A=1 gflop, B=2 gflops → B is 2x.
        assert!((speedup_ratio(1.0, 2.0, false) - 2.0).abs() < 1e-12);
        // Non-positive yields NaN.
        assert!(speedup_ratio(0.0, 1.0, true).is_nan());
        assert!(speedup_ratio(1.0, -1.0, false).is_nan());
        assert!(speedup_ratio(f64::INFINITY, 1.0, true).is_nan());
    }

    #[test]
    fn log2_binning() {
        // ratio=1 → log2=0 → middle bin (index HIST_BINS/2 for an even
        // offset; with 25 bins and edges from -4..4, bin width = 0.32,
        // log2=0 lands in bin index 12).
        let h = histogram_bins(&[1.0]);
        assert_eq!(h.bin_edges_log2.len(), HIST_BINS + 1);
        assert_eq!(h.bin_counts.len(), HIST_BINS);
        assert_eq!(h.bin_counts.iter().sum::<usize>(), 1);
        assert_eq!(h.bin_counts[12], 1);

        // ratio=16 (log2=4) clamps into the final bin.
        let h = histogram_bins(&[16.0, 1e9]);
        assert_eq!(h.bin_counts[HIST_BINS - 1], 2);

        // ratio=1/16 (log2=-4) lands in bin 0.
        let h = histogram_bins(&[1.0 / 16.0, 1e-9]);
        assert_eq!(h.bin_counts[0], 2);

        // Non-finite/zero ignored.
        let h = histogram_bins(&[f64::NAN, 0.0, -1.0]);
        assert_eq!(h.bin_counts.iter().sum::<usize>(), 0);
    }

    #[test]
    fn threshold_filter() {
        // delta_threshold=10% must drop rows where |Δ|/|A| < 10%.
        let a_ds = mini_dataset(&[("m1", 100.0), ("m2", 100.0)]);
        let b_ds = mini_dataset(&[
            ("m1", 105.0), // 5% → dropped
            ("m2", 150.0), // 50% → kept
        ]);

        let mut fmts = HashMap::new();
        fmts.insert(DataFormat::CSR, true);

        let data = generate_comparison_data(
            &a_ds,
            &b_ds,
            &fmts,
            Some(XaxisType::NonZeros),
            Some(MetricType::Time),
            DataFormat::CSR,
            false,
            false,
            None,
            CompareSide::A,
            true,
            10.0,
            "A",
            "B",
        );
        let names: Vec<_> = data
            .diff_rows
            .iter()
            .map(|r| r.problem_name.as_str())
            .collect();
        assert!(names.contains(&"m2"));
        assert!(!names.contains(&"m1"));
    }

    #[test]
    fn missing_problem_counts() {
        let a_ds = mini_dataset(&[("P1", 1.0)]);
        let b_ds = mini_dataset(&[("P2", 1.0)]);

        let mut fmts = HashMap::new();
        fmts.insert(DataFormat::CSR, true);

        let data = generate_comparison_data(
            &a_ds,
            &b_ds,
            &fmts,
            Some(XaxisType::NonZeros),
            Some(MetricType::Time),
            DataFormat::CSR,
            false,
            false,
            None,
            CompareSide::A,
            true,
            0.0,
            "A",
            "B",
        );
        assert_eq!(data.summary.missing_in_a, 1, "P2 only in B → missing in A");
        assert_eq!(data.summary.missing_in_b, 1, "P1 only in A → missing in B");
    }

    fn mini_dataset(problems: &[(&str, f64)]) -> BenchmarkDataset {
        use crate::data::models::{
            BenchmarkEntry, BenchmarkOptimal, BenchmarkProblem, MatrixColumnsMetadata,
            MatrixMetadata, MatrixRowsMetadata,
        };

        let zero_rows = MatrixRowsMetadata {
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
        };
        let zero_cols = MatrixColumnsMetadata {
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
        };
        let benchmark = problems
            .iter()
            .map(|(name, time)| {
                let mut spmv = std::collections::HashMap::new();
                spmv.insert(
                    "csr".to_string(),
                    BenchmarkEntry {
                        storage: Some(1000),
                        max_relative_norm2: None,
                        time: Some(*time),
                        repetitions: Some(1),
                        completed: true,
                        gflops_per_second: 0.0,
                        bytes_per_nnz: 0.0,
                        operational_intensity: 0.0,
                        effective_memory_bandwidth: 0.0,
                    },
                );
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
                        row_distribution: zero_rows.clone(),
                        col_distribution: zero_cols.clone(),
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

    proptest::proptest! {
        /// For any finite positive (a, b), the product of forward and
        /// reverse ratios must be 1.0 within a tiny epsilon — a basic
        /// sanity check that orientation handling is symmetric.
        #[test]
        fn speedup_ratio_inverse_property(
            a in 1e-6f64..1e6f64,
            b in 1e-6f64..1e6f64,
            lib: bool,
        ) {
            let f = speedup_ratio(a, b, lib);
            let r = speedup_ratio(b, a, lib);
            let prod = f * r;
            proptest::prop_assert!((prod - 1.0).abs() < 1e-10, "forward*reverse={prod}");
        }
    }
}

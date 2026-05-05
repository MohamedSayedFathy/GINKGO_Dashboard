//! Line-timeseries builder (Task 6).
//!
//! Produces one point per loaded dataset — a scalar aggregate of the
//! selected metric across the dataset's problems (or a single problem's
//! value when `problem_filter` is set). Callers supply the desired
//! dataset ordering via `ordered_keys`; ordering policy (commit-date vs.
//! alphabetical) lives in the UI layer so this module stays pure data.

use crate::data::models::BenchmarkDataset;
use crate::types::{AggregationKind, DataFormat, MetricType};
use crate::visualization::formula::CompiledFormula;
use crate::visualization::utils::get_y_value;
use std::collections::HashMap;

/// One point on the line chart: the X is a dataset-position index (0, 1, ...)
/// and the Y is the aggregated metric value. `label` is the short tick label
/// rendered under the X axis.
#[derive(Clone, Debug)]
pub struct TimeseriesPoint {
    pub x: f64,
    pub y: f64,
    pub dataset_key: String,
    pub label: String,
}

/// Full payload rendered by the timeseries view.
#[derive(Clone, Debug, Default)]
pub struct TimeseriesData {
    pub points: Vec<TimeseriesPoint>,
    /// Human-readable Y-axis label derived from the aggregation + metric pair.
    pub y_label: String,
}

impl AggregationKind {
    /// Reduce `values` to a single scalar, returning `None` for empty input.
    ///
    /// - Non-finite inputs are filtered first (silently — they'd poison every
    ///   aggregation anyway).
    /// - `GeometricMean` additionally rejects values `<= 0` (log-space
    ///   undefined) and logs `log::debug!` with the skip count so the user
    ///   can tell a zero came from filtering vs. from genuinely-missing data.
    #[must_use]
    pub fn apply(&self, values: &[f64]) -> Option<f64> {
        let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
        if finite.is_empty() {
            return None;
        }

        match self {
            AggregationKind::Mean => {
                let sum: f64 = finite.iter().sum();
                Some(sum / finite.len() as f64)
            }
            AggregationKind::Median => {
                let mut sorted = finite;
                sorted.sort_by(|a, b| a.total_cmp(b));
                let n = sorted.len();
                Some(if n % 2 == 1 {
                    sorted[n / 2]
                } else {
                    (sorted[n / 2 - 1] + sorted[n / 2]) * 0.5
                })
            }
            AggregationKind::GeometricMean => {
                let n_before = finite.len();
                let positives: Vec<f64> = finite.into_iter().filter(|&v| v > 0.0).collect();
                let skipped = n_before - positives.len();
                if skipped > 0 {
                    log::debug!(
                        "geomean: skipped {skipped} non-positive value(s) out of {n_before}"
                    );
                }
                if positives.is_empty() {
                    return None;
                }
                let sum_log: f64 = positives.iter().map(|v| v.ln()).sum();
                Some((sum_log / positives.len() as f64).exp())
            }
        }
    }

    pub fn as_label(&self) -> &'static str {
        match self {
            AggregationKind::Mean => "mean",
            AggregationKind::Median => "median",
            AggregationKind::GeometricMean => "geomean",
        }
    }
}

/// Build a timeseries payload for the dashboard's line-chart view.
///
/// `ordered_keys` fixes the X-axis ordering; datasets absent from `datasets`
/// or missing the requested format are still emitted as positions but with
/// no corresponding point (the line "jumps" the gap). When `problem_filter`
/// is supplied the aggregate degenerates to that problem's single value.
#[must_use]
pub fn build_timeseries(
    datasets: &HashMap<String, BenchmarkDataset>,
    ordered_keys: &[String],
    aggregation: AggregationKind,
    format: DataFormat,
    metric: MetricType,
    custom_formula: Option<&CompiledFormula>,
    problem_filter: Option<&str>,
) -> TimeseriesData {
    let mut points: Vec<TimeseriesPoint> = Vec::with_capacity(ordered_keys.len());

    for (idx, key) in ordered_keys.iter().enumerate() {
        let Some(ds) = datasets.get(key) else {
            continue;
        };

        let mut values: Vec<f64> = Vec::new();
        for problem in &ds.benchmark {
            if let Some(filter_name) = problem_filter {
                if problem.problem.name.as_str() != filter_name {
                    continue;
                }
            }
            let baseline_entry = problem.spmv.get(DataFormat::CSR.as_key());
            if let Some(y) = get_y_value(
                &problem.spmv,
                &format,
                &metric,
                &problem.problem,
                baseline_entry,
                custom_formula,
            ) {
                if y.is_finite() {
                    values.push(y);
                }
            }
        }

        if let Some(agg) = aggregation.apply(&values) {
            points.push(TimeseriesPoint {
                x: idx as f64,
                y: agg,
                dataset_key: key.clone(),
                label: key.clone(),
            });
        }
    }

    let y_label = format!("{} ({:?}, {:?})", aggregation.as_label(), metric, format);

    TimeseriesData { points, y_label }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::models::{
        BenchmarkEntry, BenchmarkOptimal, BenchmarkProblem, MatrixColumnsMetadata, MatrixMetadata,
        MatrixRowsMetadata,
    };

    /// Median of 1/2/3 → 2.
    #[test]
    fn median_three_values() {
        let datasets = build_datasets(&[("c1", &[("p1", 1.0), ("p2", 2.0), ("p3", 3.0)])]);
        let keys = vec!["c1".to_string()];
        let ts = build_timeseries(
            &datasets,
            &keys,
            AggregationKind::Median,
            DataFormat::CSR,
            MetricType::Time,
            None,
            None,
        );
        assert_eq!(ts.points.len(), 1);
        assert!((ts.points[0].y - 2.0).abs() < 1e-12);
    }

    /// Geomean of {1, 4} is 2.
    #[test]
    fn geomean_one_four() {
        let result = AggregationKind::GeometricMean.apply(&[1.0, 4.0]);
        assert!((result.expect("some") - 2.0).abs() < 1e-12);
    }

    /// Empty input → None.
    #[test]
    fn empty_input_returns_none() {
        for kind in [
            AggregationKind::Mean,
            AggregationKind::Median,
            AggregationKind::GeometricMean,
        ] {
            assert!(kind.apply(&[]).is_none());
        }
    }

    /// Geomean skips non-positive values; if nothing is left, returns None.
    #[test]
    fn geomean_rejects_non_positive() {
        let out = AggregationKind::GeometricMean.apply(&[0.0, -1.0, f64::NAN]);
        assert!(out.is_none());
        let out = AggregationKind::GeometricMean.apply(&[4.0, -1.0, 1.0]);
        assert!((out.expect("some") - 2.0).abs() < 1e-12);
    }

    /// `problem_filter` narrows the aggregate to a single problem.
    #[test]
    fn problem_filter_narrows_to_one() {
        let datasets = build_datasets(&[("c1", &[("p1", 10.0), ("p2", 20.0), ("p3", 30.0)])]);
        let keys = vec!["c1".to_string()];
        let ts = build_timeseries(
            &datasets,
            &keys,
            AggregationKind::Mean,
            DataFormat::CSR,
            MetricType::Time,
            None,
            Some("p2"),
        );
        assert_eq!(ts.points.len(), 1);
        assert!((ts.points[0].y - 20.0).abs() < 1e-12);
    }

    /// Format absent from every SPMV map → empty TimeseriesData.
    #[test]
    fn unknown_format_yields_empty() {
        let datasets = build_datasets(&[("c1", &[("p1", 1.0)])]);
        let keys = vec!["c1".to_string()];
        let ts = build_timeseries(
            &datasets,
            &keys,
            AggregationKind::Mean,
            // ELL is not populated by build_datasets; only CSR is.
            DataFormat::ELL,
            MetricType::Time,
            None,
            None,
        );
        assert!(ts.points.is_empty());
    }

    /// Multi-dataset ordering: X positions track `ordered_keys` indices.
    #[test]
    fn ordered_keys_drive_x_positions() {
        let datasets = build_datasets(&[
            ("c1", &[("p1", 1.0)]),
            ("c2", &[("p1", 2.0)]),
            ("c3", &[("p1", 3.0)]),
        ]);
        let keys = vec!["c1".to_string(), "c2".to_string(), "c3".to_string()];
        let ts = build_timeseries(
            &datasets,
            &keys,
            AggregationKind::Mean,
            DataFormat::CSR,
            MetricType::Time,
            None,
            None,
        );
        assert_eq!(ts.points.len(), 3);
        assert_eq!(ts.points[0].x, 0.0);
        assert_eq!(ts.points[1].x, 1.0);
        assert_eq!(ts.points[2].x, 2.0);
    }

    // ------------------------------------------------------------------
    // Test fixtures
    // ------------------------------------------------------------------

    fn build_datasets(specs: &[(&str, &[(&str, f64)])]) -> HashMap<String, BenchmarkDataset> {
        specs
            .iter()
            .map(|(key, problems)| ((*key).to_string(), mini_dataset(problems)))
            .collect()
    }

    fn mini_dataset(problems: &[(&str, f64)]) -> BenchmarkDataset {
        let benchmark = problems
            .iter()
            .map(|(name, time)| {
                let mut spmv = HashMap::new();
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

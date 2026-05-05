//! Cross-commit outlier detection (Task 8).
//!
//! Compares each commit's (problem, format) measurements against a rolling
//! baseline built from the `N` immediately-preceding commits in timeline
//! order. A commit is flagged as an outlier when more than
//! `threshold_percent` of its measurements deviate from the baseline median
//! by more than `sigma_threshold` sample standard deviations.
//!
//! The module is pure data: no egui types, no globals. The sidebar and the
//! LineTimeseries chart consume [`CommitOutlierReport`]s to paint badges /
//! recolour points respectively.

use crate::data::models::BenchmarkDataset;
use crate::types::{DataFormat, MetricType};
use crate::visualization::formula::CompiledFormula;
use crate::visualization::utils::get_y_value;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// All five sparse-matrix formats iterated when sampling per-commit values.
const ALL_FORMATS: [DataFormat; 5] = [
    DataFormat::CSR,
    DataFormat::COO,
    DataFormat::ELL,
    DataFormat::HYBRID,
    DataFormat::SELLP,
];

/// Per-commit aggregate statistics.
///
/// `values` is keyed by `(problem_name, format)`; pairs where the metric
/// was missing or non-finite are simply not inserted.
#[derive(Clone, Debug)]
pub struct CommitStats {
    pub dataset_key: String,
    pub values: HashMap<(String, DataFormat), f64>,
}

/// Baseline statistics for a single `(problem, format)` key.
///
/// `std_dev` uses the sample (n-1) denominator and is 0.0 whenever the
/// window holds fewer than two samples.
#[derive(Clone, Debug)]
pub struct BaselineCell {
    pub median: f64,
    pub std_dev: f64,
    pub n: usize,
}

/// User-facing configuration for outlier detection.
///
/// `enabled` is the master toggle: when `false`, the UI does not trigger
/// any computation and downstream consumers treat every commit as non-
/// outlier. The other fields tune the detector's sensitivity.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlierDetectionConfig {
    /// Number of preceding commits (in timeline order) used as the baseline.
    pub baseline_window: usize,
    /// Sigma multiplier a measurement must exceed to count as a deviation.
    pub sigma_threshold: f64,
    /// Percent of deviating measurements above which the commit is flagged.
    pub threshold_percent: f64,
    /// Which scalar metric is being analysed.
    pub metric: MetricType,
    /// Master enable switch — opt-in so the default native / wasm load path
    /// remains unchanged.
    pub enabled: bool,
}

impl Default for OutlierDetectionConfig {
    fn default() -> Self {
        Self {
            baseline_window: 5,
            sigma_threshold: 3.0,
            threshold_percent: 10.0,
            metric: MetricType::Time,
            enabled: false,
        }
    }
}

/// Report row for a single commit: how many measurements deviated and
/// whether the commit crossed the `threshold_percent` bar.
#[derive(Clone, Debug, PartialEq)]
pub struct CommitOutlierReport {
    pub dataset_key: String,
    pub total_measurements: usize,
    pub deviating_measurements: usize,
    pub deviating_percent: f64,
    pub is_outlier: bool,
}

/// Sample median of a slice of `f64`s, skipping non-finite values.
///
/// Returns `None` when the filtered slice is empty. Uses `total_cmp` so
/// the sort is total even for weirdly-ordered floats.
fn median_finite(values: &[f64]) -> Option<f64> {
    let mut finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return None;
    }
    finite.sort_by(|a, b| a.total_cmp(b));
    let n = finite.len();
    Some(if n.is_multiple_of(2) {
        (finite[n / 2 - 1] + finite[n / 2]) * 0.5
    } else {
        finite[n / 2]
    })
}

/// Sample standard deviation (n-1 denominator) of the finite entries in
/// `values`. Returns 0.0 when fewer than two finite samples remain — this
/// matches the "std_dev = 0 → no spurious flags" contract documented on
/// [`detect_outliers`].
fn sample_std_dev_finite(values: &[f64]) -> f64 {
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    let n = finite.len();
    if n < 2 {
        return 0.0;
    }
    let mean = finite.iter().sum::<f64>() / n as f64;
    let variance: f64 = finite.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
    variance.sqrt()
}

/// Build per-commit stats for every loaded dataset.
///
/// `ordered_keys` drives which commits are emitted AND in what order —
/// missing keys are silently skipped. Each `(problem, format)` pair with
/// a finite metric value contributes one entry to [`CommitStats::values`].
#[must_use]
pub fn build_commit_stats(
    datasets: &HashMap<String, BenchmarkDataset>,
    ordered_keys: &[String],
    metric: MetricType,
    custom_formula: Option<&CompiledFormula>,
) -> Vec<CommitStats> {
    let mut out: Vec<CommitStats> = Vec::with_capacity(ordered_keys.len());

    for key in ordered_keys {
        let Some(ds) = datasets.get(key) else {
            continue;
        };

        let mut values: HashMap<(String, DataFormat), f64> = HashMap::new();
        for problem in &ds.benchmark {
            let baseline_entry = problem.spmv.get(DataFormat::CSR.as_key());
            for &format in &ALL_FORMATS {
                if let Some(v) = get_y_value(
                    &problem.spmv,
                    &format,
                    &metric,
                    &problem.problem,
                    baseline_entry,
                    custom_formula,
                ) {
                    if v.is_finite() {
                        values.insert((problem.problem.name.as_str().to_string(), format), v);
                    }
                }
            }
        }

        out.push(CommitStats {
            dataset_key: key.clone(),
            values,
        });
    }

    out
}

/// Compute the outlier report for each commit in `stats` using a rolling
/// baseline of the preceding `config.baseline_window` commits.
///
/// Semantics:
/// - The first `baseline_window` commits always have `is_outlier = false`
///   because there is no prior data to compare against.
/// - For each `(problem, format)` present in commit `i`, the baseline is the
///   set of values recorded for the same key across the window. With `n >= 2`
///   finite samples the baseline's median + sample std-dev are computed.
/// - When the baseline std-dev is exactly 0.0 (all samples identical) the
///   key is counted toward `total_measurements` but never as deviating —
///   otherwise one spurious NaN would flag everything.
/// - `deviating_percent` is `100 * deviating / total` (0.0 when total = 0).
/// - `is_outlier = deviating_percent > threshold_percent`.
#[must_use]
pub fn detect_outliers(
    stats: &[CommitStats],
    config: &OutlierDetectionConfig,
) -> Vec<CommitOutlierReport> {
    let mut reports: Vec<CommitOutlierReport> = Vec::with_capacity(stats.len());

    for (i, cur) in stats.iter().enumerate() {
        let start = i.saturating_sub(config.baseline_window);
        let baseline_slice = &stats[start..i];

        // First `baseline_window` commits: no prior data, so not flagged.
        if baseline_slice.is_empty() || i < config.baseline_window {
            reports.push(CommitOutlierReport {
                dataset_key: cur.dataset_key.clone(),
                total_measurements: 0,
                deviating_measurements: 0,
                deviating_percent: 0.0,
                is_outlier: false,
            });
            continue;
        }

        let mut total: usize = 0;
        let mut deviating: usize = 0;
        // Scratch buffer reused across keys to avoid per-iteration allocation.
        let mut bucket: Vec<f64> = Vec::with_capacity(baseline_slice.len());

        for (key, &value) in &cur.values {
            if !value.is_finite() {
                // NaN / inf in the current commit never counts as a deviation.
                continue;
            }
            bucket.clear();
            for prev in baseline_slice {
                if let Some(&v) = prev.values.get(key) {
                    if v.is_finite() {
                        bucket.push(v);
                    }
                }
            }
            if bucket.len() < 2 {
                // Not enough baseline history to build a sigma — skip the key
                // entirely rather than imply a false negative.
                continue;
            }

            let Some(median) = median_finite(&bucket) else {
                continue;
            };
            let std_dev = sample_std_dev_finite(&bucket);

            total += 1;

            // Floating-point "zero" guard: even with identical samples, the
            // computed sample std-dev can land on a subnormal due to rounding
            // in `mean = sum/n` and `(v - mean).powi(2)`. Treat anything at
            // or below `|median| * f64::EPSILON` as exactly zero to preserve
            // the "no false positives on flat baselines" contract.
            let std_floor = median.abs() * f64::EPSILON;
            if std_dev <= std_floor {
                // All baseline samples were (effectively) identical — refusing
                // to flag here keeps a single-run-constant commit from
                // producing spurious alerts. Documented behaviour; see
                // module-level doc.
                continue;
            }

            if (value - median).abs() > config.sigma_threshold * std_dev {
                deviating += 1;
            }
        }

        let deviating_percent = if total > 0 {
            100.0 * deviating as f64 / total as f64
        } else {
            0.0
        };
        let is_outlier = deviating_percent > config.threshold_percent;

        reports.push(CommitOutlierReport {
            dataset_key: cur.dataset_key.clone(),
            total_measurements: total,
            deviating_measurements: deviating,
            deviating_percent,
            is_outlier,
        });
    }

    reports
}

/// Compute a stable cache key for the outlier computation.
///
/// Hashes every input that can change the output: the config knobs (with
/// `f64::to_bits` for bit-exact equality), the ordered dataset keys, the
/// active dataset count + sorted key set so loads/unloads invalidate, the
/// custom-formula source (when Custom is the selected metric), and the
/// metric discriminator itself.
#[must_use]
pub fn outlier_cache_key(
    config: &OutlierDetectionConfig,
    ordered_keys: &[String],
    dataset_keys_sorted: &[String],
    custom_formula_src: Option<&str>,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    config.baseline_window.hash(&mut hasher);
    config.sigma_threshold.to_bits().hash(&mut hasher);
    config.threshold_percent.to_bits().hash(&mut hasher);
    config.metric.hash(&mut hasher);
    config.enabled.hash(&mut hasher);
    ordered_keys.hash(&mut hasher);
    dataset_keys_sorted.len().hash(&mut hasher);
    dataset_keys_sorted.hash(&mut hasher);
    custom_formula_src.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::type_complexity)]
    fn make_stats(rows: &[(&str, &[((&str, DataFormat), f64)])]) -> Vec<CommitStats> {
        rows.iter()
            .map(|(key, pairs)| CommitStats {
                dataset_key: (*key).to_string(),
                values: pairs
                    .iter()
                    .map(|((name, fmt), v)| (((*name).to_string(), *fmt), *v))
                    .collect(),
            })
            .collect()
    }

    fn default_config() -> OutlierDetectionConfig {
        OutlierDetectionConfig::default()
    }

    #[test]
    fn empty_inputs_return_empty() {
        let out = detect_outliers(&[], &default_config());
        assert!(out.is_empty());
    }

    #[test]
    fn baseline_window_size_respected() {
        // With baseline_window = 3, the first 3 commits cannot be flagged
        // regardless of values — there is no 3-commit history behind them.
        let stats = make_stats(&[
            ("c0", &[(("p", DataFormat::CSR), 1000.0)]),
            ("c1", &[(("p", DataFormat::CSR), 1.0)]),
            ("c2", &[(("p", DataFormat::CSR), 1000.0)]),
            ("c3", &[(("p", DataFormat::CSR), 1.0)]),
        ]);
        let mut config = default_config();
        config.baseline_window = 3;
        config.threshold_percent = 0.0;
        config.sigma_threshold = 1.0;
        let reports = detect_outliers(&stats, &config);
        assert_eq!(reports.len(), 4);
        for r in &reports[..3] {
            assert!(!r.is_outlier, "commit {} flagged too early", r.dataset_key);
        }
    }

    #[test]
    fn std_dev_zero_no_false_positives() {
        // Baseline is perfectly flat: std_dev = 0. Even a wildly different
        // current value must NOT be flagged.
        let stats = make_stats(&[
            ("c0", &[(("p", DataFormat::CSR), 1.0)]),
            ("c1", &[(("p", DataFormat::CSR), 1.0)]),
            ("c2", &[(("p", DataFormat::CSR), 1.0)]),
            ("c3", &[(("p", DataFormat::CSR), 1000.0)]),
        ]);
        let mut config = default_config();
        config.baseline_window = 3;
        config.threshold_percent = 0.0;
        config.sigma_threshold = 1.0;
        let reports = detect_outliers(&stats, &config);
        let last = reports.last().expect("non-empty");
        assert!(!last.is_outlier);
        assert_eq!(last.deviating_measurements, 0);
    }

    #[test]
    fn simple_outlier_detected() {
        // Baseline has mild variance; a 5.0 current value is many sigma
        // above the baseline median ~1.0 and gets flagged at K=3.
        let stats = make_stats(&[
            ("c0", &[(("p", DataFormat::CSR), 1.0)]),
            ("c1", &[(("p", DataFormat::CSR), 1.1)]),
            ("c2", &[(("p", DataFormat::CSR), 0.9)]),
            ("c3", &[(("p", DataFormat::CSR), 1.0)]),
            ("c4", &[(("p", DataFormat::CSR), 1.0)]),
            ("c5", &[(("p", DataFormat::CSR), 5.0)]),
        ]);
        let mut config = default_config();
        config.baseline_window = 5;
        config.threshold_percent = 10.0;
        config.sigma_threshold = 3.0;
        let reports = detect_outliers(&stats, &config);
        let last = reports.last().expect("non-empty");
        assert!(last.is_outlier, "expected last commit to be flagged");
        assert_eq!(last.deviating_measurements, 1);
        assert_eq!(last.total_measurements, 1);
    }

    #[test]
    fn nan_values_skipped() {
        // A NaN in the current commit must not count as deviating, and must
        // not crash. Baseline is clean 1.0s.
        let stats = make_stats(&[
            ("c0", &[(("p", DataFormat::CSR), 1.0)]),
            ("c1", &[(("p", DataFormat::CSR), 1.1)]),
            ("c2", &[(("p", DataFormat::CSR), 0.9)]),
            ("c3", &[(("p", DataFormat::CSR), f64::NAN)]),
        ]);
        let mut config = default_config();
        config.baseline_window = 3;
        let reports = detect_outliers(&stats, &config);
        let last = reports.last().expect("non-empty");
        assert_eq!(last.deviating_measurements, 0);
        assert_eq!(last.total_measurements, 0);
        assert!(!last.is_outlier);
    }

    #[test]
    fn baseline_window_larger_than_history() {
        // N = 20 but only 5 commits exist. None of the first 5 can be
        // flagged since the required window of 20 preceding commits is
        // unreachable.
        let stats = make_stats(&[
            ("c0", &[(("p", DataFormat::CSR), 1.0)]),
            ("c1", &[(("p", DataFormat::CSR), 100.0)]),
            ("c2", &[(("p", DataFormat::CSR), 1.0)]),
            ("c3", &[(("p", DataFormat::CSR), 100.0)]),
            ("c4", &[(("p", DataFormat::CSR), 1.0)]),
        ]);
        let mut config = default_config();
        config.baseline_window = 20;
        config.threshold_percent = 0.0;
        config.sigma_threshold = 0.01;
        let reports = detect_outliers(&stats, &config);
        assert_eq!(reports.len(), 5);
        for r in &reports {
            assert!(!r.is_outlier);
        }
    }

    #[test]
    fn multiple_keys_percent_gating() {
        // Two keys, one stable and one spiking. With threshold_percent = 40
        // (50% deviating > 40), the commit flips to outlier. With
        // threshold_percent = 60, it does not.
        let stats = make_stats(&[
            (
                "c0",
                &[
                    (("p", DataFormat::CSR), 1.0),
                    (("q", DataFormat::CSR), 10.0),
                ],
            ),
            (
                "c1",
                &[
                    (("p", DataFormat::CSR), 1.1),
                    (("q", DataFormat::CSR), 10.1),
                ],
            ),
            (
                "c2",
                &[(("p", DataFormat::CSR), 0.9), (("q", DataFormat::CSR), 9.9)],
            ),
            (
                "c3",
                &[
                    (("p", DataFormat::CSR), 1.0),
                    (("q", DataFormat::CSR), 500.0),
                ],
            ),
        ]);

        let mut config = default_config();
        config.baseline_window = 3;
        config.sigma_threshold = 3.0;

        config.threshold_percent = 40.0;
        let reports = detect_outliers(&stats, &config);
        let last = reports.last().expect("non-empty");
        assert_eq!(last.total_measurements, 2);
        assert_eq!(last.deviating_measurements, 1);
        assert!(last.is_outlier);

        config.threshold_percent = 60.0;
        let reports = detect_outliers(&stats, &config);
        let last = reports.last().expect("non-empty");
        assert!(!last.is_outlier);
    }

    proptest::proptest! {
        /// With a perfectly-flat baseline (std_dev = 0), any finite positive
        /// current value must NOT flag the commit. Guards against a
        /// regression where a divide-by-zero would let sigma math falsely
        /// "deviate".
        #[test]
        fn flat_baseline_never_flags(
            baseline_value in 1e-6f64..1e6f64,
            current_value in 1e-6f64..1e6f64,
        ) {
            let stats = make_stats(&[
                ("c0", &[(("p", DataFormat::CSR), baseline_value)]),
                ("c1", &[(("p", DataFormat::CSR), baseline_value)]),
                ("c2", &[(("p", DataFormat::CSR), baseline_value)]),
                ("c3", &[(("p", DataFormat::CSR), current_value)]),
            ]);
            let mut config = default_config();
            config.baseline_window = 3;
            config.threshold_percent = 0.0;
            config.sigma_threshold = 0.01;
            let reports = detect_outliers(&stats, &config);
            let last = reports.last().expect("non-empty");
            proptest::prop_assert!(!last.is_outlier);
            proptest::prop_assert_eq!(last.deviating_measurements, 0);
        }
    }
}

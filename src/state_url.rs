//! Versioned URL-fragment state encoding for the dashboard (T9t).
//!
//! Public contract: `<origin>/<path>/#v1=<base64url-encoded-json>`.
//!
//! On startup the native binary reads `--state=<value>` and the wasm binary
//! reads `window.location.hash`, strips the `v1=` prefix, base64url-decodes
//! the remainder into JSON, deserializes it into [`StateV1`], and applies
//! it to the [`Dashboard`] via [`Dashboard::apply_state_v1`].
//!
//! ## Forward compatibility
//!
//! The format is designed so future tasks can add optional fields without
//! breaking URLs already in the wild:
//!
//! - Every field except `schema` is `Option<_>` on decode; missing fields
//!   leave the dashboard default untouched.
//! - `serde(deny_unknown_fields)` is deliberately **not** used: unknown
//!   top-level fields are silently ignored. Bumping the schema (e.g.
//!   `schema: 2` with a breaking semantic change to an existing field)
//!   requires introducing `StateV2` + a `v2=` fragment prefix; the
//!   [`decode`]-side `match` on the prefix leaves that door open.
//! - All enum string names are hand-written (not `serde`-derived) so that
//!   renaming a Rust variant cannot change the wire format. The "golden
//!   fixture" test below will fail if the mapping drifts.
//!
//! ## Decode robustness
//!
//! Any malformed input — unknown prefix, bad base64, bad JSON, wrong
//! `schema` — produces `None` + a `log::warn!` and the caller falls back
//! to `Dashboard::new()` defaults. The decode path never panics.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::data::state::{CompareSide, Dashboard, ViewMode};
use crate::types::{AggregationKind, DataFormat, MetricType, PlotType, XaxisType};
use crate::visualization::formula;

/// Current schema version. Bumping this (to 2, 3, ...) requires a new
/// `StateVN` struct and a new `vN=` fragment prefix in [`decode`].
const SCHEMA_VERSION: u8 = 1;

/// Fragment prefix for the v1 schema.
const V1_PREFIX: &str = "v1=";

/// Decoded URL-fragment state (v1 schema).
///
/// Every field except `schema` is optional. Absent fields leave the
/// corresponding dashboard value at its default.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StateV1 {
    /// Schema version. Always 1 for this struct.
    pub schema: u8,
    /// Commit SHA prefix to select (matched against `short_sha`/`sha`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// View mode (see [`view_mode_to_str`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// Plot type (see [`plot_type_to_str`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_type: Option<String>,
    /// X-axis metric (see [`xaxis_to_str`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_axis: Option<String>,
    /// Y-axis metric (see [`metric_to_str`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_metric: Option<String>,
    /// Normalize flag for the scatter / profile view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalize: Option<bool>,
    /// Set of sparse-matrix formats to activate (see [`data_format_to_str`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formats: Option<Vec<String>>,
    /// Custom Y-axis formula source text. When present, `y_metric` is ignored
    /// and `data_metric` is set to `MetricType::Custom`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_formula: Option<String>,
    /// Dataset key (as shown in the sidebar) picked for side A of the
    /// Comparison plot type. Only populated when `plot_type == "Comparison"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare_a: Option<String>,
    /// Dataset key picked for side B of the Comparison plot type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare_b: Option<String>,
    /// `"A"` or `"B"`; unknown values log-warn and are ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare_baseline: Option<String>,
    /// Comparison orientation flag. `true` = lower metric (time) is better.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare_lower_is_better: Option<bool>,
    /// Line-timeseries aggregation kernel (see [`aggregation_to_str`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts_aggregation: Option<String>,
    /// Line-timeseries format pick (see [`data_format_to_str`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts_format: Option<String>,
    /// Line-timeseries problem-filter (exact `MatrixMetadata::name` match).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts_problem_filter: Option<String>,
    /// Stacked-bar dataset key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sb_dataset: Option<String>,
    /// Stacked-bar "sort by total" toggle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sb_sort_by_total: Option<bool>,
    /// Stacked-bar top-N limiter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sb_top_n: Option<usize>,
    /// Outlier detection master enable (Task 8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outlier_enabled: Option<bool>,
    /// Outlier detection baseline window (commits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outlier_window: Option<usize>,
    /// Outlier detection sigma threshold (K).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outlier_sigma: Option<f64>,
    /// Outlier detection deviation-percent gate (X, 0..=100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outlier_percent: Option<f64>,
    /// Outlier detection metric — serialized via [`metric_to_str`]. When
    /// absent, the loaded state defaults to the in-memory `outlier_config`
    /// metric (i.e. `Time`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outlier_metric: Option<String>,
}

impl Default for StateV1 {
    fn default() -> Self {
        Self {
            schema: SCHEMA_VERSION,
            commit: None,
            view: None,
            plot_type: None,
            x_axis: None,
            y_metric: None,
            normalize: None,
            formats: None,
            y_formula: None,
            compare_a: None,
            compare_b: None,
            compare_baseline: None,
            compare_lower_is_better: None,
            ts_aggregation: None,
            ts_format: None,
            ts_problem_filter: None,
            sb_dataset: None,
            sb_sort_by_total: None,
            sb_top_n: None,
            outlier_enabled: None,
            outlier_window: None,
            outlier_sigma: None,
            outlier_percent: None,
            outlier_metric: None,
        }
    }
}

impl StateV1 {
    /// Encode `self` to the fragment string (without the leading `#`).
    ///
    /// Output shape: `"v1=<base64url-no-pad>"`.
    pub fn encode(&self) -> String {
        // `serde_json::to_vec` on a `StateV1` can only fail if a custom
        // `Serialize` impl errors; ours derive and contain only `u8`,
        // `bool`, `String`, `Option<_>`, `Vec<String>` — all infallible.
        let json = serde_json::to_vec(self).unwrap_or_else(|_| b"{\"schema\":1}".to_vec());
        let b64 = URL_SAFE_NO_PAD.encode(&json);
        format!("{V1_PREFIX}{b64}")
    }

    /// Decode a fragment into a [`StateV1`].
    ///
    /// `fragment` may optionally include a leading `#`. Unknown version
    /// prefixes, malformed base64, malformed JSON, and `schema != 1`
    /// all produce `None` + a `log::warn!`. Never panics.
    pub fn decode(fragment: &str) -> Option<Self> {
        let fragment = fragment.strip_prefix('#').unwrap_or(fragment);
        if fragment.is_empty() {
            return None;
        }

        // Match on the version prefix. Keeping this as a `match` (rather
        // than a single `strip_prefix`) is deliberate: future `StateV2`
        // slots in here without touching the `v1=` arm.
        let Some(b64) = fragment.strip_prefix(V1_PREFIX) else {
            log::warn!("state_url: unknown fragment version prefix in {fragment:?}");
            return None;
        };

        let bytes = match URL_SAFE_NO_PAD.decode(b64) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("state_url: base64 decode failed: {e}");
                return None;
            }
        };

        let state: StateV1 = match serde_json::from_slice(&bytes) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("state_url: JSON decode failed: {e}");
                return None;
            }
        };

        if state.schema != SCHEMA_VERSION {
            log::warn!(
                "state_url: unsupported schema version {} (expected {})",
                state.schema,
                SCHEMA_VERSION
            );
            return None;
        }

        Some(state)
    }
}

// ---------------------------------------------------------------------------
// Enum <-> stable string mappings.
//
// These are hand-written on purpose (see module doc comment). If you rename
// a Rust variant, keep the string stable or bump the schema.
// ---------------------------------------------------------------------------

pub fn view_mode_to_str(v: ViewMode) -> &'static str {
    match v {
        ViewMode::Benchmark => "Benchmark",
        ViewMode::Solver => "Solver",
    }
}

pub fn view_mode_from_str(s: &str) -> Option<ViewMode> {
    match s {
        "Benchmark" => Some(ViewMode::Benchmark),
        "Solver" => Some(ViewMode::Solver),
        _ => None,
    }
}

pub fn plot_type_to_str(p: PlotType) -> &'static str {
    match p {
        PlotType::Scatter => "Scatter",
        PlotType::PerformanceProfile => "PerformanceProfile",
        PlotType::Comparison => "Comparison",
        PlotType::LineTimeseries => "LineTimeseries",
        PlotType::StackedBar => "StackedBar",
    }
}

pub fn plot_type_from_str(s: &str) -> Option<PlotType> {
    match s {
        "Scatter" => Some(PlotType::Scatter),
        "PerformanceProfile" => Some(PlotType::PerformanceProfile),
        "Comparison" => Some(PlotType::Comparison),
        "LineTimeseries" => Some(PlotType::LineTimeseries),
        "StackedBar" => Some(PlotType::StackedBar),
        _ => None,
    }
}

pub fn aggregation_to_str(a: AggregationKind) -> &'static str {
    match a {
        AggregationKind::Mean => "Mean",
        AggregationKind::Median => "Median",
        AggregationKind::GeometricMean => "GeometricMean",
    }
}

pub fn aggregation_from_str(s: &str) -> Option<AggregationKind> {
    match s {
        "Mean" => Some(AggregationKind::Mean),
        "Median" => Some(AggregationKind::Median),
        "GeometricMean" => Some(AggregationKind::GeometricMean),
        _ => None,
    }
}

pub fn xaxis_to_str(x: XaxisType) -> &'static str {
    match x {
        XaxisType::Cols => "Cols",
        XaxisType::ColCv => "ColCv",
        XaxisType::Rows => "Rows",
        XaxisType::RowCv => "RowCv",
        XaxisType::NonZeros => "NonZeros",
        XaxisType::Sparsity => "Sparsity",
        XaxisType::AvgNnzPerRow => "AvgNnzPerRow",
        XaxisType::AvgNnzPerCol => "AvgNnzPerCol",
        XaxisType::MatrixShapeRatio => "MatrixShapeRatio",
    }
}

pub fn xaxis_from_str(s: &str) -> Option<XaxisType> {
    match s {
        "Cols" => Some(XaxisType::Cols),
        "ColCv" => Some(XaxisType::ColCv),
        "Rows" => Some(XaxisType::Rows),
        "RowCv" => Some(XaxisType::RowCv),
        "NonZeros" => Some(XaxisType::NonZeros),
        "Sparsity" => Some(XaxisType::Sparsity),
        "AvgNnzPerRow" => Some(XaxisType::AvgNnzPerRow),
        "AvgNnzPerCol" => Some(XaxisType::AvgNnzPerCol),
        "MatrixShapeRatio" => Some(XaxisType::MatrixShapeRatio),
        _ => None,
    }
}

pub fn metric_to_str(m: MetricType) -> &'static str {
    match m {
        MetricType::Storage => "Storage",
        MetricType::Time => "Time",
        MetricType::GflopsPerSecond => "GflopsPerSecond",
        MetricType::Repetitions => "Repetitions",
        MetricType::OperationalIntensity => "OperationalIntensity",
        MetricType::EffectiveMemoryBandwidth => "EffectiveMemoryBandwidth",
        MetricType::Custom => "Custom",
    }
}

pub fn metric_from_str(s: &str) -> Option<MetricType> {
    match s {
        "Storage" => Some(MetricType::Storage),
        "Time" => Some(MetricType::Time),
        "GflopsPerSecond" => Some(MetricType::GflopsPerSecond),
        "Repetitions" => Some(MetricType::Repetitions),
        "OperationalIntensity" => Some(MetricType::OperationalIntensity),
        "EffectiveMemoryBandwidth" => Some(MetricType::EffectiveMemoryBandwidth),
        "Custom" => Some(MetricType::Custom),
        _ => None,
    }
}

pub fn data_format_to_str(f: DataFormat) -> &'static str {
    match f {
        DataFormat::CSR => "CSR",
        DataFormat::COO => "COO",
        DataFormat::ELL => "ELL",
        DataFormat::HYBRID => "HYBRID",
        DataFormat::SELLP => "SELLP",
    }
}

pub fn data_format_from_str(s: &str) -> Option<DataFormat> {
    match s {
        "CSR" => Some(DataFormat::CSR),
        "COO" => Some(DataFormat::COO),
        "ELL" => Some(DataFormat::ELL),
        "HYBRID" => Some(DataFormat::HYBRID),
        "SELLP" => Some(DataFormat::SELLP),
        _ => None,
    }
}

/// Every sparse-matrix format variant, in a canonical order. Used when a
/// URL supplies a partial `formats` list so the remaining formats can be
/// deactivated.
const ALL_DATA_FORMATS: &[DataFormat] = &[
    DataFormat::CSR,
    DataFormat::COO,
    DataFormat::ELL,
    DataFormat::HYBRID,
    DataFormat::SELLP,
];

impl Dashboard {
    /// Encode the current dashboard state to a [`StateV1`] fragment string.
    ///
    /// Returns the fragment (without leading `#`) suitable for `window.location.hash`.
    pub fn encode_state_v1(&self) -> String {
        let y_formula = if self.plot_config.data_metric == Some(MetricType::Custom) {
            self.plot_config
                .custom_formula
                .as_ref()
                .map(|f| f.source.clone())
        } else {
            None
        };

        let y_metric = if y_formula.is_none() {
            self.plot_config
                .data_metric
                .map(|m| metric_to_str(m).to_string())
        } else {
            None
        };

        // Comparison-view fields are only serialized when that plot type is
        // active; this keeps URLs for scatter/profile users free of clutter.
        let is_comparison = self.plot_config.plot_type == PlotType::Comparison;
        let (compare_a, compare_b, compare_baseline, compare_lower_is_better) = if is_comparison {
            (
                self.comparison.commit_a.clone(),
                self.comparison.commit_b.clone(),
                Some(match self.comparison.baseline_side {
                    CompareSide::A => "A".to_string(),
                    CompareSide::B => "B".to_string(),
                }),
                Some(self.comparison.lower_is_better),
            )
        } else {
            (None, None, None, None)
        };

        // Same clutter-guard for the new plot types: emit `ts_*` / `sb_*`
        // fields only when the user has that view active.
        let is_timeseries = self.plot_config.plot_type == PlotType::LineTimeseries;
        let (ts_aggregation, ts_format, ts_problem_filter) = if is_timeseries {
            (
                Some(aggregation_to_str(self.timeseries.aggregation).to_string()),
                Some(data_format_to_str(self.timeseries.format).to_string()),
                self.timeseries.problem_filter.clone(),
            )
        } else {
            (None, None, None)
        };

        let is_stacked_bar = self.plot_config.plot_type == PlotType::StackedBar;
        let (sb_dataset, sb_sort_by_total, sb_top_n) = if is_stacked_bar {
            (
                self.stacked_bar.dataset.clone(),
                Some(self.stacked_bar.sort_by_total),
                Some(self.stacked_bar.top_n),
            )
        } else {
            (None, None, None)
        };

        // Outlier detection (Task 8). Only emit the full knob set when the
        // user has turned detection on — keeps URLs terse for the common
        // case. The `enabled = true` flag carries the opt-in signal.
        let (outlier_enabled, outlier_window, outlier_sigma, outlier_percent, outlier_metric) =
            if self.outlier_config.enabled {
                (
                    Some(true),
                    Some(self.outlier_config.baseline_window),
                    Some(self.outlier_config.sigma_threshold),
                    Some(self.outlier_config.threshold_percent),
                    Some(metric_to_str(self.outlier_config.metric).to_string()),
                )
            } else {
                (None, None, None, None, None)
            };

        StateV1 {
            schema: SCHEMA_VERSION,
            commit: self
                .git
                .selected_commit_idx
                .and_then(|i| self.git.commits.get(i))
                .map(|c| c.short_sha.clone()),
            view: Some(view_mode_to_str(self.view_mode).to_string()),
            plot_type: Some(plot_type_to_str(self.plot_config.plot_type).to_string()),
            x_axis: self.plot_config.x_axis.map(|x| xaxis_to_str(x).to_string()),
            y_metric,
            normalize: Some(self.plot_config.normalize),
            formats: Some(
                ALL_DATA_FORMATS
                    .iter()
                    .filter(|&&f| {
                        self.data_selection
                            .active_formats
                            .get(&f)
                            .copied()
                            .unwrap_or(false)
                    })
                    .map(|&f| data_format_to_str(f).to_string())
                    .collect(),
            ),
            y_formula,
            compare_a,
            compare_b,
            compare_baseline,
            compare_lower_is_better,
            ts_aggregation,
            ts_format,
            ts_problem_filter,
            sb_dataset,
            sb_sort_by_total,
            sb_top_n,
            outlier_enabled,
            outlier_window,
            outlier_sigma,
            outlier_percent,
            outlier_metric,
        }
        .encode()
    }

    /// Apply a decoded [`StateV1`] onto `self`.
    ///
    /// Unknown enum-variant strings (`view: "Foobar"`) are warned and
    /// ignored per-field. Missing fields leave the corresponding dashboard
    /// value untouched. This method never mutates fields the URL did not
    /// name — it's purely additive.
    pub fn apply_state_v1(&mut self, state: &StateV1) {
        if let Some(v) = state.view.as_deref() {
            match view_mode_from_str(v) {
                Some(mode) => self.view_mode = mode,
                None => log::warn!("state_url: unknown view variant {v:?}"),
            }
        }

        if let Some(p) = state.plot_type.as_deref() {
            match plot_type_from_str(p) {
                Some(pt) => self.plot_config.plot_type = pt,
                None => log::warn!("state_url: unknown plot_type variant {p:?}"),
            }
        }

        if let Some(x) = state.x_axis.as_deref() {
            match xaxis_from_str(x) {
                Some(ax) => self.plot_config.x_axis = Some(ax),
                None => log::warn!("state_url: unknown x_axis variant {x:?}"),
            }
        }

        if let Some(src) = state.y_formula.as_deref() {
            self.plot_config.custom_formula_text = src.to_string();
            match formula::compile(src) {
                Ok(f) => {
                    self.plot_config.custom_formula = Some(f);
                    self.plot_config.data_metric = Some(MetricType::Custom);
                }
                Err(e) => {
                    log::warn!(
                        "state_url: y_formula compile failed ({e}); falling back to y_metric"
                    );
                    // Fall through to y_metric branch below.
                    if let Some(y) = state.y_metric.as_deref() {
                        match metric_from_str(y) {
                            Some(m) => self.plot_config.data_metric = Some(m),
                            None => log::warn!("state_url: unknown y_metric variant {y:?}"),
                        }
                    }
                }
            }
        } else if let Some(y) = state.y_metric.as_deref() {
            match metric_from_str(y) {
                Some(m) => self.plot_config.data_metric = Some(m),
                None => log::warn!("state_url: unknown y_metric variant {y:?}"),
            }
        }

        if let Some(n) = state.normalize {
            self.plot_config.normalize = n;
        }

        if let Some(fmts) = state.formats.as_ref() {
            // Build the active set from the URL list, warning on unknowns.
            let mut wanted: Vec<DataFormat> = Vec::with_capacity(fmts.len());
            for s in fmts {
                match data_format_from_str(s) {
                    Some(f) => wanted.push(f),
                    None => log::warn!("state_url: unknown format variant {s:?}"),
                }
            }
            for &f in ALL_DATA_FORMATS {
                self.data_selection
                    .active_formats
                    .insert(f, wanted.contains(&f));
            }
        }

        if let Some(a) = state.compare_a.as_ref() {
            self.comparison.commit_a = Some(a.clone());
        }
        if let Some(b) = state.compare_b.as_ref() {
            self.comparison.commit_b = Some(b.clone());
        }
        if let Some(side) = state.compare_baseline.as_deref() {
            match side {
                "A" => self.comparison.baseline_side = CompareSide::A,
                "B" => self.comparison.baseline_side = CompareSide::B,
                other => log::warn!("state_url: unknown compare_baseline variant {other:?}"),
            }
        }
        if let Some(lib) = state.compare_lower_is_better {
            self.comparison.lower_is_better = lib;
        }

        if let Some(agg) = state.ts_aggregation.as_deref() {
            match aggregation_from_str(agg) {
                Some(a) => self.timeseries.aggregation = a,
                None => log::warn!("state_url: unknown ts_aggregation variant {agg:?}"),
            }
        }
        if let Some(fmt) = state.ts_format.as_deref() {
            match data_format_from_str(fmt) {
                Some(f) => self.timeseries.format = f,
                None => log::warn!("state_url: unknown ts_format variant {fmt:?}"),
            }
        }
        if let Some(pf) = state.ts_problem_filter.as_ref() {
            self.timeseries.problem_filter = Some(pf.clone());
        }
        if let Some(ds) = state.sb_dataset.as_ref() {
            self.stacked_bar.dataset = Some(ds.clone());
        }
        if let Some(sort) = state.sb_sort_by_total {
            self.stacked_bar.sort_by_total = sort;
        }
        if let Some(n) = state.sb_top_n {
            self.stacked_bar.top_n = n;
        }

        // Outlier detection (Task 8). Each field is optional and applied
        // independently — an unknown metric string logs a warning and
        // leaves the existing value in place rather than failing the whole
        // URL apply.
        if let Some(en) = state.outlier_enabled {
            self.outlier_config.enabled = en;
        }
        if let Some(w) = state.outlier_window {
            self.outlier_config.baseline_window = w;
        }
        if let Some(s) = state.outlier_sigma {
            self.outlier_config.sigma_threshold = s;
        }
        if let Some(p) = state.outlier_percent {
            self.outlier_config.threshold_percent = p;
        }
        if let Some(m) = state.outlier_metric.as_deref() {
            match metric_from_str(m) {
                Some(metric) => self.outlier_config.metric = metric,
                None => log::warn!("state_url: unknown outlier_metric variant {m:?}"),
            }
        }

        if let Some(sha_prefix) = state.commit.as_deref() {
            // Match by prefix against `sha` or `short_sha`. If commits
            // haven't loaded yet (empty list on startup), leave the
            // selection at `None` — the main update loop can re-apply
            // once the commit list populates.
            // TODO(task-7): re-apply pending commit SHA after git.commits populates.
            if let Some(idx) =
                self.git.commits.iter().position(|c| {
                    c.sha.starts_with(sha_prefix) || c.short_sha.starts_with(sha_prefix)
                })
            {
                self.git.selected_commit_idx = Some(idx);
            } else {
                log::warn!(
                    "state_url: no commit with SHA prefix {sha_prefix:?} (may re-apply after load)"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a populated StateV1, decode, assert equality. Round-trips must
    /// be lossless for every field.
    #[test]
    fn roundtrip_populated() {
        let original = StateV1 {
            schema: 1,
            commit: Some("abcd123".to_string()),
            view: Some("Benchmark".to_string()),
            plot_type: Some("Scatter".to_string()),
            x_axis: Some("NonZeros".to_string()),
            y_metric: Some("Time".to_string()),
            normalize: Some(false),
            formats: Some(vec!["CSR".to_string(), "ELL".to_string()]),
            ..StateV1::default()
        };
        let encoded = original.encode();
        assert!(encoded.starts_with("v1="));
        let decoded = StateV1::decode(&encoded).expect("decode");
        assert_eq!(decoded, original);
    }

    /// Round-trip with a `#` prefix (as delivered by `window.location.hash`).
    #[test]
    fn roundtrip_with_hash_prefix() {
        let original = StateV1 {
            schema: 1,
            view: Some("Solver".to_string()),
            ..StateV1::default()
        };
        let encoded = format!("#{}", original.encode());
        let decoded = StateV1::decode(&encoded).expect("decode");
        assert_eq!(decoded.view.as_deref(), Some("Solver"));
    }

    /// Golden fixture: a hard-coded fragment string must decode to a known
    /// StateV1. If a future implementer renames one of the enum strings,
    /// this test will break and force them to think about wire compat.
    ///
    /// The fixture corresponds to the minimal JSON `{"schema":1,"view":"Solver"}`.
    #[test]
    fn golden_fixture_decodes_to_expected() {
        let golden = "v1=eyJzY2hlbWEiOjEsInZpZXciOiJTb2x2ZXIifQ";
        let decoded = StateV1::decode(golden).expect("golden fixture must decode");
        assert_eq!(decoded.schema, 1);
        assert_eq!(decoded.view.as_deref(), Some("Solver"));
        assert!(decoded.commit.is_none());
        assert!(decoded.plot_type.is_none());
        assert!(decoded.x_axis.is_none());
        assert!(decoded.y_metric.is_none());
        assert!(decoded.normalize.is_none());
        assert!(decoded.formats.is_none());
    }

    /// Unknown version prefix: no panic, `None` returned.
    #[test]
    fn unknown_version_prefix_returns_none() {
        assert!(StateV1::decode("v2=garbage").is_none());
        assert!(StateV1::decode("v99=whatever").is_none());
        assert!(StateV1::decode("").is_none());
    }

    /// Malformed base64: no panic, `None` returned.
    #[test]
    fn malformed_base64_returns_none() {
        // '???' is not a valid base64url character set.
        assert!(StateV1::decode("v1=???").is_none());
    }

    /// Malformed JSON (valid base64 of garbage): `None`.
    #[test]
    fn malformed_json_returns_none() {
        let garbage = URL_SAFE_NO_PAD.encode(b"{not valid json at all");
        let fragment = format!("v1={garbage}");
        assert!(StateV1::decode(&fragment).is_none());
    }

    /// Unknown top-level fields decode successfully and are silently ignored
    /// (forward-compat invariant).
    #[test]
    fn unknown_fields_are_ignored() {
        let json = br#"{"schema":1,"future_field":123,"another":"xyz"}"#;
        let encoded = URL_SAFE_NO_PAD.encode(json);
        let fragment = format!("v1={encoded}");
        let decoded = StateV1::decode(&fragment).expect("decode");
        assert_eq!(decoded.schema, 1);
        assert!(decoded.view.is_none());
    }

    /// Wrong schema version: `None` + warn. Prevents `StateV2` payloads from
    /// being silently interpreted as v1.
    #[test]
    fn wrong_schema_version_returns_none() {
        let json = br#"{"schema":2}"#;
        let encoded = URL_SAFE_NO_PAD.encode(json);
        let fragment = format!("v1={encoded}");
        assert!(StateV1::decode(&fragment).is_none());
    }

    /// Unknown enum variant on `view` should NOT fail the whole decode —
    /// the string just won't map and `apply_state_v1` will warn + skip.
    #[test]
    fn unknown_enum_variant_decodes_but_does_not_apply() {
        let json = br#"{"schema":1,"view":"Foobar"}"#;
        let encoded = URL_SAFE_NO_PAD.encode(json);
        let fragment = format!("v1={encoded}");
        let decoded = StateV1::decode(&fragment).expect("decode should succeed");
        assert_eq!(decoded.view.as_deref(), Some("Foobar"));

        // Applying the unknown variant leaves view_mode at its default.
        let mut dash = Dashboard::new();
        let before = dash.view_mode;
        dash.apply_state_v1(&decoded);
        assert_eq!(dash.view_mode, before, "unknown variant must not mutate");
    }

    /// Applying a fully-populated StateV1 must mutate all the addressed
    /// dashboard fields and leave non-addressed ones alone.
    #[test]
    fn apply_state_v1_mutates_expected_fields() {
        let state = StateV1 {
            schema: 1,
            commit: None, // no commits loaded, so skipped (and that's fine)
            view: Some("Solver".to_string()),
            plot_type: Some("PerformanceProfile".to_string()),
            x_axis: Some("Rows".to_string()),
            y_metric: Some("GflopsPerSecond".to_string()),
            normalize: Some(true),
            formats: Some(vec!["CSR".to_string(), "ELL".to_string()]),
            ..StateV1::default()
        };

        let mut dash = Dashboard::new();
        dash.apply_state_v1(&state);

        assert_eq!(dash.view_mode, ViewMode::Solver);
        assert_eq!(dash.plot_config.plot_type, PlotType::PerformanceProfile);
        assert_eq!(dash.plot_config.x_axis, Some(XaxisType::Rows));
        assert_eq!(
            dash.plot_config.data_metric,
            Some(MetricType::GflopsPerSecond)
        );
        assert!(dash.plot_config.normalize);
        assert_eq!(
            dash.data_selection.active_formats.get(&DataFormat::CSR),
            Some(&true)
        );
        assert_eq!(
            dash.data_selection.active_formats.get(&DataFormat::ELL),
            Some(&true)
        );
        assert_eq!(
            dash.data_selection.active_formats.get(&DataFormat::COO),
            Some(&false)
        );
        assert_eq!(
            dash.data_selection.active_formats.get(&DataFormat::HYBRID),
            Some(&false)
        );
        assert_eq!(
            dash.data_selection.active_formats.get(&DataFormat::SELLP),
            Some(&false)
        );
    }

    /// `apply_state_v1` with a commit SHA prefix must pick up the matching
    /// index in `git.commits` after Task 7's on_commit_selected refactor.
    /// Prevents silent regression of the URL-driven commit selector.
    #[test]
    fn apply_state_v1_resolves_commit_prefix() {
        use crate::data::git::CommitInfo;

        let mut dash = Dashboard::new();
        dash.git.commits = vec![
            CommitInfo {
                sha: "abcdef0123456789abcdef0123456789abcdef01".to_string(),
                short_sha: "abcdef0".to_string(),
                author: "x <x@y>".to_string(),
                date: "2026-01-01T00:00:00Z".to_string(),
                message: "first".to_string(),
                bench_file: None,
            },
            CommitInfo {
                sha: "1234567890abcdef1234567890abcdef12345678".to_string(),
                short_sha: "1234567".to_string(),
                author: "x <x@y>".to_string(),
                date: "2026-01-02T00:00:00Z".to_string(),
                message: "second".to_string(),
                bench_file: None,
            },
        ];

        let state = StateV1 {
            schema: 1,
            commit: Some("1234567".to_string()),
            ..StateV1::default()
        };
        dash.apply_state_v1(&state);
        assert_eq!(dash.git.selected_commit_idx, Some(1));
    }

    /// `encode_state_v1` round-trips the currently-selected commit SHA prefix
    /// so the URL survives Task 7's scrubber interactions.
    #[test]
    fn encode_state_v1_preserves_selected_commit() {
        use crate::data::git::CommitInfo;

        let mut dash = Dashboard::new();
        dash.git.commits = vec![CommitInfo {
            sha: "deadbee".repeat(6) + "abcdef0000",
            short_sha: "deadbee".to_string(),
            author: "x <x@y>".to_string(),
            date: "2026-01-01T00:00:00Z".to_string(),
            message: "first".to_string(),
            bench_file: None,
        }];
        dash.git.selected_commit_idx = Some(0);

        let encoded = dash.encode_state_v1();
        let decoded = StateV1::decode(&encoded).expect("decode");
        assert_eq!(decoded.commit.as_deref(), Some("deadbee"));
    }

    /// y_formula field round-trips through encode/decode without loss.
    #[test]
    fn y_formula_roundtrips() {
        let original = StateV1 {
            schema: 1,
            y_formula: Some("gflops / time".to_string()),
            ..StateV1::default()
        };
        let encoded = original.encode();
        let decoded = StateV1::decode(&encoded).expect("decode");
        assert_eq!(decoded.y_formula.as_deref(), Some("gflops / time"));
    }

    /// A valid y_formula sets data_metric = Custom and populates formula text.
    #[test]
    fn y_formula_decodes_to_custom_metric_via_apply_state_v1() {
        let state = StateV1 {
            schema: 1,
            y_formula: Some("gflops / time".to_string()),
            ..StateV1::default()
        };
        let mut dash = Dashboard::new();
        dash.apply_state_v1(&state);
        assert_eq!(dash.plot_config.data_metric, Some(MetricType::Custom));
        assert_eq!(dash.plot_config.custom_formula_text, "gflops / time");
        assert!(
            dash.plot_config.custom_formula.is_some(),
            "formula should be compiled"
        );
    }

    /// An invalid y_formula falls back to y_metric.
    #[test]
    fn y_formula_parse_failure_falls_back_to_y_metric() {
        let state = StateV1 {
            schema: 1,
            y_formula: Some("bogus_var + 1".to_string()),
            y_metric: Some("Storage".to_string()),
            ..StateV1::default()
        };
        let mut dash = Dashboard::new();
        dash.apply_state_v1(&state);
        assert_eq!(
            dash.plot_config.data_metric,
            Some(MetricType::Storage),
            "should fall back to y_metric on parse failure"
        );
        assert!(
            dash.plot_config.custom_formula.is_none(),
            "formula should not be set after parse failure"
        );
    }

    /// All four compare_* fields survive an encode → decode round-trip
    /// unchanged, independent of the other StateV1 fields.
    #[test]
    fn compare_fields_roundtrip() {
        let original = StateV1 {
            schema: 1,
            plot_type: Some("Comparison".to_string()),
            compare_a: Some("commit-abc".to_string()),
            compare_b: Some("commit-def".to_string()),
            compare_baseline: Some("B".to_string()),
            compare_lower_is_better: Some(false),
            ..StateV1::default()
        };
        let encoded = original.encode();
        let decoded = StateV1::decode(&encoded).expect("decode");
        assert_eq!(decoded.compare_a.as_deref(), Some("commit-abc"));
        assert_eq!(decoded.compare_b.as_deref(), Some("commit-def"));
        assert_eq!(decoded.compare_baseline.as_deref(), Some("B"));
        assert_eq!(decoded.compare_lower_is_better, Some(false));
    }

    /// Default-constructed StateV1 must not emit any `compare_*` keys in the
    /// encoded payload — URL length stays minimal for non-comparison users.
    #[test]
    fn compare_fields_default_omitted() {
        let encoded = StateV1::default().encode();
        let b64 = encoded.strip_prefix("v1=").expect("v1 prefix");
        let bytes = URL_SAFE_NO_PAD.decode(b64).expect("base64");
        let json = std::str::from_utf8(&bytes).expect("utf8");
        assert!(
            !json.contains("compare_"),
            "expected no compare_* keys in default-encoded payload, got {json}"
        );
    }

    /// An unknown `compare_baseline` string must warn + leave the side alone.
    #[test]
    fn compare_baseline_unknown_is_ignored() {
        let state = StateV1 {
            schema: 1,
            compare_baseline: Some("middle".to_string()),
            ..StateV1::default()
        };
        let mut dash = Dashboard::new();
        let before = dash.comparison.baseline_side;
        dash.apply_state_v1(&state);
        assert_eq!(dash.comparison.baseline_side, before);
    }

    /// Line-timeseries fields survive an encode → decode round-trip.
    #[test]
    fn timeseries_fields_roundtrip() {
        let original = StateV1 {
            schema: 1,
            plot_type: Some("LineTimeseries".to_string()),
            ts_aggregation: Some("GeometricMean".to_string()),
            ts_format: Some("ELL".to_string()),
            ts_problem_filter: Some("matrix_42".to_string()),
            ..StateV1::default()
        };
        let encoded = original.encode();
        let decoded = StateV1::decode(&encoded).expect("decode");
        assert_eq!(decoded.ts_aggregation.as_deref(), Some("GeometricMean"));
        assert_eq!(decoded.ts_format.as_deref(), Some("ELL"));
        assert_eq!(decoded.ts_problem_filter.as_deref(), Some("matrix_42"));
    }

    /// Stacked-bar fields survive an encode → decode round-trip.
    #[test]
    fn stacked_bar_fields_roundtrip() {
        let original = StateV1 {
            schema: 1,
            plot_type: Some("StackedBar".to_string()),
            sb_dataset: Some("bench_abc".to_string()),
            sb_sort_by_total: Some(false),
            sb_top_n: Some(42),
            ..StateV1::default()
        };
        let encoded = original.encode();
        let decoded = StateV1::decode(&encoded).expect("decode");
        assert_eq!(decoded.sb_dataset.as_deref(), Some("bench_abc"));
        assert_eq!(decoded.sb_sort_by_total, Some(false));
        assert_eq!(decoded.sb_top_n, Some(42));
    }

    /// Unknown `ts_aggregation` warns + leaves the field at its default.
    #[test]
    fn timeseries_unknown_aggregation_is_ignored() {
        let state = StateV1 {
            schema: 1,
            ts_aggregation: Some("HarmonicMean".to_string()),
            ..StateV1::default()
        };
        let mut dash = Dashboard::new();
        let before = dash.timeseries.aggregation;
        dash.apply_state_v1(&state);
        assert_eq!(dash.timeseries.aggregation, before);
    }

    /// Default-constructed StateV1 must not emit any `ts_` or `sb_` keys.
    #[test]
    fn default_state_omits_new_fields() {
        let encoded = StateV1::default().encode();
        let b64 = encoded.strip_prefix("v1=").expect("v1 prefix");
        let bytes = URL_SAFE_NO_PAD.decode(b64).expect("base64");
        let json = std::str::from_utf8(&bytes).expect("utf8");
        assert!(
            !json.contains("\"ts_"),
            "expected no ts_* keys in default encoded payload, got {json}"
        );
        assert!(
            !json.contains("\"sb_"),
            "expected no sb_* keys in default encoded payload, got {json}"
        );
    }

    /// Outlier detection fields survive an encode → decode round-trip.
    #[test]
    fn outlier_fields_roundtrip() {
        let original = StateV1 {
            schema: 1,
            outlier_enabled: Some(true),
            outlier_window: Some(7),
            outlier_sigma: Some(2.5),
            outlier_percent: Some(15.0),
            outlier_metric: Some("GflopsPerSecond".to_string()),
            ..StateV1::default()
        };
        let encoded = original.encode();
        let decoded = StateV1::decode(&encoded).expect("decode");
        assert_eq!(decoded.outlier_enabled, Some(true));
        assert_eq!(decoded.outlier_window, Some(7));
        assert_eq!(decoded.outlier_sigma, Some(2.5));
        assert_eq!(decoded.outlier_percent, Some(15.0));
        assert_eq!(decoded.outlier_metric.as_deref(), Some("GflopsPerSecond"));
    }

    /// An unknown `outlier_metric` string must warn + leave the metric at
    /// its prior value rather than crash the whole apply.
    #[test]
    fn outlier_invalid_metric_falls_back_to_none() {
        use crate::data::state::Dashboard;

        let state = StateV1 {
            schema: 1,
            outlier_enabled: Some(true),
            outlier_metric: Some("NotAMetric".to_string()),
            ..StateV1::default()
        };
        let mut dash = Dashboard::new();
        let before = dash.outlier_config.metric;
        dash.apply_state_v1(&state);
        assert!(dash.outlier_config.enabled);
        assert_eq!(
            dash.outlier_config.metric, before,
            "unknown metric must not mutate"
        );
    }

    /// Default-constructed StateV1 must not emit any `outlier_*` keys —
    /// keeps the URL short for the opt-out default.
    #[test]
    fn default_state_omits_outlier_fields() {
        let encoded = StateV1::default().encode();
        let b64 = encoded.strip_prefix("v1=").expect("v1 prefix");
        let bytes = URL_SAFE_NO_PAD.decode(b64).expect("base64");
        let json = std::str::from_utf8(&bytes).expect("utf8");
        assert!(
            !json.contains("\"outlier_"),
            "expected no outlier_* keys in default encoded payload, got {json}"
        );
    }

    /// Stable string mapping survives for the new plot types.
    #[test]
    fn new_plot_types_string_mapping() {
        assert_eq!(plot_type_to_str(PlotType::LineTimeseries), "LineTimeseries");
        assert_eq!(plot_type_to_str(PlotType::StackedBar), "StackedBar");
        assert_eq!(
            plot_type_from_str("LineTimeseries"),
            Some(PlotType::LineTimeseries)
        );
        assert_eq!(plot_type_from_str("StackedBar"), Some(PlotType::StackedBar));
    }

    /// `aggregation_to_str`/`from_str` are symmetric and reject unknowns.
    #[test]
    fn aggregation_string_mapping() {
        for kind in [
            AggregationKind::Mean,
            AggregationKind::Median,
            AggregationKind::GeometricMean,
        ] {
            let s = aggregation_to_str(kind);
            assert_eq!(aggregation_from_str(s), Some(kind));
        }
        assert_eq!(aggregation_from_str("median"), None); // case-sensitive
        assert_eq!(aggregation_from_str(""), None);
    }
}

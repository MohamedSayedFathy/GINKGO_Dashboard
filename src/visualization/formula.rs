use crate::data::models::{BenchmarkEntry, MatrixMetadata};
use exmex::prelude::*;
use std::fmt;

// ---------------------------------------------------------------------------
// Variable slot enum — maps an alphabetically-ordered exmex variable index to
// its source field.  Resolved once at compile time; eval is array indexing.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum VarSlot {
    // entry fields
    Time,
    Storage,
    Repetitions,
    MaxRelativeNorm2,
    GflopsPerSecond, // also aliased as "gflops"
    BytesPerNnz,
    OperationalIntensity,
    EffectiveMemoryBandwidth,
    // matrix fields
    Rows,
    Cols,
    Nonzeros,
    Sparsity,
    AvgNnzPerRow,
    AvgNnzPerCol,
    MatrixShapeRatio,
    Psym,
    Nsym,
    RowCv,
    ColCv,
    // baseline fields
    BaselineTime,
    BaselineStorage,
    BaselineGflopsPerSecond,
    BaselineRepetitions,
    BaselineOperationalIntensity,
    BaselineEffectiveMemoryBandwidth,
    BaselineBytesPerNnz,
}

// Whitelist: (name-in-formula, VarSlot). Must be sorted by name so the
// suggestion search and the tests both work predictably.
const VARS: &[(&str, VarSlot)] = &[
    ("avg_nnz_per_col", VarSlot::AvgNnzPerCol),
    ("avg_nnz_per_row", VarSlot::AvgNnzPerRow),
    ("baseline_bytes_per_nnz", VarSlot::BaselineBytesPerNnz),
    (
        "baseline_effective_memory_bandwidth",
        VarSlot::BaselineEffectiveMemoryBandwidth,
    ),
    ("baseline_gflops", VarSlot::BaselineGflopsPerSecond),
    (
        "baseline_gflops_per_second",
        VarSlot::BaselineGflopsPerSecond,
    ),
    (
        "baseline_operational_intensity",
        VarSlot::BaselineOperationalIntensity,
    ),
    ("baseline_repetitions", VarSlot::BaselineRepetitions),
    ("baseline_storage", VarSlot::BaselineStorage),
    ("baseline_time", VarSlot::BaselineTime),
    ("bytes_per_nnz", VarSlot::BytesPerNnz),
    ("col_cv", VarSlot::ColCv),
    ("cols", VarSlot::Cols),
    (
        "effective_memory_bandwidth",
        VarSlot::EffectiveMemoryBandwidth,
    ),
    ("gflops", VarSlot::GflopsPerSecond),
    ("gflops_per_second", VarSlot::GflopsPerSecond),
    ("matrix_shape_ratio", VarSlot::MatrixShapeRatio),
    ("max_relative_norm2", VarSlot::MaxRelativeNorm2),
    ("nonzeros", VarSlot::Nonzeros),
    ("nsym", VarSlot::Nsym),
    ("operational_intensity", VarSlot::OperationalIntensity),
    ("psym", VarSlot::Psym),
    ("repetitions", VarSlot::Repetitions),
    ("row_cv", VarSlot::RowCv),
    ("rows", VarSlot::Rows),
    ("sparsity", VarSlot::Sparsity),
    ("storage", VarSlot::Storage),
    ("time", VarSlot::Time),
];

/// All known variable names, exposed for UI tooltips.
pub const KNOWN_VARIABLES: &[&str] = &[
    "avg_nnz_per_col",
    "avg_nnz_per_row",
    "baseline_bytes_per_nnz",
    "baseline_effective_memory_bandwidth",
    "baseline_gflops",
    "baseline_gflops_per_second",
    "baseline_operational_intensity",
    "baseline_repetitions",
    "baseline_storage",
    "baseline_time",
    "bytes_per_nnz",
    "col_cv",
    "cols",
    "effective_memory_bandwidth",
    "gflops",
    "gflops_per_second",
    "matrix_shape_ratio",
    "max_relative_norm2",
    "nonzeros",
    "nsym",
    "operational_intensity",
    "psym",
    "repetitions",
    "row_cv",
    "rows",
    "sparsity",
    "storage",
    "time",
];

// Compile-time guard: keep VARS and KNOWN_VARIABLES aligned. If you add an
// entry to one, add it to the other.
const _: () = assert!(
    VARS.len() == KNOWN_VARIABLES.len(),
    "VARS and KNOWN_VARIABLES must be the same length"
);

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum FormulaError {
    Empty,
    Parse(String),
    UnknownVariable {
        name: String,
        suggestion: Option<String>,
    },
}

impl fmt::Display for FormulaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormulaError::Empty => write!(f, "formula is empty"),
            FormulaError::Parse(e) => write!(f, "parse error: {e}"),
            FormulaError::UnknownVariable { name, suggestion } => {
                write!(f, "unknown variable '{name}'")?;
                if let Some(s) = suggestion {
                    write!(f, " — did you mean '{s}'?")?;
                }
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CompiledFormula
// ---------------------------------------------------------------------------

pub struct CompiledFormula {
    pub source: String,
    expr: FlatEx<f64>,
    var_slots: Vec<VarSlot>,
}

/// Compute a simple Levenshtein distance (capped at 3 to keep it O(n*m) but bounded).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    let mut row: Vec<usize> = (0..=n).collect();
    for i in 1..=m {
        let mut prev = row[0];
        row[0] = i;
        for j in 1..=n {
            let old = row[j];
            row[j] = if a[i - 1] == b[j - 1] {
                prev
            } else {
                1 + prev.min(row[j]).min(row[j - 1])
            };
            prev = old;
        }
    }
    row[n]
}

fn suggest(name: &str) -> Option<String> {
    VARS.iter()
        .map(|(k, _)| (*k, levenshtein(name, k)))
        .filter(|(_, d)| *d <= 2)
        .min_by_key(|(_, d)| *d)
        .map(|(k, _)| k.to_string())
}

/// Compile a formula source string into a `CompiledFormula`.
///
/// Variables are resolved against the whitelist at this point; per-point eval
/// is array indexing with no HashMap lookups.
pub fn compile(src: &str) -> Result<CompiledFormula, FormulaError> {
    let src = src.trim();
    if src.is_empty() {
        return Err(FormulaError::Empty);
    }

    let expr = exmex::parse::<f64>(src).map_err(|e| FormulaError::Parse(e.to_string()))?;

    // exmex returns var_names() in alphabetical order; build var_slots in that same order.
    let mut var_slots = Vec::with_capacity(expr.var_names().len());
    for var_name in expr.var_names() {
        match VARS.iter().find(|(k, _)| *k == var_name.as_str()) {
            Some((_, slot)) => var_slots.push(*slot),
            None => {
                return Err(FormulaError::UnknownVariable {
                    name: var_name.clone(),
                    suggestion: suggest(var_name),
                });
            }
        }
    }

    Ok(CompiledFormula {
        source: src.to_string(),
        expr,
        var_slots,
    })
}

impl CompiledFormula {
    /// Evaluate the formula for a single data point.
    ///
    /// Returns `None` if:
    /// - a baseline slot is referenced but `baseline` is `None`,
    /// - any value is non-finite (NaN / infinity), or
    /// - the expression itself returns a non-finite result.
    pub fn eval(
        &self,
        entry: &BenchmarkEntry,
        matrix: &MatrixMetadata,
        baseline: Option<&BenchmarkEntry>,
    ) -> Option<f64> {
        let mut buf: Vec<f64> = Vec::with_capacity(self.var_slots.len());

        for slot in &self.var_slots {
            let val = match slot {
                VarSlot::Time => entry.time.unwrap_or(f64::NAN),
                VarSlot::Storage => entry.storage.map(|s| s as f64).unwrap_or(f64::NAN),
                VarSlot::Repetitions => entry.repetitions.map(|r| r as f64).unwrap_or(f64::NAN),
                VarSlot::MaxRelativeNorm2 => entry.max_relative_norm2.unwrap_or(f64::NAN),
                VarSlot::GflopsPerSecond => entry.gflops_per_second,
                VarSlot::BytesPerNnz => entry.bytes_per_nnz,
                VarSlot::OperationalIntensity => entry.operational_intensity,
                VarSlot::EffectiveMemoryBandwidth => entry.effective_memory_bandwidth,
                VarSlot::Rows => matrix.rows as f64,
                VarSlot::Cols => matrix.cols as f64,
                VarSlot::Nonzeros => matrix.nonzeros as f64,
                VarSlot::Sparsity => matrix.sparsity,
                VarSlot::AvgNnzPerRow => matrix.avg_nnz_per_row,
                VarSlot::AvgNnzPerCol => matrix.avg_nnz_per_col,
                VarSlot::MatrixShapeRatio => matrix.matrix_shape_ratio,
                VarSlot::Psym => matrix.psym,
                VarSlot::Nsym => matrix.nsym,
                VarSlot::RowCv => matrix.row_distribution.row_cv,
                VarSlot::ColCv => matrix.col_distribution.col_cv,
                // Baseline slots — if baseline is absent, filter the point out.
                VarSlot::BaselineTime => baseline?.time.unwrap_or(f64::NAN),
                VarSlot::BaselineStorage => baseline?.storage.map(|s| s as f64).unwrap_or(f64::NAN),
                VarSlot::BaselineGflopsPerSecond => baseline?.gflops_per_second,
                VarSlot::BaselineRepetitions => {
                    baseline?.repetitions.map(|r| r as f64).unwrap_or(f64::NAN)
                }
                VarSlot::BaselineOperationalIntensity => baseline?.operational_intensity,
                VarSlot::BaselineEffectiveMemoryBandwidth => baseline?.effective_memory_bandwidth,
                VarSlot::BaselineBytesPerNnz => baseline?.bytes_per_nnz,
            };

            if !val.is_finite() {
                return None;
            }
            buf.push(val);
        }

        self.expr.eval(&buf).ok().filter(|v| v.is_finite())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::models::{
        BenchmarkEntry, MatrixColumnsMetadata, MatrixMetadata, MatrixRowsMetadata,
    };

    fn make_entry(time: f64, gflops: f64, storage: u64) -> BenchmarkEntry {
        BenchmarkEntry {
            time: Some(time),
            storage: Some(storage),
            max_relative_norm2: Some(1e-6),
            repetitions: Some(5),
            completed: true,
            gflops_per_second: gflops,
            bytes_per_nnz: 8.0,
            operational_intensity: 1.5,
            effective_memory_bandwidth: 42.0,
        }
    }

    fn make_matrix() -> MatrixMetadata {
        MatrixMetadata {
            id: 1,
            group: "g".into(),
            name: "m".into(),
            rows: 1000,
            cols: 1000,
            nonzeros: 5000,
            real: true,
            binary: false,
            is_2d3d: false,
            posdef: false,
            psym: 0.9,
            nsym: 0.8,
            kind: "k".into(),
            row_distribution: MatrixRowsMetadata {
                min: 0.0,
                q1: 0.0,
                median: 5.0,
                q3: 0.0,
                max: 10.0,
                mean: 5.0,
                variance: 4.0,
                skewness: 0.0,
                kurtosis: 0.0,
                hyperskewness: 0.0,
                hyperflatness: 0.0,
                row_irregularity: 0.0,
                row_cv: 0.4,
            },
            col_distribution: MatrixColumnsMetadata {
                min: 0.0,
                q1: 0.0,
                median: 5.0,
                q3: 0.0,
                max: 10.0,
                mean: 5.0,
                variance: 4.0,
                skewness: 0.0,
                kurtosis: 0.0,
                hyperskewness: 0.0,
                hyperflatness: 0.0,
                col_irregularity: 0.0,
                col_cv: 0.4,
            },
            sparsity: 0.005,
            avg_nnz_per_row: 5.0,
            avg_nnz_per_col: 5.0,
            matrix_shape_ratio: 1.0,
        }
    }

    #[test]
    fn parses_simple_ratio() {
        let f = compile("gflops / bytes_per_nnz").expect("should parse");
        assert_eq!(f.expr.var_names().len(), 2);
    }

    #[test]
    fn parses_log_difference() {
        let _f = compile("ln(time) - ln(baseline_time)").expect("should parse");
    }

    #[test]
    fn rejects_unknown_with_suggestion() {
        match compile("gflop / time") {
            Err(FormulaError::UnknownVariable { name, suggestion }) => {
                assert_eq!(name, "gflop");
                assert_eq!(suggestion.as_deref(), Some("gflops"));
            }
            other => panic!("expected UnknownVariable, got {:?}", other.map(|_| "ok")),
        }
    }

    #[test]
    fn rejects_parse_error() {
        assert!(matches!(compile("time +"), Err(FormulaError::Parse(_))));
    }

    #[test]
    fn rejects_empty() {
        assert!(matches!(compile(""), Err(FormulaError::Empty)));
        assert!(matches!(compile("  "), Err(FormulaError::Empty)));
    }

    #[test]
    fn rejects_injection_garbage() {
        // semicolons are not valid exmex tokens
        assert!(compile("; rm -rf /").is_err());
    }

    #[test]
    fn alphabetical_var_order() {
        // "time - gflops_per_second" — exmex orders slice alphabetically:
        // gflops_per_second < time, so buf[0] = gflops_per_second (3.0),
        // buf[1] = time (2.0), and result = 2.0 - 3.0 = -1.0.
        // If var_slots were built in source order instead of alphabetical
        // order, the slots would be swapped and the result would be +1.0,
        // so this test actually catches a swap (unlike a commutative +).
        let f = compile("time - gflops_per_second").expect("should parse");
        let entry = make_entry(2.0, 3.0, 100);
        let matrix = make_matrix();
        let result = f.eval(&entry, &matrix, None).expect("should eval");
        assert!(
            (result - (-1.0)).abs() < 1e-12,
            "expected -1.0, got {result}"
        );
    }

    #[test]
    fn returns_none_when_baseline_missing_but_referenced() {
        let f = compile("time / baseline_time").expect("should parse");
        let entry = make_entry(2.0, 3.0, 100);
        let matrix = make_matrix();
        assert!(f.eval(&entry, &matrix, None).is_none());
    }

    #[test]
    fn eval_with_baseline_present() {
        let f = compile("time / baseline_time").expect("should parse");
        let entry = make_entry(2.0, 3.0, 100);
        let baseline = make_entry(4.0, 6.0, 200);
        let matrix = make_matrix();
        let result = f
            .eval(&entry, &matrix, Some(&baseline))
            .expect("should eval");
        assert!((result - 0.5).abs() < 1e-12);
    }

    /// Pipeline test: build entry with known values, run calculate_performance_metrics,
    /// compile a formula, eval, assert numeric equality.
    #[test]
    fn pipeline_gflops_over_bytes_per_nnz() {
        let mut matrix = make_matrix();
        matrix.calculate_derived_metrics();

        let mut entry = make_entry(0.001, 0.0, 40_000);
        // optimal_bytes = 0 → falls back to reported storage
        entry.calculate_performance_metrics(&matrix, 0.0);

        let expected_gflops = (2.0 * 5000.0) / 0.001 / 1e9;
        let expected_bytes_per_nnz = 40_000.0 / 5000.0;

        let f = compile("gflops_per_second / bytes_per_nnz").expect("compile");
        let result = f.eval(&entry, &matrix, None).expect("eval");
        let expected = expected_gflops / expected_bytes_per_nnz;
        assert!(
            (result - expected).abs() < 1e-10,
            "expected {expected}, got {result}"
        );
    }
}

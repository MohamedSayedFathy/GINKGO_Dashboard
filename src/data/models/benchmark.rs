use serde::{Deserialize, Serialize};
use serde_json::Value;
use smol_str::SmolStr;
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BenchmarkOptimal {
    // Deserialized from JSON; optimal format name reserved for future highlighting.
    #[allow(dead_code)]
    pub spmv: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SolverComponents {
    // Deserialized from JSON; individual component timings reserved for future use.
    #[allow(dead_code)]
    pub components: HashMap<String, f64>,
    pub time: f64,
    pub iterations: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SolverResult {
    pub recurrent_residuals: Option<Vec<f64>>,
    pub true_residuals: Option<Vec<f64>>,
    pub implicit_residuals: Option<Vec<f64>>,
    pub iteration_timestamps: Option<Vec<f64>>,
    // Deserialized from JSON; used for normalization in future work.
    #[allow(dead_code)]
    pub rhs_norm: f64,
    pub generate: SolverComponents,
    pub apply: SolverComponents,
    // Deserialized from JSON; preconditioner metadata reserved for future display.
    #[allow(dead_code)]
    pub preconditioner: Value,
    // Deserialized from JSON; final residual norm reserved for future display.
    #[allow(dead_code)]
    pub residual_norm: f64,
    // Deserialized from JSON; repetition count reserved for future averaging.
    #[allow(dead_code)]
    pub repetitions: u64,
    pub completed: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SolverBenchmark {
    pub stencil: String,
    pub size: u64,
    pub rows: u64,
    pub cols: u64,
    // Deserialized from JSON; optimal format selection reserved for future highlighting.
    #[allow(dead_code)]
    pub optimal: BenchmarkOptimal,
    pub solver: HashMap<String, SolverResult>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BenchmarkEntry {
    pub storage: Option<u64>,
    pub max_relative_norm2: Option<f64>,
    pub time: Option<f64>,
    pub repetitions: Option<u64>,
    pub completed: bool,

    // Computed post-load; not present in real GINKGO JSON output.
    #[serde(skip)]
    pub gflops_per_second: f64,
    #[serde(skip)]
    pub bytes_per_nnz: f64,
    #[serde(skip)]
    pub operational_intensity: f64,
    #[serde(skip)]
    pub effective_memory_bandwidth: f64,
}

impl BenchmarkEntry {
    pub fn calculate_performance_metrics(&mut self, matrix: &MatrixMetadata, optimal_bytes: f64) {
        if let (Some(time), Some(storage)) = (self.time, self.storage) {
            if time > 0.0 {
                self.gflops_per_second = (2.0 * matrix.nonzeros as f64) / time / 1e9;

                if matrix.nonzeros > 0 {
                    self.bytes_per_nnz = storage as f64 / matrix.nonzeros as f64;
                }

                // Use optimal CSR bytes if available, otherwise fall back to reported storage
                let matrix_bytes = if optimal_bytes > 0.0 {
                    optimal_bytes
                } else {
                    storage as f64
                };

                let vector_bytes = (matrix.rows as f64 * 8.0) + (matrix.cols as f64 * 8.0);

                let total_mandatory_bytes = matrix_bytes + vector_bytes;

                // Operational Intensity = FLOPs / Bytes
                let flops = 2.0 * matrix.nonzeros as f64;
                self.operational_intensity = flops / total_mandatory_bytes;

                self.effective_memory_bandwidth = total_mandatory_bytes / time / 1e9;
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MatrixColumnsMetadata {
    // Deserialized from JSON; distribution stats reserved for future analysis.
    #[allow(dead_code)]
    pub min: f64,
    #[allow(dead_code)]
    pub q1: f64,
    pub median: f64,
    #[allow(dead_code)]
    pub q3: f64,
    pub max: f64,
    pub mean: f64,
    pub variance: f64,
    #[allow(dead_code)]
    pub skewness: f64,
    #[allow(dead_code)]
    pub kurtosis: f64,
    #[allow(dead_code)]
    pub hyperskewness: f64,
    #[allow(dead_code)]
    pub hyperflatness: f64,

    // Computed post-load; not present in real GINKGO JSON output.
    #[allow(dead_code)]
    #[serde(skip)]
    pub col_irregularity: f64,
    #[serde(skip)]
    pub col_cv: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MatrixRowsMetadata {
    // Deserialized from JSON; distribution stats reserved for future analysis.
    #[allow(dead_code)]
    pub min: f64,
    #[allow(dead_code)]
    pub q1: f64,
    pub median: f64,
    #[allow(dead_code)]
    pub q3: f64,
    pub max: f64,
    pub mean: f64,
    pub variance: f64,
    #[allow(dead_code)]
    pub skewness: f64,
    #[allow(dead_code)]
    pub kurtosis: f64,
    #[allow(dead_code)]
    pub hyperskewness: f64,
    #[allow(dead_code)]
    pub hyperflatness: f64,

    // Computed post-load; not present in real GINKGO JSON output.
    #[allow(dead_code)]
    #[serde(skip)]
    pub row_irregularity: f64,
    #[serde(skip)]
    pub row_cv: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MatrixMetadata {
    // Deserialized from JSON; matrix registry metadata reserved for future UI display.
    #[allow(dead_code)]
    pub id: u64,
    #[allow(dead_code)]
    pub group: SmolStr,
    pub name: SmolStr,
    pub rows: u64,
    pub cols: u64,
    pub nonzeros: u64,
    #[allow(dead_code)]
    pub real: bool,
    #[allow(dead_code)]
    pub binary: bool,
    #[serde(rename = "2d3d")]
    #[allow(dead_code)]
    pub is_2d3d: bool,
    #[allow(dead_code)]
    pub posdef: bool,
    #[allow(dead_code)]
    pub psym: f64,
    #[allow(dead_code)]
    pub nsym: f64,
    #[allow(dead_code)]
    pub kind: SmolStr,

    pub row_distribution: MatrixRowsMetadata,
    pub col_distribution: MatrixColumnsMetadata,

    // Computed post-load; not present in real GINKGO JSON output.
    #[serde(skip)]
    pub sparsity: f64,
    #[serde(skip)]
    pub avg_nnz_per_row: f64,
    #[serde(skip)]
    pub avg_nnz_per_col: f64,
    #[serde(skip)]
    pub matrix_shape_ratio: f64,
}

impl MatrixMetadata {
    pub fn calculate_derived_metrics(&mut self) {
        if self.rows > 0 && self.cols > 0 {
            self.sparsity = self.nonzeros as f64 / (self.rows as f64 * self.cols as f64);
            self.avg_nnz_per_row = self.nonzeros as f64 / self.rows as f64;
            self.avg_nnz_per_col = self.nonzeros as f64 / self.cols as f64;
            self.matrix_shape_ratio = self.rows as f64 / self.cols as f64;
        }

        if self.col_distribution.median != 0.0 {
            self.col_distribution.col_irregularity =
                self.col_distribution.max / self.col_distribution.median;
        }
        if self.col_distribution.mean != 0.0 {
            self.col_distribution.col_cv =
                self.col_distribution.variance.sqrt() / self.col_distribution.mean;
        }

        if self.row_distribution.median != 0.0 {
            self.row_distribution.row_irregularity =
                self.row_distribution.max / self.row_distribution.median;
        }
        if self.row_distribution.mean != 0.0 {
            self.row_distribution.row_cv =
                self.row_distribution.variance.sqrt() / self.row_distribution.mean;
        }
    }

    /// Calculate theoretical minimum CSR storage.
    /// Automatically uses 64-bit indices if matrix exceeds 32-bit limits.
    pub fn calculate_optimal_csr_bytes(rows: u64, nonzeros: u64) -> f64 {
        const BYTES_PER_INDEX_32: u64 = 4;
        const BYTES_PER_INDEX_64: u64 = 8;
        const BYTES_PER_VALUE_F64: u64 = 8;
        const MAX_32BIT_INDEX: u64 = u32::MAX as u64;

        let index_bytes = if rows > MAX_32BIT_INDEX || nonzeros > MAX_32BIT_INDEX {
            log::warn!(
                "Matrix exceeds 32-bit index limits (rows={}, nnz={}). Using 64-bit index calculation.",
                rows,
                nonzeros
            );
            BYTES_PER_INDEX_64
        } else {
            BYTES_PER_INDEX_32
        };

        // Ptr array: (rows + 1) * index_bytes
        // Col indices: nonzeros * index_bytes
        // Values: nonzeros * 8 bytes (f64)
        let ptr_bytes = (rows + 1) * index_bytes;
        let col_bytes = nonzeros * index_bytes;
        let val_bytes = nonzeros * BYTES_PER_VALUE_F64;

        (ptr_bytes + col_bytes + val_bytes) as f64
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BenchmarkProblem {
    // Deserialized from JSON; filename used for display in future dataset management.
    #[allow(dead_code)]
    pub filename: String,
    pub problem: MatrixMetadata,
    pub spmv: HashMap<String, BenchmarkEntry>,
    // Deserialized from JSON; optimal format reserved for future highlighting.
    #[allow(dead_code)]
    pub optimal: BenchmarkOptimal,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BenchmarkDataset {
    pub benchmark: Vec<BenchmarkProblem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_dummy_metadata(rows: u64, nonzeros: u64) -> MatrixMetadata {
        MatrixMetadata {
            id: 1,
            group: "test".into(),
            name: "test_matrix".into(),
            rows,
            cols: rows,
            nonzeros,
            real: true,
            binary: false,
            is_2d3d: false,
            posdef: false,
            psym: 0.0,
            nsym: 0.0,
            kind: "test".into(),
            // Mock unused fields as they require initialization
            row_distribution: MatrixRowsMetadata {
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
            },
            col_distribution: MatrixColumnsMetadata {
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
            },
            sparsity: 0.0,
            avg_nnz_per_row: 0.0,
            avg_nnz_per_col: 0.0,
            matrix_shape_ratio: 0.0,
        }
    }

    #[test]
    fn test_optimal_csr_bytes() {
        let bytes = MatrixMetadata::calculate_optimal_csr_bytes(4, 10);
        assert_eq!(bytes, 140.0);
    }

    #[test]
    fn test_optimal_csr_bytes_large() {
        let bytes = MatrixMetadata::calculate_optimal_csr_bytes(100, 1000);
        assert_eq!(bytes, 12404.0);
    }

    #[test]
    fn test_process_entry_gflops() {
        let meta = create_dummy_metadata(1000, 1_000_000);
        let mut entry = BenchmarkEntry {
            time: Some(0.001),
            storage: Some(100),
            max_relative_norm2: Some(1e-6),
            repetitions: Some(1),
            completed: true,
            operational_intensity: 0.0,
            effective_memory_bandwidth: 0.0,
            gflops_per_second: 0.0,
            bytes_per_nnz: 0.0,
        };

        // Optimal bytes irrelevant for GFlops, pass 0.0 or dummy
        entry.calculate_performance_metrics(&meta, 0.0);

        assert!(
            (entry.gflops_per_second - 2.0).abs() < 1e-4,
            "GFlops should be 2.0, got {}",
            entry.gflops_per_second
        );
    }

    #[test]
    fn test_effective_bandwidth() {
        let meta = create_dummy_metadata(4, 10);
        let mut entry = BenchmarkEntry {
            time: Some(2.0),
            storage: Some(0), // Dummy storage to pass "Some" check
            max_relative_norm2: None,
            repetitions: None,
            completed: true,
            gflops_per_second: 0.0,
            bytes_per_nnz: 0.0,
            operational_intensity: 0.0,
            effective_memory_bandwidth: 0.0,
        };

        // Optimal bytes specific for this test (Rows=4, NNZ=10 -> 140 bytes)
        entry.calculate_performance_metrics(&meta, 140.0);

        let expected_bw = 102.0 / 1_000_000_000.0;
        assert!(
            (entry.effective_memory_bandwidth - expected_bw).abs() < 1e-12,
            "Got {}, Expected {}",
            entry.effective_memory_bandwidth,
            expected_bw
        );
    }

    #[test]
    fn test_bandwidth_physics_vectors_included() {
        let meta = create_dummy_metadata(10, 20);
        let mut entry = BenchmarkEntry {
            time: Some(1.0),
            storage: Some(284),
            max_relative_norm2: None,
            repetitions: None,
            completed: true,
            gflops_per_second: 0.0,
            bytes_per_nnz: 0.0,
            operational_intensity: 0.0,
            effective_memory_bandwidth: 0.0,
        };

        // Pass matrix bytes only
        entry.calculate_performance_metrics(&meta, 284.0);

        let expected_bw = 444.0 / 1e9;
        assert!(
            (entry.effective_memory_bandwidth - expected_bw).abs() < 1e-12,
            "Expected BW {}, got {}",
            expected_bw,
            entry.effective_memory_bandwidth
        );
    }
}

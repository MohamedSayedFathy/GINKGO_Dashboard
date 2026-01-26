use super::models::{BenchmarkDataset, BenchmarkProblem, MatrixMetadata};

use super::AppError;

const TOLERANCE_THRESHOLD: f64 = 1e-4;

pub fn process_benchmark_data(
    raw_data: Vec<BenchmarkProblem>,
) -> Result<BenchmarkDataset, AppError> {
    let filtered_raw_data = filter_spmv(raw_data)?;

    let processed_benchmark_data: Result<Vec<BenchmarkProblem>, AppError> = filtered_raw_data
        .into_iter()
        .map(|mut problem| {
            let matrix = &mut problem.problem;

            if matrix.rows == 0 || matrix.cols == 0 {
                return Err(AppError::Logic(format!(
                    "Matrix {} has 0 rows or cols",
                    matrix.name
                )));
            }

            matrix.calculate_derived_metrics();

            // Try to use the reported CSR storage. If missing, estimate: (rows+1)*4 + nnz*4 + nnz*8
            let optimal_bytes = if let Some(csr_entry) = problem.spmv.get("csr") {
                csr_entry.storage.unwrap_or(0) as f64
            } else {
                MatrixMetadata::calculate_optimal_csr_bytes(matrix.rows, matrix.nonzeros)
            };

            for value in problem.spmv.values_mut() {
                // Standard HPC tolerance is TOLERANCE_THRESHOLD for mixed precision/iterative solvers
                // If the error is too high, we treat this run as "Failed" (DNF)
                if let Some(norm2) = value.max_relative_norm2 {
                    if norm2 > TOLERANCE_THRESHOLD {
                        // Invalid result! Scrub metrics.
                        // We keep the entry but remove performance metrics so it doesn't show up in plots as "Success"
                        value.time = None;
                        value.gflops_per_second = 0.0;
                        value.effective_memory_bandwidth = 0.0;
                        value.completed = false;
                    }
                }

                value.calculate_performance_metrics(matrix, optimal_bytes);
            }
            Ok(problem)
        })
        .collect();

    let processed_data = processed_benchmark_data?;
    Ok(post_processing(processed_data))
}

pub fn filter_spmv(dataset: Vec<BenchmarkProblem>) -> Result<Vec<BenchmarkProblem>, AppError> {
    Ok(dataset)
}

pub fn post_processing(processed_benchmark: Vec<BenchmarkProblem>) -> BenchmarkDataset {
    BenchmarkDataset {
        benchmark: processed_benchmark,
    }
}

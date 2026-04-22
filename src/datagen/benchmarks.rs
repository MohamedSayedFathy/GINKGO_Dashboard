use crate::data::models::{BenchmarkEntry, BenchmarkOptimal, BenchmarkProblem, MatrixMetadata};
use crate::types::DataFormat;
use rand::Rng;
use rand_chacha::ChaCha20Rng;
use rand_distr::{Distribution, Normal};
use std::collections::HashMap;

/// Peak GPU memory bandwidth (bytes/second) used in the roofline model.
const PEAK_BANDWIDTH_BPS: f64 = 900e9;

/// Storage (bytes) for CSR format.
/// Uses 32-bit indices unless rows or nnz exceeds u32::MAX, in which case uses 64-bit.
fn csr_storage(rows: u64, nnz: u64) -> u64 {
    let idx_bytes: u64 = if rows > u32::MAX as u64 || nnz > u32::MAX as u64 {
        8
    } else {
        4
    };
    // ptr array: (rows+1) * idx_bytes
    // col indices: nnz * idx_bytes
    // values: nnz * 8 (f64)
    (rows + 1) * idx_bytes + nnz * idx_bytes + nnz * 8
}

/// Storage (bytes) for COO format.
fn coo_storage(nnz: u64) -> u64 {
    // row indices: nnz * 4, col indices: nnz * 4, values: nnz * 8
    nnz * 4 + nnz * 4 + nnz * 8
}

/// Storage (bytes) for ELL format.
/// row_max_nnz: maximum number of non-zeros per row (from row_distribution.max).
fn ell_storage(rows: u64, row_max_nnz: f64) -> u64 {
    let max_nnz = row_max_nnz.ceil() as u64;
    // col indices: rows * max_nnz * 4, values: rows * max_nnz * 8
    rows * max_nnz * (4 + 8)
}

/// Storage (bytes) for HYBRID format.
/// Uses ELL for rows up to the median nnz, COO for the overflow.
fn hybrid_storage(rows: u64, nnz: u64, row_median_nnz: f64) -> u64 {
    let ell_width = row_median_nnz.ceil() as u64;
    let ell_nnz = rows * ell_width;
    // COO stores the overflow (nnz that don't fit in the ELL part)
    let coo_nnz = nnz.saturating_sub(ell_nnz);
    // ELL part storage
    let ell_bytes = ell_nnz * (4 + 8);
    // COO part storage
    let coo_bytes = coo_nnz * (4 + 4 + 8);
    ell_bytes + coo_bytes
}

/// Storage (bytes) for SELL-P format.
/// SELL-P uses a slice size of 32 and pads within each slice.
/// Approximated as ELL * 0.85, which is a heuristic.
/// Note: true SELL-P storage depends on per-slice max nnz, which requires the actual matrix.
/// This approximation gives plausible values for the synthetic data; real values will differ.
fn sellp_storage_approx(rows: u64, row_max_nnz: f64) -> u64 {
    let ell = ell_storage(rows, row_max_nnz);
    (ell as f64 * 0.85).round() as u64
}

/// Compute the time (seconds) for a given format, applying the roofline model and format factors.
///
/// The model: `t = bytes / PEAK_BANDWIDTH_BPS * format_efficiency`.
/// Format efficiency is derived from the row coefficient of variation.
fn format_time(
    format: DataFormat,
    storage_bytes: u64,
    row_cv: f64,
    rows: u64,
    nnz: u64,
    row_median_nnz: f64,
) -> f64 {
    let base_bytes = storage_bytes as f64;
    // Add vector bytes (input x and output y): both are f64, rows + cols entries
    // For SpMV: y = A*x, x has `cols` entries, y has `rows` entries; assume square for simplicity
    let vector_bytes = (rows as f64 + rows as f64) * 8.0;
    let total_bytes = base_bytes + vector_bytes;
    let base_time = total_bytes / PEAK_BANDWIDTH_BPS;

    // Format efficiency multiplier relative to CSR (1.0 = same as CSR)
    let factor = match format {
        DataFormat::CSR => 1.0,
        DataFormat::COO => {
            // Atomics / row reduction overhead: 1.2 to 1.5x CSR
            1.2 + row_cv * 0.2
        }
        DataFormat::ELL => {
            // Efficient for low CV (0.7x CSR), but padding blowup for high CV
            if row_cv < 0.1 {
                0.7
            } else if row_cv < 1.0 {
                // Linear interpolation from 0.7 to 5.0 over row_cv in [0.1, 1.0]
                0.7 + (row_cv - 0.1) / 0.9 * (5.0 - 0.7)
            } else {
                5.0_f64.min(row_cv * 5.0)
            }
        }
        DataFormat::HYBRID => {
            // ELL for regular rows + COO for outliers: better than both on high-CV
            // For high CV, HYBRID saves over ELL because only median-width ELL is used
            let ell_factor = if row_cv < 0.1 {
                0.7
            } else if row_cv < 1.0 {
                0.7 + (row_cv - 0.1) / 0.9 * (5.0 - 0.7)
            } else {
                5.0_f64.min(row_cv * 5.0)
            };
            // HYBRID is capped at the ELL factor but benefits from the smaller ELL footprint
            let hybrid_reduction = if row_cv > 0.5 {
                // Significant reduction for high CV due to smaller ELL width (median vs max)
                let ell_width_ratio = if row_median_nnz > 0.0 {
                    let row_max_nnz = row_median_nnz * (1.0 + row_cv * 3.0);
                    row_median_nnz / row_max_nnz
                } else {
                    0.5
                };
                let ell_nnz = rows as f64 * row_median_nnz;
                let overflow_frac = if nnz > 0 {
                    (nnz as f64 - ell_nnz).max(0.0) / nnz as f64
                } else {
                    0.0
                };
                // HYBRID time ≈ ELL_part_time + COO_part_time
                // ELL part is faster (smaller footprint), COO part is slightly slower than CSR
                ell_width_ratio + overflow_frac * 1.3
            } else {
                0.9 // Slight overhead over CSR for low CV (scheduling, two-pass approach)
            };
            ell_factor * hybrid_reduction
        }
        DataFormat::SELLP => {
            // SELL-P: slice-based padding, gentler penalty than ELL for high CV
            if row_cv < 0.1 {
                0.75 // Slightly worse than ELL for very regular matrices (slice overhead)
            } else if row_cv < 1.0 {
                // Penalty is gentler than ELL: factor in [1.0, 2.0] range
                1.0 + (row_cv - 0.1) / 0.9 * (2.0 - 1.0)
            } else {
                2.0_f64.min(1.0 + row_cv * 0.5)
            }
        }
    };

    base_time * factor
}

/// Apply multiplicative log-normal noise to a time value.
///
/// `noise_stddev` is the sigma of the log-normal: `t *= exp(N(0, sigma))`.
fn apply_noise(time: f64, noise_stddev: f64, rng: &mut ChaCha20Rng) -> f64 {
    if noise_stddev <= 0.0 {
        return time;
    }
    let normal = Normal::new(0.0_f64, noise_stddev).expect("valid normal params");
    let z: f64 = normal.sample(rng);
    time * z.exp()
}

/// Generate all 5 format [`BenchmarkEntry`] entries for a matrix, plus the optimal.
///
/// Returns `(spmv_map, optimal_format_key)`.
pub fn generate_spmv_entries(
    matrix: &MatrixMetadata,
    noise_stddev: f64,
    rng: &mut ChaCha20Rng,
) -> (HashMap<String, BenchmarkEntry>, BenchmarkOptimal) {
    let rows = matrix.rows;
    let nnz = matrix.nonzeros;

    // The row_cv needs to be computed; since calculate_derived_metrics() hasn't been called
    // (those fields are post-load), we compute it inline from the distribution.
    let row_cv = if matrix.row_distribution.mean > 0.0 {
        matrix.row_distribution.variance.sqrt() / matrix.row_distribution.mean
    } else {
        0.0
    };

    let row_max_nnz = matrix.row_distribution.max;
    let row_median_nnz = matrix.row_distribution.median;

    // Compute storage for each format
    let csr_bytes = csr_storage(rows, nnz);
    let coo_bytes = coo_storage(nnz);
    let ell_bytes = ell_storage(rows, row_max_nnz);
    let hybrid_bytes = hybrid_storage(rows, nnz, row_median_nnz);
    let sellp_bytes = sellp_storage_approx(rows, row_max_nnz);

    // Compute base times using roofline model
    let formats = [
        (DataFormat::CSR, csr_bytes),
        (DataFormat::COO, coo_bytes),
        (DataFormat::ELL, ell_bytes),
        (DataFormat::HYBRID, hybrid_bytes),
        (DataFormat::SELLP, sellp_bytes),
    ];

    let mut spmv: HashMap<String, BenchmarkEntry> = HashMap::new();
    let mut best_time = f64::MAX;
    let mut best_key = DataFormat::CSR.as_key();

    for (fmt, storage) in &formats {
        let base_time = format_time(*fmt, *storage, row_cv, rows, nnz, row_median_nnz);
        let noisy_time = apply_noise(base_time, noise_stddev, rng);

        // ELL / HYBRID / SELLP have lower numerical error (padding with zeros → exact);
        // CSR / COO have near-zero norm2 from exact scatter/gather.
        let max_relative_norm2 = match fmt {
            DataFormat::CSR | DataFormat::COO => 0.0,
            DataFormat::ELL | DataFormat::HYBRID | DataFormat::SELLP => {
                // Plausible epsilon-level rounding: ~5e-17
                5e-17_f64 * rng.random_range(0.5_f64..2.0_f64)
            }
        };

        let entry = BenchmarkEntry {
            storage: Some(*storage),
            max_relative_norm2: Some(max_relative_norm2),
            time: Some(noisy_time),
            repetitions: Some(10),
            completed: true,
            // Computed post-load; left at default so they aren't serialized.
            gflops_per_second: 0.0,
            bytes_per_nnz: 0.0,
            operational_intensity: 0.0,
            effective_memory_bandwidth: 0.0,
        };

        if noisy_time < best_time {
            best_time = noisy_time;
            best_key = fmt.as_key();
        }

        spmv.insert(fmt.as_key().to_string(), entry);
    }

    let optimal = BenchmarkOptimal {
        spmv: best_key.to_string(),
    };

    (spmv, optimal)
}

/// Build a complete [`BenchmarkProblem`] for a given matrix.
pub fn generate_benchmark_problem(
    matrix: MatrixMetadata,
    noise_stddev: f64,
    rng: &mut ChaCha20Rng,
) -> BenchmarkProblem {
    let filename = format!("/ssget/MM/{}/{}.mtx", matrix.group, matrix.name);
    let (spmv, optimal) = generate_spmv_entries(&matrix, noise_stddev, rng);

    BenchmarkProblem {
        filename,
        problem: matrix,
        spmv,
        optimal,
    }
}

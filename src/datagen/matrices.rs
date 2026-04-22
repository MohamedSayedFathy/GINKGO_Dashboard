use crate::data::models::{MatrixColumnsMetadata, MatrixMetadata, MatrixRowsMetadata};
use rand::Rng;
use rand_chacha::ChaCha20Rng;
use rand_distr::{Distribution, LogNormal};
use smol_str::SmolStr;

/// Matrix class descriptor: label (matches real SuiteSparse kind strings), row CV, posdef prob.
struct MatrixClass {
    kind: &'static str,
    group: &'static str,
    /// Coefficient of variation for row nnz distribution
    row_cv: f64,
    /// Probability that the matrix is positive definite
    posdef_prob: f64,
    /// Probability that the matrix is symmetric (psym)
    sym_prob: f64,
    /// Whether this class maps to is_2d3d = true
    is_2d3d: bool,
    /// Typical skewness for distribution stats
    skewness: f64,
    /// Typical kurtosis
    kurtosis: f64,
}

const MATRIX_CLASSES: &[MatrixClass] = &[
    MatrixClass {
        kind: "structural problem",
        group: "UF",
        row_cv: 0.3,
        posdef_prob: 0.5,
        sym_prob: 0.8,
        is_2d3d: false,
        skewness: 0.2,
        kurtosis: 3.0,
    },
    MatrixClass {
        kind: "computational fluid dynamics problem",
        group: "UF",
        row_cv: 0.1,
        posdef_prob: 0.3,
        sym_prob: 0.4,
        is_2d3d: false,
        skewness: 0.5,
        kurtosis: 4.0,
    },
    MatrixClass {
        kind: "directed graph",
        group: "UF",
        row_cv: 1.2,
        posdef_prob: 0.0,
        sym_prob: 0.05,
        is_2d3d: false,
        skewness: 2.5,
        kurtosis: 12.0,
    },
    MatrixClass {
        kind: "undirected graph",
        group: "UF",
        row_cv: 0.8,
        posdef_prob: 0.0,
        sym_prob: 1.0,
        is_2d3d: false,
        skewness: 1.5,
        kurtosis: 7.0,
    },
    MatrixClass {
        kind: "2D/3D problem",
        group: "UF",
        row_cv: 0.05,
        posdef_prob: 0.7,
        sym_prob: 0.9,
        is_2d3d: true,
        skewness: 0.1,
        kurtosis: 3.0,
    },
    MatrixClass {
        kind: "circuit simulation problem",
        group: "UF",
        row_cv: 1.5,
        posdef_prob: 0.1,
        sym_prob: 0.2,
        is_2d3d: false,
        skewness: 3.0,
        kurtosis: 18.0,
    },
    MatrixClass {
        kind: "optimization problem",
        group: "UF",
        row_cv: 0.5,
        posdef_prob: 0.2,
        sym_prob: 0.6,
        is_2d3d: false,
        skewness: 0.8,
        kurtosis: 5.0,
    },
];

/// Cumulative selection weights for the matrix classes above.
/// Must sum to 1.0 and have the same length as `MATRIX_CLASSES`.
const CLASS_WEIGHTS: &[f64] = &[0.20, 0.40, 0.55, 0.70, 0.85, 0.95, 1.00];

/// Sample a matrix class index according to `CLASS_WEIGHTS`.
fn sample_class(rng: &mut ChaCha20Rng) -> &'static MatrixClass {
    let r: f64 = rng.random();
    let idx = CLASS_WEIGHTS
        .iter()
        .position(|&w| r < w)
        .unwrap_or(MATRIX_CLASSES.len() - 1);
    &MATRIX_CLASSES[idx]
}

/// Sample nnz log-uniformly from [lo, hi].
fn log_uniform(rng: &mut ChaCha20Rng, lo: f64, hi: f64) -> f64 {
    let log_lo = lo.ln();
    let log_hi = hi.ln();
    let t: f64 = rng.random();
    (log_lo + t * (log_hi - log_lo)).exp()
}

/// Compute approximate quantiles of a log-normal distribution with given mean and variance.
/// Returns `(min, q1, median, q3, max)` as multiples of the mean — rough parametric estimates.
fn lognormal_quantiles(mean: f64, cv: f64) -> (f64, f64, f64, f64, f64) {
    // Log-normal parameterisation: mu_ln = ln(mean) - sigma_ln^2/2, sigma_ln = sqrt(ln(1+cv^2))
    let sigma_sq = (1.0 + cv * cv).ln();
    let sigma = sigma_sq.sqrt();
    let mu = mean.ln() - sigma_sq / 2.0;

    // Quantiles: q = exp(mu + z * sigma) where z is the standard normal quantile
    // z values: min≈-3, q1≈-0.674, median=0, q3≈0.674, max≈3
    let q = |z: f64| (mu + z * sigma).exp();

    // Clamp min to 1.0 (can't have 0 nnz per row for a non-empty matrix)
    let raw_min = q(-3.0);
    let min_val = raw_min.max(1.0);
    let q1 = q(-0.674).max(min_val);
    let median = q(0.0).max(q1);
    let q3 = q(0.674).max(median);
    let max_val = q(3.0).max(q3);

    (min_val, q1, median, q3, max_val)
}

/// Build a [`MatrixRowsMetadata`] from parametric values.
fn make_row_dist(mean: f64, cv: f64, skewness: f64, kurtosis: f64) -> MatrixRowsMetadata {
    let variance = (cv * mean).powi(2);
    let (min, q1, median, q3, max) = lognormal_quantiles(mean, cv);

    // Hyperskewness and hyperflatness are higher-order moments; use scaled plausible values.
    let hyperskewness = skewness.powi(2) * 5.0;
    let hyperflatness = kurtosis.powi(2) * 2.0;

    MatrixRowsMetadata {
        min,
        q1,
        median,
        q3,
        max,
        mean,
        variance,
        skewness,
        kurtosis,
        hyperskewness,
        hyperflatness,
        // Computed post-load by calculate_derived_metrics(); left at default here.
        row_irregularity: 0.0,
        row_cv: 0.0,
    }
}

/// Build a [`MatrixColumnsMetadata`] from parametric values.
fn make_col_dist(mean: f64, cv: f64, skewness: f64, kurtosis: f64) -> MatrixColumnsMetadata {
    let variance = (cv * mean).powi(2);
    let (min, q1, median, q3, max) = lognormal_quantiles(mean, cv);

    let hyperskewness = skewness.powi(2) * 5.0;
    let hyperflatness = kurtosis.powi(2) * 2.0;

    MatrixColumnsMetadata {
        min,
        q1,
        median,
        q3,
        max,
        mean,
        variance,
        skewness,
        kurtosis,
        hyperskewness,
        hyperflatness,
        // Computed post-load.
        col_irregularity: 0.0,
        col_cv: 0.0,
    }
}

/// Generate a single synthetic [`MatrixMetadata`].
pub fn generate_matrix(rng: &mut ChaCha20Rng, id: u64) -> MatrixMetadata {
    let class = sample_class(rng);

    // nnz drawn log-uniformly in [1e3, 1e7]
    let nnz = log_uniform(rng, 1e3, 1e7).round() as u64;

    // avg_nnz_per_row drawn log-uniformly in [2, 500]
    let avg_nnz_per_row = log_uniform(rng, 2.0, 500.0);

    // rows = nnz / avg_nnz_per_row, clamped to at least 1
    let rows = ((nnz as f64 / avg_nnz_per_row).round() as u64).max(1);

    // 90% square, 10% non-square
    let cols = if rng.random::<f64>() < 0.90 {
        rows
    } else {
        // Non-square: cols drawn log-uniformly in [rows/4, rows*4]
        let lo = (rows as f64 / 4.0).max(1.0);
        let hi = rows as f64 * 4.0;
        log_uniform(rng, lo, hi).round() as u64
    };

    // Add small noise to row_cv so matrices of the same class differ slightly
    let cv_noise: f64 = rng.random_range(-0.05_f64..0.05_f64);
    let row_cv = (class.row_cv + cv_noise).max(0.01);

    // col_cv is similar to row_cv (symmetric-ish matrix class)
    let col_cv = row_cv * rng.random_range(0.8_f64..1.2_f64);

    let mean_nnz_per_row = nnz as f64 / rows as f64;
    let mean_nnz_per_col = nnz as f64 / cols as f64;

    // Add small noise to higher-order stats
    let sk_noise: f64 = rng.random_range(-0.1_f64..0.1_f64);
    let skewness = class.skewness + sk_noise;
    let kurtosis = class.kurtosis * rng.random_range(0.9_f64..1.1_f64);

    let row_dist = make_row_dist(mean_nnz_per_row, row_cv, skewness, kurtosis);
    let col_dist = make_col_dist(mean_nnz_per_col, col_cv, skewness, kurtosis);

    let posdef = rng.random::<f64>() < class.posdef_prob;
    let binary = rng.random::<f64>() < 0.3;

    // psym: for undirected graphs = 1.0, others use class sym_prob
    let psym = if class.sym_prob >= 1.0 {
        1.0
    } else {
        rng.random::<f64>() * class.sym_prob
    };
    let nsym = psym * rng.random_range(0.8_f64..1.0_f64);

    let name = SmolStr::from(format!("{}_{}", class.group.to_lowercase(), id));
    let kind = SmolStr::from(class.kind);
    let group = SmolStr::from(class.group);

    // Introduce small lognormal perturbation to max nnz/row for ELL storage calculation realism
    // We perturb via LogNormal(0, 0.2) to get realistic max values
    let ln_perturbation = {
        let ln_distr = LogNormal::new(0.0_f64, 0.2_f64).expect("valid lognormal params");
        ln_distr.sample(rng)
    };
    // row_distribution.max is already set parametrically; apply perturbation for variability
    // by clamping: max must be >= mean and >= q3
    let raw_max = row_dist.max * ln_perturbation;
    let row_max = raw_max.max(row_dist.q3).max(row_dist.mean);

    // Rebuild row_dist with adjusted max
    let row_dist = MatrixRowsMetadata {
        max: row_max,
        ..row_dist
    };

    MatrixMetadata {
        id,
        group,
        name,
        rows,
        cols,
        nonzeros: nnz,
        real: true,
        binary,
        is_2d3d: class.is_2d3d,
        posdef,
        psym,
        nsym,
        kind,
        row_distribution: row_dist,
        col_distribution: col_dist,
        // Computed post-load by calculate_derived_metrics(); left at default.
        sparsity: 0.0,
        avg_nnz_per_row: 0.0,
        avg_nnz_per_col: 0.0,
        matrix_shape_ratio: 0.0,
    }
}

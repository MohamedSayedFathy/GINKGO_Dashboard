use crate::data::models::{MatrixColumnsMetadata, MatrixMetadata, MatrixRowsMetadata};
use rand::Rng;
use rand_chacha::ChaCha20Rng;
use rand_distr::{Distribution, LogNormal};
use smol_str::SmolStr;

/// Matrix class descriptor — kept *only* for the cosmetic `kind` text that
/// appears in dashboard tooltips. The numeric fields below are no longer
/// consumed by the parameter sampler (the redesign draws every numeric
/// quantity independently of the class), but the struct itself is preserved
/// so that adding new SuiteSparse-style labels stays a one-line change.
struct MatrixClass {
    kind: &'static str,
    group: &'static str,
}

const MATRIX_CLASSES: &[MatrixClass] = &[
    MatrixClass {
        kind: "structural problem",
        group: "UF",
    },
    MatrixClass {
        kind: "computational fluid dynamics problem",
        group: "UF",
    },
    MatrixClass {
        kind: "directed graph",
        group: "UF",
    },
    MatrixClass {
        kind: "undirected graph",
        group: "UF",
    },
    MatrixClass {
        kind: "2D/3D problem",
        group: "UF",
    },
    MatrixClass {
        kind: "circuit simulation problem",
        group: "UF",
    },
    MatrixClass {
        kind: "optimization problem",
        group: "UF",
    },
];

/// Cumulative selection weights for the matrix classes above.
/// Must sum to 1.0 and have the same length as `MATRIX_CLASSES`.
const CLASS_WEIGHTS: &[f64] = &[0.20, 0.40, 0.55, 0.70, 0.85, 0.95, 1.00];

/// Sample a matrix class index according to `CLASS_WEIGHTS`.
///
/// The returned class only feeds the cosmetic `kind` field (and `is_2d3d`
/// derived from a string match). All numeric matrix parameters are drawn
/// independently of this selection in `generate_matrix`.
fn sample_class(rng: &mut ChaCha20Rng) -> &'static MatrixClass {
    let r: f64 = rng.random();
    let idx = CLASS_WEIGHTS
        .iter()
        .position(|&w| r < w)
        .unwrap_or(MATRIX_CLASSES.len() - 1);
    &MATRIX_CLASSES[idx]
}

/// Sample from a mixture of log-normal components.
///
/// `components` is a slice of `(weight, mu, sigma)` tuples; weights are
/// normalized by their sum. The draw consumes RNG in a fixed sequence —
/// first one uniform sample to pick the component, then exactly one
/// log-normal sample — so the function is byte-deterministic for a given
/// `rng` state regardless of which component the picker selects.
fn sample_mixture_lognormal(rng: &mut ChaCha20Rng, components: &[(f64, f64, f64)]) -> f64 {
    debug_assert!(!components.is_empty());

    // Step 1: draw the picker uniform first, unconditionally.
    let pick: f64 = rng.random();

    // Step 2: pick the component by cumulative weight.
    let total: f64 = components.iter().map(|(w, _, _)| *w).sum();
    let target = pick * total;
    let mut acc = 0.0_f64;
    let mut chosen = components.len() - 1;
    for (i, (w, _, _)) in components.iter().enumerate() {
        acc += *w;
        if target < acc {
            chosen = i;
            break;
        }
    }
    let (_, mu, sigma) = components[chosen];

    // Step 3: draw the log-normal sample. Always exactly one RNG draw here,
    // independent of which component was picked.
    let dist = LogNormal::new(mu, sigma).expect("valid lognormal params");
    dist.sample(rng)
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

/// CV mixture: 60% low-CV regime (median ~0.37) + 40% high-CV regime
/// (median ~1.65). Used for both `row_cv` and `col_cv` (independent draws).
const CV_MIXTURE: &[(f64, f64, f64)] = &[(0.6, -1.0, 0.7), (0.4, 0.5, 0.8)];

/// Generate a single synthetic [`MatrixMetadata`].
///
/// The redesigned sampler draws every numeric quantity independently —
/// `sample_class` is still consulted to pick a cosmetic `kind` label so
/// dashboard tooltips don't all read identically, but the matrix class no
/// longer drives row_cv, skewness, kurtosis, sym_prob, or posdef_prob. The
/// per-call RNG sequence is fixed (class pick → nnz → avg_nnz → square pick
/// → maybe shape ratio → row_cv → col_cv → skewness → kurtosis → posdef →
/// binary → psym branch → maybe psym uniform → nsym → ell_perturbation) so
/// reruns with the same seed are byte-identical.
pub fn generate_matrix(rng: &mut ChaCha20Rng, id: u64) -> MatrixMetadata {
    // Cosmetic class label only — does not feed any numeric parameter.
    let class = sample_class(rng);

    // nnz drawn log-uniformly in [1e3, 1e7] (unchanged).
    let nnz = log_uniform(rng, 1e3, 1e7).round() as u64;

    // avg_nnz_per_row drawn log-uniformly in [1.5, 1000] — wider lower
    // bound than the old [2, 500] window so the X-axis spans more orders
    // of magnitude.
    let avg_nnz_per_row = log_uniform(rng, 1.5_f64, 1000.0_f64);

    // rows = nnz / avg_nnz_per_row, clamped to at least 1.
    let rows = ((nnz as f64 / avg_nnz_per_row).round() as u64).max(1);

    // Shape: 35% square; otherwise draw a continuous LogNormal(0, 0.7)
    // ratio in [0.05, 20.0]. Always draw the uniform picker first.
    let square_pick: f64 = rng.random();
    let cols = if square_pick < 0.35 {
        rows
    } else {
        let shape_ln = LogNormal::new(0.0_f64, 0.7_f64).expect("valid lognormal params");
        let ratio: f64 = shape_ln.sample(rng).clamp(0.05_f64, 20.0_f64);
        ((rows as f64 * ratio).round() as u64).max(1)
    };

    // CVs drawn independently from the mixture. No longer tied to a class
    // baseline — the mixture itself produces a clean low/high regime split.
    let row_cv = sample_mixture_lognormal(rng, CV_MIXTURE).clamp(0.01_f64, 5.0_f64);
    let col_cv = sample_mixture_lognormal(rng, CV_MIXTURE).clamp(0.01_f64, 5.0_f64);

    let mean_nnz_per_row = nnz as f64 / rows as f64;
    let mean_nnz_per_col = nnz as f64 / cols as f64;

    // Higher-order moments: independent log-normals, no class anchor.
    let sk_ln = LogNormal::new(-0.5_f64, 1.0_f64).expect("valid lognormal params");
    let skewness = sk_ln.sample(rng).clamp(0.05_f64, 15.0_f64);
    let ku_ln = LogNormal::new(1.0_f64, 0.7_f64).expect("valid lognormal params");
    let kurtosis = ku_ln.sample(rng).clamp(1.5_f64, 50.0_f64);

    let row_dist = make_row_dist(mean_nnz_per_row, row_cv, skewness, kurtosis);
    let col_dist = make_col_dist(mean_nnz_per_col, col_cv, skewness, kurtosis);

    // Independent Bernoulli flags. is_2d3d is derived from the cosmetic
    // class label — preserved so loaders that branch on `kind == "2D/3D
    // problem"` keep working — but doesn't affect numeric draws.
    let posdef = rng.random::<f64>() < 0.4_f64;
    let binary = rng.random::<f64>() < 0.3_f64;

    // psym: 30% chance of fully symmetric (= 1.0), else Uniform(0, 1).
    // Always draw the picker first; the second uniform is unconditionally
    // drawn from a sub-range that depends on the picker — to keep the RNG
    // sequence fixed regardless of the branch we draw a uniform *both*
    // branches.
    let psym_pick: f64 = rng.random();
    let psym_uniform: f64 = rng.random();
    let psym = if psym_pick < 0.3_f64 {
        1.0_f64
    } else {
        psym_uniform
    };
    let nsym = psym * rng.random_range(0.6_f64..1.0_f64);

    let is_2d3d = class.kind == "2D/3D problem";
    let name = SmolStr::from(format!("synthetic_{}", id));
    let kind = SmolStr::from(class.kind);
    let group = SmolStr::from("synthetic");
    // Keep the class.group reference live so the field isn't dead code.
    let _ = class.group;

    // Introduce small lognormal perturbation to max nnz/row for ELL storage calculation realism
    // We perturb via LogNormal(0, 0.2) to get realistic max values.
    let ln_perturbation = {
        let ln_distr = LogNormal::new(0.0_f64, 0.2_f64).expect("valid lognormal params");
        ln_distr.sample(rng)
    };
    // row_distribution.max is already set parametrically; apply perturbation for variability
    // by clamping: max must be >= mean and >= q3.
    let raw_max = row_dist.max * ln_perturbation;
    let row_max = raw_max.max(row_dist.q3).max(row_dist.mean);

    // Rebuild row_dist with adjusted max.
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
        is_2d3d,
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

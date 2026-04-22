use crate::data::models::{BenchmarkOptimal, SolverBenchmark, SolverComponents, SolverResult};
use rand::Rng;
use rand_chacha::ChaCha20Rng;
use rand_distr::{Distribution, Normal};
use serde_json::json;
use std::collections::HashMap;

/// Stencil configuration: name, grid points, convergence rate, solver-specific rho.
struct StencilSpec {
    stencil: &'static str,
    /// Total number of unknowns (grid size)
    size: u64,
    /// Convergence factor for CG
    rho_cg: f64,
    /// Convergence factor for GMRES
    rho_gmres: f64,
    /// Convergence factor for BiCGSTAB
    rho_bicgstab: f64,
    /// Approx time per iteration (seconds) — used to generate timestamps
    time_per_iter: f64,
    /// Generate phase time
    generate_time: f64,
}

const STENCIL_SPECS: &[StencilSpec] = &[
    StencilSpec {
        stencil: "5pt",
        size: 10_000,
        rho_cg: 0.95,
        rho_gmres: 0.97,
        rho_bicgstab: 0.94,
        time_per_iter: 0.034,
        generate_time: 5.5e-5,
    },
    StencilSpec {
        stencil: "7pt",
        size: 27_000,
        rho_cg: 0.96,
        rho_gmres: 0.97,
        rho_bicgstab: 0.95,
        time_per_iter: 0.090,
        generate_time: 6.0e-5,
    },
    StencilSpec {
        stencil: "27pt",
        size: 64_000,
        rho_cg: 0.97,
        rho_gmres: 0.98,
        rho_bicgstab: 0.96,
        time_per_iter: 0.45,
        generate_time: 7.0e-5,
    },
];

/// Number of iterations for each solver.
const NUM_ITERATIONS: usize = 100;

/// Generate a monotonically (non-increasing) residual curve with small multiplicative noise.
///
/// `r0` is the initial residual. `rho` is the convergence factor per iteration.
/// Noise is applied as `r_{k+1} = r_k * rho * (1 + small_noise)`.
/// The curve is guaranteed to be non-increasing: if noise would make it grow,
/// it is clamped to the previous value.
fn generate_residual_curve(
    r0: f64,
    rho: f64,
    iterations: usize,
    noise_stddev: f64,
    rng: &mut ChaCha20Rng,
) -> Vec<f64> {
    let mut curve = Vec::with_capacity(iterations + 1);
    curve.push(r0);
    let noise_distr = Normal::new(0.0_f64, noise_stddev).expect("valid normal params");

    let mut prev = r0;
    for _ in 0..iterations {
        let noise: f64 = noise_distr.sample(rng);
        let next_raw = prev * rho * (1.0 + noise);
        // Clamp to be non-increasing (allow up to 1% growth from noise)
        let next = next_raw.min(prev * 1.01).max(0.0);
        curve.push(next);
        prev = next;
    }
    curve
}

/// Generate timestamps for each iteration (linearly increasing with small jitter).
fn generate_timestamps(
    iterations: usize,
    time_per_iter: f64,
    noise_stddev: f64,
    rng: &mut ChaCha20Rng,
) -> Vec<f64> {
    let mut timestamps = Vec::with_capacity(iterations + 1);
    let noise_distr = Normal::new(0.0_f64, noise_stddev * time_per_iter).expect("valid params");

    let mut t = 0.0_f64;
    timestamps.push(t);
    for _ in 0..iterations {
        let jitter: f64 = noise_distr.sample(rng).abs();
        t += time_per_iter + jitter;
        timestamps.push(t);
    }
    timestamps
}

/// Generate a slightly-diverged copy of a residual curve (for true/implicit variants).
///
/// Each entry drifts from the reference by a small random factor, but the resulting curve
/// is guaranteed to be non-increasing (allowing up to 1% growth, matching the monotonicity
/// test tolerance).
fn diverge_residuals(reference: &[f64], drift_frac: f64, rng: &mut ChaCha20Rng) -> Vec<f64> {
    let mut out = Vec::with_capacity(reference.len());
    let mut prev = f64::MAX;
    for &r in reference {
        let drift: f64 = rng.random_range(-drift_frac..drift_frac);
        let raw = r * (1.0 + drift);
        // Clamp so the diverged curve is non-increasing (allow up to 1% growth like the
        // primary curve generator).
        let clamped = raw.min(prev * 1.01).max(0.0);
        out.push(clamped);
        prev = clamped;
    }
    out
}

/// Generate the `components` map for the apply phase.
fn generate_apply_components(total_time: f64, rng: &mut ChaCha20Rng) -> HashMap<String, f64> {
    // Decompose total_time into plausible named components
    // Proportions inspired by real GINKGO GMRES profiling output
    let mut components = HashMap::new();

    let overhead_frac: f64 = rng.random_range(0.01_f64..0.03_f64);
    let spmv_frac: f64 = rng.random_range(0.10_f64..0.20_f64);
    let dot_frac: f64 = rng.random_range(0.30_f64..0.50_f64);
    let sub_frac: f64 = rng.random_range(0.10_f64..0.20_f64);
    let check_frac: f64 = rng.random_range(0.01_f64..0.05_f64);
    // Remaining fraction goes to "other"
    let other_frac = (1.0 - overhead_frac - spmv_frac - dot_frac - sub_frac - check_frac).max(0.0);

    components.insert("overhead".to_string(), total_time * overhead_frac);
    components.insert("advanced_apply(sparse)".to_string(), total_time * spmv_frac);
    components.insert(
        "dense::compute_conj_dot_dispatch".to_string(),
        total_time * dot_frac,
    );
    components.insert("dense::sub_scaled".to_string(), total_time * sub_frac);
    components.insert(
        "check(gko::stop::Combined)".to_string(),
        total_time * check_frac,
    );
    components.insert("iteration".to_string(), total_time * other_frac);

    components
}

/// Generate the `components` map for the generate (setup) phase.
fn generate_setup_components(generate_time: f64, rng: &mut ChaCha20Rng) -> HashMap<String, f64> {
    let mut components = HashMap::new();
    let overhead_frac: f64 = rng.random_range(0.5_f64..0.9_f64);
    components.insert(
        "generate(solver::Factory)".to_string(),
        generate_time * (1.0 - overhead_frac),
    );
    components.insert("overhead".to_string(), generate_time * overhead_frac);
    components
}

/// Generate a single [`SolverResult`] for one method on one stencil.
fn generate_solver_result(
    spec: &StencilSpec,
    rho: f64,
    noise_stddev: f64,
    rng: &mut ChaCha20Rng,
) -> SolverResult {
    // r0 ≈ sqrt(size) to match real GINKGO output (rhs_norm ≈ 100 for size=10000)
    let r0 = (spec.size as f64).sqrt();

    let recurrent = generate_residual_curve(r0, rho, NUM_ITERATIONS, noise_stddev * 0.1, rng);
    let true_res = diverge_residuals(&recurrent, 0.02, rng);
    let implicit_res = diverge_residuals(&recurrent, 0.05, rng);

    let timestamps = generate_timestamps(NUM_ITERATIONS, spec.time_per_iter, noise_stddev, rng);

    let total_apply_time = *timestamps.last().unwrap_or(&0.0);
    let apply_components = generate_apply_components(total_apply_time, rng);
    let setup_components = generate_setup_components(spec.generate_time, rng);

    let generate_noise: f64 = rng.random_range(0.9_f64..1.1_f64);
    let generate_time = spec.generate_time * generate_noise;

    SolverResult {
        recurrent_residuals: Some(recurrent.clone()),
        true_residuals: Some(true_res),
        implicit_residuals: Some(implicit_res),
        iteration_timestamps: Some(timestamps),
        rhs_norm: r0,
        generate: SolverComponents {
            components: setup_components,
            time: generate_time,
            iterations: None,
        },
        apply: SolverComponents {
            components: apply_components,
            time: total_apply_time,
            iterations: Some(NUM_ITERATIONS as u64),
        },
        preconditioner: json!({}),
        residual_norm: *recurrent.last().unwrap_or(&0.0),
        repetitions: 1,
        completed: true,
    }
}

/// Generate the full `Vec<SolverBenchmark>` output.
pub fn generate_solver_benchmarks(
    noise_stddev: f64,
    rng: &mut ChaCha20Rng,
) -> Vec<SolverBenchmark> {
    STENCIL_SPECS
        .iter()
        .map(|spec| {
            let mut solver: HashMap<String, SolverResult> = HashMap::new();

            solver.insert(
                "cg".to_string(),
                generate_solver_result(spec, spec.rho_cg, noise_stddev, rng),
            );
            solver.insert(
                "gmres".to_string(),
                generate_solver_result(spec, spec.rho_gmres, noise_stddev, rng),
            );
            solver.insert(
                "bicgstab".to_string(),
                generate_solver_result(spec, spec.rho_bicgstab, noise_stddev, rng),
            );

            SolverBenchmark {
                stencil: spec.stencil.to_string(),
                size: spec.size,
                rows: spec.size,
                cols: spec.size,
                optimal: BenchmarkOptimal {
                    spmv: "csr".to_string(),
                },
                solver,
            }
        })
        .collect()
}

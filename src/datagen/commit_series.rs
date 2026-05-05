use super::{benchmarks, matrices, write_json_deterministic, GenConfig};
use crate::data::models::BenchmarkProblem;
use crate::types::DataFormat;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use rand_distr::{Distribution, Normal};
use serde::Serialize;
use smol_str::SmolStr;
use std::io;
use std::path::Path;

/// Stride between per-commit matrix-id ranges. The matrix `id` is encoded as
/// `commit_idx * COMMIT_ID_STRIDE + matrix_idx + 1`, which keeps ids unique
/// across commits (so loading two fixtures into the dashboard doesn't pretend
/// `synthetic_5` from commit A is the same matrix as `synthetic_5` from
/// commit B). Cap at 1e6 so 1000 commits × 1000 matrices fits in a u64
/// without overflow concerns.
const COMMIT_ID_STRIDE: u64 = 1_000_000;

/// Metadata for a single synthetic commit in the commit series.
#[derive(Serialize, serde::Deserialize, Debug, Clone)]
pub struct CommitMeta {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub message: String,
    pub file: String,
}

/// Derive a deterministic 7-char fake hex SHA from a commit index and base seed.
///
/// This is a synthetic hash, not a real git SHA.  Task 2 will integrate with real git.
pub fn fake_sha(commit_idx: usize, base_seed: u64) -> String {
    // Mix commit_idx and base_seed via a simple FNV-1a-inspired hash
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    h ^= commit_idx as u64;
    h = h.wrapping_mul(0x0000_0100_0000_01b3);
    h ^= base_seed;
    h = h.wrapping_mul(0x0000_0100_0000_01b3);
    format!("{:07x}", h & 0x00ff_ffff)
}

/// Per-commit author/message variation so the timeline doesn't read as one
/// developer hammering the same line on repeat. Drawn from a fixed pool
/// keyed by commit index so the output stays deterministic.
const AUTHORS: &[&str] = &[
    "Alex Chen <alex@ginkgo.dev>",
    "Maya Patel <maya@ginkgo.dev>",
    "Jonas Weber <jonas@ginkgo.dev>",
    "Sara Lima <sara@ginkgo.dev>",
    "Dmitri Volkov <dmitri@ginkgo.dev>",
];

const MESSAGE_POOL: &[&str] = &[
    "Optimize SpMV kernel",
    "Tune ELL row-padding heuristic",
    "Reorder loop nest in COO traversal",
    "Reduce shared-memory bank conflicts (HYBRID)",
    "Rework SELLP slice grouping",
    "Inline hot path in CSR matvec",
    "Cache column indices once per warp",
    "Bump compiler flags (-O3, vectorize)",
    "Pre-compute row offsets for HYBRID partition",
    "Drop redundant bounds check in inner loop",
];

/// Write a commit series to disk.
///
/// **Each commit re-rolls its matrix set from scratch** — different (rows,
/// cols, nnz, row_cv, …) per commit, with names suffixed by the commit's
/// short SHA so cross-commit name collisions can't trick the comparison
/// view into pairing unrelated matrices. Loading multiple fixtures gives
/// the dashboard `commits × matrices` distinct points instead of stacks of
/// (same-X, varying-Y) at a few X positions.
///
/// On top of the fresh matrix set, every commit also gets a per-format
/// performance multiplier driven by an independent random walk in
/// log-space (5% step σ per commit), and every (problem, format) pair is
/// jittered by 12% log-normal noise.
///
/// The middle commit gets an ELL regression bump (×1.6) that's strong
/// enough to read against the wider base noise; one earlier commit gets a
/// HYBRID improvement (×0.55) so the timeline doesn't trend monotonically.
pub fn write_commit_series(config: &GenConfig, out_dir: &Path) -> io::Result<Vec<CommitMeta>> {
    let commits = config.commits;
    let base_seed = config.seed;
    let regression_commit = commits / 2;
    // A deliberate improvement two commits before the regression so the
    // timeline shows both directions of change.
    let improvement_commit = regression_commit.saturating_sub(2);

    let mut manifest: Vec<CommitMeta> = Vec::with_capacity(commits);

    // Per-format multiplier evolves over commits as a random walk in log
    // space. The walk is seeded from `base_seed` so reruns are byte-stable.
    let formats = [
        DataFormat::CSR,
        DataFormat::COO,
        DataFormat::ELL,
        DataFormat::HYBRID,
        DataFormat::SELLP,
    ];
    let mut format_mults: std::collections::HashMap<DataFormat, f64> =
        formats.iter().copied().map(|f| (f, 1.0)).collect();

    let walk_step = Normal::new(-0.005, 0.05).expect("valid normal params");

    for commit_idx in 0..commits {
        let sha = fake_sha(commit_idx, base_seed);
        let filename = format!("bench_{}.json", sha);

        // Per-commit RNG so same args reproduce byte-identical output.
        // `1009` mixes commit_idx into the seed; the matrix-generation RNG
        // uses a different multiplier so the matrix and walk streams don't
        // accidentally correlate.
        let mut commit_rng = ChaCha20Rng::seed_from_u64(
            base_seed.wrapping_mul(1009).wrapping_add(commit_idx as u64),
        );
        let mut matrix_rng = ChaCha20Rng::seed_from_u64(
            base_seed
                .wrapping_mul(2_654_435_761)
                .wrapping_add(commit_idx as u64),
        );

        // Step the per-format random walk. Different formats drift
        // independently so commits don't move in lockstep.
        for f in formats {
            let step: f64 = walk_step.sample(&mut commit_rng);
            let m = format_mults.entry(f).or_insert(1.0);
            *m *= step.exp();
            // Clamp to a sane window so the random walk doesn't run away
            // after many commits.
            *m = m.clamp(0.5, 2.0);
        }

        let is_regression = commit_idx == regression_commit;
        let is_improvement = commit_idx == improvement_commit && commits >= 4;
        let ell_key = DataFormat::ELL.as_key();
        let hybrid_key = DataFormat::HYBRID.as_key();

        // Per-(problem, format) jitter: 12% log-normal noise so scatter
        // clouds visibly shift between commits even when the format-level
        // drift is small.
        let jitter = Normal::new(0.0, 0.12).expect("valid normal params");

        // Generate fresh matrices for *this* commit. Ids are offset by
        // commit_idx × stride so dashboard tooltips don't show two
        // unrelated matrices both named `synthetic_5`.
        let id_offset = (commit_idx as u64).wrapping_mul(COMMIT_ID_STRIDE);
        let commit_problems: Vec<BenchmarkProblem> = (0..config.matrices)
            .map(|i| {
                let id = id_offset + i as u64 + 1;
                let mut matrix = matrices::generate_matrix(&mut matrix_rng, id);
                // Suffix the matrix name with the short SHA so cross-commit
                // pairing in the comparison view honestly reports "no
                // matched problems" rather than mistaking them for the same
                // matrix.
                matrix.name = SmolStr::from(format!("{}_{}", matrix.name, sha));

                let mut p = benchmarks::generate_benchmark_problem(
                    matrix,
                    config.noise_stddev,
                    config.anomaly_rate,
                    &mut matrix_rng,
                );

                // Apply per-commit format multipliers + jitter + signal
                // bumps on top of the freshly-built per-format times.
                for fmt in formats {
                    let key = fmt.as_key();
                    let Some(entry) = p.spmv.get_mut(key) else {
                        continue;
                    };
                    let base_mult = format_mults[&fmt];
                    let problem_jitter: f64 = jitter.sample(&mut commit_rng);
                    let mut effective: f64 = base_mult * problem_jitter.exp();
                    if is_regression && key == ell_key {
                        effective *= 1.6;
                    }
                    if is_improvement && key == hybrid_key {
                        effective *= 0.55;
                    }
                    if let Some(t) = &mut entry.time {
                        *t *= effective;
                    }
                    if let Some(reps) = &mut entry.repetitions {
                        let rep_jitter: f64 = 1.0 + (commit_rng.random::<f64>() - 0.5) * 0.2;
                        *reps = ((*reps as f64) * rep_jitter).round().max(1.0) as u64;
                    }
                }
                p
            })
            .collect();

        let file_path = out_dir.join(&filename);
        let file = std::fs::File::create(&file_path)?;
        let writer = std::io::BufWriter::new(file);
        write_json_deterministic(writer, &commit_problems)?;

        // Date: 2026-01-01 + commit_idx days (proper date arithmetic)
        let base = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let day = base + chrono::Duration::days(commit_idx as i64);
        let date = format!("{}T00:00:00Z", day);

        let message = if is_regression {
            "Refactor ELL format (performance regression)".to_string()
        } else if is_improvement {
            "Vectorize HYBRID inner loop (large speedup)".to_string()
        } else {
            MESSAGE_POOL[commit_idx % MESSAGE_POOL.len()].to_string()
        };
        let author = AUTHORS[commit_idx % AUTHORS.len()].to_string();

        manifest.push(CommitMeta {
            sha,
            author,
            date,
            message,
            file: filename,
        });
    }

    // Write commits.json manifest
    let manifest_path = out_dir.join("commits.json");
    let manifest_file = std::fs::File::create(&manifest_path)?;
    let writer = std::io::BufWriter::new(manifest_file);
    write_json_deterministic(writer, &manifest)?;

    Ok(manifest)
}

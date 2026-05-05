pub mod benchmarks;
pub mod commit_series;
pub mod matrices;
pub mod rng;
pub mod solvers;

use crate::data::models::BenchmarkProblem;
use rand_chacha::ChaCha20Rng;
use std::io;
use std::path::PathBuf;

pub use rng::rng_from_seed;

/// Serialize to pretty JSON with deterministic key ordering.
///
/// Routes through `serde_json::Value` (which uses `BTreeMap` for objects)
/// so that `HashMap` fields produce sorted keys across runs.
pub fn write_json_deterministic<W: io::Write, T: serde::Serialize>(
    writer: W,
    data: &T,
) -> io::Result<()> {
    let value = serde_json::to_value(data).map_err(io::Error::other)?;
    serde_json::to_writer_pretty(writer, &value).map_err(io::Error::other)
}

/// Configuration for the synthetic data generator.
pub struct GenConfig {
    /// Number of matrices to generate.
    pub matrices: usize,
    /// Number of commit-series files. 0 = single-file mode.
    /// Must be 0 when `commit_sha` is `Some`.
    pub commits: usize,
    /// RNG seed for deterministic generation.
    pub seed: u64,
    /// Log-normal multiplicative noise sigma applied to benchmark times.
    pub noise_stddev: f64,
    /// Output directory.
    pub out_dir: PathBuf,
    /// When `Some`, activates single-commit-write mode.
    /// The value must be at least 7 hex characters; the first 7 chars are
    /// used as the short SHA for the output filename.
    pub commit_sha: Option<String>,
    /// Commit author in "Name <email>" form (for manifest enrichment).
    /// Defaults to `"Unknown <unknown@example.com>"` when `None`.
    pub commit_author: Option<String>,
    /// Commit date as an ISO 8601 string.
    /// Defaults to `"1970-01-01T00:00:00Z"` when `None` — an obvious
    /// sentinel that forces callers to supply a real value.
    pub commit_date: Option<String>,
    /// Commit subject/message line.
    /// Defaults to `""` when `None`.
    pub commit_message: Option<String>,
    /// Probability (per matrix) of receiving a per-format anomaly hit during
    /// SpMV generation. The plan calls for ~10% by default; tests override.
    pub anomaly_rate: f64,
}

/// Generate a `Vec<BenchmarkProblem>` (the benchmark.json content) deterministically.
///
/// This is the core generation function; it does not touch the filesystem.
pub fn generate_benchmark_file(config: &GenConfig, rng: &mut ChaCha20Rng) -> Vec<BenchmarkProblem> {
    (0..config.matrices)
        .map(|i| {
            let matrix = matrices::generate_matrix(rng, i as u64 + 1);
            benchmarks::generate_benchmark_problem(
                matrix,
                config.noise_stddev,
                config.anomaly_rate,
                rng,
            )
        })
        .collect()
}

/// Run the full datagen pipeline: generate files and write them to `config.out_dir`.
///
/// Returns the number of benchmark files written.
pub fn run(config: &GenConfig) -> io::Result<usize> {
    std::fs::create_dir_all(&config.out_dir)?;

    let mut rng = rng_from_seed(config.seed);

    // Single-file and SHA modes share one matrix set; commit-series mode
    // re-rolls matrices per commit, so it skips this expensive draw and
    // uses a separate solver RNG further down.
    let problems = if config.commits > 0 && config.commit_sha.is_none() {
        Vec::new()
    } else {
        generate_benchmark_file(config, &mut rng)
    };

    if let Some(sha) = &config.commit_sha {
        // Single-commit mode: write bench_<sha7>.json + upsert commits.json.
        // solver.json is intentionally omitted in this mode (per spec).
        let char_count = sha.chars().count();
        if char_count < 7 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "--sha must be at least 7 characters, got {} (\"{}\")",
                    char_count, sha
                ),
            ));
        }

        let short_sha: String = sha.chars().take(7).collect();
        let filename = format!("bench_{}.json", short_sha);
        let bench_path = config.out_dir.join(&filename);

        let file = std::fs::File::create(&bench_path)?;
        let writer = std::io::BufWriter::new(file);
        write_json_deterministic(writer, &problems)?;

        // Upsert commits.json manifest.
        let manifest_path = config.out_dir.join("commits.json");

        let mut entries: Vec<commit_series::CommitMeta> = if manifest_path.exists() {
            let bytes = std::fs::read(&manifest_path)?;
            serde_json::from_slice(&bytes).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("failed to parse commits.json: {}", e),
                )
            })?
        } else {
            Vec::new()
        };

        let new_entry = commit_series::CommitMeta {
            sha: sha.clone(),
            author: config
                .commit_author
                .clone()
                .unwrap_or_else(|| "Unknown <unknown@example.com>".to_string()),
            date: config
                .commit_date
                .clone()
                .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string()),
            message: config.commit_message.clone().unwrap_or_default(),
            file: filename,
        };

        // Replace existing entry with same SHA, or append.
        if let Some(pos) = entries.iter().position(|e| e.sha == *sha) {
            entries[pos] = new_entry;
        } else {
            entries.push(new_entry);
        }

        // Sort by sha ascending for byte-determinism across insertion orders.
        entries.sort_by(|a, b| a.sha.cmp(&b.sha));

        let manifest_file = std::fs::File::create(&manifest_path)?;
        let manifest_writer = std::io::BufWriter::new(manifest_file);
        write_json_deterministic(manifest_writer, &entries)?;

        Ok(1)
    } else if config.commits > 0 {
        // Commit-series mode: matrices are re-rolled per commit inside
        // `write_commit_series`. The main `rng` is unused here; solvers
        // get their own seed-derived stream so re-running with the same
        // `--seed` still yields byte-identical solver.json.
        let manifest = commit_series::write_commit_series(config, &config.out_dir)?;

        let mut solver_rng = rng_from_seed(config.seed.wrapping_add(7919));
        let solver_path = config.out_dir.join("solver.json");
        let solver_file = std::fs::File::create(&solver_path)?;
        let solver_writer = std::io::BufWriter::new(solver_file);
        let solver_data = solvers::generate_solver_benchmarks(config.noise_stddev, &mut solver_rng);
        write_json_deterministic(solver_writer, &solver_data)?;

        Ok(manifest.len())
    } else {
        // Single-file mode: write benchmark.json
        let bench_path = config.out_dir.join("benchmark.json");
        let file = std::fs::File::create(&bench_path)?;
        let writer = std::io::BufWriter::new(file);
        write_json_deterministic(writer, &problems)?;

        // Write solver.json
        let solver_path = config.out_dir.join("solver.json");
        let solver_file = std::fs::File::create(&solver_path)?;
        let solver_writer = std::io::BufWriter::new(solver_file);
        let solver_data = solvers::generate_solver_benchmarks(config.noise_stddev, &mut rng);
        write_json_deterministic(solver_writer, &solver_data)?;

        Ok(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::models::BenchmarkProblem;
    use crate::types::DataFormat;

    fn make_config(matrices: usize, commits: usize, seed: u64) -> GenConfig {
        GenConfig {
            matrices,
            commits,
            seed,
            noise_stddev: 0.08,
            out_dir: PathBuf::from("fixtures_test"),
            commit_sha: None,
            commit_author: None,
            commit_date: None,
            commit_message: None,
            anomaly_rate: 0.10,
        }
    }

    fn temp_subdir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "ginkgo_datagen_test_{}_{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tmp subdir");
        dir
    }

    /// Test 1: Determinism — calling generate twice with the same seed gives identical output.
    ///
    /// Note: we compare via `serde_json::Value` rather than raw strings because
    /// `HashMap` serialises keys in non-deterministic order across runs.  The
    /// *values* must be identical; key order within objects is irrelevant.
    #[test]
    fn test_determinism() {
        let config = make_config(10, 0, 42);

        let mut rng1 = rng_from_seed(config.seed);
        let result1 = generate_benchmark_file(&config, &mut rng1);

        let mut rng2 = rng_from_seed(config.seed);
        let result2 = generate_benchmark_file(&config, &mut rng2);

        let val1: serde_json::Value = serde_json::to_value(&result1).expect("serialize ok");
        let val2: serde_json::Value = serde_json::to_value(&result2).expect("serialize ok");

        assert_eq!(val1, val2, "Same seed must produce identical output");
    }

    /// Test 2: Round-trip — generated data serializes to JSON and parses back without error,
    /// and key fields survive the round-trip.
    #[test]
    fn test_round_trip() {
        let config = make_config(5, 0, 42);
        let mut rng = rng_from_seed(config.seed);
        let problems = generate_benchmark_file(&config, &mut rng);

        let json = serde_json::to_string(&problems).expect("serialize ok");
        let parsed: Vec<BenchmarkProblem> = serde_json::from_str(&json).expect("deserialize ok");

        assert_eq!(parsed.len(), problems.len(), "Round-trip preserves count");

        for (orig, rt) in problems.iter().zip(parsed.iter()) {
            assert_eq!(
                orig.problem.rows, rt.problem.rows,
                "rows survives round-trip"
            );
            assert_eq!(
                orig.problem.nonzeros, rt.problem.nonzeros,
                "nnz survives round-trip"
            );
            // All 5 format keys must be present
            for fmt in &[
                DataFormat::CSR,
                DataFormat::COO,
                DataFormat::ELL,
                DataFormat::HYBRID,
                DataFormat::SELLP,
            ] {
                assert!(
                    rt.spmv.contains_key(fmt.as_key()),
                    "spmv key '{}' missing after round-trip",
                    fmt.as_key()
                );
            }
        }
    }

    /// Test 3: Physical sanity (statistical) — ELL is *usually* faster than
    /// COO for low row_cv, and ELL is *usually* slower than HYBRID for high
    /// row_cv. With the wider per-format noise the redesign introduces,
    /// per-matrix invariants no longer hold; we sample only the CV tails
    /// and require ≥ 65% of each tail's matrices to follow the expected
    /// ordering.
    ///
    /// Note: the redesign's CV mixture sampler (LogNormal(-1.0, 0.7) for
    /// the 60% low-regime branch) places its median at ~0.37, so the very
    /// strict `< 0.05` lower threshold the plan suggested would yield
    /// effectively zero matrices in 200 draws. We use `< 0.20` for the
    /// low-CV tail (still inside the regime where `format_time` orders
    /// ELL strictly below COO) and `> 2.5` for the high-CV tail as planned.
    #[test]
    fn test_physical_sanity() {
        // Larger sample so both CV tails are populated.
        let config = make_config(200, 0, 42);
        let mut rng = rng_from_seed(config.seed);
        let problems = generate_benchmark_file(&config, &mut rng);

        let mut low_cv_total = 0_usize;
        let mut low_cv_pass = 0_usize;
        let mut high_cv_total = 0_usize;
        let mut high_cv_pass = 0_usize;

        for p in &problems {
            let row_cv = if p.problem.row_distribution.mean > 0.0 {
                p.problem.row_distribution.variance.sqrt() / p.problem.row_distribution.mean
            } else {
                continue;
            };

            let ell_time = p.spmv.get(DataFormat::ELL.as_key()).and_then(|e| e.time);
            let coo_time = p.spmv.get(DataFormat::COO.as_key()).and_then(|e| e.time);
            let hybrid_time = p.spmv.get(DataFormat::HYBRID.as_key()).and_then(|e| e.time);

            if row_cv < 0.20 {
                if let (Some(ell), Some(coo)) = (ell_time, coo_time) {
                    low_cv_total += 1;
                    if ell < coo {
                        low_cv_pass += 1;
                    }
                }
            }

            if row_cv > 2.5 {
                if let (Some(ell), Some(hybrid)) = (ell_time, hybrid_time) {
                    high_cv_total += 1;
                    if ell > hybrid {
                        high_cv_pass += 1;
                    }
                }
            }
        }

        assert!(
            low_cv_total >= 5,
            "Need at least 5 matrices with row_cv < 0.20 for a meaningful test, got {}",
            low_cv_total
        );
        assert!(
            high_cv_total >= 5,
            "Need at least 5 matrices with row_cv > 2.5 for a meaningful test, got {}",
            high_cv_total
        );

        // Require at least 65% of each tail to follow the expected ordering.
        let low_threshold = (low_cv_total * 65).div_ceil(100); // ceil(total * 0.65)
        let high_threshold = (high_cv_total * 65).div_ceil(100);
        assert!(
            low_cv_pass >= low_threshold,
            "Low-CV tail: only {}/{} matrices have ELL < COO (need >= {})",
            low_cv_pass,
            low_cv_total,
            low_threshold
        );
        assert!(
            high_cv_pass >= high_threshold,
            "High-CV tail: only {}/{} matrices have ELL > HYBRID (need >= {})",
            high_cv_pass,
            high_cv_total,
            high_threshold
        );
    }

    /// Test 4: Solver monotonicity — every residual curve is non-increasing
    /// (allowing up to 1% growth from noise: r[i+1] <= r[i] * 1.01).
    #[test]
    fn test_solver_monotonicity() {
        let config = make_config(5, 0, 42);
        let mut rng = rng_from_seed(config.seed);
        // Burn same matrices as in single-file mode to advance RNG to solver phase
        let _ = generate_benchmark_file(&config, &mut rng);

        let solver_data = solvers::generate_solver_benchmarks(config.noise_stddev, &mut rng);

        for bench in &solver_data {
            for (method_name, result) in &bench.solver {
                for (curve_name, curve_opt) in [
                    ("recurrent", &result.recurrent_residuals),
                    ("true", &result.true_residuals),
                    ("implicit", &result.implicit_residuals),
                ] {
                    if let Some(curve) = curve_opt {
                        for i in 0..curve.len().saturating_sub(1) {
                            let allowed_max = curve[i] * 1.01;
                            assert!(
                                curve[i + 1] <= allowed_max,
                                "Non-monotone {} residuals in {}/{}: r[{}]={}, r[{}]={} (max allowed {})",
                                curve_name,
                                bench.stencil,
                                method_name,
                                i,
                                curve[i],
                                i + 1,
                                curve[i + 1],
                                allowed_max
                            );
                        }
                    }
                }
            }
        }
    }

    /// Test 5: Commit series — with commits=5, exactly 5 bench files + 1 commits.json are
    /// produced, and the regression commit has higher ELL times than its neighbors.
    #[test]
    fn test_commit_series() {
        let tmp = std::env::temp_dir().join("ginkgo_datagen_test_commit_series");
        std::fs::create_dir_all(&tmp).expect("create temp dir");

        let config = GenConfig {
            // 80 matrices per commit gives a stable median; 40 was on the
            // edge of being noisy enough to swallow the ×1.6 ELL bump.
            matrices: 80,
            commits: 5,
            seed: 42,
            noise_stddev: 0.08,
            out_dir: tmp.clone(),
            commit_sha: None,
            commit_author: None,
            commit_date: None,
            commit_message: None,
            anomaly_rate: 0.10,
        };

        let result = run(&config);
        assert!(result.is_ok(), "run() should succeed: {:?}", result);
        assert_eq!(result.unwrap(), 5, "Expected 5 commit files");

        // Check commits.json exists
        let manifest_path = tmp.join("commits.json");
        assert!(manifest_path.exists(), "commits.json must exist");

        // Parse manifest
        let manifest_json = std::fs::read_to_string(&manifest_path).expect("read manifest");
        let manifest: Vec<commit_series::CommitMeta> =
            serde_json::from_str(&manifest_json).expect("parse manifest");

        assert_eq!(manifest.len(), 5, "Manifest must have 5 entries");

        // Check all bench files exist
        for meta in &manifest {
            let bench_path = tmp.join(&meta.file);
            assert!(bench_path.exists(), "Bench file {} must exist", meta.file);
        }

        // Verify regression commit has higher ELL times than its neighbors
        let regression_idx = 5 / 2; // = 2

        let load_ell_times = |file: &str| -> Vec<f64> {
            let path = tmp.join(file);
            let json = std::fs::read_to_string(path).expect("read bench file");
            let problems: Vec<BenchmarkProblem> =
                serde_json::from_str(&json).expect("parse bench file");
            problems
                .iter()
                .filter_map(|p| p.spmv.get(DataFormat::ELL.as_key()).and_then(|e| e.time))
                .collect()
        };

        let prev_times = load_ell_times(&manifest[regression_idx - 1].file);
        let regr_times = load_ell_times(&manifest[regression_idx].file);
        let next_times = load_ell_times(&manifest[regression_idx + 1].file);

        // With per-commit matrix resampling, prev / regr / next have
        // different matrices entirely (different rows, nnz, etc.), so
        // per-matrix comparison is meaningless. Compare medians instead:
        // the ×1.6 ELL bump should lift the regression commit's median
        // visibly above its neighbours' medians, even with the new wider
        // noise.
        fn median(mut v: Vec<f64>) -> f64 {
            assert!(!v.is_empty(), "median of empty list");
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let n = v.len();
            if n.is_multiple_of(2) {
                (v[n / 2 - 1] + v[n / 2]) / 2.0
            } else {
                v[n / 2]
            }
        }

        let prev_med = median(prev_times);
        let regr_med = median(regr_times);
        let next_med = median(next_times);

        // Each commit's matrix set differs in size/shape, so the absolute
        // medians can drift considerably. We require the regression commit's
        // median ELL time to be at least 10% above each neighbour's median.
        // The deterministic ×1.6 bump is far above 10% on average; the
        // 10% threshold gives headroom for the per-commit format walk
        // (clamped to ±50%) and the matrix-resampling variance.
        let prev_ratio = regr_med / prev_med;
        let next_ratio = regr_med / next_med;
        assert!(
            prev_ratio > 1.10,
            "Regression commit's median ELL time should be >1.10x previous commit's median; \
             got prev_med={prev_med:.3e}, regr_med={regr_med:.3e}, ratio={prev_ratio:.3}"
        );
        assert!(
            next_ratio > 1.10,
            "Regression commit's median ELL time should be >1.10x next commit's median; \
             got regr_med={regr_med:.3e}, next_med={next_med:.3e}, ratio={next_ratio:.3}"
        );

        // Sanity: the matrices should genuinely be different per commit —
        // confirm by checking that the prev and regr files have disjoint
        // matrix names (commit-unique suffix).
        let load_names = |file: &str| -> Vec<String> {
            let path = tmp.join(file);
            let json = std::fs::read_to_string(path).expect("read bench file");
            let problems: Vec<BenchmarkProblem> =
                serde_json::from_str(&json).expect("parse bench file");
            problems
                .iter()
                .map(|p| p.problem.name.to_string())
                .collect()
        };
        let prev_names = load_names(&manifest[regression_idx - 1].file);
        let regr_names = load_names(&manifest[regression_idx].file);
        let prev_set: std::collections::HashSet<_> = prev_names.iter().collect();
        let overlap = regr_names.iter().filter(|n| prev_set.contains(n)).count();
        assert_eq!(
            overlap, 0,
            "Per-commit matrix names must be disjoint (got {overlap} collisions)"
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Test 6: Single-commit mode writes bench_<sha7>.json and NOT benchmark.json.
    #[test]
    fn test_sha_mode_writes_correct_filename() {
        let tmp = temp_subdir("sha_filename");

        let config = GenConfig {
            matrices: 5,
            commits: 0,
            seed: 1,
            noise_stddev: 0.08,
            out_dir: tmp.clone(),
            commit_sha: Some("abcd1234567890abcd1234567890abcd1234567890".to_string()),
            commit_author: Some("Test <t@example.com>".to_string()),
            commit_date: Some("2026-04-19T00:00:00Z".to_string()),
            commit_message: Some("test commit".to_string()),
            anomaly_rate: 0.10,
        };

        let result = run(&config);
        assert!(result.is_ok(), "run() should succeed: {:?}", result);
        assert_eq!(result.unwrap(), 1);

        assert!(
            tmp.join("bench_abcd123.json").exists(),
            "bench_abcd123.json must exist"
        );
        assert!(
            !tmp.join("benchmark.json").exists(),
            "benchmark.json must NOT exist in single-commit mode"
        );
        assert!(tmp.join("commits.json").exists(), "commits.json must exist");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Test 7: Single-commit mode upserts the manifest — second call with same
    /// SHA replaces the entry rather than appending a duplicate.
    #[test]
    fn test_sha_mode_upserts_manifest() {
        let tmp = temp_subdir("sha_upsert");

        let sha = "abcd1234567890abcd1234567890abcd1234567890".to_string();

        let make = |author: &str| GenConfig {
            matrices: 5,
            commits: 0,
            seed: 1,
            noise_stddev: 0.08,
            out_dir: tmp.clone(),
            commit_sha: Some(sha.clone()),
            commit_author: Some(author.to_string()),
            commit_date: Some("2026-04-19T00:00:00Z".to_string()),
            commit_message: Some("test".to_string()),
            anomaly_rate: 0.10,
        };

        run(&make("First Author <first@example.com>")).expect("first run");
        run(&make("Second Author <second@example.com>")).expect("second run");

        let manifest_json =
            std::fs::read_to_string(tmp.join("commits.json")).expect("read manifest");
        let entries: Vec<commit_series::CommitMeta> =
            serde_json::from_str(&manifest_json).expect("parse manifest");

        assert_eq!(
            entries.len(),
            1,
            "Upsert must keep exactly one entry per SHA"
        );
        assert_eq!(
            entries[0].author, "Second Author <second@example.com>",
            "Second call's author must win"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Test 8: Single-commit mode appends distinct SHAs and keeps them sorted.
    #[test]
    fn test_sha_mode_appends_different_shas() {
        let tmp = temp_subdir("sha_append");

        let make = |sha: &str| GenConfig {
            matrices: 5,
            commits: 0,
            seed: 1,
            noise_stddev: 0.08,
            out_dir: tmp.clone(),
            commit_sha: Some(sha.to_string()),
            commit_author: None,
            commit_date: None,
            commit_message: None,
            anomaly_rate: 0.10,
        };

        run(&make("feed456abcdef1234567890abcdef1234567890ab")).expect("first sha");
        run(&make("abcd123abcdef1234567890abcdef1234567890ab")).expect("second sha");

        let manifest_json =
            std::fs::read_to_string(tmp.join("commits.json")).expect("read manifest");
        let entries: Vec<commit_series::CommitMeta> =
            serde_json::from_str(&manifest_json).expect("parse manifest");

        assert_eq!(
            entries.len(),
            2,
            "Two distinct SHAs must produce two entries"
        );
        // Sorted ascending: abcd123... < feed456...
        assert!(
            entries[0].sha < entries[1].sha,
            "Entries must be sorted by sha ascending: {:?} vs {:?}",
            entries[0].sha,
            entries[1].sha
        );
        assert!(
            tmp.join("bench_feed456.json").exists(),
            "bench_feed456.json must exist"
        );
        assert!(
            tmp.join("bench_abcd123.json").exists(),
            "bench_abcd123.json must exist"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Test 9: Single-commit mode is byte-deterministic — same args produce
    /// identical commits.json, and inserting two SHAs in reverse order also
    /// yields the same bytes.
    #[test]
    fn test_sha_mode_is_deterministic() {
        let tmp_a = temp_subdir("sha_det_a");
        let tmp_b = temp_subdir("sha_det_b");

        let sha1 = "abcd123abcdef1234567890abcdef1234567890ab".to_string();
        let sha2 = "feed456abcdef1234567890abcdef1234567890ab".to_string();

        let make_cfg = |sha: &str, out: &PathBuf| GenConfig {
            matrices: 5,
            commits: 0,
            seed: 77,
            noise_stddev: 0.08,
            out_dir: out.clone(),
            commit_sha: Some(sha.to_string()),
            commit_author: Some("Det Test <det@example.com>".to_string()),
            commit_date: Some("2026-01-01T00:00:00Z".to_string()),
            commit_message: Some("determinism check".to_string()),
            anomaly_rate: 0.10,
        };

        // Run A: sha1 then sha2.
        run(&make_cfg(&sha1, &tmp_a)).expect("a sha1");
        run(&make_cfg(&sha2, &tmp_a)).expect("a sha2");

        // Run B: sha2 then sha1 (reverse order).
        run(&make_cfg(&sha2, &tmp_b)).expect("b sha2");
        run(&make_cfg(&sha1, &tmp_b)).expect("b sha1");

        let manifest_a =
            std::fs::read_to_string(tmp_a.join("commits.json")).expect("read manifest a");
        let manifest_b =
            std::fs::read_to_string(tmp_b.join("commits.json")).expect("read manifest b");

        assert_eq!(
            manifest_a, manifest_b,
            "Manifests from forward and reverse insertion orders must be byte-identical"
        );

        let _ = std::fs::remove_dir_all(&tmp_a);
        let _ = std::fs::remove_dir_all(&tmp_b);
    }
}

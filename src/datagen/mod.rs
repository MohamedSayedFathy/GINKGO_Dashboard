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
}

/// Generate a `Vec<BenchmarkProblem>` (the benchmark.json content) deterministically.
///
/// This is the core generation function; it does not touch the filesystem.
pub fn generate_benchmark_file(config: &GenConfig, rng: &mut ChaCha20Rng) -> Vec<BenchmarkProblem> {
    (0..config.matrices)
        .map(|i| {
            let matrix = matrices::generate_matrix(rng, i as u64 + 1);
            benchmarks::generate_benchmark_problem(matrix, config.noise_stddev, rng)
        })
        .collect()
}

/// Run the full datagen pipeline: generate files and write them to `config.out_dir`.
///
/// Returns the number of benchmark files written.
pub fn run(config: &GenConfig) -> io::Result<usize> {
    std::fs::create_dir_all(&config.out_dir)?;

    let mut rng = rng_from_seed(config.seed);

    let problems = generate_benchmark_file(config, &mut rng);

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
        // Commit-series mode: write N bench_<sha>.json files + commits.json
        let manifest = commit_series::write_commit_series(
            &problems,
            config.commits,
            config.seed,
            &config.out_dir,
        )?;

        // Write solver.json (only in commit-series and single-file modes)
        let solver_path = config.out_dir.join("solver.json");
        let solver_file = std::fs::File::create(&solver_path)?;
        let solver_writer = std::io::BufWriter::new(solver_file);
        let solver_data = solvers::generate_solver_benchmarks(config.noise_stddev, &mut rng);
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

    /// Test 3: Physical sanity — ELL faster than COO for low row_cv; HYBRID faster than ELL for
    /// high row_cv.
    ///
    /// We use seed 42 and scan the generated set for matrices meeting each criterion.
    #[test]
    fn test_physical_sanity() {
        // Use a large enough set to find matrices in both CV regimes.
        let config = make_config(100, 0, 42);
        let mut rng = rng_from_seed(config.seed);
        let problems = generate_benchmark_file(&config, &mut rng);

        let mut found_low_cv = false;
        let mut found_high_cv = false;

        for p in &problems {
            let row_cv = if p.problem.row_distribution.mean > 0.0 {
                p.problem.row_distribution.variance.sqrt() / p.problem.row_distribution.mean
            } else {
                continue;
            };

            let ell_time = p.spmv.get(DataFormat::ELL.as_key()).and_then(|e| e.time);
            let coo_time = p.spmv.get(DataFormat::COO.as_key()).and_then(|e| e.time);
            let hybrid_time = p.spmv.get(DataFormat::HYBRID.as_key()).and_then(|e| e.time);

            if row_cv < 0.1 {
                if let (Some(ell), Some(coo)) = (ell_time, coo_time) {
                    assert!(
                        ell < coo,
                        "For low row_cv ({:.3}), ELL ({:.6}s) should be faster than COO ({:.6}s)",
                        row_cv,
                        ell,
                        coo
                    );
                    found_low_cv = true;
                }
            }

            if row_cv > 1.5 {
                if let (Some(ell), Some(hybrid)) = (ell_time, hybrid_time) {
                    assert!(
                        ell > hybrid,
                        "For high row_cv ({:.3}), ELL ({:.6}s) should be slower than HYBRID ({:.6}s)",
                        row_cv,
                        ell,
                        hybrid
                    );
                    found_high_cv = true;
                }
            }
        }

        assert!(
            found_low_cv,
            "No matrix with row_cv < 0.1 found — increase matrix count or check class sampling"
        );
        assert!(
            found_high_cv,
            "No matrix with row_cv > 1.5 found — increase matrix count or check class sampling"
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
            matrices: 20,
            commits: 5,
            seed: 42,
            noise_stddev: 0.08,
            out_dir: tmp.clone(),
            commit_sha: None,
            commit_author: None,
            commit_date: None,
            commit_message: None,
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

        assert_eq!(
            prev_times.len(),
            regr_times.len(),
            "All commits must have same matrix count"
        );

        // Count how many matrices have regression_ell > prev_ell and regression_ell > next_ell
        let n = prev_times.len();
        let mut regression_count = 0;
        for i in 0..n {
            if regr_times[i] > prev_times[i] && regr_times[i] > next_times[i] {
                regression_count += 1;
            }
        }

        assert!(
            regression_count >= n / 2,
            "Regression commit must have higher ELL times than neighbors for at least half the matrices. \
             Got {}/{} matrices with regression.",
            regression_count,
            n
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

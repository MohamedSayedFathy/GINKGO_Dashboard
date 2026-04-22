use super::write_json_deterministic;
use crate::data::models::BenchmarkProblem;
use crate::types::DataFormat;
use serde::Serialize;
use std::io;
use std::path::Path;

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

/// Write a commit series to disk.
///
/// For each commit:
/// - Applies a cumulative 1%/commit performance improvement (time × 0.99^commit_idx).
/// - For the middle commit, injects a regression: ELL times × 1.2.
/// - Writes `bench_<sha>.json` and appends an entry to the manifest.
/// - Emits `commits.json` with the full manifest at the end.
pub fn write_commit_series(
    problems: &[BenchmarkProblem],
    commits: usize,
    base_seed: u64,
    out_dir: &Path,
) -> io::Result<Vec<CommitMeta>> {
    let regression_commit = commits / 2;

    let mut manifest: Vec<CommitMeta> = Vec::with_capacity(commits);

    for commit_idx in 0..commits {
        let sha = fake_sha(commit_idx, base_seed);
        let filename = format!("bench_{}.json", sha);

        // Performance drift: 1% improvement per commit (compounding)
        let improvement = 0.99_f64.powi(commit_idx as i32);
        let is_regression = commit_idx == regression_commit;

        let ell_key = DataFormat::ELL.as_key();

        // Build modified problems for this commit
        let commit_problems: Vec<BenchmarkProblem> = problems
            .iter()
            .map(|p| {
                let mut p2 = p.clone();
                for (key, entry) in &mut p2.spmv {
                    if let Some(t) = &mut entry.time {
                        let mut new_t = *t * improvement;
                        // Inject regression on ELL format for the regression commit
                        if is_regression && key == ell_key {
                            new_t *= 1.2;
                        }
                        *t = new_t;
                    }
                }
                p2
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
        } else {
            "Optimize SpMV kernel".to_string()
        };

        manifest.push(CommitMeta {
            sha,
            author: "Dev User <dev@example.com>".to_string(),
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

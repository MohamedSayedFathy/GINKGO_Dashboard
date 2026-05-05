//! Git repository integration for commit-aware benchmarking.
//!
//! This module walks a local git repo (via `gix`), associates each commit
//! with an optional benchmark JSON file, and exposes the resulting
//! [`CommitInfo`] list to the rest of the dashboard.
//!
//! Benchmark-file discovery has two modes:
//! 1. **Manifest mode** (preferred): parse `<repo>/benchmarks/commits.json` as
//!    a `Vec<CommitMeta>` (produced by the datagen tool) and match each
//!    git commit SHA to a manifest entry by prefix.
//! 2. **Convention mode** (fallback): look for
//!    `<repo>/benchmarks/bench_<short_sha>.json`, where `<short_sha>` is
//!    the 7-char hex prefix of the commit SHA.
//!
//! All filesystem/git errors are bubbled up as [`GitError`]; no panics.

//! ## Target-gating
//!
//! The *types* in this module (`CommitInfo`, `GitState`, `GitError`) are
//! defined unconditionally so that `Dashboard`, `LoadResult`, and the
//! sidebar UI have a stable shape on both native and wasm32 builds.
//!
//! The *implementation* (`load_commits` and its gix/fs helpers) is gated
//! behind `cfg(not(target_arch = "wasm32"))` — gix has no web story, and
//! W1's scope is "make it build for wasm", not "port git integration to
//! the browser". The wasm stub in `state.rs::load_git_repo` surfaces a
//! user-visible error if invoked on wasm; see W2/W3 for the real web
//! data-source plan.

#[cfg(not(target_arch = "wasm32"))]
use crate::data::state::LoadProgress;
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;

/// How many commits to walk between progress callback invocations. Matches
/// the cadence used for streaming JSON loads; see `PROGRESS_BATCH_SIZE` in
/// `state.rs` for the rationale (once-per-batch, not once-per-item).
#[cfg(not(target_arch = "wasm32"))]
const GIT_PROGRESS_BATCH_SIZE: usize = 32;

/// Errors that can occur while loading commits from a git repository.
#[derive(Debug, Error)]
pub enum GitError {
    #[error("Failed to open git repository at {path:?}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Failed to locate HEAD: {0}")]
    Head(Box<dyn std::error::Error + Send + Sync>),

    #[error("Commit walk failed: {0}")]
    Walk(Box<dyn std::error::Error + Send + Sync>),

    #[error("Failed to decode commit {sha}: {source}")]
    Decode {
        sha: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Failed to parse manifest at {path:?}: {source}")]
    ManifestParse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("IO error for {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Network/HTTP error while fetching the manifest over HTTP (web build).
    ///
    /// Cross-target on purpose: wraps a plain `String` message plus optional
    /// HTTP status so the same `AppError::Git` arm carries both gix errors
    /// on native and fetch errors on wasm. `status` is `None` when the
    /// request never produced a response (DNS failure, network offline,
    /// CORS preflight rejection, etc.).
    #[error("HTTP fetch failed for {url}: {message}{}", status.map(|s| format!(" (status {s})")).unwrap_or_default())]
    Fetch {
        url: String,
        status: Option<u16>,
        message: String,
    },
}

/// Metadata for one commit in the local repository.
///
/// Fields mirror the common subset of git log plus an optional
/// pointer to a benchmark JSON file associated with this commit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitInfo {
    /// Full 40-character hex SHA.
    pub sha: String,
    /// 7-character hex prefix of `sha`.
    pub short_sha: String,
    /// Author in the form "Name <email>".
    pub author: String,
    /// Commit date as an RFC 3339 / ISO 8601 string (UTC).
    pub date: String,
    /// Subject line (first line of commit message).
    pub message: String,
    /// Path to the benchmark JSON file for this commit, if discovered.
    pub bench_file: Option<PathBuf>,
}

/// Persistent git state held on the dashboard.
#[derive(Default)]
pub struct GitState {
    /// Repository root currently loaded, if any.
    pub repo_path: Option<PathBuf>,
    /// Commits loaded from the repo (newest first, as returned by the walk).
    pub commits: Vec<CommitInfo>,
    /// Index into `commits` of the user-selected row.
    pub selected_commit_idx: Option<usize>,
    /// Last error message surfaced to the UI, if any.
    pub last_error: Option<String>,
    /// Last selected-commit index at which `prefetch_adjacent_commits` fired.
    /// Prevents repeated prefetch attempts on idle frames where the selection
    /// has not changed.
    pub last_prefetch_idx: Option<usize>,
    /// Whether a prefetch (adjacent-commit background load) is already in
    /// flight. Set to `true` when a prefetch is launched, cleared when the
    /// resulting `LoadResult::Benchmark` is applied.
    pub prefetch_in_flight: bool,
}

/// Manifest entry shape produced by the `datagen` tool.
///
/// This mirrors `crate::datagen::commit_series::CommitMeta` so that the
/// manifest can be parsed regardless of whether the `datagen` feature is
/// enabled in this build.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub message: String,
    /// Filename (relative to the `benchmarks/` directory).
    pub file: String,
}

/// Convert a list of parsed `ManifestEntry` values into `CommitInfo` rows
/// suitable for display in the git panel.
///
/// This helper is cross-target because both the native gix-driven code path
/// and the wasm HTTP-fetch path need the same `ManifestEntry -> CommitInfo`
/// mapping. Keeping it pure (no I/O, no platform types) makes it unit
/// testable on native without any fetch-mocking machinery.
///
/// `bench_file` is populated with `benchmarks/<entry.file>` as a
/// `PathBuf`. On wasm callers should treat this as a *relative URL* (Task 7
/// will fetch it via `ehttp`); on native it's a *path relative to the repo
/// root* and would normally be absolutised before `fs::File::open`.
pub fn manifest_to_commits(entries: Vec<ManifestEntry>) -> Vec<CommitInfo> {
    entries
        .into_iter()
        .map(|entry| {
            let short_sha: String = entry.sha.chars().take(7).collect();
            let bench_file = Some(PathBuf::from(format!("benchmarks/{}", entry.file)));
            CommitInfo {
                sha: entry.sha,
                short_sha,
                author: entry.author,
                date: entry.date,
                message: entry.message,
                bench_file,
            }
        })
        .collect()
}

/// Open a repo, walk its commits, and resolve benchmark files for each one.
///
/// `repo_path` must point at a local git working tree (or bare repo).
/// The walk is capped at `max_commits` to handle very large histories.
///
/// `on_progress` is invoked approximately every `GIT_PROGRESS_BATCH_SIZE`
/// commits with the running count; callers can forward it to a channel to
/// drive a UI progress indicator. The callback is infallible — it takes a
/// `LoadProgress` by value and has no return — so that forwarding-to-channel
/// send errors (e.g. UI thread has dropped) silently no-op in the caller.
///
/// This function is pure with respect to the program state: it only
/// reads the filesystem. Returns a `Result` so the UI can render errors.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_commits<F>(
    repo_path: &Path,
    max_commits: usize,
    mut on_progress: F,
) -> Result<Vec<CommitInfo>, GitError>
where
    F: FnMut(LoadProgress),
{
    let repo = gix::open(repo_path).map_err(|e| GitError::Open {
        path: repo_path.to_path_buf(),
        source: Box::new(e),
    })?;

    on_progress(LoadProgress {
        current: 0,
        total: None,
        phase: "Opening repository",
    });

    // Resolve HEAD to a single starting commit id.
    let head_id = repo
        .head_id()
        .map_err(|e| GitError::Head(Box::new(e)))?
        .detach();

    // Walk commits in commit-time descending order (newest first).
    let walk = repo
        .rev_walk(std::iter::once(head_id))
        .sorting(gix::traverse::commit::simple::Sorting::ByCommitTimeNewestFirst)
        .all()
        .map_err(|e| GitError::Walk(Box::new(e)))?;

    // Load manifest (if present) once, outside the per-commit loop.
    let manifest_lookup = load_manifest_lookup(repo_path)?;

    let bench_dir = repo_path.join("benchmarks");

    let mut commits: Vec<CommitInfo> = Vec::new();

    for (i, info_result) in walk.take(max_commits).enumerate() {
        let info = info_result.map_err(|e| GitError::Walk(Box::new(e)))?;
        let id = info.id;
        let sha = id.to_hex().to_string();

        // Load the full commit to access author + message + time.
        let commit = info.object().map_err(|e| GitError::Decode {
            sha: sha.clone(),
            source: Box::new(e),
        })?;

        let author_sig = commit.author().map_err(|e| GitError::Decode {
            sha: sha.clone(),
            source: Box::new(e),
        })?;
        let author = format!(
            "{} <{}>",
            bstr_to_string(author_sig.name),
            bstr_to_string(author_sig.email)
        );

        let time = commit.time().map_err(|e| GitError::Decode {
            sha: sha.clone(),
            source: Box::new(e),
        })?;
        let date = format_git_time(time);

        let message_raw = commit.message_raw_sloppy();
        let message = subject_line(message_raw);

        let short_sha: String = sha.chars().take(7).collect();

        let bench_file = resolve_bench_file(&sha, &short_sha, &bench_dir, manifest_lookup.as_ref());

        commits.push(CommitInfo {
            sha,
            short_sha,
            author,
            date,
            message,
            bench_file,
        });

        if (i + 1) % GIT_PROGRESS_BATCH_SIZE == 0 {
            on_progress(LoadProgress {
                current: i + 1,
                total: None,
                phase: "Walking commits",
            });
        }
    }

    on_progress(LoadProgress {
        current: commits.len(),
        total: None,
        phase: "Walking commits",
    });

    Ok(commits)
}

/// Load the manifest (if present) and return a SHA-prefix -> absolute-path map.
///
/// A missing manifest is not an error and yields `Ok(None)`.
#[cfg(not(target_arch = "wasm32"))]
fn load_manifest_lookup(repo_path: &Path) -> Result<Option<HashMap<String, PathBuf>>, GitError> {
    let bench_dir = repo_path.join("benchmarks");
    let manifest_path = bench_dir.join("commits.json");
    if !manifest_path.exists() {
        return Ok(None);
    }

    let bytes = std::fs::read(&manifest_path).map_err(|e| GitError::Io {
        path: manifest_path.clone(),
        source: e,
    })?;
    let entries: Vec<ManifestEntry> =
        serde_json::from_slice(&bytes).map_err(|e| GitError::ManifestParse {
            path: manifest_path.clone(),
            source: e,
        })?;

    let mut lookup: HashMap<String, PathBuf> = HashMap::with_capacity(entries.len());
    for entry in entries {
        let abs = bench_dir.join(&entry.file);
        lookup.insert(entry.sha, abs);
    }
    Ok(Some(lookup))
}

/// Resolve a benchmark file for `sha` using manifest-then-convention precedence.
///
/// `manifest` is a prebuilt lookup where the key is the SHA (possibly a
/// short prefix) and the value is an absolute path on disk.
#[cfg(not(target_arch = "wasm32"))]
fn resolve_bench_file(
    sha: &str,
    short_sha: &str,
    bench_dir: &Path,
    manifest: Option<&HashMap<String, PathBuf>>,
) -> Option<PathBuf> {
    if let Some(lookup) = manifest {
        if let Some(path) = match_manifest(sha, lookup) {
            if path.exists() {
                return Some(path);
            }
        }
    }

    // Convention mode: bench_<short_sha>.json
    let convention_path = bench_dir.join(format!("bench_{}.json", short_sha));
    if convention_path.exists() {
        return Some(convention_path);
    }

    None
}

/// Match a full 40-char `git_sha` to a manifest entry whose key may be a
/// shorter prefix (e.g. 7 chars). Returns an owned `PathBuf` on match.
#[cfg(not(target_arch = "wasm32"))]
fn match_manifest(git_sha: &str, manifest: &HashMap<String, PathBuf>) -> Option<PathBuf> {
    // Fast path: exact match.
    if let Some(p) = manifest.get(git_sha) {
        return Some(p.clone());
    }
    // Prefix path: manifest keys are typically 7-char abbreviations.
    for (key, path) in manifest {
        if !key.is_empty() && git_sha.starts_with(key.as_str()) {
            return Some(path.clone());
        }
    }
    None
}

/// Decode a `BStr` (the gix byte-string type) into a lossy UTF-8 `String`.
#[cfg(not(target_arch = "wasm32"))]
fn bstr_to_string(bs: &gix::bstr::BStr) -> String {
    String::from_utf8_lossy(bs.as_ref()).into_owned()
}

/// Format a `gix_date::Time` as RFC 3339 / ISO 8601 UTC.
///
/// Use chrono (already a dep) for deterministic UTC output independent of the
/// commit's recorded timezone offset.
#[cfg(not(target_arch = "wasm32"))]
fn format_git_time(time: gix::date::Time) -> String {
    let secs = time.seconds;
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| format!("unix:{}", secs))
}

/// Extract the first line of a commit message as a `String`.
#[cfg(not(target_arch = "wasm32"))]
fn subject_line(raw: &gix::bstr::BStr) -> String {
    let bytes: &[u8] = raw.as_ref();
    let end = bytes
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_arch = "wasm32"))]
    use std::io::Write;

    fn temp_subdir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("ginkgo_git_test_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tmp subdir");
        dir
    }

    /// Manifest parsing test — write a plausible `commits.json` and verify
    /// we can reconstruct the SHA → path lookup.
    #[test]
    fn test_manifest_parsing() {
        let dir = temp_subdir("manifest_parse");
        let bench_dir = dir.join("benchmarks");
        std::fs::create_dir_all(&bench_dir).unwrap();

        let entries = vec![
            ManifestEntry {
                sha: "abcdef0".to_string(),
                author: "Dev <dev@example.com>".to_string(),
                date: "2026-01-01T00:00:00Z".to_string(),
                message: "first".to_string(),
                file: "bench_abcdef0.json".to_string(),
            },
            ManifestEntry {
                sha: "1234567".to_string(),
                author: "Dev <dev@example.com>".to_string(),
                date: "2026-01-02T00:00:00Z".to_string(),
                message: "second".to_string(),
                file: "bench_1234567.json".to_string(),
            },
        ];

        let mut f = std::fs::File::create(bench_dir.join("commits.json")).unwrap();
        f.write_all(&serde_json::to_vec_pretty(&entries).unwrap())
            .unwrap();
        drop(f);

        // Touch the referenced bench files so the existence-check inside
        // load_manifest_lookup is satisfied (lookup itself doesn't check).
        std::fs::File::create(bench_dir.join("bench_abcdef0.json")).unwrap();
        std::fs::File::create(bench_dir.join("bench_1234567.json")).unwrap();

        let lookup = load_manifest_lookup(&dir)
            .expect("manifest parse")
            .expect("manifest present");
        assert_eq!(lookup.len(), 2);
        assert_eq!(
            lookup.get("abcdef0"),
            Some(&bench_dir.join("bench_abcdef0.json"))
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Prefix-match test — a 40-char git SHA should find the matching 7-char
    /// manifest key and return its path.
    #[test]
    fn test_sha_prefix_matching() {
        let mut lookup: HashMap<String, PathBuf> = HashMap::new();
        lookup.insert(
            "abcdef0".to_string(),
            PathBuf::from("/bench/bench_abcdef0.json"),
        );
        lookup.insert(
            "1234567".to_string(),
            PathBuf::from("/bench/bench_1234567.json"),
        );

        let full_sha = "abcdef0123456789abcdef0123456789abcdef01";
        let hit = match_manifest(full_sha, &lookup).expect("should resolve via prefix");
        assert_eq!(hit, PathBuf::from("/bench/bench_abcdef0.json"));

        let miss = match_manifest("deadbeef0000000000000000000000000000beef", &lookup);
        assert!(miss.is_none(), "no matching prefix should return None");
    }

    /// Convention-mode test — with no manifest, a `bench_<short_sha>.json`
    /// file in `benchmarks/` should be discovered.
    #[test]
    fn test_convention_fallback() {
        let dir = temp_subdir("conv_fallback");
        let bench_dir = dir.join("benchmarks");
        std::fs::create_dir_all(&bench_dir).unwrap();

        let short_sha = "abc1234";
        let full_sha = format!("{}{}", short_sha, "0".repeat(33));
        let expected = bench_dir.join("bench_abc1234.json");
        std::fs::File::create(&expected).unwrap();

        let resolved = resolve_bench_file(&full_sha, short_sha, &bench_dir, None);
        assert_eq!(resolved, Some(expected));

        // Without the file, resolution should return None.
        let resolved_miss = resolve_bench_file("dead000", "dead000", &bench_dir, None);
        assert_eq!(resolved_miss, None);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `manifest_to_commits`: pure conversion from parsed manifest entries
    /// to `CommitInfo` rows. Verifies short_sha truncation, bench_file URL
    /// format (`benchmarks/<file>`), field passthrough, and vector length.
    #[test]
    fn test_manifest_to_commits() {
        let entries = vec![
            ManifestEntry {
                sha: "abcdef0123456789abcdef0123456789abcdef01".to_string(),
                author: "Dev <dev@example.com>".to_string(),
                date: "2026-01-01T00:00:00Z".to_string(),
                message: "first".to_string(),
                file: "bench_abcdef0.json".to_string(),
            },
            ManifestEntry {
                sha: "1234567".to_string(),
                author: "Other <other@example.com>".to_string(),
                date: "2026-01-02T00:00:00Z".to_string(),
                message: "second".to_string(),
                file: "bench_1234567.json".to_string(),
            },
        ];

        let commits = manifest_to_commits(entries);
        assert_eq!(commits.len(), 2);

        // Long SHA gets truncated to 7 chars; short SHA is preserved whole
        // (chars().take(7) tolerates shorter input).
        assert_eq!(commits[0].sha.len(), 40);
        assert_eq!(commits[0].short_sha, "abcdef0");
        assert_eq!(commits[1].sha, "1234567");
        assert_eq!(commits[1].short_sha, "1234567");

        // Field passthrough.
        assert_eq!(commits[0].author, "Dev <dev@example.com>");
        assert_eq!(commits[0].date, "2026-01-01T00:00:00Z");
        assert_eq!(commits[0].message, "first");

        // bench_file is populated with the benchmarks/<file> prefix (URL-as-PathBuf).
        assert_eq!(
            commits[0].bench_file,
            Some(PathBuf::from("benchmarks/bench_abcdef0.json"))
        );
        assert_eq!(
            commits[1].bench_file,
            Some(PathBuf::from("benchmarks/bench_1234567.json"))
        );
    }

    /// Integration test — initialise a real temp git repo via the `git` CLI,
    /// commit two files, and verify we can read them back. Marked `#[ignore]`
    /// because `git` may not be installed in every CI configuration.
    #[test]
    #[ignore = "requires `git` binary on PATH"]
    fn test_load_commits_from_real_repo() {
        use std::process::Command;

        let dir = temp_subdir("real_repo");

        let run_git = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .expect("run git");
            assert!(status.success(), "git {:?} failed", args);
        };

        run_git(&["init", "-q", "-b", "main"]);
        run_git(&["config", "user.email", "test@example.com"]);
        run_git(&["config", "user.name", "Test User"]);

        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        run_git(&["add", "a.txt"]);
        run_git(&["commit", "-q", "-m", "first commit"]);

        std::fs::write(dir.join("b.txt"), "world").unwrap();
        run_git(&["add", "b.txt"]);
        run_git(&["commit", "-q", "-m", "second commit\n\nbody ignored"]);

        let progress_events = std::cell::RefCell::new(Vec::<LoadProgress>::new());
        let commits =
            load_commits(&dir, 10, |p| progress_events.borrow_mut().push(p)).expect("load commits");
        assert_eq!(
            commits.len(),
            2,
            "expected 2 commits, got {}",
            commits.len()
        );
        // Newest-first ordering.
        assert_eq!(commits[0].message, "second commit");
        assert_eq!(commits[1].message, "first commit");
        assert_eq!(commits[0].sha.len(), 40);
        assert_eq!(commits[0].short_sha.len(), 7);
        assert!(commits[0].author.contains("Test User"));
        assert!(commits[0].bench_file.is_none());

        // Progress callback contract: at minimum the "Opening repository" event
        // and a final "Walking commits" event with the full count.
        let events = progress_events.borrow();
        assert!(
            events.len() >= 2,
            "expected >= 2 progress events, got {}",
            events.len()
        );
        assert_eq!(events.first().unwrap().phase, "Opening repository");
        let last = events.last().unwrap();
        assert_eq!(last.phase, "Walking commits");
        assert_eq!(last.current, 2);

        std::fs::remove_dir_all(&dir).ok();
    }
}

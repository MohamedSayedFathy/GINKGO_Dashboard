//! Wasm-only HTTP commit source.
//!
//! Replaces the native `gix`-driven `git::load_commits` path for web builds.
//! Fetches `benchmarks/commits.json` from the same origin via `ehttp` and
//! forwards the resulting commit list through the existing `LoadUpdate`
//! channel that the UI thread already drains in
//! [`Dashboard::check_loading_status`].
//!
//! Scope (W3):
//! - Populate the commit list so the sidebar renders selectable commits.
//! - Do *not* download per-commit bench files on selection — Task 7 owns
//!   that flow. `CommitInfo.bench_file` stores `benchmarks/<file>` as a
//!   `PathBuf` that Task 7 will later recognise as a relative URL.
//!
//! Error shape: HTTP/network failures surface as `GitError::Fetch`; parse
//! failures surface as `GitError::ManifestParse` with a synthetic path so
//! the error message still reads sensibly.

#![cfg(target_arch = "wasm32")]

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use super::git::{manifest_to_commits, GitError, ManifestEntry};
use super::state::{LoadProgress, LoadResult, LoadUpdate};
use super::AppError;

/// Same-origin URL for the commits manifest. Resolved against the document
/// base URL (what `trunk serve` serves today, what W4 will deploy to GitHub
/// Pages tomorrow), so no host/scheme hardcoding is needed.
const COMMITS_URL: &str = "benchmarks/commits.json";

/// Type alias matching the `LoadChannel` message shape used on native.
///
/// Wasm is single-threaded, but `std::sync::mpsc::Sender` still enforces
/// its `Send` bound at compile time; all captured types here are `Send` so
/// we don't have to pull in a separate channel crate.
type LoadUpdateMessage = Result<LoadUpdate, AppError>;

/// Kick off an async fetch of `benchmarks/commits.json` and forward the
/// parsed commits through `tx`.
///
/// Returns immediately. The `ehttp::fetch` closure fires later on a JS
/// microtask when the `web-sys::fetch` promise resolves; at that point we
/// send exactly one terminal message (`Done` on success, top-level `Err`
/// on any failure) and request a repaint so the UI picks it up on the
/// next frame.
pub fn load_commits_http(ctx: egui::Context, tx: Sender<LoadUpdateMessage>) {
    // Best-effort progress emission: inform the UI that a fetch is in
    // flight before we hand off to the ehttp callback. Ignore send errors
    // (the receiver may have been dropped if the user reloaded mid-flight).
    let _ = tx.send(Ok(LoadUpdate::Progress(LoadProgress {
        current: 0,
        total: None,
        phase: "Fetching commits.json",
    })));
    ctx.request_repaint();

    let request = ehttp::Request::get(COMMITS_URL);

    ehttp::fetch(request, move |response: ehttp::Result<ehttp::Response>| {
        let terminal: LoadUpdateMessage = match response {
            Ok(resp) if resp.ok => {
                match serde_json::from_slice::<Vec<ManifestEntry>>(&resp.bytes) {
                    Ok(entries) => Ok(LoadUpdate::Done(LoadResult::Git(manifest_to_commits(
                        entries,
                    )))),
                    Err(e) => Err(AppError::Git(GitError::ManifestParse {
                        path: PathBuf::from(COMMITS_URL),
                        source: e,
                    })),
                }
            }
            // `resp.ok == false` means the server returned a non-2xx status
            // (404, 500, ...). `ehttp` still delivers this as `Ok(resp)`; we
            // convert it to a `Fetch` error so the UI shows a useful message.
            Ok(resp) => Err(AppError::Git(GitError::Fetch {
                url: COMMITS_URL.to_string(),
                status: Some(resp.status),
                message: if resp.status_text.is_empty() {
                    format!("HTTP {}", resp.status)
                } else {
                    resp.status_text.clone()
                },
            })),
            Err(msg) => Err(AppError::Git(GitError::Fetch {
                url: COMMITS_URL.to_string(),
                status: None,
                message: msg,
            })),
        };

        let _ = tx.send(terminal);
        ctx.request_repaint();
    });
}

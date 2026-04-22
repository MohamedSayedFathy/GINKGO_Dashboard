//! Wasm entry point.
//!
//! This module is gated `#[cfg(target_arch = "wasm32")]` at the point of
//! inclusion in `lib.rs`, so all code here assumes the wasm target. Native
//! builds never see these symbols and therefore never pull in
//! `wasm_bindgen` / `web_sys` usage.
//!
//! The `index.html` at the repo root contains a `<canvas id="the_canvas_id">`.
//! `trunk` compiles this crate's `cdylib` target, runs `wasm-bindgen` against
//! the produced wasm module, and the generated JS glue invokes the
//! `#[wasm_bindgen(start)]` function below automatically on load.
//!
//! The eframe 0.32 `WebRunner::start` signature is:
//!
//! ```ignore
//! pub async fn start(
//!     &self,
//!     canvas: web_sys::HtmlCanvasElement,
//!     web_options: crate::WebOptions,
//!     app_creator: epi::AppCreator<'static>,
//! ) -> Result<(), JsValue>
//! ```
//!
//! i.e. it takes an already-resolved `HtmlCanvasElement` (not a string id),
//! so we do the DOM lookup here.

use eframe::web_sys;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::data::state::Dashboard;
use crate::state_url::StateV1;

/// DOM id of the `<canvas>` element that `index.html` ships. Must stay in
/// sync with the `id=...` attribute there.
const CANVAS_ID: &str = "the_canvas_id";

/// Entry point invoked by `wasm-bindgen`'s generated JS shim on module load.
///
/// Returns `Err` only if the canvas lookup fails or `WebRunner::start`
/// returns an error before it hands off to `spawn_local`; panics inside the
/// async body are caught by `console_error_panic_hook`.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    // Route panics to the browser console with a proper stack trace instead
    // of the opaque "unreachable executed" message wasm emits by default.
    console_error_panic_hook::set_once();

    // Route `log::info!` etc. to `console.log`. We deliberately swallow the
    // `Err` from `init_with_level` rather than `expect`-ing it: the only way
    // it fails is if a logger is already installed (e.g. a hot-reload
    // scenario or a future double-call), and that is not worth panicking the
    // whole app for. `let _ = ...` matches the pattern eframe's own examples
    // use.
    let _ = console_log::init_with_level(log::Level::Info);

    let canvas = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(CANVAS_ID))
        .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .ok_or_else(|| {
            JsValue::from_str(&format!(
                "canvas element #{CANVAS_ID} not found or is not an HtmlCanvasElement"
            ))
        })?;

    let web_options = eframe::WebOptions::default();

    // T9t: decode the URL fragment (if any) and pre-apply to the dashboard
    // before handing it to `WebRunner`. `window.location.hash` returns the
    // fragment with a leading `#`; `StateV1::decode` strips it.
    //
    // Known limitation: the `benchmarks/commits.json` fetch is async and
    // has almost certainly not completed by the time we apply the state
    // here, so any `commit` field in the URL will be silently skipped if
    // `git.commits` is still empty.
    // TODO(task-7): re-apply pending commit SHA after git.commits populates.
    let mut dashboard = Dashboard::new();
    if let Some(hash) = web_sys::window().and_then(|w| w.location().hash().ok()) {
        if let Some(state) = StateV1::decode(&hash) {
            dashboard.apply_state_v1(&state);
        }
    }

    wasm_bindgen_futures::spawn_local(async move {
        let start_result = eframe::WebRunner::new()
            .start(canvas, web_options, Box::new(|_cc| Ok(Box::new(dashboard))))
            .await;
        if let Err(err) = start_result {
            log::error!("eframe WebRunner failed to start: {err:?}");
        }
    });

    Ok(())
}

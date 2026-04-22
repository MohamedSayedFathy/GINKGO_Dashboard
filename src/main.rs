// Native entry point. The wasm entry point (a `#[wasm_bindgen(start)]`
// function that calls `eframe::WebRunner`) is W2's deliverable and lives
// in `src/lib.rs` when it exists. This `bin` target still needs a `main`
// symbol to satisfy the linker on `cargo check --target wasm32-unknown-unknown`,
// hence the empty wasm stub below.

#[cfg(not(target_arch = "wasm32"))]
use ginkgo_dashboard_lib::data::state::Dashboard;
#[cfg(not(target_arch = "wasm32"))]
use ginkgo_dashboard_lib::state_url::StateV1;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions::default();

    // --state=<value> CLI flag (T9t). Parsed by hand rather than pulling
    // in `clap` as a default dep — the flag has exactly one shape.
    //
    // The value may be the full fragment (`v1=...`) or the bare base64
    // payload; `StateV1::decode` strips a leading `#` itself, and accepts
    // the `v1=` prefix. Missing/invalid flags fall back to defaults.
    let mut dashboard = Dashboard::new();
    if let Some(raw) = parse_state_flag(std::env::args().skip(1)) {
        if let Some(state) = StateV1::decode(&raw) {
            dashboard.apply_state_v1(&state);
        }
    }

    eframe::run_native(
        "GINKGO Dashboard",
        options,
        Box::new(|_ctx| Ok(Box::new(dashboard))),
    )
}

/// Scan an argv iterator for `--state=<value>` or `--state <value>` and
/// return the value. Returns `None` if absent. Later occurrences win.
#[cfg(not(target_arch = "wasm32"))]
fn parse_state_flag<I>(args: I) -> Option<String>
where
    I: IntoIterator<Item = String>,
{
    let mut iter = args.into_iter();
    let mut found: Option<String> = None;
    while let Some(arg) = iter.next() {
        if let Some(rest) = arg.strip_prefix("--state=") {
            found = Some(rest.to_string());
        } else if arg == "--state" {
            if let Some(next) = iter.next() {
                found = Some(next);
            }
        }
    }
    found
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // Intentionally empty: the real wasm entry point is a
    // `#[wasm_bindgen(start)]` function in the library crate (W2).
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::parse_state_flag;

    #[test]
    fn parse_equals_form() {
        let v = parse_state_flag(["--state=v1=abc".to_string()]);
        assert_eq!(v.as_deref(), Some("v1=abc"));
    }

    #[test]
    fn parse_space_form() {
        let v = parse_state_flag(["--state".to_string(), "v1=abc".to_string()]);
        assert_eq!(v.as_deref(), Some("v1=abc"));
    }

    #[test]
    fn parse_absent() {
        let v = parse_state_flag(["--other".to_string(), "xyz".to_string()]);
        assert_eq!(v, None);
    }
}

//! PDF export (Task 10, native only).
//!
//! We parse the SVG we just emitted with `usvg` and hand the tree to
//! `svg2pdf::to_pdf`. The crate's default features (`image`, `filters`,
//! `text`) would drag in a system font database we don't use — our SVGs
//! reference only the generic `sans-serif` family and contain no raster
//! images — so we depend on it with `default-features = false` in
//! `Cargo.toml` to keep the binary lean.
//!
//! This module is native-only; the sidebar gates the "Save PDF" button
//! behind `#[cfg(not(target_arch = "wasm32"))]`. PDF export on the web
//! would require shipping the same `usvg` tree-parser to wasm plus a
//! browser Save-As flow, which the plan defers to a future task.

use svg2pdf::{usvg, ConversionOptions, PageOptions};

/// Errors surfaced to the UI when PDF export fails.
///
/// We deliberately stringify the underlying crate errors rather than
/// re-export their types: callers only need a user-visible message, and
/// bubbling `usvg::Error` / `svg2pdf::ConversionError` up the module tree
/// would widen the public API for no benefit.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("SVG parsing failed: {0}")]
    Parse(String),
    #[error("PDF conversion failed: {0}")]
    Convert(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Convert the SVG string produced by [`crate::export::svg::SvgCanvas`]
/// into a standalone PDF byte buffer.
///
/// `usvg::Options::default()` does NOT auto-load system fonts — text would
/// parse but render as empty geometry. We populate the font database with
/// the system fonts so the generic `sans-serif` family our SVG references
/// actually resolves to a real face.
pub fn svg_to_pdf(svg_str: &str) -> Result<Vec<u8>, ExportError> {
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();

    let tree =
        usvg::Tree::from_str(svg_str, &options).map_err(|e| ExportError::Parse(e.to_string()))?;

    // `embed_text: true` makes svg2pdf subset and embed the actual glyphs
    // referenced by the SVG. Compared to converting text to vector paths,
    // this keeps the PDF smaller and copy-pasteable in viewers.
    let conv = ConversionOptions {
        embed_text: true,
        ..ConversionOptions::default()
    };

    svg2pdf::to_pdf(&tree, conv, PageOptions::default())
        .map_err(|e| ExportError::Convert(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::svg::{SvgCanvas, SvgSeries};

    #[test]
    fn pdf_from_minimal_svg() {
        let canvas = SvgCanvas {
            series: vec![SvgSeries::Scatter {
                name: "pt".to_string(),
                color: (10, 20, 30),
                points: vec![(0.5, 0.5)],
                radius: 3.0,
            }],
            ..SvgCanvas::default()
        };
        let svg = canvas.render();
        let pdf = svg_to_pdf(&svg).expect("valid minimal SVG converts to PDF");
        assert!(
            pdf.starts_with(b"%PDF-"),
            "PDF magic missing: first 8 bytes = {:?}",
            &pdf[..pdf.len().min(8)]
        );
    }

    #[test]
    fn parse_error_is_not_panic() {
        let err = svg_to_pdf("not svg").unwrap_err();
        // Either the parse or the convert path may fire depending on how
        // permissive `usvg` is — the contract is only that we bubble a
        // structured error, not which variant.
        let msg = err.to_string();
        assert!(!msg.is_empty(), "error must stringify");
    }
}

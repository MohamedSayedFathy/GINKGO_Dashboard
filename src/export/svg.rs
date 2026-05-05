//! Pure-Rust SVG writer (Task 10).
//!
//! Emits a self-contained `<svg>` document by string-concatenation. The
//! output is intentionally deterministic: no `HashMap` iteration, no
//! wall-clock timestamps, and all numeric formatting goes through
//! `format_f64` so the same `SvgCanvas` always hashes to the same bytes.
//!
//! We avoid any rendering crate (resvg, tiny-skia, etc.) here because our
//! plot shapes are geometric primitives that map 1:1 to SVG circles /
//! polylines / rectangles. A few hundred lines of manual writer is simpler
//! to audit than pulling 50k lines of rasterization code at build time.
//!
//! Font handling: only the generic `sans-serif` family is referenced. When
//! a downstream renderer (browser, `svg2pdf`) resolves this it picks a
//! system default, so no glyphs are ever embedded in the document.
//!
//! Known limitation: log-scale axes are linearized here — if the caller
//! flags `log_scale_x`, the series builder walks the data in linear space
//! and the export notes it with `log::warn!`. Full log-axis support (tick
//! spacing, label formatting) is left for a follow-up.

use crate::visualization::plotting::{PlotData, PlotSeries};
use crate::visualization::utils::get_format_color;
use egui::Color32;

/// Inset around the plot area in device pixels.
#[derive(Clone, Copy, Debug)]
pub struct SvgMargin {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Default for SvgMargin {
    fn default() -> Self {
        Self {
            top: 40.0,
            right: 30.0,
            bottom: 60.0,
            left: 70.0,
        }
    }
}

/// Single series primitive. Every plot type we support reduces to one of
/// these — scatter points, polyline connectors, or rectangle bars — so the
/// writer can stay tiny.
#[derive(Clone, Debug)]
pub enum SvgSeries {
    Scatter {
        name: String,
        color: (u8, u8, u8),
        points: Vec<(f64, f64)>,
        radius: f64,
    },
    Line {
        name: String,
        color: (u8, u8, u8),
        points: Vec<(f64, f64)>,
        stroke_width: f64,
        dashed: bool,
    },
    /// Rectangular bars. Each tuple is `(x_center, y_base, height)` in data
    /// coordinates; the writer picks a deterministic bar width based on the
    /// number of bars so stacked layouts align vertically.
    Bar {
        name: String,
        color: (u8, u8, u8),
        bars: Vec<(f64, f64, f64)>,
    },
}

impl SvgSeries {
    fn name(&self) -> &str {
        match self {
            SvgSeries::Scatter { name, .. }
            | SvgSeries::Line { name, .. }
            | SvgSeries::Bar { name, .. } => name,
        }
    }

    fn color(&self) -> (u8, u8, u8) {
        match self {
            SvgSeries::Scatter { color, .. }
            | SvgSeries::Line { color, .. }
            | SvgSeries::Bar { color, .. } => *color,
        }
    }
}

/// Full canvas descriptor. Built by series-builder functions (see
/// [`build_scatter_svg`] and friends) from the same payload the on-screen
/// renderer consumes, then handed to [`SvgCanvas::render`] for the textual
/// output.
#[derive(Clone, Debug)]
pub struct SvgCanvas {
    pub width: f64,
    pub height: f64,
    pub margin: SvgMargin,
    pub title: Option<String>,
    pub x_axis_label: Option<String>,
    pub y_axis_label: Option<String>,
    pub x_range: (f64, f64),
    pub y_range: (f64, f64),
    /// Log10 mapping on the X axis. Required for benchmark data spanning
    /// orders of magnitude (e.g. nnz from 1e3 to 1e7) — linear mapping would
    /// squash 99% of points into a corner.
    pub x_log: bool,
    pub y_log: bool,
    pub series: Vec<SvgSeries>,
}

impl Default for SvgCanvas {
    fn default() -> Self {
        Self {
            width: 900.0,
            height: 600.0,
            margin: SvgMargin::default(),
            title: None,
            x_axis_label: None,
            y_axis_label: None,
            x_range: (0.0, 1.0),
            y_range: (0.0, 1.0),
            x_log: false,
            y_log: false,
            series: Vec::new(),
        }
    }
}

/// Auto-pick log scale: positive data spanning more than 2 orders of
/// magnitude is unreadable on a linear axis. Returns false when any value
/// is non-positive (log10 undefined) or the span is too small.
#[must_use]
pub fn should_auto_log(range: (f64, f64)) -> bool {
    let (lo, hi) = range;
    lo > 0.0 && hi.is_finite() && hi / lo > 100.0
}

/// Convert an egui `Color32` to the `(u8, u8, u8)` triple used by the SVG
/// writer. Alpha is dropped; our palette is fully opaque and the export is
/// flat-shaded anyway.
#[must_use]
pub fn color_to_rgb(c: Color32) -> (u8, u8, u8) {
    (c.r(), c.g(), c.b())
}

/// Pick a numeric format for axis tick labels.
///
/// When the range spans more than three orders of magnitude we switch to
/// scientific notation so labels don't balloon on mixed-scale data (e.g.
/// non-zero counts from 10 to 10^6). Otherwise three-decimal fixed is
/// compact enough while still distinguishing neighbouring ticks.
#[must_use]
pub fn format_axis_value(value: f64, range: (f64, f64)) -> String {
    let (lo, hi) = range;
    let span = (hi - lo).abs();
    let max_abs = lo.abs().max(hi.abs());
    // Guard zero / near-zero ranges: sci-notation on 0..0 is nonsense.
    let use_scientific = if max_abs > 0.0 && span > 0.0 {
        let ratio = max_abs / span.max(f64::MIN_POSITIVE);
        ratio > 1000.0 || max_abs >= 1e4 || (max_abs > 0.0 && max_abs < 1e-2)
    } else {
        false
    };
    if use_scientific {
        format!("{value:.2e}")
    } else {
        format!("{value:.3}")
    }
}

/// Format an `f64` for SVG numeric attributes.
///
/// Keeps three decimals (sub-pixel precision is noise for our plot sizes)
/// and strips trailing zeros / stray decimal points to cut file size.
/// Non-finite values fall back to `0` so a malformed caller can't emit
/// an unparseable document.
fn format_f64(v: f64) -> String {
    if !v.is_finite() {
        return "0".to_string();
    }
    let s = format!("{v:.3}");
    // Strip trailing zeros then the trailing dot if any.
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Escape the XML-special five so arbitrary user-supplied strings (series
/// names, axis labels, titles) can't break the document or inject markup.
fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

impl SvgCanvas {
    /// Plot-area bounds in device pixels.
    fn plot_area(&self) -> (f64, f64, f64, f64) {
        let x0 = self.margin.left;
        let y0 = self.margin.top;
        let x1 = self.width - self.margin.right;
        let y1 = self.height - self.margin.bottom;
        (x0, y0, x1, y1)
    }

    /// Map a data-space point to device pixels. Y is inverted because SVG
    /// origin is top-left. If the caller passed a zero-width range (can
    /// happen for empty series) we fall back to the plot-area midpoint so
    /// the output is visually sensible rather than NaN.
    ///
    /// When `x_log` / `y_log` are set the input data is log10-transformed
    /// before mapping; non-positive points fall back to the axis minimum.
    fn map_point(&self, data_x: f64, data_y: f64) -> (f64, f64) {
        let (x0, y0, x1, y1) = self.plot_area();
        let (xlo, xhi) = self.effective_x_range();
        let (ylo, yhi) = self.effective_y_range();
        let dx = if self.x_log {
            data_x.max(self.x_range.0).log10()
        } else {
            data_x
        };
        let dy = if self.y_log {
            data_y.max(self.y_range.0).log10()
        } else {
            data_y
        };
        let x_span = xhi - xlo;
        let y_span = yhi - ylo;

        let px_x = if x_span.abs() > f64::EPSILON {
            x0 + ((dx - xlo) / x_span) * (x1 - x0)
        } else {
            (x0 + x1) * 0.5
        };
        let px_y = if y_span.abs() > f64::EPSILON {
            y1 - ((dy - ylo) / y_span) * (y1 - y0)
        } else {
            (y0 + y1) * 0.5
        };
        (px_x, px_y)
    }

    /// X range in the mapping space (log10 when `x_log`).
    fn effective_x_range(&self) -> (f64, f64) {
        if self.x_log {
            (
                self.x_range.0.max(f64::MIN_POSITIVE).log10(),
                self.x_range.1.max(f64::MIN_POSITIVE).log10(),
            )
        } else {
            self.x_range
        }
    }

    fn effective_y_range(&self) -> (f64, f64) {
        if self.y_log {
            (
                self.y_range.0.max(f64::MIN_POSITIVE).log10(),
                self.y_range.1.max(f64::MIN_POSITIVE).log10(),
            )
        } else {
            self.y_range
        }
    }

    /// Render this canvas into a self-contained `<svg>` document.
    ///
    /// The output structure is fixed:
    /// 1. `<rect>` background (white).
    /// 2. `<line>` axes plus tick marks + tick labels.
    /// 3. Series in declaration order (caller controls z-order).
    /// 4. Legend in the top-right.
    /// 5. Title (centered, if set).
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.push_header(&mut out);
        self.push_background(&mut out);
        self.push_axes(&mut out);
        self.push_series(&mut out);
        self.push_legend(&mut out);
        self.push_title(&mut out);
        self.push_axis_labels(&mut out);
        out.push_str("</svg>\n");
        out
    }

    fn push_header(&self, out: &mut String) {
        out.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" \
             viewBox=\"0 0 {w} {h}\" font-family=\"sans-serif\">\n",
            w = format_f64(self.width),
            h = format_f64(self.height),
        ));
    }

    fn push_background(&self, out: &mut String) {
        out.push_str(&format!(
            "  <rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" fill=\"white\"/>\n",
            w = format_f64(self.width),
            h = format_f64(self.height),
        ));
    }

    fn push_axes(&self, out: &mut String) {
        let (x0, y0, x1, y1) = self.plot_area();
        // X axis (bottom).
        out.push_str(&format!(
            "  <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"black\" stroke-width=\"1\"/>\n",
            format_f64(x0),
            format_f64(y1),
            format_f64(x1),
            format_f64(y1),
        ));
        // Y axis (left).
        out.push_str(&format!(
            "  <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"black\" stroke-width=\"1\"/>\n",
            format_f64(x0),
            format_f64(y0),
            format_f64(x0),
            format_f64(y1),
        ));

        // Five ticks per axis. On log axes the ticks are evenly spaced in
        // log space (i.e. powers of ten when the span aligns), with labels
        // showing the back-transformed data value.
        const TICKS: usize = 5;
        let (xlo_eff, xhi_eff) = self.effective_x_range();
        let (ylo_eff, yhi_eff) = self.effective_y_range();
        let (xlo_data, _) = self.x_range;
        let (ylo_data, _) = self.y_range;
        for i in 0..TICKS {
            let t = i as f64 / (TICKS - 1) as f64;

            // X ticks.
            let eff_x = xlo_eff + (xhi_eff - xlo_eff) * t;
            let data_x = if self.x_log { 10f64.powf(eff_x) } else { eff_x };
            let (px, py) = self.map_point(data_x, ylo_data);
            out.push_str(&format!(
                "  <line x1=\"{x}\" y1=\"{y}\" x2=\"{x}\" y2=\"{y2}\" stroke=\"black\" stroke-width=\"1\"/>\n",
                x = format_f64(px),
                y = format_f64(py),
                y2 = format_f64(py + 5.0),
            ));
            out.push_str(&format!(
                "  <text x=\"{x}\" y=\"{y}\" font-size=\"10\" text-anchor=\"middle\">{lbl}</text>\n",
                x = format_f64(px),
                y = format_f64(py + 18.0),
                lbl = escape_xml(&format_axis_value(data_x, self.x_range)),
            ));

            // Y ticks.
            let eff_y = ylo_eff + (yhi_eff - ylo_eff) * t;
            let data_y = if self.y_log { 10f64.powf(eff_y) } else { eff_y };
            let (px, py) = self.map_point(xlo_data, data_y);
            out.push_str(&format!(
                "  <line x1=\"{x}\" y1=\"{y}\" x2=\"{x2}\" y2=\"{y}\" stroke=\"black\" stroke-width=\"1\"/>\n",
                x = format_f64(px - 5.0),
                y = format_f64(py),
                x2 = format_f64(px),
            ));
            out.push_str(&format!(
                "  <text x=\"{x}\" y=\"{y}\" font-size=\"10\" text-anchor=\"end\">{lbl}</text>\n",
                x = format_f64(px - 8.0),
                y = format_f64(py + 3.0),
                lbl = escape_xml(&format_axis_value(data_y, self.y_range)),
            ));
        }
    }

    fn push_series(&self, out: &mut String) {
        // Deterministic bar width: bars on the same axis share a single
        // width computed from the X range divided by the visible bar count.
        // Falls back to 1 data-unit if the range degenerates.
        let bar_data_width = self.estimate_bar_width();

        for s in &self.series {
            match s {
                SvgSeries::Scatter {
                    color,
                    points,
                    radius,
                    ..
                } => {
                    for (dx, dy) in points {
                        let (px, py) = self.map_point(*dx, *dy);
                        out.push_str(&format!(
                            "  <circle cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\" fill=\"rgb({cr},{cg},{cb})\"/>\n",
                            cx = format_f64(px),
                            cy = format_f64(py),
                            r = format_f64(*radius),
                            cr = color.0,
                            cg = color.1,
                            cb = color.2,
                        ));
                    }
                }
                SvgSeries::Line {
                    color,
                    points,
                    stroke_width,
                    dashed,
                    ..
                } => {
                    if points.is_empty() {
                        continue;
                    }
                    let mut coords = String::new();
                    for (i, (dx, dy)) in points.iter().enumerate() {
                        let (px, py) = self.map_point(*dx, *dy);
                        if i > 0 {
                            coords.push(' ');
                        }
                        coords.push_str(&format_f64(px));
                        coords.push(',');
                        coords.push_str(&format_f64(py));
                    }
                    let dash_attr = if *dashed {
                        " stroke-dasharray=\"6,4\""
                    } else {
                        ""
                    };
                    out.push_str(&format!(
                        "  <polyline points=\"{pts}\" fill=\"none\" stroke=\"rgb({cr},{cg},{cb})\" stroke-width=\"{sw}\"{d}/>\n",
                        pts = coords,
                        cr = color.0,
                        cg = color.1,
                        cb = color.2,
                        sw = format_f64(*stroke_width),
                        d = dash_attr,
                    ));
                }
                SvgSeries::Bar { color, bars, .. } => {
                    for (xc, ybase, height) in bars {
                        let (px0, py_base) = self.map_point(*xc - bar_data_width * 0.5, *ybase);
                        let (px1, py_top) =
                            self.map_point(*xc + bar_data_width * 0.5, *ybase + *height);
                        let x = px0.min(px1);
                        let y = py_top.min(py_base);
                        let w = (px1 - px0).abs().max(1.0);
                        let h = (py_base - py_top).abs().max(0.0);
                        out.push_str(&format!(
                            "  <rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" fill=\"rgb({cr},{cg},{cb})\"/>\n",
                            x = format_f64(x),
                            y = format_f64(y),
                            w = format_f64(w),
                            h = format_f64(h),
                            cr = color.0,
                            cg = color.1,
                            cb = color.2,
                        ));
                    }
                }
            }
        }
    }

    /// Estimate a shared bar width in DATA coordinates. We take the smallest
    /// distance between distinct bar centres across every Bar series — this
    /// lets stacked bars paint at the same X positions without overlap
    /// regardless of how the caller sorted their input.
    fn estimate_bar_width(&self) -> f64 {
        let mut centres: Vec<f64> = Vec::new();
        for s in &self.series {
            if let SvgSeries::Bar { bars, .. } = s {
                for (xc, _, _) in bars {
                    centres.push(*xc);
                }
            }
        }
        if centres.is_empty() {
            return 1.0;
        }
        centres.sort_by(f64::total_cmp);
        centres.dedup_by(|a, b| (*a - *b).abs() < f64::EPSILON);
        if centres.len() < 2 {
            // Single bar — use 60% of the plot X span to stay visible.
            return (self.x_range.1 - self.x_range.0).abs().max(1.0) * 0.6;
        }
        let mut min_gap = f64::INFINITY;
        for w in centres.windows(2) {
            let gap = (w[1] - w[0]).abs();
            if gap > 0.0 && gap < min_gap {
                min_gap = gap;
            }
        }
        if !min_gap.is_finite() {
            return 1.0;
        }
        // 80% of the smallest gap leaves a small visual break between bars.
        min_gap * 0.8
    }

    fn push_legend(&self, out: &mut String) {
        // Filter out series with empty names (auxiliary / unnamed overlays).
        let named: Vec<&SvgSeries> = self
            .series
            .iter()
            .filter(|s| !s.name().is_empty())
            .collect();
        if named.is_empty() {
            return;
        }
        let (_, _, x1, y0) = self.plot_area();
        let x = x1 - 160.0;
        let mut y = y0 + 12.0;
        out.push_str(&format!(
            "  <rect x=\"{x}\" y=\"{y_rect}\" width=\"155\" height=\"{h}\" \
             fill=\"white\" stroke=\"black\" stroke-width=\"0.5\" opacity=\"0.9\"/>\n",
            x = format_f64(x - 4.0),
            y_rect = format_f64(y - 10.0),
            h = format_f64(named.len() as f64 * 14.0 + 6.0),
        ));
        for s in named {
            let (r, g, b) = s.color();
            out.push_str(&format!(
                "  <rect x=\"{x}\" y=\"{y}\" width=\"10\" height=\"10\" fill=\"rgb({r},{g},{b})\"/>\n",
                x = format_f64(x),
                y = format_f64(y - 8.0),
                r = r,
                g = g,
                b = b,
            ));
            out.push_str(&format!(
                "  <text x=\"{x}\" y=\"{y}\" font-size=\"11\">{name}</text>\n",
                x = format_f64(x + 14.0),
                y = format_f64(y),
                name = escape_xml(s.name()),
            ));
            y += 14.0;
        }
    }

    fn push_title(&self, out: &mut String) {
        if let Some(t) = &self.title {
            out.push_str(&format!(
                "  <text x=\"{x}\" y=\"{y}\" font-size=\"16\" text-anchor=\"middle\" font-weight=\"bold\">{t}</text>\n",
                x = format_f64(self.width * 0.5),
                y = format_f64(self.margin.top - 14.0),
                t = escape_xml(t),
            ));
        }
    }

    fn push_axis_labels(&self, out: &mut String) {
        let (x0, _, x1, y1) = self.plot_area();
        if let Some(lbl) = &self.x_axis_label {
            out.push_str(&format!(
                "  <text x=\"{x}\" y=\"{y}\" font-size=\"12\" text-anchor=\"middle\">{lbl}</text>\n",
                x = format_f64((x0 + x1) * 0.5),
                y = format_f64(y1 + 40.0),
                lbl = escape_xml(lbl),
            ));
        }
        if let Some(lbl) = &self.y_axis_label {
            // Rotated label on the left margin.
            let tx = self.margin.left - 45.0;
            let ty = (self.plot_area().1 + self.plot_area().3) * 0.5;
            out.push_str(&format!(
                "  <text x=\"{x}\" y=\"{y}\" font-size=\"12\" text-anchor=\"middle\" \
                 transform=\"rotate(-90,{x},{y})\">{lbl}</text>\n",
                x = format_f64(tx),
                y = format_f64(ty),
                lbl = escape_xml(lbl),
            ));
        }
    }
}

/// Compute a (min, max) range with a 5% pad on each side, falling back to
/// `(0, 1)` when the input is empty or degenerate.
#[must_use]
pub fn padded_range(values: impl Iterator<Item = f64>) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for v in values {
        if !v.is_finite() {
            continue;
        }
        if v < lo {
            lo = v;
        }
        if v > hi {
            hi = v;
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return (0.0, 1.0);
    }
    if (hi - lo).abs() < f64::EPSILON {
        // Single-valued: pad by +/- 0.5 so the renderer doesn't collapse
        // the axis to zero width.
        return (lo - 0.5, hi + 0.5);
    }
    let pad = (hi - lo) * 0.05;
    let mut padded_lo = lo - pad;
    // If the data is entirely non-negative, don't push the lower bound
    // below zero — nnz/time/storage axes shouldn't start at a negative
    // value that wastes plot area.
    if lo >= 0.0 && padded_lo < 0.0 {
        padded_lo = 0.0;
    }
    (padded_lo, hi + pad)
}

/// Common knobs passed from the UI into every series builder. Keeps the
/// signature stable across plot types even when some builders ignore most
/// of the fields.
#[derive(Clone, Debug, Default)]
pub struct ExportConfig {
    pub title: Option<String>,
    pub x_axis_label: Option<String>,
    pub y_axis_label: Option<String>,
    /// Hint only — the writer linearizes and warns if set.
    pub log_scale_x: bool,
}

/// Build an SVG canvas from a [`PlotData`] emitted by the scatter or
/// performance-profile pipeline. Both plot kinds produce the same
/// `PlotSeries` shape; the `render` flag on each series drives whether it's
/// laid out as points or as a step-line polyline.
#[must_use]
pub fn build_plot_data_svg(
    plot_data: &PlotData,
    config: &ExportConfig,
    is_performance_profile: bool,
) -> SvgCanvas {
    let series = plot_data_to_svg_series(&plot_data.series, is_performance_profile);

    // Axis ranges from the live points — ignores empty series so single-
    // entry legends don't squash the axes to zero.
    let x_values = plot_data
        .series
        .iter()
        .flat_map(|s| s.points.iter().map(|p| p[0]));
    let y_values = plot_data
        .series
        .iter()
        .flat_map(|s| s.points.iter().map(|p| p[1]));

    let x_range = padded_range(x_values);
    let y_range = padded_range(y_values);
    // Auto-log when the user asked for it OR the data span warrants it.
    // Performance profile X is already log-friendly (tau ratios) so respect
    // the on-screen flag verbatim there.
    let x_log = config.log_scale_x || (!is_performance_profile && should_auto_log(x_range));
    let y_log = !is_performance_profile && should_auto_log(y_range);

    SvgCanvas {
        title: config.title.clone(),
        x_axis_label: config.x_axis_label.clone(),
        y_axis_label: config.y_axis_label.clone(),
        x_range,
        y_range,
        x_log,
        y_log,
        series,
        ..SvgCanvas::default()
    }
}

fn plot_data_to_svg_series(series: &[PlotSeries], is_performance_profile: bool) -> Vec<SvgSeries> {
    let mut out: Vec<SvgSeries> = Vec::with_capacity(series.len());
    for s in series {
        let rgb = color_to_rgb(s.color);
        let points: Vec<(f64, f64)> = s.points.iter().map(|p| (p[0], p[1])).collect();
        if is_performance_profile || s.is_auxiliary {
            out.push(SvgSeries::Line {
                name: if s.is_auxiliary {
                    String::new()
                } else {
                    s.name.clone()
                },
                color: rgb,
                points,
                stroke_width: 1.5,
                dashed: s.is_auxiliary,
            });
        } else {
            out.push(SvgSeries::Scatter {
                name: s.name.clone(),
                color: rgb,
                points,
                radius: 3.0,
            });
        }
    }
    out
}

/// Build an SVG canvas from the timeseries payload.
#[must_use]
pub fn build_timeseries_svg(
    data: &crate::visualization::timeseries::TimeseriesData,
    config: &ExportConfig,
) -> SvgCanvas {
    let points: Vec<(f64, f64)> = data.points.iter().map(|p| (p.x, p.y)).collect();
    // Default egui line colour used by the on-screen renderer — keeps the
    // export visually consistent with the dashboard.
    let line_color = (90u8, 150u8, 220u8);

    let mut series = Vec::with_capacity(2);
    if !points.is_empty() {
        series.push(SvgSeries::Line {
            name: String::new(),
            color: line_color,
            points: points.clone(),
            stroke_width: 2.0,
            dashed: false,
        });
        series.push(SvgSeries::Scatter {
            name: "series".to_string(),
            color: line_color,
            points,
            radius: 4.0,
        });
    }

    let x_values = data.points.iter().map(|p| p.x);
    let y_values = data.points.iter().map(|p| p.y);
    let x_range = padded_range(x_values);
    let y_range = padded_range(y_values);
    // X is dataset index (0..n) — never log. Y often spans orders of
    // magnitude (time, gflops) — auto-log when warranted.
    let y_log = should_auto_log(y_range);

    SvgCanvas {
        title: config.title.clone(),
        x_axis_label: config.x_axis_label.clone(),
        y_axis_label: Some(
            config
                .y_axis_label
                .clone()
                .unwrap_or_else(|| data.y_label.clone()),
        ),
        x_range,
        y_range,
        y_log,
        series,
        ..SvgCanvas::default()
    }
}

/// Build an SVG canvas from the stacked-bar payload.
#[must_use]
pub fn build_stacked_bar_svg(
    data: &crate::visualization::stacked_bar::StackedBarData,
    config: &ExportConfig,
) -> SvgCanvas {
    let n = data.problem_labels.len();
    let mut series = Vec::with_capacity(data.series.len());
    // Running sums track the stack floor for each column; walk series in
    // declaration order so segments sit directly on top of the previous
    // layer — matches the on-screen `stack_on` behaviour.
    let mut running: Vec<f64> = vec![0.0; n];
    for s in &data.series {
        let fmt_color = color_to_rgb(get_format_color(&s.format));
        let mut bars = Vec::with_capacity(s.values.len());
        for (i, &v) in s.values.iter().enumerate() {
            if i >= running.len() {
                break;
            }
            let base = running[i];
            let height = v.max(0.0);
            bars.push((i as f64, base, height));
            running[i] = base + height;
        }
        series.push(SvgSeries::Bar {
            name: format!("{:?}", s.format),
            color: fmt_color,
            bars,
        });
    }

    let x_range = if n == 0 {
        (0.0, 1.0)
    } else {
        (-0.5, (n as f64) - 0.5)
    };
    let y_max = data
        .totals
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let y_range = if y_max.is_finite() && y_max > 0.0 {
        (0.0, y_max * 1.05)
    } else {
        (0.0, 1.0)
    };

    // Stacked-bar Y stays linear: visual segment lengths must add up to
    // the total, which only works in linear space. The user can switch to
    // line timeseries (which auto-logs) if they need orders-of-magnitude
    // separation.
    SvgCanvas {
        title: config.title.clone(),
        x_axis_label: config.x_axis_label.clone(),
        y_axis_label: config.y_axis_label.clone(),
        x_range,
        y_range,
        series,
        ..SvgCanvas::default()
    }
}

/// Render a side-by-side A/B comparison into a single SVG string.
///
/// The two panes don't share axes — comparison benchmarks can live on
/// wildly different Y scales — so rather than reproject both into one
/// canvas we render each pane independently via [`SvgCanvas::render`],
/// strip its outer `<svg>` wrapper, and wrap both in a parent document
/// with `<g transform="translate(…)">` offsets. One file in, one PDF
/// page out, no data-space distortion.
#[must_use]
pub fn render_comparison_pair(
    data: &crate::visualization::compare::ComparisonPlotData,
    config: &ExportConfig,
) -> String {
    let cfg_a = ExportConfig {
        title: Some(
            config
                .title
                .as_ref()
                .map_or_else(|| "A".to_string(), |t| format!("{t} (A)")),
        ),
        x_axis_label: config.x_axis_label.clone(),
        y_axis_label: config.y_axis_label.clone(),
        log_scale_x: config.log_scale_x,
    };
    let cfg_b = ExportConfig {
        title: Some(
            config
                .title
                .as_ref()
                .map_or_else(|| "B".to_string(), |t| format!("{t} (B)")),
        ),
        ..cfg_a.clone()
    };
    let pane_a = build_plot_data_svg(&data.pane_a, &cfg_a, false);
    let pane_b = build_plot_data_svg(&data.pane_b, &cfg_b, false);

    let a_svg = pane_a.render();
    let b_svg = pane_b.render();

    let total_w = pane_a.width + pane_b.width;
    let height = pane_a.height.max(pane_b.height);

    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" \
         viewBox=\"0 0 {w} {h}\" font-family=\"sans-serif\">\n",
        w = format_f64(total_w),
        h = format_f64(height),
    ));
    out.push_str(&format!(
        "  <rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" fill=\"white\"/>\n",
        w = format_f64(total_w),
        h = format_f64(height),
    ));
    // Inline each pane minus its own `<svg …>` wrapper.
    out.push_str(&format!(
        "  <g transform=\"translate(0,0)\">\n{}\n  </g>\n",
        strip_svg_wrapper(&a_svg)
    ));
    out.push_str(&format!(
        "  <g transform=\"translate({ox},0)\">\n{}\n  </g>\n",
        strip_svg_wrapper(&b_svg),
        ox = format_f64(pane_a.width),
    ));
    out.push_str("</svg>\n");
    out
}

/// Strip the leading `<svg …>` open tag and the trailing `</svg>` close so
/// the inner content can be nested inside a parent SVG document. Returns
/// the input unchanged if the wrappers are not found — callers won't hit
/// this path because [`SvgCanvas::render`] always emits them, but it keeps
/// the function total.
fn strip_svg_wrapper(s: &str) -> String {
    // Find end of opening <svg …>.
    let Some(open_end) = s.find('>') else {
        return s.to_string();
    };
    // Guard: ensure the tag we found actually is `<svg`.
    if !s[..open_end].trim_start().starts_with("<svg") {
        return s.to_string();
    }
    let rest = &s[open_end + 1..];
    let Some(close_start) = rest.rfind("</svg>") else {
        return rest.to_string();
    };
    rest[..close_start].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_canvas() -> SvgCanvas {
        SvgCanvas {
            width: 400.0,
            height: 300.0,
            margin: SvgMargin::default(),
            title: Some("Demo".to_string()),
            x_axis_label: Some("x".to_string()),
            y_axis_label: Some("y".to_string()),
            x_range: (0.0, 10.0),
            y_range: (0.0, 100.0),
            x_log: false,
            y_log: false,
            series: vec![
                SvgSeries::Scatter {
                    name: "alpha".to_string(),
                    color: (255, 0, 0),
                    points: vec![(1.0, 10.0), (2.0, 20.0)],
                    radius: 3.0,
                },
                SvgSeries::Line {
                    name: "beta".to_string(),
                    color: (0, 128, 0),
                    points: vec![(0.0, 0.0), (5.0, 50.0), (10.0, 100.0)],
                    stroke_width: 1.5,
                    dashed: false,
                },
            ],
        }
    }

    #[test]
    fn svg_is_well_formed() {
        let svg = sample_canvas().render();
        assert!(svg.starts_with("<svg"), "starts with <svg:\n{svg}");
        assert!(svg.trim_end().ends_with("</svg>"), "ends with </svg>");

        // Every attribute-opening `"` must have a matching closing `"` on
        // the same line — catches a truncated-attribute bug immediately.
        for (i, line) in svg.lines().enumerate() {
            let quote_count = line.chars().filter(|c| *c == '"').count();
            assert_eq!(quote_count % 2, 0, "line {i} has unbalanced quotes: {line}");
        }

        // Each series name we put in must appear literally.
        assert!(svg.contains("alpha"));
        assert!(svg.contains("beta"));
    }

    #[test]
    fn deterministic_output() {
        let a = sample_canvas().render();
        let b = sample_canvas().render();
        assert_eq!(a, b, "identical input must yield byte-identical SVG");
    }

    #[test]
    fn empty_canvas_renders() {
        let canvas = SvgCanvas {
            series: Vec::new(),
            title: Some("Empty".to_string()),
            ..SvgCanvas::default()
        };
        let svg = canvas.render();
        assert!(svg.starts_with("<svg"));
        assert!(svg.trim_end().ends_with("</svg>"));
        assert!(svg.contains("Empty"));
    }

    #[test]
    fn coordinate_mapping_round_trip() {
        let c = SvgCanvas {
            width: 200.0,
            height: 100.0,
            margin: SvgMargin {
                top: 10.0,
                right: 10.0,
                bottom: 10.0,
                left: 10.0,
            },
            x_range: (0.0, 10.0),
            y_range: (0.0, 10.0),
            ..SvgCanvas::default()
        };
        // Lower-left of the data space maps to (margin.left, height - margin.bottom).
        let (px, py) = c.map_point(0.0, 0.0);
        assert!((px - 10.0).abs() < 1e-6, "px={px}");
        assert!((py - 90.0).abs() < 1e-6, "py={py}");

        // Upper-right maps to (width - margin.right, margin.top).
        let (px, py) = c.map_point(10.0, 10.0);
        assert!((px - 190.0).abs() < 1e-6, "px={px}");
        assert!((py - 10.0).abs() < 1e-6, "py={py}");

        // Midpoint of the data space maps to the middle of the plot area.
        let (px, py) = c.map_point(5.0, 5.0);
        assert!((px - 100.0).abs() < 1e-6, "px={px}");
        assert!((py - 50.0).abs() < 1e-6, "py={py}");
    }

    #[test]
    fn format_axis_value_picks_scientific_notation() {
        // Range spanning six orders of magnitude should go scientific.
        let s = format_axis_value(1_000.0, (1.0, 1_000_000.0));
        assert!(s.contains('e'), "expected scientific for wide range: {s}");

        // Narrow range uses fixed.
        let s = format_axis_value(5.0, (0.0, 10.0));
        assert!(!s.contains('e'), "expected fixed for narrow range: {s}");
    }

    #[test]
    fn format_f64_trims_trailing_zeros() {
        assert_eq!(format_f64(1.0), "1");
        assert_eq!(format_f64(1.500), "1.5");
        assert_eq!(format_f64(0.0), "0");
        assert_eq!(format_f64(f64::NAN), "0");
        assert_eq!(format_f64(f64::INFINITY), "0");
    }

    #[test]
    fn escape_xml_covers_specials() {
        assert_eq!(escape_xml("a&b<c>\"'"), "a&amp;b&lt;c&gt;&quot;&apos;");
        assert_eq!(escape_xml("plain"), "plain");
    }

    #[test]
    fn padded_range_handles_empty() {
        let (lo, hi) = padded_range(std::iter::empty());
        assert_eq!((lo, hi), (0.0, 1.0));
    }

    #[test]
    fn padded_range_single_value_pads() {
        let (lo, hi) = padded_range([4.2].into_iter());
        assert!(lo < 4.2 && hi > 4.2);
    }

    #[test]
    fn strip_svg_wrapper_removes_outer_tags() {
        let svg = "<svg xmlns=\"x\"><rect/></svg>";
        let stripped = strip_svg_wrapper(svg);
        assert_eq!(stripped, "<rect/>");
    }
}

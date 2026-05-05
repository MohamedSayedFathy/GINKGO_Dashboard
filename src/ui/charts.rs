use crate::data::state::{CompareSide, Dashboard};
use crate::types::{
    AggregationKind, DataFormat, DataMode, MetricType, PlotType, ProfileFilter, XaxisType,
};
use crate::visualization::compare::{generate_comparison_data, ComparisonPlotData, DiffRow};
use crate::visualization::plotting::{generate_plot_data, PlotData};
use crate::visualization::stacked_bar::{build_stacked_bar, StackedBarData};
use crate::visualization::timeseries::{build_timeseries, TimeseriesData};
use crate::visualization::tooltip::generate_tooltip_text;
use crate::visualization::utils::format_metric_suffix;
use eframe::egui::{self, Ui};
use egui_plot::{Legend, Plot};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// A `ProfileFilter` variant whose `f64` payloads are stored as raw bits so
/// the whole enum is `Hash` + `Eq` without any manual implementation.
#[derive(Hash, PartialEq, Eq)]
enum CacheKeyProfileFilter {
    None,
    MaxTau(u64),
    TrimPercent(u64),
}

impl From<ProfileFilter> for CacheKeyProfileFilter {
    fn from(f: ProfileFilter) -> Self {
        match f {
            ProfileFilter::None => Self::None,
            ProfileFilter::MaxTau(v) => Self::MaxTau(v.to_bits()),
            ProfileFilter::TrimPercent(v) => Self::TrimPercent(v.to_bits()),
        }
    }
}

/// All fields that influence plot output, hashable via `derive(Hash)`.
///
/// `active_formats` is stored as a sorted `Vec` so that `Hash` is order-stable.
#[derive(Hash)]
struct CacheKey {
    mode: Option<DataMode>,
    active_dataset: Vec<String>,
    active_formats: Vec<(DataFormat, bool)>,
    x_axis: Option<XaxisType>,
    data_metric: Option<MetricType>,
    baseline_format: DataFormat,
    plot_type: PlotType,
    normalize: bool,
    profile_filter: CacheKeyProfileFilter,
    filter_outliers: bool,
    log_scale_x: bool,
    show_percentile_bands: bool,
    /// Source text of the active custom formula, or `None` when not in Custom mode.
    custom_formula_src: Option<String>,
}

/// Hash key for the Line-Timeseries cache (Task 6).
///
/// `ordered_keys` is hashed bit-for-bit so changing the commit-ordering
/// source (git repo vs. alphabetical) invalidates the cache cleanly.
#[derive(Hash)]
struct TimeseriesCacheKey {
    ordered_keys: Vec<String>,
    aggregation: AggregationKind,
    format: DataFormat,
    data_metric: Option<MetricType>,
    custom_formula_src: Option<String>,
    problem_filter: Option<String>,
}

/// Hash key for the Stacked-Bar cache (Task 6).
#[derive(Hash)]
struct StackedBarCacheKey {
    dataset_key: Option<String>,
    active_formats: Vec<(DataFormat, bool)>,
    data_metric: Option<MetricType>,
    custom_formula_src: Option<String>,
    sort_by_total: bool,
    top_n: usize,
}

/// Hash key for the Comparison cache. Mirrors [`CacheKey`] but keyed on the
/// A/B commit selections plus comparison-specific knobs (baseline side,
/// orientation, threshold). Diff threshold is hashed bit-exact via
/// `f64::to_bits` — matches the pattern used for `ProfileFilter` above.
#[derive(Hash)]
struct CompareCacheKey {
    commit_a: Option<String>,
    commit_b: Option<String>,
    active_formats: Vec<(DataFormat, bool)>,
    x_axis: Option<XaxisType>,
    data_metric: Option<MetricType>,
    baseline_format: DataFormat,
    normalize: bool,
    filter_outliers: bool,
    custom_formula_src: Option<String>,
    baseline_side: CompareSide,
    lower_is_better: bool,
    diff_threshold_bits: u64,
}

pub fn render_charts(app: &mut Dashboard, ui: &mut Ui) {
    match app.plot_config.plot_type {
        PlotType::Comparison => {
            render_comparison_view(app, ui);
            return;
        }
        PlotType::LineTimeseries => {
            render_line_timeseries_view(app, ui);
            return;
        }
        PlotType::StackedBar => {
            render_stacked_bar_view(app, ui);
            return;
        }
        PlotType::Scatter | PlotType::PerformanceProfile => {}
    }

    // Build sorted formats vec for stable hashing.
    let mut sorted_formats: Vec<(DataFormat, bool)> = app
        .data_selection
        .active_formats
        .iter()
        .map(|(&k, &v)| (k, v))
        .collect();
    sorted_formats.sort_by_key(|(fmt, _)| *fmt);

    let key = CacheKey {
        mode: app.data_selection.mode,
        active_dataset: app.data_selection.active_dataset.clone(),
        active_formats: sorted_formats,
        x_axis: app.plot_config.x_axis,
        data_metric: app.plot_config.data_metric,
        baseline_format: app.plot_config.baseline_format,
        plot_type: app.plot_config.plot_type,
        normalize: app.plot_config.normalize,
        profile_filter: CacheKeyProfileFilter::from(app.plot_config.profile_filter),
        filter_outliers: app.plot_config.filter_outliers,
        log_scale_x: app.plot_config.log_scale_x,
        show_percentile_bands: app.plot_config.show_percentile_bands,
        custom_formula_src: app
            .plot_config
            .custom_formula
            .as_ref()
            .map(|f| f.source.clone()),
    };

    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let cache_key: u64 = hasher.finish();

    let formula = app.plot_config.custom_formula.as_ref();

    let plot_data: Rc<_> = if app.plot_cache.key != cache_key {
        let data = generate_plot_data(
            &app.data_selection.dataset,
            &app.data_selection.active_dataset,
            &app.data_selection.active_formats,
            app.data_selection.mode,
            app.plot_config.x_axis,
            app.plot_config.data_metric,
            app.plot_config.baseline_format,
            app.plot_config.normalize,
            app.plot_config.filter_outliers,
            app.plot_config.plot_type,
            app.plot_config.profile_filter,
            app.plot_config.log_scale_x,
            app.plot_config.show_percentile_bands,
            formula,
        );

        let rc_data = Rc::new(data);
        app.plot_cache.key = cache_key;
        app.plot_cache.data = Some(Rc::clone(&rc_data));
        rc_data
    } else {
        match app.plot_cache.data.as_ref() {
            Some(data) => Rc::clone(data),
            None => {
                log::warn!("Cache miss despite matching key - regenerating");
                let data = generate_plot_data(
                    &app.data_selection.dataset,
                    &app.data_selection.active_dataset,
                    &app.data_selection.active_formats,
                    app.data_selection.mode,
                    app.plot_config.x_axis,
                    app.plot_config.data_metric,
                    app.plot_config.baseline_format,
                    app.plot_config.normalize,
                    app.plot_config.filter_outliers,
                    app.plot_config.plot_type,
                    app.plot_config.profile_filter,
                    app.plot_config.log_scale_x,
                    app.plot_config.show_percentile_bands,
                    formula,
                );
                Rc::new(data)
            }
        }
    };

    let plot_data_for_closure = Rc::clone(&plot_data);

    let plot_id = format!(
        "Benchmark Plot - {:?} - {:?} - {:?}",
        app.plot_config.x_axis, app.plot_config.data_metric, app.plot_config.plot_type
    );

    let current_plot_type = app.plot_config.plot_type;

    let mut plot = Plot::new(plot_id)
        .legend(Legend::default())
        .show_axes([true, true]);

    // Outlier filtering is now handled in data generation (cached)
    let filtered_series = &plot_data.series;

    // Auto-log axis decision for the Scatter view. Benchmark fields like
    // nnz, storage, time, sparsity routinely span 4+ orders of magnitude;
    // a linear axis squashes 90% of points into the leftmost 10% of the
    // plot. We detect a positive, > 100x span on either axis and apply a
    // log10 transform inline (egui_plot doesn't expose a native log axis).
    // The on-screen behaviour matches the SVG export's `should_auto_log`.
    let is_scatter = matches!(current_plot_type, PlotType::Scatter);
    let (auto_log_x, auto_log_y) = if is_scatter {
        let mut x_min = f64::INFINITY;
        let mut x_max = f64::NEG_INFINITY;
        let mut y_min = f64::INFINITY;
        let mut y_max = f64::NEG_INFINITY;
        for s in filtered_series.iter() {
            if s.is_auxiliary {
                continue;
            }
            for p in &s.points {
                let (x, y) = (p[0], p[1]);
                if x.is_finite() {
                    x_min = x_min.min(x);
                    x_max = x_max.max(x);
                }
                if y.is_finite() {
                    y_min = y_min.min(y);
                    y_max = y_max.max(y);
                }
            }
        }
        let needs_log =
            |lo: f64, hi: f64| -> bool { lo > 0.0 && hi.is_finite() && hi / lo > 100.0 };
        (needs_log(x_min, x_max), needs_log(y_min, y_max))
    } else {
        (false, false)
    };

    let use_log_scale =
        app.plot_config.log_scale_x && current_plot_type == PlotType::PerformanceProfile;

    if current_plot_type == PlotType::PerformanceProfile {
        plot = plot
            .x_axis_label("Performance Ratio (tau)")
            .y_axis_label("Probability (rho)")
            .x_grid_spacer(egui_plot::log_grid_spacer(10)); // Log scale grid
    }

    if auto_log_x {
        plot = plot.x_grid_spacer(egui_plot::log_grid_spacer(10));
    }
    if auto_log_y {
        plot = plot.y_grid_spacer(egui_plot::log_grid_spacer(10));
    }

    let plot = plot
        .x_axis_formatter(move |mark, _range| {
            let x = mark.value;
            if current_plot_type == PlotType::PerformanceProfile {
                if use_log_scale {
                    return format!("{:.1}", 10.0_f64.powf(x));
                }
                return format!("{:.1}", x);
            }
            if auto_log_x {
                return format_metric_suffix(10.0_f64.powf(x));
            }
            format_metric_suffix(x)
        })
        .y_axis_formatter(move |mark, _range| {
            if auto_log_y {
                format_metric_suffix(10.0_f64.powf(mark.value))
            } else {
                format_metric_suffix(mark.value)
            }
        })
        .label_formatter(move |name, value| {
            if let Some(series) = plot_data_for_closure.series.iter().find(|s| s.name == name) {
                // Transform the cursor's plot-space (potentially log) value
                // back to data space before searching the series points.
                let data_x = if auto_log_x {
                    10.0_f64.powf(value.x)
                } else {
                    value.x
                };
                let data_y = if auto_log_y {
                    10.0_f64.powf(value.y)
                } else {
                    value.y
                };
                let point = egui_plot::PlotPoint::new(data_x, data_y);
                let series_plot_points: Vec<egui_plot::PlotPoint> = series
                    .points
                    .iter()
                    .map(|p| egui_plot::PlotPoint::new(p[0], p[1]))
                    .collect();

                return generate_tooltip_text(
                    name,
                    &point,
                    &series_plot_points,
                    &series.metadata,
                    current_plot_type,
                );
            }
            format!("{}\nX: {:.2}\nY: {:.2}", name, value.x, value.y)
        });

    let is_performance_profile = app.plot_config.plot_type == PlotType::PerformanceProfile;

    // Helper: log10 a positive-only point list, dropping non-positive values
    // because log10 is undefined there (and the plot would show garbage).
    let transform_points = |raw: &[[f64; 2]]| -> Vec<[f64; 2]> {
        if !auto_log_x && !auto_log_y {
            return raw.to_vec();
        }
        raw.iter()
            .filter_map(|p| {
                let x = if auto_log_x {
                    if p[0] > 0.0 {
                        p[0].log10()
                    } else {
                        return None;
                    }
                } else {
                    p[0]
                };
                let y = if auto_log_y {
                    if p[1] > 0.0 {
                        p[1].log10()
                    } else {
                        return None;
                    }
                } else {
                    p[1]
                };
                Some([x, y])
            })
            .collect()
    };

    plot.show(ui, |plot_ui| {
        for series in filtered_series {
            let pts = transform_points(&series.points);
            if series.is_auxiliary {
                // Auxiliary series (like percentile bands) are always rendered as lines
                let line =
                    egui_plot::Line::new(series.name.clone(), egui_plot::PlotPoints::from(pts))
                        .color(series.color)
                        .style(egui_plot::LineStyle::Dashed { length: 10.0 });
                plot_ui.line(line);
                continue;
            }

            // Only draw lines for Performance Profile mode
            if is_performance_profile {
                let line =
                    egui_plot::Line::new(series.name.clone(), egui_plot::PlotPoints::from(pts))
                        .color(series.color)
                        .width(1.0);
                plot_ui.line(line);
            } else {
                let points =
                    egui_plot::Points::new(series.name.clone(), egui_plot::PlotPoints::from(pts))
                        .shape(series.marker)
                        .color(series.color)
                        .radius(3.0);
                plot_ui.points(points);
            }
        }
    });
}

/// Render the A-vs-B comparison: side-by-side scatter panes (top) plus a
/// diff table + histogram row (bottom). Handles the pre-load gate (both
/// A and B must be loaded and picked in the sidebar) and the cache.
fn render_comparison_view(app: &mut Dashboard, ui: &mut Ui) {
    let a_key = app.comparison.commit_a.clone();
    let b_key = app.comparison.commit_b.clone();
    let (a_key, b_key) = match (a_key, b_key) {
        (Some(a), Some(b)) if a != b => (a, b),
        _ => {
            ui.label(
                egui::RichText::new(
                    "Pick two different datasets in the sidebar to compare (A and B).",
                )
                .color(egui::Color32::from_rgb(180, 140, 40)),
            );
            return;
        }
    };

    let ds_a = match app.data_selection.dataset.get(&a_key) {
        Some(d) => d.clone(),
        None => {
            ui.colored_label(
                egui::Color32::RED,
                format!("Dataset A '{a_key}' not loaded."),
            );
            return;
        }
    };
    let ds_b = match app.data_selection.dataset.get(&b_key) {
        Some(d) => d.clone(),
        None => {
            ui.colored_label(
                egui::Color32::RED,
                format!("Dataset B '{b_key}' not loaded."),
            );
            return;
        }
    };

    // Cache key.
    let mut sorted_formats: Vec<(DataFormat, bool)> = app
        .data_selection
        .active_formats
        .iter()
        .map(|(&k, &v)| (k, v))
        .collect();
    sorted_formats.sort_by_key(|(fmt, _)| *fmt);

    let key = CompareCacheKey {
        commit_a: Some(a_key.clone()),
        commit_b: Some(b_key.clone()),
        active_formats: sorted_formats,
        x_axis: app.plot_config.x_axis,
        data_metric: app.plot_config.data_metric,
        baseline_format: app.plot_config.baseline_format,
        normalize: app.plot_config.normalize,
        filter_outliers: app.plot_config.filter_outliers,
        custom_formula_src: app
            .plot_config
            .custom_formula
            .as_ref()
            .map(|f| f.source.clone()),
        baseline_side: app.comparison.baseline_side,
        lower_is_better: app.comparison.lower_is_better,
        diff_threshold_bits: app.comparison.diff_threshold.to_bits(),
    };
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let cache_key: u64 = hasher.finish();

    let formula = app.plot_config.custom_formula.as_ref();

    let data: Rc<ComparisonPlotData> = if app.comparison_cache.key == cache_key {
        match app.comparison_cache.data.as_ref() {
            Some(d) => Rc::clone(d),
            None => {
                let generated = generate_comparison_data(
                    &ds_a,
                    &ds_b,
                    &app.data_selection.active_formats,
                    app.plot_config.x_axis,
                    app.plot_config.data_metric,
                    app.plot_config.baseline_format,
                    app.plot_config.normalize,
                    app.plot_config.filter_outliers,
                    formula,
                    app.comparison.baseline_side,
                    app.comparison.lower_is_better,
                    app.comparison.diff_threshold,
                    &a_key,
                    &b_key,
                );
                Rc::new(generated)
            }
        }
    } else {
        let generated = generate_comparison_data(
            &ds_a,
            &ds_b,
            &app.data_selection.active_formats,
            app.plot_config.x_axis,
            app.plot_config.data_metric,
            app.plot_config.baseline_format,
            app.plot_config.normalize,
            app.plot_config.filter_outliers,
            formula,
            app.comparison.baseline_side,
            app.comparison.lower_is_better,
            app.comparison.diff_threshold,
            &a_key,
            &b_key,
        );
        let rc = Rc::new(generated);
        app.comparison_cache.key = cache_key;
        app.comparison_cache.data = Some(Rc::clone(&rc));
        rc
    };

    // Compute shared bounds up front so we can pass them to both panes.
    let (x_min, x_max) = series_bounds(&data.pane_a, &data.pane_b, |p| p[0]);
    let (y_min, y_max) = if app.comparison.shared_y_range {
        series_bounds(&data.pane_a, &data.pane_b, |p| p[1])
    } else {
        (f64::NAN, f64::NAN)
    };

    let shared_y = app.comparison.shared_y_range;

    let available = ui.available_size();
    let pane_height = (available.y * 0.55).max(220.0);
    let lower_height = (available.y * 0.40).max(180.0);

    ui.allocate_ui_with_layout(
        egui::Vec2::new(available.x, pane_height),
        egui::Layout::left_to_right(egui::Align::TOP),
        |ui| {
            let pane_width = (available.x - 8.0) * 0.5;
            ui.allocate_ui(egui::Vec2::new(pane_width, pane_height), |ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(format!("A: {a_key}")).strong());
                    render_compare_pane(
                        ui,
                        "compare_pane_a",
                        &data.pane_a,
                        (x_min, x_max),
                        (y_min, y_max),
                        shared_y,
                    );
                });
            });
            ui.allocate_ui(egui::Vec2::new(pane_width, pane_height), |ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(format!("B: {b_key}")).strong());
                    render_compare_pane(
                        ui,
                        "compare_pane_b",
                        &data.pane_b,
                        (x_min, x_max),
                        (y_min, y_max),
                        shared_y,
                    );
                });
            });
        },
    );

    ui.separator();

    ui.allocate_ui_with_layout(
        egui::Vec2::new(available.x, lower_height),
        egui::Layout::left_to_right(egui::Align::TOP),
        |ui| {
            let table_w = (available.x - 8.0) * 0.6;
            let hist_w = (available.x - 8.0) * 0.4;
            ui.allocate_ui(egui::Vec2::new(table_w, lower_height), |ui| {
                render_diff_table(ui, &data.diff_rows);
            });
            ui.allocate_ui(egui::Vec2::new(hist_w, lower_height), |ui| {
                render_histogram(ui, data.as_ref());
            });
        },
    );
}

/// Union bounds of both panes projected via `project`; returns (NaN, NaN)
/// when no finite data is present.
fn series_bounds(a: &PlotData, b: &PlotData, project: fn(&[f64; 2]) -> f64) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for pd in [a, b] {
        for s in &pd.series {
            if s.is_auxiliary {
                continue;
            }
            for p in &s.points {
                let v = project(p);
                if v.is_finite() {
                    if v < lo {
                        lo = v;
                    }
                    if v > hi {
                        hi = v;
                    }
                }
            }
        }
    }
    if lo.is_finite() && hi.is_finite() && lo <= hi {
        (lo, hi)
    } else {
        (f64::NAN, f64::NAN)
    }
}

fn render_compare_pane(
    ui: &mut Ui,
    plot_id: &'static str,
    pane: &PlotData,
    xr: (f64, f64),
    yr: (f64, f64),
    shared_y: bool,
) {
    // Link X axes across both panes via a shared group id; Y is opt-in.
    let link_group = ui.id().with("compare_link_group");
    let link_vec = egui::Vec2b {
        x: true,
        y: shared_y,
    };

    let mut plot = Plot::new(plot_id)
        .legend(Legend::default())
        .show_axes([true, true])
        .link_axis(link_group, link_vec)
        .x_axis_formatter(|mark, _range| format_metric_suffix(mark.value))
        .y_axis_formatter(|mark, _range| format_metric_suffix(mark.value));

    // Default bounds encourage auto-fit on first draw but honor our union.
    if xr.0.is_finite() && xr.1.is_finite() && xr.0 < xr.1 {
        plot = plot.default_x_bounds(xr.0, xr.1);
    }
    if shared_y && yr.0.is_finite() && yr.1.is_finite() && yr.0 < yr.1 {
        plot = plot.default_y_bounds(yr.0, yr.1);
    }

    plot.show(ui, |plot_ui| {
        for s in &pane.series {
            if s.is_auxiliary {
                continue;
            }
            let points = egui_plot::Points::new(
                s.name.clone(),
                egui_plot::PlotPoints::from(s.points.clone()),
            )
            .shape(s.marker)
            .color(s.color)
            .radius(3.0);
            plot_ui.points(points);
        }
    });
}

/// Render the diff table as an `egui::Grid` inside a bounded scroll area.
/// Renders at most `MAX_ROWS` — anything beyond that gets summarised in a
/// footer row so the table never becomes a UI thread hog on 10k-problem
/// datasets.
fn render_diff_table(ui: &mut Ui, rows: &[DiffRow]) {
    const MAX_ROWS: usize = 500;

    ui.label(egui::RichText::new("Diff Table").strong());
    egui::ScrollArea::vertical()
        .max_height(ui.available_height() - 8.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("compare_diff_table")
                .striped(true)
                .num_columns(6)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Problem").strong());
                    ui.label(egui::RichText::new("Format").strong());
                    ui.label(egui::RichText::new("A").strong());
                    ui.label(egui::RichText::new("B").strong());
                    ui.label(egui::RichText::new("Δ").strong());
                    ui.label(egui::RichText::new("Ratio").strong());
                    ui.end_row();

                    for row in rows.iter().take(MAX_ROWS) {
                        ui.label(row.problem_name.as_str());
                        ui.label(format!("{:?}", row.format));
                        ui.label(fmt_cell(row.value_a));
                        ui.label(fmt_cell(row.value_b));
                        ui.label(fmt_cell(row.delta));
                        let ratio_text = if row.ratio.is_finite() {
                            format!("{:.3}x", row.ratio)
                        } else {
                            "—".to_string()
                        };
                        let color = if !row.ratio.is_finite() {
                            egui::Color32::GRAY
                        } else if row.ratio > 1.01 {
                            egui::Color32::from_rgb(80, 180, 80)
                        } else if row.ratio < 0.99 {
                            egui::Color32::from_rgb(220, 100, 100)
                        } else {
                            ui.visuals().text_color()
                        };
                        ui.colored_label(color, ratio_text);
                        ui.end_row();
                    }

                    if rows.len() > MAX_ROWS {
                        let extra = rows.len() - MAX_ROWS;
                        ui.label(
                            egui::RichText::new(format!("... +{extra} more"))
                                .italics()
                                .color(egui::Color32::GRAY),
                        );
                        ui.end_row();
                    }
                });
        });
}

fn fmt_cell(v: f64) -> String {
    if v.is_nan() {
        "—".to_string()
    } else {
        format_metric_suffix(v)
    }
}

/// Render the log2 speedup histogram + summary lines.
fn render_histogram(ui: &mut Ui, data: &ComparisonPlotData) {
    ui.label(egui::RichText::new("Speedup distribution").strong());

    let bars: Vec<egui_plot::Bar> = data
        .histogram
        .bin_edges_log2
        .windows(2)
        .zip(data.histogram.bin_counts.iter())
        .map(|(edges, &count)| {
            let center = (edges[0] + edges[1]) * 0.5;
            let width = edges[1] - edges[0];
            egui_plot::Bar::new(center, count as f64).width(width)
        })
        .collect();

    let chart =
        egui_plot::BarChart::new("speedup_hist", bars).color(egui::Color32::from_rgb(90, 150, 220));

    Plot::new("compare_hist")
        .legend(Legend::default())
        .show_axes([true, true])
        .height(ui.available_height() * 0.6)
        .x_axis_formatter(|mark, _range| {
            let ratio = 2.0_f64.powf(mark.value);
            if !(0.01..100.0).contains(&ratio) {
                format!("{ratio:.1e}x")
            } else {
                format!("{ratio:.2}x")
            }
        })
        .show(ui, |plot_ui| {
            plot_ui.bar_chart(chart);
        });

    ui.add_space(4.0);
    let s = &data.summary;
    let gm = if s.geometric_mean_speedup.is_finite() {
        format!("{:.3}x", s.geometric_mean_speedup)
    } else {
        "—".to_string()
    };
    ui.label(format!("geomean: {gm}"));
    ui.label(format!(
        "↑ {} improvements   ↓ {} regressions   = {} unchanged",
        s.improvements, s.regressions, s.unchanged
    ));
    if s.missing_in_a + s.missing_in_b > 0 {
        ui.label(
            egui::RichText::new(format!(
                "missing in A: {}   missing in B: {}",
                s.missing_in_a, s.missing_in_b
            ))
            .color(egui::Color32::from_rgb(180, 140, 40)),
        );
    }
}

/// Compute the dataset ordering for the line-timeseries view.
///
/// When a git repo is loaded (i.e. `git.commits` is non-empty), map each
/// `active_dataset` key to its earliest-matching commit index (by
/// `bench_file` filename, commit SHA, or short-SHA substring). Otherwise
/// fall back to `sorted_dataset_keys` so the view still renders a stable
/// alphabetical sequence.
fn timeseries_ordered_keys(app: &Dashboard) -> Vec<String> {
    let active = &app.data_selection.active_dataset;

    if app.git.commits.is_empty() {
        // Fall back to alphabetical. Keep only keys in `active` so disabled
        // datasets don't clutter the X axis.
        return app
            .data_selection
            .sorted_dataset_keys
            .iter()
            .filter(|k| active.contains(k))
            .cloned()
            .collect();
    }

    // Build a (commit_idx, key) table and sort by index. A dataset key may
    // reference a commit by bench_file stem, short SHA, or a substring the
    // user typed when importing — cover all three to stay lenient.
    let mut indexed: Vec<(usize, String)> = Vec::new();
    for key in active {
        let key_lower = key.to_lowercase();
        let idx = app.git.commits.iter().position(|c| {
            let stem = c
                .bench_file
                .as_ref()
                .and_then(|p| p.file_stem())
                .and_then(|s| s.to_str())
                .unwrap_or("");
            !stem.is_empty() && key_lower.contains(&stem.to_lowercase())
                || key_lower.contains(&c.short_sha.to_lowercase())
                || key_lower.contains(&c.sha.to_lowercase())
        });
        match idx {
            Some(i) => indexed.push((i, key.clone())),
            None => indexed.push((usize::MAX, key.clone())),
        }
    }

    // Git walk is newest-first, so sort ascending to lay oldest on the left.
    // Unknowns (usize::MAX) sink to the right alphabetically.
    indexed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    // Reverse so older commits (higher idx) appear on the LEFT — `bench_`
    // filenames typically reflect chronological order, and the user expects
    // "time increases to the right".
    indexed.reverse();
    indexed.into_iter().map(|(_, k)| k).collect()
}

/// Render the Line-Timeseries view: one point per loaded dataset with an
/// aggregated metric on Y. Caches the computed payload keyed on every
/// input that affects output.
fn render_line_timeseries_view(app: &mut Dashboard, ui: &mut Ui) {
    let ordered_keys = timeseries_ordered_keys(app);

    if ordered_keys.is_empty() {
        ui.label(
            egui::RichText::new("Load datasets to see a timeseries.")
                .color(egui::Color32::from_rgb(180, 140, 40)),
        );
        return;
    }

    let key = TimeseriesCacheKey {
        ordered_keys: ordered_keys.clone(),
        aggregation: app.timeseries.aggregation,
        format: app.timeseries.format,
        data_metric: app.plot_config.data_metric,
        custom_formula_src: app
            .plot_config
            .custom_formula
            .as_ref()
            .map(|f| f.source.clone()),
        problem_filter: app.timeseries.problem_filter.clone(),
    };
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let cache_key: u64 = hasher.finish();

    let formula = app.plot_config.custom_formula.as_ref();

    let data: Rc<TimeseriesData> = if app.timeseries_cache.key == cache_key {
        match app.timeseries_cache.data.as_ref() {
            Some(d) => Rc::clone(d),
            None => {
                let generated = build_timeseries(
                    &app.data_selection.dataset,
                    &ordered_keys,
                    app.timeseries.aggregation,
                    app.timeseries.format,
                    app.plot_config.data_metric.unwrap_or(MetricType::Time),
                    formula,
                    app.timeseries.problem_filter.as_deref(),
                );
                Rc::new(generated)
            }
        }
    } else {
        let generated = build_timeseries(
            &app.data_selection.dataset,
            &ordered_keys,
            app.timeseries.aggregation,
            app.timeseries.format,
            app.plot_config.data_metric.unwrap_or(MetricType::Time),
            formula,
            app.timeseries.problem_filter.as_deref(),
        );
        let rc = Rc::new(generated);
        app.timeseries_cache.key = cache_key;
        app.timeseries_cache.data = Some(Rc::clone(&rc));
        rc
    };

    if data.points.is_empty() {
        ui.label(
            egui::RichText::new(
                "No data for the current format / metric combination — \
                 try a different format or check that the loaded datasets \
                 contain the selected problem.",
            )
            .color(egui::Color32::from_rgb(180, 140, 40)),
        );
        return;
    }

    // Split points into normal vs. outlier series (Task 8). The detector
    // runs only when enabled in the sidebar; otherwise `outlier_keys` is
    // empty and every point lands in the normal series.
    let outlier_keys: std::collections::HashSet<String> = if app.outlier_config.enabled {
        let reports = app.outlier_reports(&ordered_keys);
        reports
            .iter()
            .filter(|r| r.is_outlier)
            .map(|r| r.dataset_key.clone())
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    let points_xy: Vec<[f64; 2]> = data.points.iter().map(|p| [p.x, p.y]).collect();
    let outlier_points_xy: Vec<[f64; 2]> = data
        .points
        .iter()
        .filter(|p| outlier_keys.contains(&p.dataset_key))
        .map(|p| [p.x, p.y])
        .collect();

    // Cloned for closure: the axis-formatter may outlive this render call.
    let labels_for_axis: Vec<(f64, String)> = data
        .points
        .iter()
        .map(|p| (p.x, short_tick_label(&p.label)))
        .collect();
    let y_label = data.y_label.clone();

    let plot = Plot::new("timeseries_plot")
        .legend(Legend::default())
        .show_axes([true, true])
        .y_axis_label(y_label)
        .y_axis_formatter(|mark, _range| format_metric_suffix(mark.value))
        .x_axis_formatter(move |mark, _range| {
            labels_for_axis
                .iter()
                .find(|(x, _)| (x - mark.value).abs() < 0.5)
                .map(|(_, s)| s.clone())
                .unwrap_or_else(|| format!("{:.0}", mark.value))
        });

    // Vertical marker at the currently-selected commit's X position
    // (Task 7). Matching is done by dataset_key: the commit switcher stores
    // `"bench_<short_sha>"` in `active_dataset`, and the timeseries payload
    // keeps `dataset_key` on each point. If the selected commit's dataset
    // isn't among the plotted points (e.g. not loaded yet, or filtered out
    // by ordering), no marker is drawn.
    let marker_x: Option<f64> = app
        .git
        .selected_commit_idx
        .and_then(|idx| app.git.commits.get(idx))
        .map(crate::data::state::Dashboard::commit_dataset_key)
        .and_then(|key| {
            data.points
                .iter()
                .find(|p| p.dataset_key == key)
                .map(|p| p.x)
        });

    plot.show(ui, |plot_ui| {
        let line = egui_plot::Line::new("series", egui_plot::PlotPoints::from(points_xy.clone()))
            .color(egui::Color32::from_rgb(90, 150, 220))
            .width(2.0);
        plot_ui.line(line);

        let points = egui_plot::Points::new("series", egui_plot::PlotPoints::from(points_xy))
            .shape(egui_plot::MarkerShape::Circle)
            .color(egui::Color32::from_rgb(90, 150, 220))
            .radius(4.0);
        plot_ui.points(points);

        // Overlay flagged outlier points in warning red (Task 8). Drawn
        // AFTER the normal series so they paint on top without a second
        // line segment — the connective line stays a single colour.
        if !outlier_points_xy.is_empty() {
            let outlier_points =
                egui_plot::Points::new("outliers", egui_plot::PlotPoints::from(outlier_points_xy))
                    .shape(egui_plot::MarkerShape::Circle)
                    .color(egui::Color32::from_rgb(230, 60, 40))
                    .radius(5.5);
            plot_ui.points(outlier_points);
        }

        if let Some(x) = marker_x {
            let vline = egui_plot::VLine::new("selected commit", x)
                .color(egui::Color32::LIGHT_YELLOW)
                .width(1.0);
            plot_ui.vline(vline);
        }
    });
}

/// Tick-label shortener: benchmark filenames can be long (e.g.
/// `bench_abcdef0#12-34-56`); trim to the leading 10 chars so the X axis
/// doesn't overflow.
fn short_tick_label(s: &str) -> String {
    const MAX: usize = 10;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let mut out: String = s.chars().take(MAX).collect();
    out.push('…');
    out
}

/// Render the Stacked-Bar view. A dataset must be pre-picked in the
/// sidebar; otherwise we surface a hint.
fn render_stacked_bar_view(app: &mut Dashboard, ui: &mut Ui) {
    let dataset_key = match app.stacked_bar.dataset.clone() {
        Some(k) => k,
        None => {
            ui.label(
                egui::RichText::new("Pick a dataset in the sidebar to see a breakdown.")
                    .color(egui::Color32::from_rgb(180, 140, 40)),
            );
            return;
        }
    };

    let dataset = match app.data_selection.dataset.get(&dataset_key) {
        Some(d) => d.clone(),
        None => {
            ui.colored_label(
                egui::Color32::RED,
                format!("Dataset '{dataset_key}' not loaded."),
            );
            return;
        }
    };

    if !app.data_selection.active_formats.values().any(|&v| v) {
        ui.label(
            egui::RichText::new("Enable at least one format in the sidebar.")
                .color(egui::Color32::from_rgb(180, 140, 40)),
        );
        return;
    }

    let mut sorted_formats: Vec<(DataFormat, bool)> = app
        .data_selection
        .active_formats
        .iter()
        .map(|(&k, &v)| (k, v))
        .collect();
    sorted_formats.sort_by_key(|(fmt, _)| *fmt);

    let key = StackedBarCacheKey {
        dataset_key: Some(dataset_key.clone()),
        active_formats: sorted_formats,
        data_metric: app.plot_config.data_metric,
        custom_formula_src: app
            .plot_config
            .custom_formula
            .as_ref()
            .map(|f| f.source.clone()),
        sort_by_total: app.stacked_bar.sort_by_total,
        top_n: app.stacked_bar.top_n,
    };
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let cache_key: u64 = hasher.finish();

    let formula = app.plot_config.custom_formula.as_ref();

    let data: Rc<StackedBarData> = if app.stacked_bar_cache.key == cache_key {
        match app.stacked_bar_cache.data.as_ref() {
            Some(d) => Rc::clone(d),
            None => {
                let generated = build_stacked_bar(
                    &dataset,
                    &app.data_selection.active_formats,
                    app.plot_config.data_metric.unwrap_or(MetricType::Storage),
                    formula,
                    app.stacked_bar.sort_by_total,
                    app.stacked_bar.top_n,
                );
                Rc::new(generated)
            }
        }
    } else {
        let generated = build_stacked_bar(
            &dataset,
            &app.data_selection.active_formats,
            app.plot_config.data_metric.unwrap_or(MetricType::Storage),
            formula,
            app.stacked_bar.sort_by_total,
            app.stacked_bar.top_n,
        );
        let rc = Rc::new(generated);
        app.stacked_bar_cache.key = cache_key;
        app.stacked_bar_cache.data = Some(Rc::clone(&rc));
        rc
    };

    if data.problem_labels.is_empty() {
        ui.label(
            egui::RichText::new("Empty dataset or no active formats.")
                .color(egui::Color32::from_rgb(180, 140, 40)),
        );
        return;
    }

    // Build one BarChart per format; stack them bottom-up via `stack_on`.
    // We materialise charts left-to-right in FORMAT_ORDER so the bottom
    // chart anchors the stack and the `others` slice only references
    // charts already emitted.
    let labels_for_axis: Vec<(f64, String)> = data
        .problem_labels
        .iter()
        .enumerate()
        .map(|(i, name)| (i as f64, short_tick_label(name)))
        .collect();

    let mut charts: Vec<egui_plot::BarChart> = Vec::with_capacity(data.series.len());
    for series in &data.series {
        let bars: Vec<egui_plot::Bar> = series
            .values
            .iter()
            .enumerate()
            .map(|(i, &v)| egui_plot::Bar::new(i as f64, v).width(0.8))
            .collect();
        let name = format!("{:?}", series.format);
        let chart = egui_plot::BarChart::new(name, bars).color(series.color);
        // Stack on everything emitted so far.
        let prev_refs: Vec<&egui_plot::BarChart> = charts.iter().collect();
        charts.push(chart.stack_on(&prev_refs));
    }

    Plot::new("stacked_bar_plot")
        .legend(Legend::default())
        .show_axes([true, true])
        .y_axis_formatter(|mark, _range| format_metric_suffix(mark.value))
        .x_axis_formatter(move |mark, _range| {
            labels_for_axis
                .iter()
                .find(|(x, _)| (x - mark.value).abs() < 0.5)
                .map(|(_, s)| s.clone())
                .unwrap_or_default()
        })
        .show(ui, |plot_ui| {
            for chart in charts {
                plot_ui.bar_chart(chart);
            }
        });
}

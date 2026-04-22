use crate::data::state::Dashboard;
use crate::types::{DataFormat, DataMode, MetricType, PlotType, ProfileFilter, XaxisType};
use crate::visualization::plotting::generate_plot_data;
use crate::visualization::tooltip::generate_tooltip_text;
use crate::visualization::utils::format_metric_suffix;
use eframe::egui::Ui;
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

pub fn render_charts(app: &mut Dashboard, ui: &mut Ui) {
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

    let use_log_scale =
        app.plot_config.log_scale_x && current_plot_type == PlotType::PerformanceProfile;

    if current_plot_type == PlotType::PerformanceProfile {
        plot = plot
            .x_axis_label("Performance Ratio (tau)")
            .y_axis_label("Probability (rho)")
            .x_grid_spacer(egui_plot::log_grid_spacer(10)); // Log scale grid
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
            format_metric_suffix(x)
        })
        .y_axis_formatter(|mark, _range| format_metric_suffix(mark.value))
        .label_formatter(move |name, value| {
            if let Some(series) = plot_data_for_closure.series.iter().find(|s| s.name == name) {
                let point = egui_plot::PlotPoint::new(value.x, value.y);
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

    plot.show(ui, |plot_ui| {
        for series in filtered_series {
            if series.is_auxiliary {
                // Auxiliary series (like percentile bands) are always rendered as lines
                let line = egui_plot::Line::new(
                    series.name.clone(),
                    egui_plot::PlotPoints::from(series.points.clone()),
                )
                .color(series.color)
                .style(egui_plot::LineStyle::Dashed { length: 10.0 });
                plot_ui.line(line);
                continue;
            }

            // Only draw lines for Performance Profile mode
            if is_performance_profile {
                let line = egui_plot::Line::new(
                    series.name.clone(),
                    egui_plot::PlotPoints::from(series.points.clone()),
                )
                .color(series.color)
                .width(1.0);
                plot_ui.line(line);
            } else {
                let points = egui_plot::Points::new(
                    series.name.clone(),
                    egui_plot::PlotPoints::from(series.points.clone()),
                )
                .shape(series.marker)
                .color(series.color)
                .radius(3.0);
                plot_ui.points(points);
            }
        }
    });
}

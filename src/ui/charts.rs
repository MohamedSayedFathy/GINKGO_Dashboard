use crate::data::state::Dashboard;
use crate::types::{PlotType, ProfileFilter};
use crate::visualization::plotting::generate_plot_data;
use crate::visualization::tooltip::generate_tooltip_text;
use crate::visualization::utils::format_metric_suffix;
use eframe::egui::Ui;
use egui_plot::{Legend, Plot};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

pub fn render_charts(app: &mut Dashboard, ui: &mut Ui) {
    // Generate cache key
    let mut hasher = DefaultHasher::new();
    app.mode.hash(&mut hasher);
    app.active_dataset.hash(&mut hasher);

    let mut sorted_formats: Vec<_> = app.active_formats.iter().map(|(k, v)| (k, *v)).collect();
    // Use natural ordering (requires Ord on DataFormat)
    sorted_formats.sort_by(|a, b| a.0.cmp(b.0));
    for (fmt, active) in sorted_formats {
        fmt.hash(&mut hasher);
        active.hash(&mut hasher);
    }

    app.x_axis.hash(&mut hasher);
    app.data_metric.hash(&mut hasher);
    app.baseline_format.hash(&mut hasher);
    app.plot_type.hash(&mut hasher);
    app.normalize.hash(&mut hasher);

    match app.profile_filter {
        ProfileFilter::None => 0.hash(&mut hasher),
        ProfileFilter::MaxTau(v) => {
            1.hash(&mut hasher);
            v.to_bits().hash(&mut hasher);
        }
        ProfileFilter::TrimPercent(v) => {
            2.hash(&mut hasher);
            v.to_bits().hash(&mut hasher);
        }
    }

    app.filter_outliers.hash(&mut hasher);
    app.log_scale_x.hash(&mut hasher);
    app.show_percentile_bands.hash(&mut hasher);

    let cache_key = hasher.finish().to_string();

    let plot_data: Rc<_> = if app.plot_cache_key != cache_key {
        let data = generate_plot_data(
            &app.dataset,
            &app.active_dataset,
            &app.active_formats,
            app.mode,
            app.x_axis,
            app.data_metric,
            app.baseline_format,
            app.normalize,
            app.filter_outliers,
            app.plot_type,
            app.profile_filter,
            app.log_scale_x,
            app.show_percentile_bands,
        );

        let rc_data = Rc::new(data);
        app.plot_cache_key = cache_key;
        app.cached_plot_data = Some(Rc::clone(&rc_data));
        rc_data
    } else {
        match app.cached_plot_data.as_ref() {
            Some(data) => Rc::clone(data),
            None => {
                log::warn!("Cache miss despite matching key - regenerating");
                let data = generate_plot_data(
                    &app.dataset,
                    &app.active_dataset,
                    &app.active_formats,
                    app.mode,
                    app.x_axis,
                    app.data_metric,
                    app.baseline_format,
                    app.normalize,
                    app.filter_outliers,
                    app.plot_type,
                    app.profile_filter,
                    app.log_scale_x,
                    app.show_percentile_bands,
                );
                Rc::new(data)
            }
        }
    };

    let plot_data_for_closure = Rc::clone(&plot_data);

    let plot_id = format!(
        "Benchmark Plot - {:?} - {:?} - {:?}",
        app.x_axis, app.data_metric, app.plot_type
    );

    let current_plot_type = app.plot_type;

    let mut plot = Plot::new(plot_id)
        .legend(Legend::default())
        .show_axes([true, true]);

    // Outlier filtering is now handled in data generation (cached)
    let filtered_series = &plot_data.series;

    let use_log_scale = app.log_scale_x && current_plot_type == PlotType::PerformanceProfile;

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

    let is_performance_profile = app.plot_type == PlotType::PerformanceProfile;

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

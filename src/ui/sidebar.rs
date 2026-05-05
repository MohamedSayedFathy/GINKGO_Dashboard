use crate::data::state::{CompareSide, Dashboard, SolverXAxis, ViewMode};
use crate::types::{
    AggregationKind, DataFormat, DataMode, MetricType, PlotType, ProfileFilter, XaxisType,
};
use crate::visualization::formula;
use eframe::egui::{
    self, CollapsingHeader, ComboBox, Context, DragValue, Grid, ProgressBar, RichText, SidePanel,
    Slider, Spinner, TextEdit, Ui,
};

/// Badge color for commits flagged as cross-commit outliers (Task 8).
///
/// Amber matches the "warning, not critical" connotation used elsewhere
/// (e.g. Data Sources filter banner).
const OUTLIER_BADGE_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 120, 40);

pub fn render_axis_controls(app: &mut Dashboard, ui: &mut Ui) {
    ui.label("X-Axis:");
    ComboBox::from_id_salt("xaxis_combo")
        .selected_text(format!(
            "{:?}",
            app.plot_config.x_axis.unwrap_or(XaxisType::NonZeros)
        ))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut app.plot_config.x_axis, Some(XaxisType::Cols), "Cols");
            ui.selectable_value(&mut app.plot_config.x_axis, Some(XaxisType::ColCv), "ColCv");
            ui.selectable_value(&mut app.plot_config.x_axis, Some(XaxisType::Rows), "Rows");
            ui.selectable_value(&mut app.plot_config.x_axis, Some(XaxisType::RowCv), "RowCv");
            ui.selectable_value(
                &mut app.plot_config.x_axis,
                Some(XaxisType::NonZeros),
                "NonZeros",
            );
            ui.selectable_value(
                &mut app.plot_config.x_axis,
                Some(XaxisType::Sparsity),
                "Sparsity",
            );
            ui.selectable_value(
                &mut app.plot_config.x_axis,
                Some(XaxisType::AvgNnzPerRow),
                "AvgNnzPerRow",
            );
            ui.selectable_value(
                &mut app.plot_config.x_axis,
                Some(XaxisType::AvgNnzPerCol),
                "AvgNnzPerCol",
            );
            ui.selectable_value(
                &mut app.plot_config.x_axis,
                Some(XaxisType::MatrixShapeRatio),
                "MatrixShapeRatio",
            );
        });
    ui.end_row();

    ui.label("Y-Axis:");
    ComboBox::from_id_salt("yaxis_combo")
        .selected_text(format!(
            "{:?}",
            app.plot_config.data_metric.unwrap_or(MetricType::Time)
        ))
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut app.plot_config.data_metric,
                Some(MetricType::Storage),
                "Storage",
            );
            ui.selectable_value(
                &mut app.plot_config.data_metric,
                Some(MetricType::Time),
                "Time",
            );
            ui.selectable_value(
                &mut app.plot_config.data_metric,
                Some(MetricType::GflopsPerSecond),
                "GflopsPerSecond",
            );
            ui.selectable_value(
                &mut app.plot_config.data_metric,
                Some(MetricType::Repetitions),
                "Repetitions",
            );
            ui.selectable_value(
                &mut app.plot_config.data_metric,
                Some(MetricType::OperationalIntensity),
                "Operational Intensity",
            );
            ui.selectable_value(
                &mut app.plot_config.data_metric,
                Some(MetricType::EffectiveMemoryBandwidth),
                "EffectiveMemoryBandwidth",
            );
            ui.selectable_value(
                &mut app.plot_config.data_metric,
                Some(MetricType::Custom),
                "Custom",
            );
        });
    ui.end_row();

    if app.plot_config.data_metric == Some(MetricType::Custom) {
        ui.label("Formula:");
        let hover_tip = formula::KNOWN_VARIABLES.join(", ");
        let response = ui
            .add(
                TextEdit::singleline(&mut app.plot_config.custom_formula_text)
                    .desired_width(220.0)
                    .hint_text("e.g. gflops / time"),
            )
            .on_hover_text(&hover_tip);

        if response.changed() {
            match formula::compile(&app.plot_config.custom_formula_text) {
                Ok(f) => {
                    app.plot_config.custom_formula = Some(f);
                    app.plot_config.custom_formula_error = None;
                }
                Err(e) => {
                    // Keep the last valid compiled formula so the plot keeps rendering.
                    app.plot_config.custom_formula_error = Some(e.to_string());
                }
            }
        }

        if let Some(err) = app.plot_config.custom_formula_error.as_ref() {
            ui.end_row();
            ui.label(""); // spacer in grid
            ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
        }

        if app.plot_config.plot_type == PlotType::PerformanceProfile {
            ui.end_row();
            ui.label(""); // spacer in grid
            ui.colored_label(
                egui::Color32::from_rgb(180, 140, 40),
                "baseline_* variables are unavailable in Performance Profile view.",
            );
        }

        ui.end_row();
    }
}

pub fn render_side_panel(app: &mut Dashboard, ctx: &Context) {
    match app.data_selection.active_dataset.len() {
        0 => app.data_selection.mode = None,
        1 => app.data_selection.mode = Some(DataMode::Single),
        _ => app.data_selection.mode = Some(DataMode::Multi),
    }

    SidePanel::right("side_panel")
        .resizable(false)
        .show(ctx, |ui| {
            render_view_mode_selector(app, ui);
            ui.separator();

            if app.view_mode == ViewMode::Solver {
                render_solver_panel(app, ui, ctx);
            } else {
                render_benchmark_panel(app, ui);
                ui.add_space(5.0);
                render_export_controls(app, ui);
            }

            ui.add_space(10.0);

            ui.add_space(10.0);
            app.loading.file_dialog.update(ctx);
            app.loading.git_file_dialog.update(ctx);

            if let Some(path) = app.loading.file_dialog.take_picked() {
                app.loading.picked_file = Some(path.to_path_buf());
                app.process_file(ctx);
            }

            if let Some(path) = app.loading.git_file_dialog.take_picked() {
                app.load_git_repo(ctx, path.to_path_buf());
            }
        });
}

/// Render the Export collapsing section (Task 10).
///
/// Two buttons when running natively — SVG + PDF — and one on wasm
/// (PDF conversion drags in `svg2pdf` / `usvg`, neither of which is
/// currently built for wasm in this crate). The status label beneath
/// shows the result of the most recent export: a path + byte count on
/// success, an error message on failure, or nothing when idle.
fn render_export_controls(app: &mut Dashboard, ui: &mut Ui) {
    CollapsingHeader::new(RichText::new("Export").strong())
        .id_salt("export_header")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .button("Save SVG")
                    .on_hover_text("Write the current plot as SVG to ./exports/")
                    .clicked()
                {
                    app.export.last_message = None;
                    app.export_current_svg();
                }
                #[cfg(not(target_arch = "wasm32"))]
                if ui
                    .button("Save PDF")
                    .on_hover_text("Write the current plot as PDF to ./exports/")
                    .clicked()
                {
                    app.export.last_message = None;
                    app.export_current_pdf();
                }
            });
            if let Some(msg) = app.export.last_message.as_ref() {
                ui.add_space(4.0);
                let color = if msg.contains("failed") {
                    egui::Color32::from_rgb(220, 80, 80)
                } else {
                    egui::Color32::from_rgb(120, 200, 120)
                };
                ui.colored_label(color, msg);
            }
        });
}

/// Render a progress indicator for the current in-flight load.
///
/// Three visual states:
/// - `progress = Some(total = Some(n))` → filled `ProgressBar` with percentage.
/// - `progress = Some(total = None)`    → `Spinner` + "Loaded X (phase)".
/// - `progress = None`                  → `Spinner` + "Loading...".
///
/// Safe to call unconditionally; returns early when `is_loading == false`.
fn render_load_progress(app: &Dashboard, ui: &mut Ui) {
    if !app.loading.is_loading {
        return;
    }

    ui.group(|ui| match &app.loading.progress {
        Some(p) => match p.total {
            Some(total) if total > 0 => {
                let fraction = (p.current as f32 / total as f32).clamp(0.0, 1.0);
                ui.label(format!("{}…", p.phase));
                ui.add(ProgressBar::new(fraction).show_percentage());
            }
            _ => {
                ui.horizontal(|ui| {
                    ui.add(Spinner::new());
                    ui.label(format!("{}: {} items", p.phase, p.current));
                });
            }
        },
        None => {
            ui.horizontal(|ui| {
                ui.add(Spinner::new());
                ui.label("Loading…");
            });
        }
    });
    ui.add_space(5.0);
}

fn render_view_mode_selector(app: &mut Dashboard, ui: &mut Ui) {
    ui.heading("View Mode");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.view_mode, ViewMode::Benchmark, "Benchmark");
        ui.selectable_value(&mut app.view_mode, ViewMode::Solver, "Solver");
    });
}

fn render_solver_panel(app: &mut Dashboard, ui: &mut Ui, _ctx: &Context) {
    ui.heading("Solver Controls");
    ui.add_space(5.0);

    render_load_progress(app, ui);

    let is_loading = app.loading.is_loading;
    if ui
        .add_enabled(!is_loading, egui::Button::new("📂 Import Solver Data"))
        .clicked()
    {
        app.loading.file_dialog.pick_file();
    }

    if app.solver.data.is_some() {
        ui.separator();

        // Solver Selection
        ui.label("Select Solvers:");
        if let Some(solver_data) = &app.solver.data {
            if app.solver.selected_idx < solver_data.len() {
                let benchmark = &solver_data[app.solver.selected_idx];
                let mut methods: Vec<&String> = benchmark.solver.keys().collect();
                methods.sort();

                for method in methods {
                    let mut is_selected = app.solver.selected_methods.contains(method);
                    if ui.checkbox(&mut is_selected, method).changed() {
                        if is_selected {
                            app.solver.selected_methods.insert(method.clone());
                        } else {
                            app.solver.selected_methods.remove(method);
                        }
                    }
                }
            }
        }

        ui.separator();

        // X-Axis
        ui.label("X-Axis:");
        ui.radio_value(
            &mut app.solver.x_axis,
            SolverXAxis::Iteration,
            "Iteration Index",
        );
        ui.radio_value(&mut app.solver.x_axis, SolverXAxis::Time, "Time (s)");

        ui.separator();

        // Plot Configuration
        ui.label("Residuals:");
        ui.checkbox(&mut app.solver.show_recurrent, "Recurrent");
        ui.checkbox(&mut app.solver.show_true, "True");
        ui.checkbox(&mut app.solver.show_implicit, "Implicit");
        ui.checkbox(&mut app.solver.show_timestamp, "Time");

        ui.separator();
        ui.label("Options:");
        ui.checkbox(&mut app.solver.log_scale, "Log Scale");
    }
}

fn render_benchmark_panel(app: &mut Dashboard, ui: &mut Ui) {
    render_git_panel(app, ui);
    ui.add_space(5.0);
    render_outlier_controls(app, ui);
    ui.add_space(5.0);
    ui.separator();

    ui.heading("Data Sources");
    ui.add_space(10.0);

    if let Some(error) = &app.loading.last_error {
        ui.colored_label(egui::Color32::RED, format!("Error: {}", error));
        ui.add_space(5.0);
        if ui.button("Clear Error").clicked() {
            app.loading.last_error = None;
        }
        ui.separator();
    }

    render_load_progress(app, ui);

    render_filter_stats(app, ui);

    ui.heading("Data Configuration");
    ui.add_space(5.0);

    let is_loading = app.loading.is_loading;
    ui.group(|ui| {
        if ui
            .add_enabled(!is_loading, egui::Button::new("📂 Import New Dataset"))
            .clicked()
        {
            app.loading.file_dialog.pick_file();
        }
        ui.add_space(5.0);

        ui.label(RichText::new("Datasets:").strong());
        if app.data_selection.sorted_dataset_keys.is_empty() {
            ui.label("No datasets loaded.");
        }

        let keys: Vec<String> = app.data_selection.sorted_dataset_keys.clone();
        for key in &keys {
            let mut is_active = app.data_selection.active_dataset.contains(key);
            if ui.checkbox(&mut is_active, key).changed() {
                if is_active {
                    app.data_selection.active_dataset.push(key.clone());
                } else {
                    app.data_selection.active_dataset.retain(|x| x != key);
                }
            }
        }
        ui.add_space(5.0);

        // Formats Section (Multi-mode only)
        if let Some(DataMode::Multi) = app.data_selection.mode {
            ui.separator();
            ui.label(RichText::new("Formats:").strong());
            ui.horizontal_wrapped(|ui| {
                let formats = [
                    DataFormat::CSR,
                    DataFormat::COO,
                    DataFormat::ELL,
                    DataFormat::HYBRID,
                    DataFormat::SELLP,
                ];

                for format in formats {
                    let is_active = app
                        .data_selection
                        .active_formats
                        .entry(format)
                        .or_insert(true);
                    ui.checkbox(is_active, format!("{:?}", format));
                }
            });
        }
    });
    ui.add_space(5.0);

    ui.separator();
    ui.heading("Visualization");
    ui.label("Graph Options:");

    render_chart_options(app, ui);
}

/// Render the Comparison-plot sidebar controls: A/B pickers, baseline side,
/// Y-link toggle, "lower is better" orientation flag, and the diff-table
/// threshold knob. Disabled with a hint when fewer than two datasets are
/// loaded — per Task 5, the user must pre-load both via the existing Git
/// panel / file loader before they can compare.
fn render_comparison_controls(app: &mut Dashboard, ui: &mut Ui) {
    ui.group(|ui| {
        ui.label(egui::RichText::new("Comparison").strong());
        let keys = app.data_selection.sorted_dataset_keys.clone();
        let enough = keys.len() >= 2;

        if !enough {
            ui.label(
                egui::RichText::new("Load two datasets to compare.")
                    .color(egui::Color32::from_rgb(180, 140, 40)),
            );
        }

        Grid::new("comparison_grid")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("A:");
                ui.add_enabled_ui(enough, |ui| {
                    let selected_a = app
                        .comparison
                        .commit_a
                        .as_deref()
                        .unwrap_or("(select)")
                        .to_string();
                    ComboBox::from_id_salt("compare_a_combo")
                        .selected_text(selected_a)
                        .show_ui(ui, |ui| {
                            for k in &keys {
                                let is_sel = app.comparison.commit_a.as_deref() == Some(k.as_str());
                                if ui.selectable_label(is_sel, k).clicked() {
                                    app.comparison.commit_a = Some(k.clone());
                                }
                            }
                        });
                });
                ui.end_row();

                ui.label("B:");
                ui.add_enabled_ui(enough, |ui| {
                    let selected_b = app
                        .comparison
                        .commit_b
                        .as_deref()
                        .unwrap_or("(select)")
                        .to_string();
                    ComboBox::from_id_salt("compare_b_combo")
                        .selected_text(selected_b)
                        .show_ui(ui, |ui| {
                            for k in &keys {
                                let is_sel = app.comparison.commit_b.as_deref() == Some(k.as_str());
                                if ui.selectable_label(is_sel, k).clicked() {
                                    app.comparison.commit_b = Some(k.clone());
                                }
                            }
                        });
                });
                ui.end_row();

                ui.label("Baseline:");
                ui.add_enabled_ui(enough, |ui| {
                    ComboBox::from_id_salt("compare_baseline_combo")
                        .selected_text(match app.comparison.baseline_side {
                            CompareSide::A => "A",
                            CompareSide::B => "B",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut app.comparison.baseline_side,
                                CompareSide::A,
                                "A",
                            );
                            ui.selectable_value(
                                &mut app.comparison.baseline_side,
                                CompareSide::B,
                                "B",
                            );
                        });
                });
                ui.end_row();

                ui.label("Threshold (%):");
                ui.add_enabled_ui(enough, |ui| {
                    ui.add(
                        DragValue::new(&mut app.comparison.diff_threshold)
                            .speed(0.5)
                            .range(0.0..=100.0),
                    )
                    .on_hover_text(
                        "Hide diff-table rows whose |Δ|/|A| is below this percent. \
                         Histogram and summary still include every finite ratio.",
                    );
                });
                ui.end_row();
            });

        ui.add_enabled_ui(enough, |ui| {
            ui.checkbox(&mut app.comparison.shared_y_range, "Shared Y range")
                .on_hover_text("Align Y bounds across both panes.");
            ui.checkbox(&mut app.comparison.lower_is_better, "Lower is better")
                .on_hover_text(
                    "Speedup orientation: when on (e.g. time), ratio = A/B so \
                     ratio > 1 means B is faster. Turn off for gflops/bandwidth.",
                );
        });
    });
}

/// Render the Line-Timeseries sidebar controls (Task 6). Exposes the
/// aggregation kernel, single-format picker, and optional problem filter.
fn render_timeseries_controls(app: &mut Dashboard, ui: &mut Ui) {
    ui.group(|ui| {
        ui.label(RichText::new("Line Timeseries").strong());

        let keys = app.data_selection.sorted_dataset_keys.clone();
        if keys.is_empty() {
            ui.label(
                RichText::new("Load datasets to see a timeseries.")
                    .color(egui::Color32::from_rgb(180, 140, 40)),
            );
        }

        Grid::new("timeseries_grid")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("Aggregation:");
                ComboBox::from_id_salt("ts_agg_combo")
                    .selected_text(format!("{:?}", app.timeseries.aggregation))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut app.timeseries.aggregation,
                            AggregationKind::Median,
                            "Median",
                        );
                        ui.selectable_value(
                            &mut app.timeseries.aggregation,
                            AggregationKind::Mean,
                            "Mean",
                        );
                        ui.selectable_value(
                            &mut app.timeseries.aggregation,
                            AggregationKind::GeometricMean,
                            "Geomean",
                        );
                    });
                ui.end_row();

                ui.label("Format:");
                ComboBox::from_id_salt("ts_format_combo")
                    .selected_text(format!("{:?}", app.timeseries.format))
                    .show_ui(ui, |ui| {
                        for f in [
                            DataFormat::CSR,
                            DataFormat::COO,
                            DataFormat::ELL,
                            DataFormat::HYBRID,
                            DataFormat::SELLP,
                        ] {
                            ui.selectable_value(&mut app.timeseries.format, f, format!("{:?}", f));
                        }
                    });
                ui.end_row();

                ui.label("Problem:");
                // Collect all problem names from the active dataset set so
                // the filter ComboBox reflects what's actually loadable.
                let mut problem_names: Vec<String> = Vec::new();
                for ds_key in &app.data_selection.active_dataset {
                    if let Some(ds) = app.data_selection.dataset.get(ds_key) {
                        for p in &ds.benchmark {
                            let name = p.problem.name.as_str().to_string();
                            if !problem_names.contains(&name) {
                                problem_names.push(name);
                            }
                        }
                    }
                }
                problem_names.sort();

                let current = app
                    .timeseries
                    .problem_filter
                    .clone()
                    .unwrap_or_else(|| "(all)".to_string());
                ComboBox::from_id_salt("ts_problem_combo")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(app.timeseries.problem_filter.is_none(), "(all)")
                            .clicked()
                        {
                            app.timeseries.problem_filter = None;
                        }
                        for name in &problem_names {
                            let is_sel =
                                app.timeseries.problem_filter.as_deref() == Some(name.as_str());
                            if ui.selectable_label(is_sel, name).clicked() {
                                app.timeseries.problem_filter = Some(name.clone());
                            }
                        }
                    });
                ui.end_row();
            });
    });
}

/// Render the Stacked-Bar sidebar controls (Task 6). A single-dataset
/// picker, sort toggle, and top-N limiter.
fn render_stacked_bar_controls(app: &mut Dashboard, ui: &mut Ui) {
    ui.group(|ui| {
        ui.label(RichText::new("Stacked Bar").strong());

        let keys = app.data_selection.sorted_dataset_keys.clone();
        if keys.is_empty() {
            ui.label(
                RichText::new("Load a dataset to view a breakdown.")
                    .color(egui::Color32::from_rgb(180, 140, 40)),
            );
        }

        Grid::new("stacked_bar_grid")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("Dataset:");
                let selected = app
                    .stacked_bar
                    .dataset
                    .as_deref()
                    .unwrap_or("(select)")
                    .to_string();
                ComboBox::from_id_salt("sb_dataset_combo")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for k in &keys {
                            let is_sel = app.stacked_bar.dataset.as_deref() == Some(k.as_str());
                            if ui.selectable_label(is_sel, k).clicked() {
                                app.stacked_bar.dataset = Some(k.clone());
                            }
                        }
                    });
                ui.end_row();

                ui.label("Top N:");
                ui.add(
                    DragValue::new(&mut app.stacked_bar.top_n)
                        .speed(1.0)
                        .range(5..=500),
                )
                .on_hover_text("Cap bars shown; use sort-by-total to see heaviest first.");
                ui.end_row();
            });

        ui.checkbox(&mut app.stacked_bar.sort_by_total, "Sort by total")
            .on_hover_text("Rank problems by summed metric, largest first.");
    });
}

/// Render the collapsing "Outlier detection" sidebar panel (Task 8).
///
/// Exposes the config knobs — baseline window, sigma threshold, percent
/// deviation gate, and metric — and shows a small "N outliers / M commits"
/// summary once detection is enabled. Controls other than the master
/// checkbox are disabled until detection is enabled, so the dashboard
/// never forces users who don't want this feature to pay the per-frame
/// computation.
fn render_outlier_controls(app: &mut Dashboard, ui: &mut Ui) {
    CollapsingHeader::new(RichText::new("Outlier detection").strong())
        .id_salt("outlier_detection_header")
        .default_open(false)
        .show(ui, |ui| {
            ui.checkbox(&mut app.outlier_config.enabled, "Enable outlier detection")
                .on_hover_text(
                    "Flag commits whose benchmarks deviate more than K sigma \
                 from the median of the last N commits.",
                );

            let enabled = app.outlier_config.enabled;

            Grid::new("outlier_grid")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Baseline window (commits):");
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.add(
                            DragValue::new(&mut app.outlier_config.baseline_window)
                                .speed(1.0)
                                .range(2..=50),
                        );
                    });
                    ui.end_row();

                    ui.label("Sigma threshold (K):");
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.add(
                            DragValue::new(&mut app.outlier_config.sigma_threshold)
                                .speed(0.1)
                                .range(1.0..=10.0),
                        );
                    });
                    ui.end_row();

                    ui.label("Deviation percent (X):");
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.add(
                            DragValue::new(&mut app.outlier_config.threshold_percent)
                                .speed(1.0)
                                .range(0.0..=100.0),
                        );
                    });
                    ui.end_row();

                    ui.label("Metric:");
                    ui.add_enabled_ui(enabled, |ui| {
                        ComboBox::from_id_salt("outlier_metric_combo")
                            .selected_text(format!("{:?}", app.outlier_config.metric))
                            .show_ui(ui, |ui| {
                                for m in [
                                    MetricType::Time,
                                    MetricType::GflopsPerSecond,
                                    MetricType::Storage,
                                    MetricType::Repetitions,
                                    MetricType::OperationalIntensity,
                                    MetricType::EffectiveMemoryBandwidth,
                                    MetricType::Custom,
                                ] {
                                    ui.selectable_value(
                                        &mut app.outlier_config.metric,
                                        m,
                                        format!("{:?}", m),
                                    );
                                }
                            });
                    });
                    ui.end_row();
                });

            if enabled {
                let ordered = app.outlier_ordered_keys();
                let reports = app.outlier_reports(&ordered);
                let total = reports.len();
                let flagged = reports.iter().filter(|r| r.is_outlier).count();
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "{flagged} outlier(s) detected out of {total} commits"
                    ))
                    .color(if flagged > 0 {
                        OUTLIER_BADGE_COLOR
                    } else {
                        egui::Color32::from_rgb(100, 200, 100)
                    }),
                );
            }
        });
}

/// Render the sidebar section for git-repo-driven benchmark navigation.
///
/// Shows a "Load Git Repo" button, the currently loaded repo path (if any),
/// and a scrollable commit list. Selecting a commit updates the index in
/// `GitState` but does not load the associated benchmark file — that's
/// handled by the commit-switcher task (Task 7).
fn render_git_panel(app: &mut Dashboard, ui: &mut Ui) {
    ui.heading("Git Repository");
    ui.add_space(5.0);

    let is_loading = app.loading.is_loading;
    ui.group(|ui| {
        // Native: pick a local repo directory via the native file dialog
        // and walk it with `gix`. Wasm: no directory picker — fetch a
        // prebuilt `benchmarks/commits.json` from the same origin. Split
        // at the button rather than abstracting behind a trait since the
        // two code paths already live in separate cfg-gated modules.
        #[cfg(not(target_arch = "wasm32"))]
        {
            if ui
                .add_enabled(!is_loading, egui::Button::new("📂 Load Git Repo"))
                .clicked()
            {
                app.loading.git_file_dialog.pick_directory();
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let ctx = ui.ctx().clone();
            if ui
                .add_enabled(!is_loading, egui::Button::new("📂 Load Benchmarks"))
                .clicked()
            {
                app.load_git_repo(&ctx, std::path::PathBuf::new());
            }
        }

        if let Some(err) = &app.git.last_error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, format!("Git error: {}", err));
            if ui.button("Clear").clicked() {
                app.git.last_error = None;
            }
        }

        if let Some(path) = &app.git.repo_path {
            ui.add_space(4.0);
            let display = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_else(|| path.to_str().unwrap_or("<invalid path>"));
            ui.label(RichText::new(format!("Repo: {}", display)).strong())
                .on_hover_text(path.to_string_lossy().into_owned());
            ui.label(format!("Commits: {}", app.git.commits.len()));
        }

        if !app.git.commits.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new("History:").strong());
            render_commit_list(app, ui);
            ui.add_space(6.0);
            render_commit_switcher(app, ui);
        }
    });
}

/// Interactive commit switcher (Task 7).
///
/// Three controls that all read and write `git.selected_commit_idx`:
/// - A horizontal [`Slider`] over `0..=last_idx` (no value readout; the
///   ComboBox + step buttons give the textual form).
/// - A [`ComboBox`] listing `"<short_sha> <subject>"` so keyboard users can
///   jump directly without scrubbing.
/// - Prev / Next step buttons with tooltips mentioning the `[` / `]`
///   keybinds handled in `impl App for Dashboard`.
///
/// Any change propagates through `Dashboard::on_commit_selected`, which
/// either activates an already-loaded dataset or kicks off a single-file
/// load (per Task 7 spec).
fn render_commit_switcher(app: &mut Dashboard, ui: &mut Ui) {
    const MESSAGE_MAX: usize = 50;
    let len = app.git.commits.len();
    if len == 0 {
        return;
    }
    let last = len - 1;
    let is_loading = app.loading.is_loading;
    let ctx = ui.ctx().clone();

    let outlier_keys = compute_outlier_key_set(app);

    ui.label(RichText::new("Switch Commit:").strong());

    // Slider: index only. A tooltip on hover shows the currently-selected
    // commit's short-SHA + subject + date. Per-position tooltip-at-pointer
    // would require digging into the raw response + `hover_pos()` math; the
    // current-selection tooltip is simpler and still useful.
    let mut idx_val = app.git.selected_commit_idx.unwrap_or(0);
    let slider_response = ui.add_enabled(
        !is_loading,
        Slider::new(&mut idx_val, 0..=last).show_value(false),
    );
    // Tooltip body: the currently-addressed commit.
    if let Some(commit) = app.git.commits.get(idx_val) {
        let date_short = commit.date.get(..10).unwrap_or(&commit.date);
        let msg = truncate_str(&commit.message, MESSAGE_MAX);
        slider_response
            .clone()
            .on_hover_text(format!("[{}] {} — {}", commit.short_sha, date_short, msg));
    }
    if slider_response.changed() && app.git.selected_commit_idx != Some(idx_val) {
        app.on_commit_selected(&ctx, idx_val);
    }

    // ComboBox: full list, one line per commit.
    let current_text = match app
        .git
        .selected_commit_idx
        .and_then(|i| app.git.commits.get(i))
    {
        Some(c) => {
            let key = Dashboard::commit_dataset_key(c);
            let badge = if outlier_keys.contains(&key) {
                "[!] "
            } else {
                ""
            };
            format!(
                "{}{} {}",
                badge,
                c.short_sha,
                truncate_str(&c.message, MESSAGE_MAX)
            )
        }
        None => "(none)".to_string(),
    };

    // `commits` is borrowed immutably by the closure below, so we collect
    // the per-row render tuples ahead of time.
    let rows: Vec<(usize, String, String, bool)> = app
        .git
        .commits
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let key = Dashboard::commit_dataset_key(c);
            let is_outlier = outlier_keys.contains(&key);
            let badge = if is_outlier { "[!] " } else { "" };
            (
                i,
                format!(
                    "{}{} {}",
                    badge,
                    c.short_sha,
                    truncate_str(&c.message, MESSAGE_MAX)
                ),
                format!(
                    "{}\n{}\n{}",
                    c.sha,
                    c.author,
                    truncate_str(&c.message, MESSAGE_MAX * 2)
                ),
                is_outlier,
            )
        })
        .collect();

    let mut pending: Option<usize> = None;
    ui.add_enabled_ui(!is_loading, |ui| {
        ComboBox::from_id_salt("commit_switcher_combo")
            .selected_text(current_text)
            .width(ui.available_width().min(260.0))
            .show_ui(ui, |ui| {
                for (i, label, tip, is_outlier) in &rows {
                    let sel = app.git.selected_commit_idx == Some(*i);
                    let text = if *is_outlier {
                        RichText::new(label).color(OUTLIER_BADGE_COLOR)
                    } else {
                        RichText::new(label)
                    };
                    if ui.selectable_label(sel, text).on_hover_text(tip).clicked() {
                        pending = Some(*i);
                    }
                }
            });
    });
    if let Some(i) = pending {
        app.on_commit_selected(&ctx, i);
    }

    // Prev / Next buttons: each steps by one in the commit list. Disabled
    // at the corresponding end. Tooltips surface the `[` / `]` keybinds.
    ui.horizontal(|ui| {
        let cur = app.git.selected_commit_idx;
        let can_back = !is_loading && cur.is_none_or(|i| i > 0);
        let can_fwd = !is_loading && cur.is_none_or(|i| i < last);
        let prev_resp = ui
            .add_enabled(can_back, egui::Button::new("◀ Prev"))
            .on_hover_text("Step to newer commit (keybind: [ )");
        if prev_resp.clicked() {
            app.step_commit(-1);
            if let Some(idx) = app.git.selected_commit_idx {
                app.on_commit_selected(&ctx, idx);
            }
        }
        let next_resp = ui
            .add_enabled(can_fwd, egui::Button::new("Next ▶"))
            .on_hover_text("Step to older commit (keybind: ] )");
        if next_resp.clicked() {
            app.step_commit(1);
            if let Some(idx) = app.git.selected_commit_idx {
                app.on_commit_selected(&ctx, idx);
            }
        }
    });
}

/// Render the scrollable commit list. Each row is a selectable label that
/// updates `git.selected_commit_idx` on click.
///
/// When outlier detection is enabled (Task 8), commits flagged as outliers
/// gain a `[!]` prefix and their label is rendered in the amber badge
/// colour so keyboard users and visual scanners both see them.
fn render_commit_list(app: &mut Dashboard, ui: &mut Ui) {
    const MESSAGE_MAX: usize = 50;
    let ctx = ui.ctx().clone();

    let outlier_keys = compute_outlier_key_set(app);

    // Collect row descriptors up front; `on_commit_selected` needs `&mut app`
    // so we can't hold the iterator borrow across the click branch.
    let rows: Vec<(usize, String, String, bool)> = app
        .git
        .commits
        .iter()
        .enumerate()
        .map(|(idx, commit)| {
            let marker = if commit.bench_file.is_some() {
                "✓"
            } else {
                " "
            };
            let date_short = commit.date.get(..10).unwrap_or(&commit.date);
            let message = truncate_str(&commit.message, MESSAGE_MAX);
            let key = Dashboard::commit_dataset_key(commit);
            let is_outlier = outlier_keys.contains(&key);
            let badge = if is_outlier { "[!] " } else { "" };
            let label = format!(
                "{}{} [{}] {} — {}",
                badge, marker, commit.short_sha, date_short, message
            );
            let tip = format!("{}\n{}\n{}", commit.sha, commit.author, commit.message);
            (idx, label, tip, is_outlier)
        })
        .collect();

    let mut pending: Option<usize> = None;
    egui::ScrollArea::vertical()
        .max_height(220.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (idx, label, tip, is_outlier) in &rows {
                let selected = app.git.selected_commit_idx == Some(*idx);
                let text = if *is_outlier {
                    RichText::new(label).color(OUTLIER_BADGE_COLOR)
                } else {
                    RichText::new(label)
                };
                if ui
                    .selectable_label(selected, text)
                    .on_hover_text(tip)
                    .clicked()
                {
                    pending = Some(*idx);
                }
            }
        });
    if let Some(idx) = pending {
        app.on_commit_selected(&ctx, idx);
    }
}

/// Collect the set of dataset keys flagged as outliers.
///
/// Returns an empty set when detection is disabled so the badge / recolour
/// paths short-circuit without touching the cache.
fn compute_outlier_key_set(app: &mut Dashboard) -> std::collections::HashSet<String> {
    if !app.outlier_config.enabled {
        return std::collections::HashSet::new();
    }
    let ordered = app.outlier_ordered_keys();
    let reports = app.outlier_reports(&ordered);
    reports
        .iter()
        .filter(|r| r.is_outlier)
        .map(|r| r.dataset_key.clone())
        .collect()
}

/// Truncate a string to at most `max` characters, appending `…` on overflow.
///
/// Counts Unicode scalar values, not bytes, so multi-byte characters aren't
/// split mid-sequence.
fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

fn render_filter_stats(app: &mut Dashboard, ui: &mut Ui) {
    // Display matrix count with filtering info from detailed stats
    if let Some(plot_data) = &app.plot_cache.data {
        let stats = &plot_data.filter_stats;
        let filtered_total = stats.total_matrices.saturating_sub(stats.shown_matrices);

        if stats.total_matrices > 0 {
            let text = if filtered_total > 0 {
                format!(
                    "📊 Showing {} / {} ({:.1}%)",
                    stats.shown_matrices,
                    stats.total_matrices,
                    (stats.shown_matrices as f64 / stats.total_matrices as f64) * 100.0
                )
            } else {
                format!("📊 Showing All {} Matrices", stats.total_matrices)
            };

            let color = if filtered_total > 0 {
                egui::Color32::from_rgb(220, 180, 100)
            } else {
                egui::Color32::from_rgb(100, 200, 100)
            };

            ui.label(RichText::new(text).color(color))
                .on_hover_ui(|ui| {
                    ui.heading("Filtering Breakdown");
                    if filtered_total == 0 {
                        ui.label("No data filtered.");
                    } else {
                        if stats.filtered_missing_time > 0 {
                            ui.label(format!(
                                "• Missing Metrics: {}",
                                stats.filtered_missing_time
                            ));
                        }
                        if stats.filtered_no_format_data > 0 {
                            ui.label(format!(
                                "• Missing Format/Baseline: {}",
                                stats.filtered_no_format_data
                            ));
                        }
                        if stats.filtered_invalid_values > 0 {
                            ui.label(format!(
                                "• Invalid Values (Inf/NaN): {}",
                                stats.filtered_invalid_values
                            ));
                        }
                        if stats.filtered_outliers > 0 {
                            ui.label(format!("• Outliers Filtered: {}", stats.filtered_outliers));
                        }
                    }
                });
            ui.add_space(5.0);
        }
    }
}

fn render_chart_options(app: &mut Dashboard, ui: &mut Ui) {
    ui.group(|ui| {
        ui.label(RichText::new("Chart Setup").strong());

        Grid::new("chart_setup_grid")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("Type:");
                ComboBox::from_id_salt("plot_type_combo")
                    .selected_text(format!("{:?}", app.plot_config.plot_type))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut app.plot_config.plot_type,
                            PlotType::Scatter,
                            "Scatter",
                        );
                        ui.selectable_value(
                            &mut app.plot_config.plot_type,
                            PlotType::PerformanceProfile,
                            "Performance Profile",
                        );
                        ui.selectable_value(
                            &mut app.plot_config.plot_type,
                            PlotType::Comparison,
                            "Comparison (A vs B)",
                        );
                        ui.selectable_value(
                            &mut app.plot_config.plot_type,
                            PlotType::LineTimeseries,
                            "Line Timeseries",
                        );
                        ui.selectable_value(
                            &mut app.plot_config.plot_type,
                            PlotType::StackedBar,
                            "Stacked Bar",
                        );
                    });
                ui.end_row();

                render_axis_controls(app, ui);
            });
    });
    ui.add_space(5.0);

    if app.plot_config.plot_type == PlotType::Comparison {
        render_comparison_controls(app, ui);
        ui.add_space(5.0);
    }

    if app.plot_config.plot_type == PlotType::LineTimeseries {
        render_timeseries_controls(app, ui);
        ui.add_space(5.0);
    }

    if app.plot_config.plot_type == PlotType::StackedBar {
        render_stacked_bar_controls(app, ui);
        ui.add_space(5.0);
    }

    if app.plot_config.plot_type == PlotType::Scatter {
        ui.checkbox(
            &mut app.plot_config.show_percentile_bands,
            "Show Percentile Bands (25/75%)",
        )
        .on_hover_text("Show 25th and 75th percentile bands for each matrix type");

        ui.checkbox(
            &mut app.plot_config.filter_outliers,
            "Filter Outliers (show 90% only)",
        )
        .on_hover_text("Exclude values below 5th and above 95th percentile");
    }

    // Normalization Section
    let is_performance_profile = app.plot_config.plot_type == PlotType::PerformanceProfile;

    if is_performance_profile {
        ui.separator();
        ui.heading("Profile Controls");
        ui.add_space(5.0);

        ui.group(|ui| {
            Grid::new("profile_controls_grid")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Filter:");

                    let current_filter_name = match app.plot_config.profile_filter {
                        ProfileFilter::None => "None",
                        ProfileFilter::MaxTau(_) => "Max Tau",
                        ProfileFilter::TrimPercent(_) => "Trim %",
                    };

                    ComboBox::from_id_salt("profile_filter_combo")
                        .selected_text(current_filter_name)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(
                                    matches!(app.plot_config.profile_filter, ProfileFilter::None),
                                    "None",
                                )
                                .clicked()
                            {
                                app.plot_config.profile_filter = ProfileFilter::None;
                            }
                            if ui
                                .selectable_label(
                                    matches!(
                                        app.plot_config.profile_filter,
                                        ProfileFilter::MaxTau(_)
                                    ),
                                    "Max Tau",
                                )
                                .clicked()
                                && !matches!(
                                    app.plot_config.profile_filter,
                                    ProfileFilter::MaxTau(_)
                                )
                            {
                                app.plot_config.profile_filter = ProfileFilter::MaxTau(10.0);
                            }
                            if ui
                                .selectable_label(
                                    matches!(
                                        app.plot_config.profile_filter,
                                        ProfileFilter::TrimPercent(_)
                                    ),
                                    "Trim %",
                                )
                                .clicked()
                                && !matches!(
                                    app.plot_config.profile_filter,
                                    ProfileFilter::TrimPercent(_)
                                )
                            {
                                app.plot_config.profile_filter = ProfileFilter::TrimPercent(5.0);
                            }
                        });
                    ui.end_row();

                    // Parameter input based on selection
                    match &mut app.plot_config.profile_filter {
                        ProfileFilter::None => {}
                        ProfileFilter::MaxTau(val) => {
                            ui.label("T_max:");
                            ui.add(DragValue::new(val).speed(0.1).range(1.0..=1000.0));
                            ui.end_row();
                        }
                        ProfileFilter::TrimPercent(val) => {
                            ui.label("Trim %:");
                            ui.add(DragValue::new(val).speed(0.1).range(0.0..=50.0));
                            ui.end_row();
                        }
                    }
                });
        });

        ui.add_space(5.0);
        ui.checkbox(&mut app.plot_config.log_scale_x, "Log Scale X-Axis")
            .on_hover_text(
            "Use logarithmic scale for performance ratio (X-axis). Recommended for high variance.",
        );

        app.plot_config.normalize = false;
    } else if let Some(DataMode::Single) = app.data_selection.mode {
        ui.separator();
        ui.heading("Normalization");
        ui.add_space(5.0);

        ui.horizontal(|ui| {
            ui.label("Baseline Format:");
            ComboBox::from_id_salt("baseline_combo")
                .selected_text(format!("{:?}", app.plot_config.baseline_format))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut app.plot_config.baseline_format,
                        DataFormat::CSR,
                        "CSR",
                    );
                    ui.selectable_value(
                        &mut app.plot_config.baseline_format,
                        DataFormat::COO,
                        "COO",
                    );
                    ui.selectable_value(
                        &mut app.plot_config.baseline_format,
                        DataFormat::ELL,
                        "ELL",
                    );
                    ui.selectable_value(
                        &mut app.plot_config.baseline_format,
                        DataFormat::HYBRID,
                        "HYBRID",
                    );
                    ui.selectable_value(
                        &mut app.plot_config.baseline_format,
                        DataFormat::SELLP,
                        "SELLP",
                    );
                });
        });

        ui.checkbox(&mut app.plot_config.normalize, "Normalize to Baseline")
            .on_hover_text("Divide all values by the baseline format's value");
    } else {
        app.plot_config.normalize = false;
    }
}

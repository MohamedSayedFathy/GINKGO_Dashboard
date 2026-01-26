use crate::data::state::{Dashboard, SolverXAxis, ViewMode};
use crate::types::{DataFormat, DataMode, MetricType, PlotType, ProfileFilter, XaxisType};
use eframe::egui::{self, ComboBox, Context, DragValue, Grid, RichText, SidePanel, Ui};

pub fn render_axis_controls(app: &mut Dashboard, ui: &mut Ui) {
    ui.label("X-Axis:");
    ComboBox::from_id_salt("xaxis_combo")
        .selected_text(format!("{:?}", app.x_axis.unwrap_or(XaxisType::NonZeros)))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut app.x_axis, Some(XaxisType::Cols), "Cols");
            ui.selectable_value(&mut app.x_axis, Some(XaxisType::ColCv), "ColCv");
            ui.selectable_value(&mut app.x_axis, Some(XaxisType::Rows), "Rows");
            ui.selectable_value(&mut app.x_axis, Some(XaxisType::RowCv), "RowCv");
            ui.selectable_value(&mut app.x_axis, Some(XaxisType::NonZeros), "NonZeros");
            ui.selectable_value(&mut app.x_axis, Some(XaxisType::Sparsity), "Sparsity");
            ui.selectable_value(
                &mut app.x_axis,
                Some(XaxisType::AvgNnzPerRow),
                "AvgNnzPerRow",
            );
            ui.selectable_value(
                &mut app.x_axis,
                Some(XaxisType::AvgNnzPerCol),
                "AvgNnzPerCol",
            );
            ui.selectable_value(
                &mut app.x_axis,
                Some(XaxisType::MatrixShapeRatio),
                "MatrixShapeRatio",
            );
        });
    ui.end_row();

    ui.label("Y-Axis:");
    ComboBox::from_id_salt("yaxis_combo")
        .selected_text(format!("{:?}", app.data_metric.unwrap_or(MetricType::Time)))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut app.data_metric, Some(MetricType::Storage), "Storage");
            ui.selectable_value(&mut app.data_metric, Some(MetricType::Time), "Time");
            ui.selectable_value(
                &mut app.data_metric,
                Some(MetricType::GflopsPerSecond),
                "GflopsPerSecond",
            );
            ui.selectable_value(
                &mut app.data_metric,
                Some(MetricType::Repetitions),
                "Repetitions",
            );
            ui.selectable_value(
                &mut app.data_metric,
                Some(MetricType::OperationalIntensity),
                "Operational Intensity",
            );
            ui.selectable_value(
                &mut app.data_metric,
                Some(MetricType::EffectiveMemoryBandwidth),
                "EffectiveMemoryBandwidth",
            );
        });
    ui.end_row();
}

pub fn render_side_panel(app: &mut Dashboard, ctx: &Context) {
    match app.active_dataset.len() {
        0 => app.mode = None,
        1 => app.mode = Some(DataMode::Single),
        _ => app.mode = Some(DataMode::Multi),
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
            }

            ui.add_space(10.0);

            ui.add_space(10.0);
            app.file_dialog.update(ctx);

            if let Some(path) = app.file_dialog.take_picked() {
                app.picked_file = Some(path.to_path_buf());
                app.process_file();
            }
        });
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

    if ui.button("📂 Import Solver Data").clicked() {
        app.file_dialog.pick_file();
    }

    if app.solver_data.is_some() {
        ui.separator();

        // Solver Selection
        ui.label("Select Solvers:");
        if let Some(solver_data) = &app.solver_data {
            if app.solver_selected_idx < solver_data.len() {
                let benchmark = &solver_data[app.solver_selected_idx];
                let mut methods: Vec<&String> = benchmark.solver.keys().collect();
                methods.sort();

                for method in methods {
                    let mut is_selected = app.solver_selected_methods.contains(method);
                    if ui.checkbox(&mut is_selected, method).changed() {
                        if is_selected {
                            app.solver_selected_methods.insert(method.clone());
                        } else {
                            app.solver_selected_methods.remove(method);
                        }
                    }
                }
            }
        }

        ui.separator();

        // X-Axis
        ui.label("X-Axis:");
        ui.radio_value(
            &mut app.solver_x_axis,
            SolverXAxis::Iteration,
            "Iteration Index",
        );
        ui.radio_value(&mut app.solver_x_axis, SolverXAxis::Time, "Time (s)");

        ui.separator();

        // Plot Configuration
        ui.label("Residuals:");
        ui.checkbox(&mut app.solver_show_recurrent, "Recurrent");
        ui.checkbox(&mut app.solver_show_true, "True");
        ui.checkbox(&mut app.solver_show_implicit, "Implicit");
        ui.checkbox(&mut app.solver_show_timestamp, "Time");

        ui.separator();
        ui.label("Options:");
        ui.checkbox(&mut app.solver_log_scale, "Log Scale");
    }
}

fn render_benchmark_panel(app: &mut Dashboard, ui: &mut Ui) {
    ui.heading("Data Sources");
    ui.add_space(10.0);

    if let Some(error) = &app.last_error {
        ui.colored_label(egui::Color32::RED, format!("Error: {}", error));
        ui.add_space(5.0);
        if ui.button("Clear Error").clicked() {
            app.last_error = None;
        }
        ui.separator();
    }

    render_filter_stats(app, ui);

    ui.heading("Data Configuration");
    ui.add_space(5.0);

    ui.group(|ui| {
        if ui.button("📂 Import New Dataset").clicked() {
            app.file_dialog.pick_file();
        }
        ui.add_space(5.0);

        ui.label(RichText::new("Datasets:").strong());
        if app.sorted_dataset_keys.is_empty() {
            ui.label("No datasets loaded.");
        }

        for key in &app.sorted_dataset_keys {
            let mut is_active = app.active_dataset.contains(key);
            if ui.checkbox(&mut is_active, key).changed() {
                if is_active {
                    app.active_dataset.push(key.clone());
                } else {
                    app.active_dataset.retain(|x| x != key);
                }
            }
        }
        ui.add_space(5.0);

        // Formats Section (Multi-mode only)
        if let Some(DataMode::Multi) = app.mode {
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
                    let is_active = app.active_formats.entry(format).or_insert(true);
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

fn render_filter_stats(app: &mut Dashboard, ui: &mut Ui) {
    // Display matrix count with filtering info from detailed stats
    if let Some(plot_data) = &app.cached_plot_data {
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
                    .selected_text(format!("{:?}", app.plot_type))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut app.plot_type, PlotType::Scatter, "Scatter");
                        ui.selectable_value(
                            &mut app.plot_type,
                            PlotType::PerformanceProfile,
                            "Performance Profile",
                        );
                    });
                ui.end_row();

                render_axis_controls(app, ui);
            });
    });
    ui.add_space(5.0);

    if app.plot_type == PlotType::Scatter {
        ui.checkbox(
            &mut app.show_percentile_bands,
            "Show Percentile Bands (25/75%)",
        )
        .on_hover_text("Show 25th and 75th percentile bands for each matrix type");

        ui.checkbox(&mut app.filter_outliers, "Filter Outliers (show 90% only)")
            .on_hover_text("Exclude values below 5th and above 95th percentile");
    }

    // Normalization Section
    let is_performance_profile = app.plot_type == PlotType::PerformanceProfile;

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

                    let current_filter_name = match app.profile_filter {
                        ProfileFilter::None => "None",
                        ProfileFilter::MaxTau(_) => "Max Tau",
                        ProfileFilter::TrimPercent(_) => "Trim %",
                    };

                    ComboBox::from_id_salt("profile_filter_combo")
                        .selected_text(current_filter_name)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(
                                    matches!(app.profile_filter, ProfileFilter::None),
                                    "None",
                                )
                                .clicked()
                            {
                                app.profile_filter = ProfileFilter::None;
                            }
                            if ui
                                .selectable_label(
                                    matches!(app.profile_filter, ProfileFilter::MaxTau(_)),
                                    "Max Tau",
                                )
                                .clicked()
                            {
                                if !matches!(app.profile_filter, ProfileFilter::MaxTau(_)) {
                                    app.profile_filter = ProfileFilter::MaxTau(10.0);
                                }
                            }
                            if ui
                                .selectable_label(
                                    matches!(app.profile_filter, ProfileFilter::TrimPercent(_)),
                                    "Trim %",
                                )
                                .clicked()
                            {
                                if !matches!(app.profile_filter, ProfileFilter::TrimPercent(_)) {
                                    app.profile_filter = ProfileFilter::TrimPercent(5.0);
                                }
                            }
                        });
                    ui.end_row();

                    // Parameter input based on selection
                    match &mut app.profile_filter {
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
        ui.checkbox(&mut app.log_scale_x, "Log Scale X-Axis")
            .on_hover_text(
            "Use logarithmic scale for performance ratio (X-axis). Recommended for high variance.",
        );

        app.normalize = false;
    } else if let Some(DataMode::Single) = app.mode {
        ui.separator();
        ui.heading("Normalization");
        ui.add_space(5.0);

        ui.horizontal(|ui| {
            ui.label("Baseline Format:");
            ComboBox::from_id_salt("baseline_combo")
                .selected_text(format!("{:?}", app.baseline_format))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.baseline_format, DataFormat::CSR, "CSR");
                    ui.selectable_value(&mut app.baseline_format, DataFormat::COO, "COO");
                    ui.selectable_value(&mut app.baseline_format, DataFormat::ELL, "ELL");
                    ui.selectable_value(&mut app.baseline_format, DataFormat::HYBRID, "HYBRID");
                    ui.selectable_value(&mut app.baseline_format, DataFormat::SELLP, "SELLP");
                });
        });

        ui.checkbox(&mut app.normalize, "Normalize to Baseline")
            .on_hover_text("Divide all values by the baseline format's value");
    } else {
        app.normalize = false;
    }
}

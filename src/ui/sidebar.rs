use crate::data::state::{Dashboard, SolverXAxis, ViewMode};
use crate::types::{DataFormat, DataMode, MetricType, PlotType, ProfileFilter, XaxisType};
use crate::visualization::formula;
use eframe::egui::{
    self, ComboBox, Context, DragValue, Grid, ProgressBar, RichText, SidePanel, Spinner, TextEdit,
    Ui,
};

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
        }
    });
}

/// Render the scrollable commit list. Each row is a selectable label that
/// updates `git.selected_commit_idx` on click.
fn render_commit_list(app: &mut Dashboard, ui: &mut Ui) {
    const MESSAGE_MAX: usize = 50;

    egui::ScrollArea::vertical()
        .max_height(220.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (idx, commit) in app.git.commits.iter().enumerate() {
                let selected = app.git.selected_commit_idx == Some(idx);
                let marker = if commit.bench_file.is_some() {
                    "✓"
                } else {
                    " "
                };
                let date_short = commit.date.get(..10).unwrap_or(&commit.date);
                let message = truncate_str(&commit.message, MESSAGE_MAX);
                let label = format!(
                    "{} [{}] {} — {}",
                    marker, commit.short_sha, date_short, message
                );
                let resp = ui.selectable_label(selected, label).on_hover_text(format!(
                    "{}\n{}\n{}",
                    commit.sha, commit.author, commit.message
                ));
                if resp.clicked() {
                    app.git.selected_commit_idx = Some(idx);
                }
            }
        });
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
                    });
                ui.end_row();

                render_axis_controls(app, ui);
            });
    });
    ui.add_space(5.0);

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

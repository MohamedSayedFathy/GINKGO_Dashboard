use crate::data::state::{Dashboard, SolverXAxis};
use crate::visualization::solver::{
    calculate_comparison_stats, get_plot_points, get_solver_colors,
};
use eframe::egui::{self, Color32, RichText, Ui};
use egui_plot::{Legend, Line, LineStyle, Plot};

pub fn render_solver_view(app: &mut Dashboard, ui: &mut Ui) {
    let solver_data = match app.solver.data.as_ref() {
        Some(data) if !data.is_empty() => data,
        Some(_) => {
            ui.label("Empty solver data.");
            return;
        }
        None => {
            ui.centered_and_justified(|ui| {
                ui.label(
                    "No solver data loaded. Use 'Import Solver Data' to load a solver.json file.",
                );
            });
            return;
        }
    };

    ui.horizontal(|ui| {
        ui.label("Benchmark:");
        let current_idx = app.solver.selected_idx;
        if current_idx >= solver_data.len() {
            app.solver.selected_idx = 0;
        }

        egui::ComboBox::from_id_salt("dataset_selector")
            .selected_text(format!(
                "{} - {}",
                solver_data[app.solver.selected_idx].stencil,
                solver_data[app.solver.selected_idx].size
            ))
            .show_ui(ui, |ui| {
                for (i, benchmark) in solver_data.iter().enumerate() {
                    ui.selectable_value(
                        &mut app.solver.selected_idx,
                        i,
                        format!("{} - {}", benchmark.stencil, benchmark.size),
                    );
                }
            });
    });

    let benchmark = &solver_data[app.solver.selected_idx];

    ui.separator();

    if app.solver.selected_methods.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.heading("📊 Solver Convergence Analysis");
            ui.add_space(10.0);
            ui.label("Step 1: Select benchmark problem above");
            ui.label("Step 2: Check one or more solver methods");
            ui.label("Step 3: Toggle residual types to display");
            ui.add_space(10.0);
            ui.add_space(10.0);
            ui.colored_label(
                Color32::from_rgb(150, 150, 150),
                "Tip: Select multiple solvers to compare their convergence behavior",
            );
        });
        return;
    }

    ui.add_space(10.0);

    // --- Convergence Plot (Balanced Height) ---
    let plot_height = ui.available_height() * 0.55;
    let x_axis_label = match app.solver.x_axis {
        SolverXAxis::Iteration => "Iteration",
        SolverXAxis::Time => "Time (s)",
    };

    Plot::new("convergence_plot")
        .height(plot_height)
        .x_axis_label(x_axis_label)
        .y_axis_label(if app.solver.log_scale {
            "Value (Log10)"
        } else {
            "Value"
        })
        .legend(Legend::default())
        .show(ui, |plot_ui| {
            let mut solver_idx = 0;
            let mut sorted_methods: Vec<_> = app.solver.selected_methods.iter().collect();
            sorted_methods.sort();

            for solver_method in sorted_methods {
                if let Some(result) = benchmark.solver.get(solver_method) {
                    let (recurrent_color, true_color, implicit_color) =
                        get_solver_colors(solver_idx);
                    solver_idx += 1;

                    if app.solver.show_recurrent {
                        if let Some(residuals) = &result.recurrent_residuals {
                            let points = get_plot_points(
                                residuals,
                                result.iteration_timestamps.as_ref(),
                                app.solver.x_axis,
                                app.solver.log_scale,
                            );
                            plot_ui.line(
                                Line::new(format!("{} (R)", solver_method), points)
                                    .color(recurrent_color),
                            );
                        }
                    }

                    if app.solver.show_true {
                        if let Some(residuals) = &result.true_residuals {
                            let points = get_plot_points(
                                residuals,
                                result.iteration_timestamps.as_ref(),
                                app.solver.x_axis,
                                app.solver.log_scale,
                            );
                            plot_ui.line(
                                Line::new(format!("{} (T)", solver_method), points)
                                    .color(true_color)
                                    .style(LineStyle::Dashed { length: 10.0 }),
                            );
                        }
                    }

                    if app.solver.show_implicit {
                        if let Some(residuals) = &result.implicit_residuals {
                            let points = get_plot_points(
                                residuals,
                                result.iteration_timestamps.as_ref(),
                                app.solver.x_axis,
                                app.solver.log_scale,
                            );
                            plot_ui.line(
                                Line::new(format!("{} (I)", solver_method), points)
                                    .color(implicit_color)
                                    .style(LineStyle::Dotted { spacing: 5.0 }),
                            );
                        }
                    }

                    // Cumulative Time - PURPLE
                    if app.solver.show_timestamp {
                        if let Some(times) = &result.iteration_timestamps {
                            let points = get_plot_points(
                                times,
                                result.iteration_timestamps.as_ref(),
                                app.solver.x_axis,
                                app.solver.log_scale,
                            );
                            plot_ui.line(
                                Line::new(format!("{} (Time)", solver_method), points)
                                    .color(Color32::from_rgb(147, 112, 219))
                                    .style(LineStyle::Solid),
                            );
                        }
                    }
                }
            }
        });

    ui.add_space(10.0);

    // --- Details Panel (Full Width, Tabbed) ---
    ui.separator();
    ui.heading("Solver Details");

    // Tabs for each selected solver
    ui.horizontal(|ui| {
        let sorted_methods: Vec<_> = app.solver.selected_methods.iter().collect();

        for method in &sorted_methods {
            let is_active = app
                .solver
                .selected_detail_method
                .as_ref()
                .map(|active| active == *method)
                .unwrap_or(false);

            if ui.selectable_label(is_active, *method).clicked() {
                app.solver.selected_detail_method = Some((*method).clone());
            }
        }
    });

    ui.separator();

    // Show details for active tab
    let active_method = app
        .solver
        .selected_detail_method
        .clone()
        .and_then(|method| {
            if app.solver.selected_methods.contains(&method) {
                Some(method)
            } else {
                None
            }
        })
        .or_else(|| app.solver.selected_methods.iter().next().cloned());

    if let Some(solver_method) = active_method {
        // Auto-select first solver if none selected
        if app.solver.selected_detail_method.is_none() {
            app.solver.selected_detail_method = Some(solver_method.clone());
        }

        if let Some(result) = benchmark.solver.get(solver_method.as_str()) {
            let stats = calculate_comparison_stats(
                &solver_method,
                result,
                &benchmark.solver,
                &app.solver.selected_methods,
            );

            // Use columns for compact horizontal layout
            ui.columns(4, |columns| {
                columns[0].vertical(|ui| {
                    ui.strong("Status");
                    ui.colored_label(
                        if result.completed {
                            Color32::from_rgb(0, 150, 0)
                        } else {
                            Color32::from_rgb(200, 0, 0)
                        },
                        if result.completed {
                            "✓ Converged"
                        } else {
                            "✗ Failed"
                        },
                    );

                    ui.add_space(10.0);
                    ui.strong("Iterations");
                    ui.label(format!("{}", result.apply.iterations.unwrap_or(0)));
                });

                columns[1].vertical(|ui| {
                    ui.strong("Generate Phase");
                    ui.label(format!("{:.4}s", result.generate.time));

                    ui.add_space(10.0);
                    ui.strong("Apply Phase");
                    ui.label(format!("{:.4}s", result.apply.time));
                });

                columns[2].vertical(|ui| {
                    ui.strong("Total Time");
                    ui.label(
                        RichText::new(format!("{:.4}s", stats.total_time))
                            .strong()
                            .size(16.0),
                    );

                    ui.add_space(10.0);
                    ui.strong("Final Residual");
                    if let Some(final_res) = stats.final_residual {
                        ui.label(format!("{:.2e}", final_res));
                    }
                });

                columns[3].vertical(|ui| {
                    ui.strong("Comparison");

                    if stats.is_fastest {
                        ui.label(
                            RichText::new("⭐ Fastest solver").color(Color32::from_rgb(0, 120, 0)),
                        );
                    } else {
                        ui.label(format!(
                            "{:.2}x slower than {}",
                            stats.slowdown_factor, stats.fastest_solver_name
                        ));
                    }
                });
            });
        }
    }
}

mod data;
mod types;
mod ui;
mod visualization;

#[cfg(test)]
mod vis_tests;

use crate::data::state::Dashboard;
use crate::ui::{charts, sidebar};
use eframe::{App, Frame};

impl App for Dashboard {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut Frame) {
        self.check_loading_status();

        sidebar::render_side_panel(self, ctx);

        eframe::egui::CentralPanel::default().show(ctx, |ui| {
            if self.view_mode == crate::data::state::ViewMode::Solver {
                crate::ui::solver_view::render_solver_view(self, ui);
            } else {
                charts::render_charts(self, ui);
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions::default();

    eframe::run_native(
        "GINKGO Dashboard",
        options,
        Box::new(|_ctx| Ok(Box::new(Dashboard::new()))),
    )
}

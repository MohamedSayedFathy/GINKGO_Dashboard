pub mod data;
pub mod state_url;
pub mod types;
pub mod ui;
pub mod visualization;

#[cfg(test)]
mod vis_tests;

#[cfg(feature = "datagen")]
pub mod datagen;

// Wasm entry point: `#[wasm_bindgen(start)]` function lives here. Gated to
// wasm32 so native builds are untouched. See the module docs in `wasm.rs`
// for the rationale on putting it in the library crate (cdylib) rather
// than the bin.
#[cfg(target_arch = "wasm32")]
mod wasm;

use data::state::{Dashboard, ViewMode};
use eframe::{App, Frame};

impl App for Dashboard {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut Frame) {
        self.check_loading_status();

        ui::sidebar::render_side_panel(self, ctx);

        eframe::egui::CentralPanel::default().show(ctx, |ui| {
            if self.view_mode == ViewMode::Solver {
                crate::ui::solver_view::render_solver_view(self, ui);
            } else {
                ui::charts::render_charts(self, ui);
            }
        });
    }
}

pub mod data;
pub mod export;
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

        // Commit-switcher keybinds (Task 7): `[` steps back, `]` steps
        // forward. Skip when a TextEdit (or any focusable widget) has the
        // keyboard — e.g. while editing the custom-formula input, `[` and
        // `]` must reach the text buffer unchanged.
        //
        // `ctx.wants_keyboard_input()` already wraps `memory.focused().is_some()`
        // in the pinned egui 0.32.3, so one check is enough.
        if !ctx.wants_keyboard_input() {
            let (step_fwd, step_back) = ctx.input(|i| {
                (
                    i.key_pressed(eframe::egui::Key::CloseBracket),
                    i.key_pressed(eframe::egui::Key::OpenBracket),
                )
            });
            if step_fwd {
                self.step_commit(1);
                if let Some(idx) = self.git.selected_commit_idx {
                    self.on_commit_selected(ctx, idx);
                }
            }
            if step_back {
                self.step_commit(-1);
                if let Some(idx) = self.git.selected_commit_idx {
                    self.on_commit_selected(ctx, idx);
                }
            }
        }

        // Opportunistically prefetch neighbouring commits after any selection
        // change. Guards inside the method make this cheap when the selection
        // hasn't moved or a load is already in flight.
        self.prefetch_adjacent_commits(ctx);

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

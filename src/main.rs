//! Fleet: a desktop window that runs omp, pi, and Claude Code sessions side by side,
//! grouped by project and task.

mod app;
mod db;
mod paths;
mod path_env;
mod sessions;
mod store;
mod tab;
mod terminal;
mod ui;

use app::AgentApp;
use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 680.0])
            .with_title("Fleet"),
        ..Default::default()
    };
    eframe::run_native(
        "Fleet",
        options,
        Box::new(|_cc| Ok(Box::new(AgentApp::new()))),
    )
}

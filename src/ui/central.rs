//! Central panel: the tab strip and the active agent's terminal.

use crate::app::{Action, AgentApp};
use crate::tab::Slot;
use eframe::egui;

const LIGHT_RED: egui::Color32 = egui::Color32::LIGHT_RED;

pub fn show(ctx: &egui::Context, app: &mut AgentApp, actions: &mut Vec<Action>) {
    egui::CentralPanel::default().show(ctx, |ui| {
        tab_strip(ui, app, actions);
        ui.separator();

        let Some(i) = app.active.filter(|&i| i < app.tabs.len()) else {
            ui.label("No tab open. Pick a session on the left or start a new one.");
            return;
        };
        let tab = &app.tabs[i];
        ui.horizontal(|ui| {
            ui.strong(tab.title.as_str());
            ui.weak(format!("{} · {}", tab.agent.label(), tab.cwd));
        });
        ui.weak(app.status.as_str());
        ui.separator();
        // The terminal takes whatever space remains below the header.
        terminal(ui, app.slots.get_mut(i), i, actions);
    });
}

fn tab_strip(ui: &mut egui::Ui, app: &AgentApp, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        for (i, tab) in app.tabs.iter().enumerate() {
            if ui
                .selectable_label(app.active == Some(i), tab.title.as_str())
                .clicked()
            {
                actions.push(Action::Activate(i));
            }
            if ui.small_button("x").clicked() {
                actions.push(Action::Close(i));
            }
            ui.separator();
        }
    });
}

fn terminal(ui: &mut egui::Ui, slot: Option<&mut Slot>, index: usize, actions: &mut Vec<Action>) {
    match slot {
        Some(Slot::Live(term)) if term.exited() => {
            ui.label("The agent has exited.");
            if ui.button("Restart").clicked() {
                actions.push(Action::Restart(index));
            }
        }
        Some(Slot::Live(term)) => term.show(ui),
        Some(Slot::Failed(msg)) => {
            ui.colored_label(LIGHT_RED, msg.as_str());
            if ui.button("Retry").clicked() {
                actions.push(Action::Restart(index));
            }
        }
        Some(Slot::Idle) | None => {
            ui.label("Starting…");
        }
    }
}

//! Left panel: the agent and task pickers, the folder, and the session list.

use crate::app::{Action, AgentApp};
use crate::sessions::{Agent, SessionInfo};
use eframe::egui;
use std::time::SystemTime;

pub fn show(ctx: &egui::Context, app: &AgentApp, actions: &mut Vec<Action>) {
    egui::SidePanel::left("sessions")
        .default_width(340.0)
        .show(ctx, |ui| {
            // Added first so the buttons stay pinned to the bottom of the panel.
            egui::TopBottomPanel::bottom("bottom_buttons").show_inside(ui, |ui| {
                ui.add_space(6.0);
                if full_width_button(ui, "Tasks") {
                    actions.push(Action::ShowTasks);
                }
                ui.add_space(4.0);
                if full_width_button(ui, "Projects") {
                    actions.push(Action::ShowProjects);
                }
                ui.add_space(6.0);
            });

            ui.heading("Agent sessions");
            ui.weak(format!("Project: {}", app.project_label()));
            agent_picker(ui, app.agent, actions);
            ui.horizontal(|ui| {
                ui.label("Folder");
                // Read-only: the folder follows the open project (or home), so it cannot drift elsewhere.
                ui.weak(app.new_cwd.as_str());
            });
            task_filter(ui, app, actions);
            ui.horizontal(|ui| {
                if ui.button(format!("New {} session", app.agent.label())).clicked() {
                    actions.push(Action::New);
                }
                if ui.button("Refresh").clicked() {
                    actions.push(Action::Refresh);
                }
            });

            ui.separator();
            let visible = app.visible_session_indices();
            ui.label(format!("{} session(s)", visible.len()));
            if visible.is_empty() && app.selected_task.is_some() {
                ui.weak(format!("No {} sessions linked to this task.", app.agent.label()));
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for i in visible {
                    session_row(ui, &app.sessions[i], i, actions);
                }
            });
        });
}

fn full_width_button(ui: &mut egui::Ui, label: &str) -> bool {
    ui.add_sized([ui.available_width(), 36.0], egui::Button::new(label))
        .clicked()
}

fn agent_picker(ui: &mut egui::Ui, current: Agent, actions: &mut Vec<Action>) {
    let mut agent = current;
    egui::ComboBox::from_label("Agent")
        .selected_text(current.label())
        .show_ui(ui, |ui| {
            for a in Agent::ALL {
                ui.selectable_value(&mut agent, a, a.label());
            }
        });
    if agent != current {
        actions.push(Action::SetAgent(agent));
    }
}

/// Task picker and the "Clear filter" button that returns the sidebar to all sessions.
fn task_filter(ui: &mut egui::Ui, app: &AgentApp, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label("Task");
        let title = app
            .selected_row()
            .map_or("All sessions", |r| r.task.title.as_str());
        egui::ComboBox::from_id_salt("task_filter")
            .selected_text(title)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(app.selected_task.is_none(), "All sessions")
                    .clicked()
                {
                    actions.push(Action::SelectTask(None));
                }
                for row in &app.task_rows {
                    let selected = app.selected_task == Some(row.task.id);
                    if ui.selectable_label(selected, row.task.title.as_str()).clicked() {
                        actions.push(Action::SelectTask(Some(row.task.id)));
                    }
                }
            });
        if ui
            .add_enabled(app.selected_task.is_some(), egui::Button::new("Clear filter"))
            .clicked()
        {
            actions.push(Action::SelectTask(None));
        }
    });
}

fn session_row(ui: &mut egui::Ui, s: &SessionInfo, index: usize, actions: &mut Vec<Action>) {
    ui.group(|ui| {
        ui.strong(s.name.as_str());
        let folder = if s.cwd.is_empty() {
            "(unknown folder)"
        } else {
            s.cwd.as_str()
        };
        ui.label(folder);
        ui.horizontal(|ui| {
            ui.weak(age(s.modified));
            if ui.button("Open").clicked() {
                actions.push(Action::Open(index));
            }
        });
    });
}

fn age(t: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(t)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

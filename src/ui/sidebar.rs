//! Left panel: the task filter, the agent picker, the folder, and the session list.
//! With a task selected, the list shows that task's linked sessions from the database.

use crate::app::{Action, AgentApp, TaskRow};
use crate::db::TaskSession;
use crate::sessions::{Agent, SessionInfo};
use crate::tab::Tab;
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
            task_filter(ui, app, actions);
            agent_picker(ui, app.agent, actions);
            ui.horizontal(|ui| {
                ui.label("Folder");
                // Read-only: the folder follows the open project (or home), so it cannot drift elsewhere.
                ui.weak(app.new_cwd.as_str());
            });
            ui.horizontal(|ui| {
                if ui.button(format!("New {} session", app.agent.label())).clicked() {
                    actions.push(Action::New);
                }
                if ui.button("Refresh").clicked() {
                    actions.push(Action::Refresh);
                }
            });

            ui.separator();
            match app.selected_row() {
                Some(row) => task_sessions(ui, row, actions),
                None => folder_sessions(ui, app, actions),
            }
        });
}

fn full_width_button(ui: &mut egui::Ui, label: &str) -> bool {
    ui.add_sized([ui.available_width(), 36.0], egui::Button::new(label))
        .clicked()
}

/// Task picker and the "Remove filter" button. With no task selected, the sidebar shows the folder's sessions.
fn task_filter(ui: &mut egui::Ui, app: &AgentApp, actions: &mut Vec<Action>) {
    ui.group(|ui| {
        ui.strong("Task filter");
        ui.horizontal(|ui| {
            let title = app
                .selected_row()
                .map_or("All sessions", |r| r.task.title.as_str());
            egui::ComboBox::from_id_salt("task_filter")
                .width(200.0)
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
            let enabled = app.selected_task.is_some();
            if ui
                .add_enabled(enabled, egui::Button::new("Remove filter"))
                .clicked()
            {
                actions.push(Action::SelectTask(None));
            }
        });
    });
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

/// The sessions linked to the selected task, from any agent and folder.
fn task_sessions(ui: &mut egui::Ui, row: &TaskRow, actions: &mut Vec<Action>) {
    ui.label(format!("{} linked session(s)", row.sessions.len()));
    if row.sessions.is_empty() {
        ui.weak("No sessions linked to this task yet. Attach one in Tasks, or start a new session.");
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
        for link in &row.sessions {
            task_session_row(ui, link, actions);
        }
    });
}

fn task_session_row(ui: &mut egui::Ui, link: &TaskSession, actions: &mut Vec<Action>) {
    ui.group(|ui| {
        ui.strong(link.title.as_str());
        let agent = Agent::from_key(&link.agent);
        let who = agent.map_or(link.agent.as_str(), |a| a.label());
        ui.label(format!("{who} · {}", link.cwd));
        if let Some(agent) = agent {
            if ui.button("Open").clicked() {
                let tab = Tab::resume(agent, &link.session_id, &link.cwd, &link.title);
                actions.push(Action::OpenTab(tab));
            }
        }
    });
}

/// The folder's sessions for the selected agent, newest first.
fn folder_sessions(ui: &mut egui::Ui, app: &AgentApp, actions: &mut Vec<Action>) {
    ui.label(format!("{} session(s)", app.sessions.len()));
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (i, s) in app.sessions.iter().enumerate() {
            session_row(ui, s, i, actions);
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

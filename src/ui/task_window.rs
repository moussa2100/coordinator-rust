//! The Tasks window: a list or a status board for the open project, with each task's sessions.

use crate::app::{Action, AgentApp, TaskRow};
use crate::db::{Status, TaskSession};
use crate::sessions::{Agent, SessionInfo};
use crate::tab::Tab;
use eframe::egui;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskView {
    List,
    Board,
}

const LIGHT_RED: egui::Color32 = egui::Color32::LIGHT_RED;

pub fn show(ctx: &egui::Context, app: &mut AgentApp, actions: &mut Vec<Action>) {
    if !app.show_tasks {
        return;
    }
    let mut open = true;
    egui::Window::new("Tasks")
        .open(&mut open)
        .default_width(720.0)
        .default_height(480.0)
        .show(ctx, |ui| window_body(ui, app, actions));
    app.show_tasks = open;
}

fn window_body(ui: &mut egui::Ui, app: &mut AgentApp, actions: &mut Vec<Action>) {
    header(ui, app, actions);
    ui.separator();

    if app.active_project.is_none() {
        ui.weak("Open a project to see and add its tasks.");
        return;
    }

    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.new_task_title)
                .hint_text("New task title")
                .desired_width(360.0),
        );
        if ui.button("Add task").clicked() {
            actions.push(Action::AddTask);
        }
    });
    if let Some(err) = app.task_error.as_deref() {
        ui.colored_label(LIGHT_RED, err);
    }
    ui.separator();

    let view = app.task_view;
    let rows: &[TaskRow] = &app.task_rows;
    let sidebar: &[SessionInfo] = &app.sessions;
    let picks = &mut app.attach_pick;
    match view {
        TaskView::List => list(ui, rows, sidebar, picks, actions),
        TaskView::Board => board(ui, rows, sidebar, picks, actions),
    }
}

/// View switch and the name of the open project.
fn header(ui: &mut egui::Ui, app: &AgentApp, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        if ui
            .selectable_label(app.task_view == TaskView::List, "List")
            .clicked()
        {
            actions.push(Action::SetView(TaskView::List));
        }
        if ui
            .selectable_label(app.task_view == TaskView::Board, "Board")
            .clicked()
        {
            actions.push(Action::SetView(TaskView::Board));
        }
        ui.separator();
        match app.active_project.as_ref() {
            Some(p) => ui.strong(p.name.as_str()),
            None => ui.weak("No project open"),
        };
    });
}

fn list(
    ui: &mut egui::Ui,
    rows: &[TaskRow],
    sidebar: &[SessionInfo],
    picks: &mut HashMap<i64, usize>,
    actions: &mut Vec<Action>,
) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        if rows.is_empty() {
            ui.weak("No tasks yet. Add one above.");
        }
        for row in rows {
            let pick = picks.entry(row.task.id).or_insert(0);
            task_card(ui, row, sidebar, pick, actions, false);
        }
    });
}

/// One column per status. Cards move between columns with the < and > buttons.
fn board(
    ui: &mut egui::Ui,
    rows: &[TaskRow],
    sidebar: &[SessionInfo],
    picks: &mut HashMap<i64, usize>,
    actions: &mut Vec<Action>,
) {
    ui.columns(3, |cols| {
        for (c, status) in Status::ALL.iter().enumerate() {
            let col = &mut cols[c];
            let in_column = rows.iter().filter(|r| r.task.status == *status);
            let count = in_column.clone().count();
            col.strong(format!("{} ({count})", status.label()));
            col.separator();
            egui::ScrollArea::vertical()
                .id_salt(("board_column", c))
                .show(col, |col| {
                    for row in in_column {
                        let pick = picks.entry(row.task.id).or_insert(0);
                        task_card(col, row, sidebar, pick, actions, true);
                    }
                });
        }
    });
}

/// One task: its title, status, linked sessions, and controls.
fn task_card(
    ui: &mut egui::Ui,
    row: &TaskRow,
    sidebar: &[SessionInfo],
    pick: &mut usize,
    actions: &mut Vec<Action>,
    board: bool,
) {
    let id = row.task.id;
    let status = row.task.status;
    ui.group(|ui| {
        ui.strong(row.task.title.as_str());
        if board {
            status_arrows(ui, id, status, actions);
        } else {
            status_combo(ui, id, status, actions);
        }
        for link in &row.sessions {
            linked_session(ui, id, link, actions);
        }
        attach_row(ui, id, sidebar, pick, actions);
        if ui.small_button("Delete task").clicked() {
            actions.push(Action::DeleteTask(id));
        }
    });
}

fn status_combo(ui: &mut egui::Ui, id: i64, status: Status, actions: &mut Vec<Action>) {
    egui::ComboBox::from_id_salt(("status", id))
        .selected_text(status.label())
        .show_ui(ui, |ui| {
            for s in Status::ALL {
                if ui.selectable_label(s == status, s.label()).clicked() {
                    actions.push(Action::SetStatus(id, s));
                }
            }
        });
}

fn status_arrows(ui: &mut egui::Ui, id: i64, status: Status, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        if ui.small_button("<").clicked() {
            actions.push(Action::SetStatus(id, status.shift(-1)));
        }
        ui.weak(status.label());
        if ui.small_button(">").clicked() {
            actions.push(Action::SetStatus(id, status.shift(1)));
        }
    });
}

fn linked_session(ui: &mut egui::Ui, task_id: i64, link: &TaskSession, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let agent = Agent::from_key(&link.agent);
        let who = agent.map_or_else(|| link.agent.clone(), |a| a.label().to_string());
        ui.label(format!("{who} · {}", link.title));
        if let Some(agent) = agent {
            if ui.small_button("Open").clicked() {
                actions.push(Action::OpenTab(Tab::resume(
                    agent,
                    &link.session_id,
                    &link.cwd,
                    &link.title,
                )));
            }
            if ui.small_button("Unlink").clicked() {
                actions.push(Action::Unlink(task_id, agent, link.session_id.clone()));
            }
        }
    });
}

/// Picker of sidebar sessions plus the Attach button that links the chosen one to the task.
fn attach_row(
    ui: &mut egui::Ui,
    id: i64,
    sidebar: &[SessionInfo],
    pick: &mut usize,
    actions: &mut Vec<Action>,
) {
    if sidebar.is_empty() {
        ui.weak("No sessions for this agent in the project folder to attach.");
        return;
    }
    let idx = (*pick).min(sidebar.len() - 1);
    *pick = idx;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(("attach", id))
            .width(200.0)
            .selected_text(sidebar[idx].name.as_str())
            .show_ui(ui, |ui| {
                for (i, s) in sidebar.iter().enumerate() {
                    ui.selectable_value(&mut *pick, i, s.name.as_str());
                }
            });
        if ui.small_button("Attach").clicked() {
            actions.push(Action::Attach(id, idx));
        }
    });
}

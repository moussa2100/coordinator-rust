//! The Projects window: the add-project form and the list of saved projects.

use crate::app::{Action, AgentApp};
use eframe::egui;

const LIGHT_GREEN: egui::Color32 = egui::Color32::LIGHT_GREEN;
const LIGHT_RED: egui::Color32 = egui::Color32::LIGHT_RED;

pub fn show(ctx: &egui::Context, app: &mut AgentApp, actions: &mut Vec<Action>) {
    if !app.show_projects {
        return;
    }
    let mut open = true;
    egui::Window::new("Projects")
        .open(&mut open)
        .default_width(460.0)
        .show(ctx, |ui| window_body(ui, app, actions));
    app.show_projects = open;
}

fn window_body(ui: &mut egui::Ui, app: &mut AgentApp, actions: &mut Vec<Action>) {
    // A single tab for now. The strip is here so more tabs can sit beside it later.
    ui.horizontal(|ui| {
        let _ = ui.selectable_label(true, "Projects");
    });
    ui.separator();

    add_form(ui, app, actions);
    if let Some(err) = app.project_error.as_deref() {
        ui.colored_label(LIGHT_RED, err);
    }
    ui.separator();

    project_list(ui, app, actions);
}

fn add_form(ui: &mut egui::Ui, app: &mut AgentApp, actions: &mut Vec<Action>) {
    ui.strong("Add a project");
    egui::Grid::new("add_project")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut app.new_project_name);
            ui.end_row();
            ui.label("Folder");
            ui.add(
                egui::TextEdit::singleline(&mut app.new_project_path)
                    .hint_text("e.g. C:/code/my-app"),
            );
            ui.end_row();
        });
    if ui.button("Add project").clicked() {
        actions.push(Action::AddProject);
    }
}

fn project_list(ui: &mut egui::Ui, app: &AgentApp, actions: &mut Vec<Action>) {
    ui.strong(format!("Saved projects ({})", app.projects.len()));
    egui::ScrollArea::vertical()
        .max_height(300.0)
        .show(ui, |ui| {
            if app.projects.is_empty() {
                ui.weak("No projects yet.");
            }
            for (i, p) in app.projects.iter().enumerate() {
                let is_open = app.active_project.as_ref().is_some_and(|a| a.id == p.id);
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.strong(p.name.as_str());
                        if is_open {
                            ui.colored_label(LIGHT_GREEN, "open");
                        }
                    });
                    ui.weak(p.path.as_str());
                    ui.horizontal(|ui| {
                        if ui.button("Open").clicked() {
                            actions.push(Action::OpenProject(i));
                        }
                        if ui.button("Remove").clicked() {
                            actions.push(Action::RemoveProject(i));
                        }
                    });
                });
            }
        });
}

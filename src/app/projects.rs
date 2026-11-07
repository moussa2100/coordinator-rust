//! Projects: saved folders, and the one open at a time that sets the working folder.

use super::AgentApp;
use crate::paths::home_string;
use std::path::Path;

impl AgentApp {
    pub(crate) fn reload_projects(&mut self) {
        self.projects = self.db.projects().unwrap_or_default();
    }

    /// Label for the current folder: the open project, or the home folder when none is open.
    pub(crate) fn project_label(&self) -> String {
        match &self.active_project {
            Some(p) => format!("{} · {}", p.name, p.path),
            None => format!("Home · {}", home_string()),
        }
    }

    pub(crate) fn show_projects_window(&mut self) {
        self.show_projects = true;
        self.project_error = None;
    }

    /// Makes a project the open one. Its folder becomes the default for new sessions,
    /// and the choice is saved so the next launch opens it.
    pub(crate) fn open_project(&mut self, index: usize) {
        let Some(project) = self.projects.get(index).cloned() else {
            return;
        };
        self.status = match self.db.set_last_project(project.id) {
            Ok(()) => format!("Opened project {}.", project.name),
            Err(e) => format!("Could not save project choice: {e}"),
        };
        self.new_cwd = project.path.clone();
        self.active_project = Some(project);
        self.show_projects = false;
        self.reload_tasks();
        self.refresh();
    }

    pub(crate) fn add_project(&mut self) {
        let name = self.new_project_name.trim().to_string();
        let path = self.new_project_path.trim().to_string();
        if let Some(problem) = self.project_problem(&name, &path) {
            self.project_error = Some(problem);
            return;
        }
        match self.db.add_project(&name, &path) {
            Ok(_) => {
                self.reload_projects();
                self.new_project_name.clear();
                self.new_project_path.clear();
                self.project_error = None;
            }
            Err(e) => self.project_error = Some(format!("Could not save project: {e}")),
        }
    }

    /// Removes a project from the list. Files on disk are not touched.
    pub(crate) fn remove_project(&mut self, index: usize) {
        let Some(project) = self.projects.get(index).cloned() else {
            return;
        };
        match self.db.remove_project(project.id) {
            Ok(()) => {
                self.reload_projects();
                if self.active_project.as_ref().is_some_and(|a| a.id == project.id) {
                    self.active_project = None;
                    self.new_cwd = home_string();
                    self.refresh();
                }
                self.reload_tasks();
                self.status = format!("Removed project {}.", project.name);
            }
            Err(e) => self.project_error = Some(format!("Could not remove project: {e}")),
        }
    }

    /// The reason a project cannot be added, or None when it can.
    fn project_problem(&self, name: &str, path: &str) -> Option<String> {
        if name.is_empty() {
            Some("Enter a project name.".to_string())
        } else if path.is_empty() {
            Some("Enter a folder path.".to_string())
        } else if !Path::new(path).is_dir() {
            Some(format!("That folder does not exist: {path}"))
        } else if self.projects.iter().any(|p| p.path == path) {
            Some("A project already uses that folder.".to_string())
        } else {
            None
        }
    }
}

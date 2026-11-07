//! Tasks of the open project: their linked sessions, the sidebar filter, and task writes.

use super::AgentApp;
use crate::db::{Status, Task, TaskSession};
use crate::sessions::Agent;

/// A task with the sessions linked to it, cached so the window does not query the database every frame.
pub struct TaskRow {
    pub task: Task,
    pub sessions: Vec<TaskSession>,
}

impl AgentApp {
    /// Reloads the tasks of the open project, with the sessions linked to each.
    pub(crate) fn reload_tasks(&mut self) {
        let Some(project_id) = self.active_project.as_ref().map(|p| p.id) else {
            self.task_rows.clear();
            self.selected_task = None;
            return;
        };
        let tasks = self.db.tasks(project_id).unwrap_or_default();
        self.task_rows = tasks
            .into_iter()
            .map(|task| {
                let sessions = self.db.sessions_for_task(task.id).unwrap_or_default();
                TaskRow { task, sessions }
            })
            .collect();
        if self.selected_row().is_none() {
            self.selected_task = None;
        }
    }

    /// The task the sidebar is filtered to, if any.
    pub(crate) fn selected_row(&self) -> Option<&TaskRow> {
        let id = self.selected_task?;
        self.task_rows.iter().find(|r| r.task.id == id)
    }

    /// Indices into `sessions` that the sidebar shows: all of them, or only those linked to the selected task.
    pub(crate) fn visible_session_indices(&self) -> Vec<usize> {
        let linked = self.selected_row().map(|r| r.sessions.as_slice());
        self.sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                linked.is_none_or(|links| {
                    links
                        .iter()
                        .any(|l| l.agent == s.agent.key() && l.session_id == s.id)
                })
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub(crate) fn show_tasks_window(&mut self) {
        self.show_tasks = true;
        self.task_error = None;
        self.reload_tasks();
    }

    pub(crate) fn add_task(&mut self) {
        let title = self.new_task_title.trim().to_string();
        let Some(project_id) = self.active_project.as_ref().map(|p| p.id) else {
            self.task_error = Some("Open a project before adding tasks.".to_string());
            return;
        };
        if title.is_empty() {
            self.task_error = Some("Enter a task title.".to_string());
            return;
        }
        match self.db.add_task(project_id, &title) {
            Ok(_) => {
                self.new_task_title.clear();
                self.task_error = None;
                self.reload_tasks();
            }
            Err(e) => self.task_error = Some(format!("Could not add task: {e}")),
        }
    }

    /// Links the sidebar session at `index` to a task. The same session can be linked to other tasks too.
    pub(crate) fn attach_session(&mut self, task_id: i64, index: usize) {
        let Some(s) = self.sessions.get(index).cloned() else {
            return;
        };
        match self
            .db
            .link_session(task_id, s.agent.key(), &s.id, &s.name, &s.cwd)
        {
            Ok(()) => {
                self.task_error = None;
                self.reload_tasks();
            }
            Err(e) => self.task_error = Some(format!("Could not attach session: {e}")),
        }
    }

    pub(crate) fn set_status(&mut self, task_id: i64, status: Status) {
        let result = self.db.set_status(task_id, status);
        self.finish_task_write(result, "update task");
    }

    /// Deletes the task and its links. The sessions themselves are not deleted.
    pub(crate) fn delete_task(&mut self, task_id: i64) {
        self.attach_pick.remove(&task_id);
        let result = self.db.delete_task(task_id);
        self.finish_task_write(result, "delete task");
    }

    pub(crate) fn unlink_session(&mut self, task_id: i64, agent: Agent, session_id: &str) {
        let result = self.db.unlink_session(task_id, agent.key(), session_id);
        self.finish_task_write(result, "unlink session");
    }

    /// Shows a failed task write in the Tasks window, then reloads the tasks either way.
    fn finish_task_write(&mut self, result: rusqlite::Result<()>, what: &str) {
        if let Err(e) = result {
            self.task_error = Some(format!("Could not {what}: {e}"));
        }
        self.reload_tasks();
    }
}

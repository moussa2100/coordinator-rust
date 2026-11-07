//! Everything the UI can ask the app to do. Widgets queue these; `apply` runs them after drawing.

use super::AgentApp;
use crate::db::Status;
use crate::sessions::Agent;
use crate::tab::Tab;
use crate::ui::task_window::TaskView;

pub enum Action {
    SetAgent(Agent),
    Refresh,
    New,
    Open(usize),
    Activate(usize),
    Close(usize),
    Restart(usize),
    ShowProjects,
    AddProject,
    OpenProject(usize),
    RemoveProject(usize),
    ShowTasks,
    SetView(TaskView),
    AddTask,
    SetStatus(i64, Status),
    DeleteTask(i64),
    /// Links the session at this index in the sidebar list to the task.
    Attach(i64, usize),
    Unlink(i64, Agent, String),
    OpenTab(Tab),
    SelectTask(Option<i64>),
}

impl AgentApp {
    pub(crate) fn apply(&mut self, action: Action) {
        match action {
            Action::SetAgent(agent) => {
                self.agent = agent;
                self.refresh();
            }
            Action::Refresh => {
                self.refresh();
                self.sync_new_sessions();
            }
            Action::New => self.new_session(),
            Action::Open(i) => self.open_session(i),
            Action::Activate(i) => self.activate_tab(i),
            Action::Close(i) => self.close_tab(i),
            Action::Restart(i) => self.restart_tab(i),
            Action::ShowProjects => self.show_projects_window(),
            Action::AddProject => self.add_project(),
            Action::OpenProject(i) => self.open_project(i),
            Action::RemoveProject(i) => self.remove_project(i),
            Action::ShowTasks => self.show_tasks_window(),
            Action::SetView(view) => self.task_view = view,
            Action::SelectTask(id) => self.selected_task = id,
            Action::AddTask => self.add_task(),
            Action::SetStatus(id, status) => self.set_status(id, status),
            Action::DeleteTask(id) => self.delete_task(id),
            Action::Attach(id, index) => self.attach_session(id, index),
            Action::Unlink(id, agent, session_id) => self.unlink_session(id, agent, &session_id),
            Action::OpenTab(tab) => self.open_tab(tab),
        }
    }
}

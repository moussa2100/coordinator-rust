//! Application state and the eframe loop. Behavior lives in the submodules; drawing lives in `ui`.

mod action;
mod projects;
mod tabs;
mod tasks;

pub use action::Action;
pub use tasks::TaskRow;

use crate::db::{Database, Project};
use crate::paths::{app_dir, db_path, home_string, store_path};
use crate::sessions::{self, Agent, SessionInfo};
use crate::store;
use crate::tab::{Slot, Tab};
use crate::ui;
use crate::ui::task_window::TaskView;
use eframe::egui;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

pub struct AgentApp {
    pub(crate) agent: Agent,
    pub(crate) sessions: Vec<SessionInfo>,
    pub(crate) tabs: Vec<Tab>,
    /// Parallel to `tabs`: slots[i] belongs to tabs[i].
    pub(crate) slots: Vec<Slot>,
    pub(crate) active: Option<usize>,
    pub(crate) new_cwd: String,
    pub(crate) status: String,
    pub(crate) tabs_path: PathBuf,
    pub(crate) db: Database,
    pub(crate) projects: Vec<Project>,
    /// The project opened last. None means the home folder.
    pub(crate) active_project: Option<Project>,
    pub(crate) show_projects: bool,
    pub(crate) new_project_name: String,
    pub(crate) new_project_path: String,
    pub(crate) project_error: Option<String>,
    pub(crate) task_rows: Vec<TaskRow>,
    pub(crate) show_tasks: bool,
    pub(crate) task_view: TaskView,
    pub(crate) new_task_title: String,
    pub(crate) task_error: Option<String>,
    /// Which sidebar session each task's "Attach" picker points at, keyed by task id.
    pub(crate) attach_pick: HashMap<i64, usize>,
    /// The task the sidebar is filtered to. None shows every session in the project folder.
    pub(crate) selected_task: Option<i64>,
    /// When pending new sessions were last checked for their transcript.
    pub(crate) last_link: Instant,
}

impl AgentApp {
    pub fn new() -> Self {
        let _ = fs::create_dir_all(app_dir());
        let tabs_path = store_path();
        let saved = store::load(&tabs_path);
        let slots: Vec<Slot> = saved.tabs.iter().map(|_| Slot::Idle).collect();
        let active = saved.active.filter(|&i| i < saved.tabs.len());

        let (db, db_error) = match Database::open(&db_path()) {
            Ok(db) => (db, None),
            Err(e) => (
                Database::in_memory().expect("in-memory database"),
                Some(format!("Projects will not be saved: {e}")),
            ),
        };
        let projects = db.projects().unwrap_or_default();
        let last_id = db.last_project_id().ok().flatten();
        let active_project = last_id.and_then(|id| projects.iter().find(|p| p.id == id).cloned());
        let new_cwd = active_project
            .as_ref()
            .map(|p| p.path.clone())
            .unwrap_or_else(home_string);

        let status = db_error.unwrap_or_else(|| {
            if saved.tabs.is_empty() {
                "No open tabs.".to_string()
            } else {
                format!("Restored {} tab(s).", saved.tabs.len())
            }
        });

        let mut app = Self {
            agent: Agent::Claude,
            sessions: Vec::new(),
            tabs: saved.tabs,
            slots,
            active,
            new_cwd,
            status,
            tabs_path,
            db,
            projects,
            active_project,
            show_projects: false,
            new_project_name: String::new(),
            new_project_path: String::new(),
            project_error: None,
            task_rows: Vec::new(),
            show_tasks: false,
            task_view: TaskView::List,
            new_task_title: String::new(),
            task_error: None,
            attach_pick: HashMap::new(),
            selected_task: None,
            last_link: Instant::now(),
        };
        app.refresh();
        app.reload_tasks();
        app
    }

    /// Reloads the session list for the agent and folder currently selected.
    pub(crate) fn refresh(&mut self) {
        self.sessions = sessions::list_sessions(self.agent, &self.new_cwd);
    }
}

impl eframe::App for AgentApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.start_active_tab();
        self.link_pending_sessions(ctx);

        // Widgets only queue actions; they run after every panel has been drawn.
        let mut actions: Vec<Action> = Vec::new();
        ui::sidebar::show(ctx, self, &mut actions);
        ui::task_window::show(ctx, self, &mut actions);
        ui::project_window::show(ctx, self, &mut actions);
        ui::central::show(ctx, self, &mut actions);
        for action in actions {
            self.apply(action);
        }
    }
}

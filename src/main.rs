mod db;
mod sessions;
mod terminal;

use db::{Database, Project, Status, Task, TaskSession};
use eframe::egui;
use serde::{Deserialize, Serialize};
use sessions::{Agent, SessionInfo};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use terminal::EmbeddedTerminal;

/// Initial pseudo-terminal size; the view resizes it to fit the window on the first frame.
const START_ROWS: u16 = 30;
const START_COLS: u16 = 100;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Tab {
    agent: Agent,
    /// None for a new session that has not been assigned an id yet.
    session_id: Option<String>,
    cwd: String,
    title: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    tabs: Vec<Tab>,
    active: Option<usize>,
}

/// The live agent behind a tab. Not saved: restored tabs start when they are activated.
enum Slot {
    Idle,
    Live(EmbeddedTerminal),
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskView {
    List,
    Board,
}

/// A task with the sessions linked to it, cached so the window does not query the database every frame.
struct TaskRow {
    task: Task,
    sessions: Vec<TaskSession>,
}

enum Action {
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
}

struct AgentApp {
    agent: Agent,
    sessions: Vec<SessionInfo>,
    tabs: Vec<Tab>,
    /// Parallel to `tabs`: slots[i] belongs to tabs[i].
    slots: Vec<Slot>,
    active: Option<usize>,
    new_cwd: String,
    status: String,
    store: PathBuf,
    db: Database,
    projects: Vec<Project>,
    /// The project opened last. None means the home folder.
    active_project: Option<Project>,
    show_projects: bool,
    new_project_name: String,
    new_project_path: String,
    project_error: Option<String>,
    task_rows: Vec<TaskRow>,
    show_tasks: bool,
    task_view: TaskView,
    new_task_title: String,
    task_error: Option<String>,
    /// Which sidebar session each task's "Attach" picker points at, keyed by task id.
    attach_pick: HashMap<i64, usize>,
}

impl AgentApp {
    fn new() -> Self {
        let _ = fs::create_dir_all(app_dir());
        let store = store_path();
        let saved = load_saved(&store);
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
            store,
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
        };
        app.refresh();
        app.reload_tasks();
        app
    }

    fn refresh(&mut self) {
        self.sessions = sessions::list_sessions(self.agent, &self.new_cwd);
    }

    fn reload_projects(&mut self) {
        self.projects = self.db.projects().unwrap_or_default();
    }

    /// Reloads the tasks of the open project, with the sessions linked to each.
    fn reload_tasks(&mut self) {
        let Some(project_id) = self.active_project.as_ref().map(|p| p.id) else {
            self.task_rows.clear();
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
    }

    /// Label for the current folder: the open project, or the home folder when none is open.
    fn project_label(&self) -> String {
        match &self.active_project {
            Some(p) => format!("{} · {}", p.name, p.path),
            None => format!("Home · {}", home_string()),
        }
    }

    fn persist(&mut self) {
        let saved = Saved {
            tabs: self.tabs.clone(),
            active: self.active,
        };
        if let Err(e) = save(&self.store, &saved) {
            self.status = format!("Could not save tabs: {e}");
        }
    }

    /// Activates the tab for this session, or adds one. The agent starts when the tab is shown.
    fn open_tab(&mut self, tab: Tab) {
        let existing = tab.session_id.as_ref().and_then(|id| {
            self.tabs
                .iter()
                .position(|t| t.agent == tab.agent && t.session_id.as_ref() == Some(id))
        });
        let index = match existing {
            Some(i) => i,
            None => {
                self.tabs.push(tab);
                self.slots.push(Slot::Idle);
                self.tabs.len() - 1
            }
        };
        self.active = Some(index);
        self.persist();
    }

    /// Spawns the agent for tab `index` inside the app window.
    fn start(&mut self, index: usize) {
        let Some(tab) = self.tabs.get(index).cloned() else {
            return;
        };
        let (program, args) = sessions::command_line(tab.agent, tab.session_id.as_deref());
        let cwd = if Path::new(&tab.cwd).is_dir() {
            tab.cwd.clone()
        } else {
            home_string()
        };
        let slot = match EmbeddedTerminal::spawn(&program, &args, &cwd, START_ROWS, START_COLS) {
            Ok(term) => Slot::Live(term),
            Err(e) => {
                self.status = e.clone();
                Slot::Failed(e)
            }
        };
        self.slots[index] = slot;
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        // Dropping a live slot ends its agent process.
        self.slots.remove(index);
        self.active = if self.tabs.is_empty() {
            None
        } else {
            self.active.map(|a| {
                if a > index {
                    a - 1
                } else {
                    a.min(self.tabs.len() - 1)
                }
            })
        };
        self.persist();
    }

    fn new_session(&mut self) {
        let cwd = self.new_cwd.trim().to_string();
        let tab = Tab {
            agent: self.agent,
            session_id: None,
            cwd,
            title: format!("New {}", self.agent.label()),
        };
        self.open_tab(tab);
    }

    /// Makes a project the open one. Its folder becomes the default for new sessions,
    /// and the choice is saved so the next launch opens it.
    fn open_project(&mut self, index: usize) {
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

    fn add_project(&mut self) {
        let name = self.new_project_name.trim().to_string();
        let path = self.new_project_path.trim().to_string();
        let problem = if name.is_empty() {
            Some("Enter a project name.".to_string())
        } else if path.is_empty() {
            Some("Enter a folder path.".to_string())
        } else if !Path::new(&path).is_dir() {
            Some(format!("That folder does not exist: {path}"))
        } else if self.projects.iter().any(|p| p.path == path) {
            Some("A project already uses that folder.".to_string())
        } else {
            None
        };
        if let Some(problem) = problem {
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

    fn remove_project(&mut self, index: usize) {
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

    fn add_task(&mut self) {
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
    fn attach_session(&mut self, task_id: i64, index: usize) {
        let Some(s) = self.sessions.get(index).cloned() else {
            return;
        };
        match self
            .db
            .link_session(task_id, agent_key(s.agent), &s.id, &s.name, &s.cwd)
        {
            Ok(()) => {
                self.task_error = None;
                self.reload_tasks();
            }
            Err(e) => self.task_error = Some(format!("Could not attach session: {e}")),
        }
    }
}

impl eframe::App for AgentApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The active tab's agent starts on first view, so restored tabs do not all launch at once.
        if let Some(i) = self.active {
            if matches!(self.slots.get(i), Some(Slot::Idle)) {
                self.start(i);
            }
        }

        let mut actions: Vec<Action> = Vec::new();
        let project_label = self.project_label();

        egui::SidePanel::left("sessions")
            .default_width(340.0)
            .show(ctx, |ui| {
                // Added first so the buttons stay pinned to the bottom of the panel.
                egui::TopBottomPanel::bottom("bottom_buttons").show_inside(ui, |ui| {
                    ui.add_space(6.0);
                    if ui
                        .add_sized([ui.available_width(), 36.0], egui::Button::new("Tasks"))
                        .clicked()
                    {
                        actions.push(Action::ShowTasks);
                    }
                    ui.add_space(4.0);
                    if ui
                        .add_sized([ui.available_width(), 36.0], egui::Button::new("Projects"))
                        .clicked()
                    {
                        actions.push(Action::ShowProjects);
                    }
                    ui.add_space(6.0);
                });

                ui.heading("Agent sessions");
                ui.weak(format!("Project: {project_label}"));

                let mut agent = self.agent;
                egui::ComboBox::from_label("Agent")
                    .selected_text(self.agent.label())
                    .show_ui(ui, |ui| {
                        for a in Agent::ALL {
                            ui.selectable_value(&mut agent, a, a.label());
                        }
                    });
                if agent != self.agent {
                    actions.push(Action::SetAgent(agent));
                }

                ui.horizontal(|ui| {
                    ui.label("Folder");
                    // Read-only: the folder follows the open project (or home), so it cannot drift elsewhere.
                    ui.weak(self.new_cwd.as_str());
                });
                ui.horizontal(|ui| {
                    if ui.button(format!("New {} session", self.agent.label())).clicked() {
                        actions.push(Action::New);
                    }
                    if ui.button("Refresh").clicked() {
                        actions.push(Action::Refresh);
                    }
                });

                ui.separator();
                ui.label(format!("{} session(s)", self.sessions.len()));
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, s) in self.sessions.iter().enumerate() {
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
                                    actions.push(Action::Open(i));
                                }
                            });
                        });
                    }
                });
            });

        // The Tasks window: List or Board, for the open project.
        if self.show_tasks {
            let AgentApp {
                show_tasks,
                task_rows,
                task_view,
                new_task_title,
                task_error,
                attach_pick,
                sessions,
                active_project,
                ..
            } = self;
            let view = *task_view;
            egui::Window::new("Tasks")
                .open(show_tasks)
                .default_width(720.0)
                .default_height(480.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(view == TaskView::List, "List").clicked() {
                            actions.push(Action::SetView(TaskView::List));
                        }
                        if ui.selectable_label(view == TaskView::Board, "Board").clicked() {
                            actions.push(Action::SetView(TaskView::Board));
                        }
                        ui.separator();
                        match active_project.as_ref() {
                            Some(p) => ui.strong(p.name.as_str()),
                            None => ui.weak("No project open"),
                        };
                    });
                    ui.separator();

                    if active_project.is_none() {
                        ui.weak("Open a project to see and add its tasks.");
                        return;
                    }

                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut *new_task_title)
                                .hint_text("New task title")
                                .desired_width(360.0),
                        );
                        if ui.button("Add task").clicked() {
                            actions.push(Action::AddTask);
                        }
                    });
                    if let Some(err) = task_error.as_deref() {
                        ui.colored_label(egui::Color32::LIGHT_RED, err);
                    }
                    ui.separator();

                    let rows: &[TaskRow] = task_rows.as_slice();
                    let sidebar: &[SessionInfo] = sessions.as_slice();
                    match view {
                        TaskView::List => {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                if rows.is_empty() {
                                    ui.weak("No tasks yet. Add one above.");
                                }
                                for row in rows {
                                    let pick = attach_pick.entry(row.task.id).or_insert(0);
                                    task_card(ui, row, sidebar, pick, &mut actions, false);
                                }
                            });
                        }
                        TaskView::Board => {
                            ui.columns(3, |cols| {
                                for (c, status) in Status::ALL.iter().enumerate() {
                                    let col = &mut cols[c];
                                    let count = rows.iter().filter(|r| r.task.status == *status).count();
                                    col.strong(format!("{} ({count})", status.label()));
                                    col.separator();
                                    egui::ScrollArea::vertical().id_salt(("board_column", c)).show(col, |col| {
                                        for row in rows.iter().filter(|r| r.task.status == *status) {
                                            let pick = attach_pick.entry(row.task.id).or_insert(0);
                                            task_card(col, row, sidebar, pick, &mut actions, true);
                                        }
                                    });
                                }
                            });
                        }
                    }
                });
        }

        // The Projects window: one tab for now, with the project list and an add form.
        if self.show_projects {
            let AgentApp {
                show_projects,
                projects,
                active_project,
                new_project_name,
                new_project_path,
                project_error,
                ..
            } = self;
            egui::Window::new("Projects")
                .open(show_projects)
                .default_width(460.0)
                .show(ctx, |ui| {
                    // A single tab for now. The strip is here so more tabs can sit beside it later.
                    ui.horizontal(|ui| {
                        let _ = ui.selectable_label(true, "Projects");
                    });
                    ui.separator();

                    ui.strong("Add a project");
                    egui::Grid::new("add_project").num_columns(2).show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut *new_project_name);
                        ui.end_row();
                        ui.label("Folder");
                        ui.add(
                            egui::TextEdit::singleline(&mut *new_project_path)
                                .hint_text("e.g. C:/code/my-app"),
                        );
                        ui.end_row();
                    });
                    if ui.button("Add project").clicked() {
                        actions.push(Action::AddProject);
                    }
                    if let Some(err) = project_error.as_deref() {
                        ui.colored_label(egui::Color32::LIGHT_RED, err);
                    }
                    ui.separator();

                    ui.strong(format!("Saved projects ({})", projects.len()));
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            if projects.is_empty() {
                                ui.weak("No projects yet.");
                            }
                            for (i, p) in projects.iter().enumerate() {
                                let is_open = active_project.as_ref().is_some_and(|a| a.id == p.id);
                                ui.group(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.strong(p.name.as_str());
                                        if is_open {
                                            ui.colored_label(egui::Color32::LIGHT_GREEN, "open");
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
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, tab) in self.tabs.iter().enumerate() {
                    if ui
                        .selectable_label(self.active == Some(i), tab.title.as_str())
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
            ui.separator();

            match self.active.and_then(|i| self.tabs.get(i).cloned().map(|t| (i, t))) {
                None => {
                    ui.label("No tab open. Pick a session on the left or start a new one.");
                }
                Some((i, tab)) => {
                    ui.horizontal(|ui| {
                        ui.strong(tab.title.as_str());
                        ui.weak(format!("{} · {}", tab.agent.label(), tab.cwd));
                    });
                    ui.weak(self.status.as_str());
                    ui.separator();
                    // The terminal takes whatever space remains below the header.
                    match self.slots.get_mut(i) {
                        Some(Slot::Live(term)) if term.exited() => {
                            ui.label("The agent has exited.");
                            if ui.button("Restart").clicked() {
                                actions.push(Action::Restart(i));
                            }
                        }
                        Some(Slot::Live(term)) => term.show(ui),
                        Some(Slot::Failed(msg)) => {
                            ui.colored_label(egui::Color32::LIGHT_RED, msg.as_str());
                            if ui.button("Retry").clicked() {
                                actions.push(Action::Restart(i));
                            }
                        }
                        Some(Slot::Idle) | None => {
                            ui.label("Starting…");
                        }
                    }
                }
            }
        });

        for action in actions {
            match action {
                Action::SetAgent(agent) => {
                    self.agent = agent;
                    self.refresh();
                }
                Action::Refresh => self.refresh(),
                Action::New => self.new_session(),
                Action::Open(i) => {
                    if let Some(s) = self.sessions.get(i).cloned() {
                        let tab = session_tab(&s, &self.new_cwd);
                        self.open_tab(tab);
                    }
                }
                Action::Activate(i) => {
                    self.active = Some(i);
                    self.persist();
                }
                Action::Close(i) => self.close_tab(i),
                Action::Restart(i) => {
                    if i < self.slots.len() {
                        self.slots[i] = Slot::Idle;
                    }
                }
                Action::ShowProjects => {
                    self.show_projects = true;
                    self.project_error = None;
                }
                Action::AddProject => self.add_project(),
                Action::OpenProject(i) => self.open_project(i),
                Action::RemoveProject(i) => self.remove_project(i),
                Action::ShowTasks => {
                    self.show_tasks = true;
                    self.task_error = None;
                    self.reload_tasks();
                }
                Action::SetView(view) => self.task_view = view,
                Action::AddTask => self.add_task(),
                Action::SetStatus(id, status) => {
                    if let Err(e) = self.db.set_status(id, status) {
                        self.task_error = Some(format!("Could not update task: {e}"));
                    }
                    self.reload_tasks();
                }
                Action::DeleteTask(id) => {
                    self.attach_pick.remove(&id);
                    if let Err(e) = self.db.delete_task(id) {
                        self.task_error = Some(format!("Could not delete task: {e}"));
                    }
                    self.reload_tasks();
                }
                Action::Attach(id, index) => self.attach_session(id, index),
                Action::Unlink(id, agent, session_id) => {
                    if let Err(e) = self.db.unlink_session(id, agent_key(agent), &session_id) {
                        self.task_error = Some(format!("Could not unlink session: {e}"));
                    }
                    self.reload_tasks();
                }
                Action::OpenTab(tab) => self.open_tab(tab),
            }
        }
    }
}

/// One task: its title, status, linked sessions, and controls. On the board, status moves with < and >.
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
            ui.horizontal(|ui| {
                if ui.small_button("<").clicked() {
                    actions.push(Action::SetStatus(id, status.shift(-1)));
                }
                ui.weak(status.label());
                if ui.small_button(">").clicked() {
                    actions.push(Action::SetStatus(id, status.shift(1)));
                }
            });
        } else {
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

        for link in &row.sessions {
            ui.horizontal(|ui| {
                let agent = agent_from_key(&link.agent);
                let who = agent.map_or(link.agent.clone(), |a| a.label().to_string());
                ui.label(format!("{who} · {}", link.title));
                if let Some(agent) = agent {
                    if ui.small_button("Open").clicked() {
                        actions.push(Action::OpenTab(Tab {
                            agent,
                            session_id: Some(link.session_id.clone()),
                            cwd: link.cwd.clone(),
                            title: link.title.clone(),
                        }));
                    }
                    if ui.small_button("Unlink").clicked() {
                        actions.push(Action::Unlink(id, agent, link.session_id.clone()));
                    }
                }
            });
        }

        if sidebar.is_empty() {
            ui.weak("No sessions for this agent in the project folder to attach.");
        } else {
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

        if ui.small_button("Delete task").clicked() {
            actions.push(Action::DeleteTask(id));
        }
    });
}

fn session_tab(s: &SessionInfo, fallback_cwd: &str) -> Tab {
    Tab {
        agent: s.agent,
        session_id: Some(s.id.clone()),
        cwd: if s.cwd.is_empty() {
            fallback_cwd.to_string()
        } else {
            s.cwd.clone()
        },
        title: s.name.clone(),
    }
}

fn agent_key(agent: Agent) -> &'static str {
    match agent {
        Agent::Omp => "omp",
        Agent::Pi => "pi",
        Agent::Claude => "claude",
    }
}

fn agent_from_key(key: &str) -> Option<Agent> {
    match key {
        "omp" => Some(Agent::Omp),
        "pi" => Some(Agent::Pi),
        "claude" => Some(Agent::Claude),
        _ => None,
    }
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

fn home_string() -> String {
    dirs::home_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

/// Folder for the app's data: tabs.json and the SQLite database.
fn app_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("orca-agents")
}

fn store_path() -> PathBuf {
    app_dir().join("tabs.json")
}

fn db_path() -> PathBuf {
    app_dir().join("orca-agents.db")
}

fn load_saved(path: &PathBuf) -> Saved {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(path: &PathBuf, saved: &Saved) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(saved).map_err(std::io::Error::other)?;
    fs::write(path, json)
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 680.0])
            .with_title("Orca Agents"),
        ..Default::default()
    };
    eframe::run_native(
        "Orca Agents",
        options,
        Box::new(|_cc| Ok(Box::new(AgentApp::new()))),
    )
}

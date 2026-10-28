mod sessions;
mod terminal;

use eframe::egui;
use serde::{Deserialize, Serialize};
use sessions::{Agent, SessionInfo};
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

enum Action {
    SetAgent(Agent),
    Refresh,
    New,
    Open(usize),
    Activate(usize),
    Close(usize),
    Restart(usize),
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
}

impl AgentApp {
    fn new() -> Self {
        let store = store_path();
        let saved = load_saved(&store);
        let slots: Vec<Slot> = saved.tabs.iter().map(|_| Slot::Idle).collect();
        let active = saved.active.filter(|&i| i < saved.tabs.len());
        let status = if saved.tabs.is_empty() {
            "No open tabs.".to_string()
        } else {
            format!("Restored {} tab(s).", saved.tabs.len())
        };
        let mut app = Self {
            agent: Agent::Claude,
            sessions: Vec::new(),
            tabs: saved.tabs,
            slots,
            active,
            new_cwd: home_string(),
            status,
            store,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        self.sessions = sessions::list_sessions(self.agent);
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

        egui::SidePanel::left("sessions")
            .default_width(340.0)
            .show(ctx, |ui| {
                ui.heading("Agent sessions");

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
                    ui.text_edit_singleline(&mut self.new_cwd);
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
                            ui.strong(format!("{} · {}", s.agent.label(), short_id(&s.id)));
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
            }
        }
    }
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
        title: format!("{} · {}", s.agent.label(), short_id(&s.id)),
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
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

fn store_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("orca-agents")
        .join("tabs.json")
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

//! Open tabs: starting and closing agents, activating tabs, and linking new sessions to tasks.

use super::AgentApp;
use crate::paths::home_string;
use crate::sessions::{self, SessionInfo};
use crate::store::{self, Saved};
use crate::tab::{Slot, Tab};
use crate::terminal::EmbeddedTerminal;
use eframe::egui;
use std::path::Path;
use std::time::{Duration, Instant};

/// Initial pseudo-terminal size; the view resizes it to fit the window on the first frame.
const START_ROWS: u16 = 30;
const START_COLS: u16 = 100;
/// How often new sessions started under a task are checked for their transcript.
const LINK_EVERY: Duration = Duration::from_secs(2);

impl AgentApp {
    /// Saves the open tabs and which one is active.
    pub(crate) fn persist(&mut self) {
        let saved = Saved {
            tabs: self.tabs.clone(),
            active: self.active,
        };
        if let Err(e) = store::save(&self.tabs_path, &saved) {
            self.status = format!("Could not save tabs: {e}");
        }
    }

    /// Activates the tab for this session, or adds one. The agent starts when the tab is shown.
    pub(crate) fn open_tab(&mut self, tab: Tab) {
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

    pub(crate) fn activate_tab(&mut self, index: usize) {
        self.active = Some(index);
        self.persist();
    }

    /// Opens a tab for a session from the sidebar. Sessions with no recorded folder use the project folder.
    pub(crate) fn open_session(&mut self, index: usize) {
        let Some(s) = self.sessions.get(index) else {
            return;
        };
        let cwd = if s.cwd.is_empty() {
            self.new_cwd.as_str()
        } else {
            s.cwd.as_str()
        };
        let tab = Tab::resume(s.agent, &s.id, cwd, &s.name);
        self.open_tab(tab);
    }

    pub(crate) fn restart_tab(&mut self, index: usize) {
        if index < self.slots.len() {
            self.slots[index] = Slot::Idle;
        }
    }

    pub(crate) fn close_tab(&mut self, index: usize) {
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

    /// Opens a tab for a new session in the current folder. Under a selected task, the folder's
    /// existing sessions are recorded so the new transcript can be told apart later.
    pub(crate) fn new_session(&mut self) {
        let cwd = self.new_cwd.trim().to_string();
        let (task_id, known_ids) = match self.selected_task {
            Some(id) => (
                Some(id),
                sessions::list_sessions(self.agent, &cwd)
                    .into_iter()
                    .map(|s| s.id)
                    .collect(),
            ),
            None => (None, Vec::new()),
        };
        self.open_tab(Tab::new_session(self.agent, cwd, task_id, known_ids));
    }

    /// Spawns the agent for tab `index` inside the app window.
    pub(crate) fn start(&mut self, index: usize) {
        let Some(tab) = self.tabs.get(index) else {
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

    /// The active tab's agent starts on first view, so restored tabs do not all launch at once.
    pub(crate) fn start_active_tab(&mut self) {
        if let Some(i) = self
            .active
            .filter(|&i| matches!(self.slots.get(i), Some(Slot::Idle)))
        {
            self.start(i);
        }
    }

    /// True while a new session started under a task has no linked transcript yet.
    fn pending_links(&self) -> bool {
        self.tabs
            .iter()
            .any(|t| t.task_id.is_some() && t.session_id.is_none())
    }

    /// Checks pending new sessions every `LINK_EVERY` and keeps the window repainting until none remain.
    pub(crate) fn link_pending_sessions(&mut self, ctx: &egui::Context) {
        if !self.pending_links() {
            return;
        }
        ctx.request_repaint_after(LINK_EVERY);
        if self.last_link.elapsed() >= LINK_EVERY {
            self.last_link = Instant::now();
            self.sync_new_sessions();
        }
    }

    /// Links each pending new session to its task once its transcript appears.
    /// Refreshes the sidebar and task list when something was linked.
    pub(crate) fn sync_new_sessions(&mut self) {
        let mut linked = false;
        for i in 0..self.tabs.len() {
            let tab = &self.tabs[i];
            let Some(task_id) = tab.task_id.filter(|_| tab.session_id.is_none()) else {
                continue;
            };
            let agent = tab.agent;
            let found = sessions::list_sessions(agent, &tab.cwd);
            let Some(s) = only_new(&tab.known_ids, &found) else {
                continue;
            };
            match self
                .db
                .link_session(task_id, agent.key(), &s.id, &s.name, &s.cwd)
            {
                Ok(()) => {
                    let tab = &mut self.tabs[i];
                    tab.session_id = Some(s.id);
                    tab.title = s.name;
                    tab.task_id = None;
                    tab.known_ids.clear();
                    linked = true;
                }
                Err(e) => self.status = format!("Could not link new session to task: {e}"),
            }
        }
        if linked {
            self.persist();
            self.reload_tasks();
            self.refresh();
        }
    }
}

/// The one session in `found` that is not in `known`. None when there are none or several,
/// so an ambiguous match is left pending rather than linked to the wrong session.
fn only_new(known: &[String], found: &[SessionInfo]) -> Option<SessionInfo> {
    let mut new = found.iter().filter(|s| !known.contains(&s.id));
    let first = new.next()?;
    if new.next().is_some() {
        None
    } else {
        Some(first.clone())
    }
}

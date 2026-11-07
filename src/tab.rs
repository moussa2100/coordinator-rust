//! A tab is one agent session shown in the app. Its terminal (`Slot`) exists only at runtime.

use crate::sessions::Agent;
use crate::terminal::EmbeddedTerminal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub agent: Agent,
    /// None for a new session that has not been assigned an id yet.
    pub session_id: Option<String>,
    pub cwd: String,
    pub title: String,
    /// Task a new session belongs to. Cleared once its transcript is linked to that task.
    #[serde(default)]
    pub task_id: Option<i64>,
    /// Session ids that existed when a new session under a task started.
    #[serde(default)]
    pub known_ids: Vec<String>,
}

impl Tab {
    /// A tab that resumes an existing session by id.
    pub fn resume(agent: Agent, session_id: &str, cwd: &str, title: &str) -> Self {
        Self {
            agent,
            session_id: Some(session_id.to_string()),
            cwd: cwd.to_string(),
            title: title.to_string(),
            task_id: None,
            known_ids: Vec::new(),
        }
    }

    /// A tab for a session not started yet. Under `task_id`, the session is linked to that task
    /// once its transcript appears as the only one not listed in `known_ids`.
    pub fn new_session(agent: Agent, cwd: String, task_id: Option<i64>, known_ids: Vec<String>) -> Self {
        Self {
            agent,
            session_id: None,
            cwd,
            title: format!("New {}", agent.label()),
            task_id,
            known_ids,
        }
    }
}

/// The live agent behind a tab. Not saved: restored tabs start when they are activated.
pub enum Slot {
    Idle,
    Live(EmbeddedTerminal),
    Failed(String),
}

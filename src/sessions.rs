//! Session discovery and command lines for omp, pi, and Claude Code.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Omp,
    Pi,
    Claude,
}

impl Agent {
    pub const ALL: [Agent; 3] = [Agent::Omp, Agent::Pi, Agent::Claude];

    pub fn label(self) -> &'static str {
        match self {
            Agent::Omp => "omp",
            Agent::Pi => "pi",
            Agent::Claude => "Claude Code",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Agent::Omp => "omp",
            Agent::Pi => "pi",
            Agent::Claude => "claude",
        }
    }

    /// Flag that resumes one session by id.
    fn resume_flag(self) -> &'static str {
        match self {
            Agent::Omp | Agent::Claude => "--resume",
            Agent::Pi => "--session",
        }
    }

    /// Directory that holds this agent's transcripts.
    fn sessions_root(self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        let configured = std::env::var("PI_CODING_AGENT_DIR")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        match self {
            Agent::Omp => Some(
                configured
                    .unwrap_or_else(|| home.join(".omp").join("agent"))
                    .join("sessions"),
            ),
            Agent::Pi => Some(
                configured
                    .unwrap_or_else(|| home.join(".pi").join("agent"))
                    .join("sessions"),
            ),
            Agent::Claude => Some(home.join(".claude").join("projects")),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub agent: Agent,
    pub id: String,
    pub cwd: String,
    pub modified: SystemTime,
}

/// Newest-first sessions for one agent. Missing directories yield an empty list.
pub fn list_sessions(agent: Agent) -> Vec<SessionInfo> {
    let Some(root) = agent.sessions_root() else {
        return Vec::new();
    };
    let mut files = Vec::new();
    collect_jsonl(&root, 3, &mut files);

    let mut sessions: Vec<SessionInfo> = files
        .into_iter()
        .filter_map(|path| {
            let modified = fs::metadata(&path).ok()?.modified().ok()?;
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            let (header_id, cwd) = read_header(&path);
            // Claude names each transcript after its session id; pi and omp write the id in the header.
            let id = match agent {
                Agent::Claude => stem,
                _ => header_id.unwrap_or(stem),
            };
            Some(SessionInfo {
                agent,
                id,
                cwd: cwd.unwrap_or_default(),
                modified,
            })
        })
        .collect();

    sessions.sort_by(|a, b| b.modified.cmp(&a.modified));
    sessions.truncate(200);
    sessions
}

fn collect_jsonl(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl(&path, depth - 1, out);
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            out.push(path);
        }
    }
}

/// Reads the session id and working folder from the first lines of a transcript.
fn read_header(path: &Path) -> (Option<String>, Option<String>) {
    let Ok(file) = fs::File::open(path) else {
        return (None, None);
    };
    let mut id = None;
    let mut cwd = None;
    for line in BufReader::new(file).lines().take(25).map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if cwd.is_none() {
            cwd = value.get("cwd").and_then(|v| v.as_str()).map(String::from);
        }
        if id.is_none() {
            id = value.get("id").and_then(|v| v.as_str()).map(String::from);
        }
        if id.is_some() && cwd.is_some() {
            break;
        }
    }
    (id, cwd)
}

/// Program and arguments that start the agent, resuming `session_id` when given.
/// On Windows the agent runs through `cmd /C` so npm-installed `.cmd` shims resolve.
pub fn command_line(agent: Agent, session_id: Option<&str>) -> (String, Vec<String>) {
    let mut args = Vec::new();
    if let Some(id) = session_id {
        args.push(agent.resume_flag().to_string());
        args.push(id.to_string());
    }
    if cfg!(windows) {
        let mut wrapped = vec!["/C".to_string(), agent.command().to_string()];
        wrapped.extend(args);
        ("cmd".to_string(), wrapped)
    } else {
        (agent.command().to_string(), args)
    }
}

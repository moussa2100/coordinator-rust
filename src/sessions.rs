//! Session discovery and command lines for omp, pi, and Claude Code.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Longest name shown in the list and tabs before it is cut short.
const MAX_NAME_CHARS: usize = 60;

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

    /// Stored form of the agent in the database. Matches the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Agent::Omp => "omp",
            Agent::Pi => "pi",
            Agent::Claude => "claude",
        }
    }

    pub fn from_key(key: &str) -> Option<Agent> {
        Agent::ALL.into_iter().find(|a| a.key() == key)
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
    /// What the list shows: the session's title, else its first user message, else a short id.
    pub name: String,
    pub cwd: String,
    pub modified: SystemTime,
}

/// Newest-first sessions for one agent that were recorded in `folder`.
/// Missing directories yield an empty list. Sessions with no recorded folder are left out,
/// since they cannot be matched to one.
pub fn list_sessions(agent: Agent, folder: &str) -> Vec<SessionInfo> {
    let Some(root) = agent.sessions_root() else {
        return Vec::new();
    };
    let wanted = normalize_folder(folder);
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut files = Vec::new();
    collect_jsonl(&root, 3, &mut files);

    let mut sessions: Vec<SessionInfo> = files
        .into_iter()
        .filter_map(|path| {
            let (header_id, cwd) = read_header(&path);
            // The folder filter runs before the name scan, so only matching files are read in full.
            let cwd = cwd.filter(|c| normalize_folder(c) == wanted)?;
            let modified = fs::metadata(&path).ok()?.modified().ok()?;
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            // Claude names each transcript after its session id; pi and omp write the id in the header.
            let id = match agent {
                Agent::Claude => stem,
                _ => header_id.unwrap_or(stem),
            };
            let name = session_name(&path).unwrap_or_else(|| id.chars().take(8).collect());
            Some(SessionInfo {
                agent,
                id,
                name,
                cwd,
                modified,
            })
        })
        .collect();

    sessions.sort_by(|a, b| b.modified.cmp(&a.modified));
    sessions.truncate(200);
    sessions
}

/// The name a transcript should show. Omp writes a `title` record and pi writes a
/// `session_info` name; the last one wins because later records update it. Without either,
/// the first user message is used. Returns None when the file has no usable text.
fn session_name(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut title: Option<String> = None;
    let mut first_user: Option<String> = None;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        // Substring checks first, so most lines are never parsed as JSON.
        let is_title = line.contains("\"type\":\"title\"");
        let is_info = line.contains("\"type\":\"session_info\"");
        let is_user = first_user.is_none() && line.contains("\"role\":\"user\"");
        if !is_title && !is_info && !is_user {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if is_title || is_info {
            let key = if is_title { "title" } else { "name" };
            if let Some(name) = value
                .get(key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                title = Some(name.to_string());
            }
        } else {
            first_user = user_text(&value);
        }
    }
    title.or(first_user)
}

/// Text of a user message, shortened for display. Handles string and array content.
fn user_text(value: &serde_json::Value) -> Option<String> {
    let message = value.get("message")?;
    if message.get("role")?.as_str()? != "user" {
        return None;
    }
    let text = match message.get("content")? {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    tidy(&text)
}

/// The first line with real words, stripped of markdown marks and cut to `MAX_NAME_CHARS`.
fn tidy(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(|l| l.trim().trim_matches(|c: char| "#*_`:>-".contains(c)).trim())
        .find(|l| l.chars().any(char::is_alphanumeric))?;
    let short: String = line.chars().take(MAX_NAME_CHARS).collect();
    if line.chars().count() > MAX_NAME_CHARS {
        Some(format!("{short}…"))
    } else {
        Some(short)
    }
}

/// A folder path that compares equal across slash style, trailing slashes, and (on Windows) case.
fn normalize_folder(path: &str) -> String {
    let mut p = path.trim().replace('\\', "/");
    while p.len() > 1 && p.ends_with('/') {
        p.pop();
    }
    if cfg!(windows) {
        p.make_ascii_lowercase();
    }
    p
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
    // Without this, omp moves a session started in the home folder into a temp directory.
    if agent == Agent::Omp {
        args.push("--allow-home".to_string());
    }
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

//! Open tabs saved to tabs.json, so they survive a restart.

use crate::tab::Tab;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Default, Serialize, Deserialize)]
pub struct Saved {
    pub tabs: Vec<Tab>,
    pub active: Option<usize>,
}

/// Reads the saved tabs. A missing or unreadable file yields no tabs.
pub fn load(path: &Path) -> Saved {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, saved: &Saved) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(saved).map_err(std::io::Error::other)?;
    fs::write(path, json)
}

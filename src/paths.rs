//! Locations of the app's data files and the home folder.

use std::path::PathBuf;

/// Folder for the app's data: tabs.json and the SQLite database.
pub fn app_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("orca-agents")
}

pub fn store_path() -> PathBuf {
    app_dir().join("tabs.json")
}

pub fn db_path() -> PathBuf {
    app_dir().join("orca-agents.db")
}

/// The home folder as text, or empty when it is unknown.
pub fn home_string() -> String {
    dirs::home_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

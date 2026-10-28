//! SQLite storage for projects and the last opened project.

use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub path: String,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// A database that lives only in memory. Used when the file cannot be opened.
    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS projects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            CREATE TABLE IF NOT EXISTS app_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;
        Ok(Self { conn })
    }

    /// All projects, sorted by name.
    pub fn projects(&self) -> rusqlite::Result<Vec<Project>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, path FROM projects ORDER BY name COLLATE NOCASE, id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Project {
                id: r.get(0)?,
                name: r.get(1)?,
                path: r.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn add_project(&self, name: &str, path: &str) -> rusqlite::Result<Project> {
        self.conn.execute(
            "INSERT INTO projects (name, path) VALUES (?1, ?2)",
            params![name, path],
        )?;
        Ok(Project {
            id: self.conn.last_insert_rowid(),
            name: name.to_string(),
            path: path.to_string(),
        })
    }

    /// Removes the project entry. Files on disk are not touched.
    /// If this project was the last one opened, the remembered choice is cleared too.
    pub fn remove_project(&self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM projects WHERE id = ?1", params![id])?;
        self.conn.execute(
            "DELETE FROM app_state WHERE key = 'last_project_id' AND value = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    pub fn set_last_project(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO app_state (key, value) VALUES ('last_project_id', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// The id of the project opened last, if one is remembered.
    pub fn last_project_id(&self) -> rusqlite::Result<Option<i64>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM app_state WHERE key = 'last_project_id'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value.and_then(|v| v.parse().ok()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_round_trip() {
        let db = Database::in_memory().unwrap();
        assert!(db.projects().unwrap().is_empty());
        assert_eq!(db.last_project_id().unwrap(), None);

        let a = db.add_project("Alpha", "C:/a").unwrap();
        let b = db.add_project("beta", "C:/b").unwrap();
        let names: Vec<String> = db.projects().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["Alpha".to_string(), "beta".to_string()]);

        db.set_last_project(b.id).unwrap();
        assert_eq!(db.last_project_id().unwrap(), Some(b.id));
        db.set_last_project(a.id).unwrap();
        assert_eq!(db.last_project_id().unwrap(), Some(a.id));

        db.remove_project(a.id).unwrap();
        assert_eq!(db.projects().unwrap().len(), 1);
        assert_eq!(db.last_project_id().unwrap(), None);
    }

    #[test]
    fn duplicate_path_is_rejected() {
        let db = Database::in_memory().unwrap();
        db.add_project("one", "C:/same").unwrap();
        assert!(db.add_project("two", "C:/same").is_err());
    }

    #[test]
    fn data_survives_reopen() {
        let path = std::env::temp_dir().join(format!("orca-agents-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let db = Database::open(&path).unwrap();
            let p = db.add_project("Saved", "C:/saved").unwrap();
            db.set_last_project(p.id).unwrap();
        }
        let db = Database::open(&path).unwrap();
        let projects = db.projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(db.last_project_id().unwrap(), Some(projects[0].id));
        drop(db);
        let _ = std::fs::remove_file(&path);
    }
}

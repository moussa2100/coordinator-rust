//! SQLite storage for projects, tasks, task-session links, and the last opened project.

use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub path: String,
}

/// Task status. It changes only when the user moves a task, never from session activity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Todo,
    Doing,
    Done,
}

impl Status {
    pub const ALL: [Status; 3] = [Status::Todo, Status::Doing, Status::Done];

    pub fn label(self) -> &'static str {
        match self {
            Status::Todo => "To do",
            Status::Doing => "Doing",
            Status::Done => "Done",
        }
    }

    /// The status `delta` columns away, stopping at the first and last column.
    pub fn shift(self, delta: i32) -> Status {
        let i = Status::ALL.iter().position(|s| *s == self).unwrap_or(0) as i32;
        Status::ALL[(i + delta).clamp(0, 2) as usize]
    }

    fn as_db(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Doing => "doing",
            Status::Done => "done",
        }
    }

    fn from_db(value: &str) -> Status {
        match value {
            "doing" => Status::Doing,
            "done" => Status::Done,
            _ => Status::Todo,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub status: Status,
}

/// A session linked to a task. `agent` is the stored key: "omp", "pi", or "claude".
#[derive(Clone, Debug, PartialEq)]
pub struct TaskSession {
    pub agent: String,
    pub session_id: String,
    pub title: String,
    pub cwd: String,
}

pub struct Database {
    conn: Connection,
}

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS projects (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL,
        path TEXT NOT NULL UNIQUE,
        created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
    );
    CREATE TABLE IF NOT EXISTS app_state (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS tasks (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        title TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'todo',
        created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
    );
    CREATE INDEX IF NOT EXISTS tasks_by_project ON tasks(project_id);
    -- One row per (task, session). A session can appear under several tasks.
    CREATE TABLE IF NOT EXISTS task_sessions (
        task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
        agent TEXT NOT NULL,
        session_id TEXT NOT NULL,
        title TEXT NOT NULL,
        cwd TEXT NOT NULL,
        PRIMARY KEY (task_id, agent, session_id)
    );
";

impl Database {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// A database that lives only in memory. Used when the file cannot be opened.
    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        // Off by default in SQLite. Needed so deleting a project removes its tasks.
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(SCHEMA)?;
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

    /// Removes the project, its tasks, and their session links. Files on disk are not touched.
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

    /// Tasks in one project, oldest first.
    pub fn tasks(&self, project_id: i64) -> rusqlite::Result<Vec<Task>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, title, status FROM tasks WHERE project_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map(params![project_id], |r| {
            let status: String = r.get(2)?;
            Ok(Task {
                id: r.get(0)?,
                title: r.get(1)?,
                status: Status::from_db(&status),
            })
        })?;
        rows.collect()
    }

    pub fn add_task(&self, project_id: i64, title: &str) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO tasks (project_id, title) VALUES (?1, ?2)",
            params![project_id, title],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn set_status(&self, task_id: i64, status: Status) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE tasks SET status = ?1 WHERE id = ?2",
            params![status.as_db(), task_id],
        )?;
        Ok(())
    }

    /// Deletes the task and its links. The sessions themselves are not deleted.
    pub fn delete_task(&self, task_id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM tasks WHERE id = ?1", params![task_id])?;
        Ok(())
    }

    /// Sessions linked to one task, in the order they were linked.
    pub fn sessions_for_task(&self, task_id: i64) -> rusqlite::Result<Vec<TaskSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT agent, session_id, title, cwd FROM task_sessions WHERE task_id = ?1 ORDER BY rowid",
        )?;
        let rows = stmt.query_map(params![task_id], |r| {
            Ok(TaskSession {
                agent: r.get(0)?,
                session_id: r.get(1)?,
                title: r.get(2)?,
                cwd: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Links a session to a task. Linking the same session to the same task again does nothing.
    pub fn link_session(
        &self,
        task_id: i64,
        agent: &str,
        session_id: &str,
        title: &str,
        cwd: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO task_sessions (task_id, agent, session_id, title, cwd)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![task_id, agent, session_id, title, cwd],
        )?;
        Ok(())
    }

    pub fn unlink_session(
        &self,
        task_id: i64,
        agent: &str,
        session_id: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM task_sessions WHERE task_id = ?1 AND agent = ?2 AND session_id = ?3",
            params![task_id, agent, session_id],
        )?;
        Ok(())
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
        let path = std::env::temp_dir().join(format!("fleet-test-{}.db", std::process::id()));
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

    #[test]
    fn task_sessions_link_to_many_tasks_and_cascade() {
        let db = Database::in_memory().unwrap();
        let p = db.add_project("P", "C:/p").unwrap();
        let t1 = db.add_task(p.id, "first").unwrap();
        let t2 = db.add_task(p.id, "second").unwrap();

        db.link_session(t1, "pi", "s1", "Title", "C:/p").unwrap();
        db.link_session(t2, "pi", "s1", "Title", "C:/p").unwrap();
        db.link_session(t2, "pi", "s1", "Title", "C:/p").unwrap();
        assert_eq!(db.sessions_for_task(t1).unwrap().len(), 1);
        assert_eq!(db.sessions_for_task(t2).unwrap().len(), 1);

        db.set_status(t1, Status::Done).unwrap();
        assert_eq!(db.tasks(p.id).unwrap()[0].status, Status::Done);
        assert_eq!(db.tasks(p.id).unwrap()[1].status, Status::Todo);

        db.unlink_session(t2, "pi", "s1").unwrap();
        assert!(db.sessions_for_task(t2).unwrap().is_empty());
        assert_eq!(db.sessions_for_task(t1).unwrap().len(), 1);

        db.remove_project(p.id).unwrap();
        assert!(db.tasks(p.id).unwrap().is_empty());
        assert!(db.sessions_for_task(t1).unwrap().is_empty());
    }

    #[test]
    fn status_shift_stops_at_ends() {
        assert_eq!(Status::Todo.shift(-1), Status::Todo);
        assert_eq!(Status::Todo.shift(1), Status::Doing);
        assert_eq!(Status::Done.shift(1), Status::Done);
    }
}

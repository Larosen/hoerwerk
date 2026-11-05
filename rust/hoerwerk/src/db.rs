//! SQLite – the same file, schema and migrations as the Python version (contract/schema.sql).
//! One connection behind a mutex: Hörwerk writes a few times per second at most, SQLite serialises writers anyway.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001.sql"),
    include_str!("../migrations/0002.sql"),
    include_str!("../migrations/0003.sql"),
    include_str!("../migrations/0004.sql"),
    include_str!("../migrations/0005.sql"),
];

pub struct Db {
    conn: Mutex<Connection>,
}

pub type Result<T> = rusqlite::Result<T>;

impl Db {
    pub fn open(path: &Path) -> std::result::Result<Self, String> {
        let new = !path.exists();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let conn = Connection::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if new {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)); // API keys inside
        }
        conn.busy_timeout(std::time::Duration::from_secs(30))
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| e.to_string())?;
        let db = Db { conn: Mutex::new(conn) };
        db.migrate()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn memory() -> Self {
        let db = Db {
            conn: Mutex::new(Connection::open_in_memory().unwrap()),
        };
        db.migrate().unwrap();
        db
    }

    fn migrate(&self) -> std::result::Result<(), String> {
        let mut c = self.lock();
        let version: i64 = c
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if version as usize > MIGRATIONS.len() {
            return Err(format!(
                "Database is newer (version {version}) than this program – please update / Datenbank ist neuer als dieses Programm"
            ));
        }
        for (n, script) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = c.transaction().map_err(|e| e.to_string())?;
            // statement by statement, as the Python version runs them
            for stmt in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                tx.execute(stmt, []).map_err(|e| format!("migration {}: {e}", n + 1))?;
            }
            tx.pragma_update(None, "user_version", (n + 1) as i64)
                .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A JSON document from the key/value table.
    pub fn get(&self, key: &str) -> Option<Value> {
        let c = self.lock();
        let text: Option<String> = c
            .query_row("SELECT value FROM kv WHERE key=?", [key], |r| r.get(0))
            .optional()
            .ok()
            .flatten();
        text.and_then(|t| serde_json::from_str(&t).ok())
    }

    pub fn put(&self, key: &str, value: &Value) -> Result<()> {
        self.lock().execute(
            "INSERT INTO kv(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, compact(value)],
        )?;
        Ok(())
    }

    /// Everything into the main file and close (the shutdown path).
    pub fn close(&self) {
        let c = self.lock();
        let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
    }
}

/// JSON as the Python version writes it: compact, UTF-8 kept.
pub fn compact(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_database_has_the_current_schema() {
        let db = Db::memory();
        let v: i64 = db.lock().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
        db.put("x", &serde_json::json!({"ä": 1})).unwrap();
        assert_eq!(db.get("x").unwrap()["ä"], 1);
    }
}

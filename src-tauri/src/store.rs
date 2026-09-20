use crate::model::*;
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Store {
    pub conn: Connection,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(dir.join("usage.sqlite3")).map_err(|e| e.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(|e| e.to_string())?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA secure_delete=ON;
             CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS alerts (provider TEXT, account TEXT, window_id TEXT, resets INTEGER, threshold INTEGER, UNIQUE(provider,account,window_id,resets,threshold));
             CREATE TABLE IF NOT EXISTS sessions (session TEXT PRIMARY KEY, account TEXT NOT NULL, digest TEXT NOT NULL, at INTEGER NOT NULL, snapshot TEXT NOT NULL, revoked INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS account_guard (provider TEXT PRIMARY KEY, account TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS bridge (id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL);
             DROP INDEX IF EXISTS history_lookup;
             DROP TABLE IF EXISTS history;
             DROP TABLE IF EXISTS history_cutoff;",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    pub fn settings(&self) -> Settings {
        self.conn
            .query_row("SELECT json FROM settings WHERE id=1", [], |row| {
                row.get::<_, String>(0)
            })
            .ok()
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default()
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO settings VALUES(1,?)",
                [serde_json::to_string(settings).map_err(|e| e.to_string())?],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn housekeeping(&self, timestamp: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM alerts WHERE resets < ?", [timestamp - 86400])
            .map_err(|e| e.to_string())?;
        self.conn
            .execute(
                "DELETE FROM sessions WHERE at < ?",
                [timestamp - 30 * 86400],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn claim_alert(
        &self,
        snapshot: &Snapshot,
        window: &UsageWindow,
        threshold: i64,
    ) -> Result<bool, String> {
        let Some(account) = &snapshot.account_id else {
            return Ok(false);
        };
        let rows = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO alerts VALUES(?,?,?,?,?)",
                params![
                    snapshot.provider,
                    account,
                    window.id,
                    window.resets_at,
                    threshold
                ],
            )
            .map_err(|e| e.to_string())?;
        if threshold == 10 {
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO alerts VALUES(?,?,?,?,20)",
                    params![snapshot.provider, account, window.id, window.resets_at],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(rows > 0)
    }

    pub fn unclaim_alert(&self, snapshot: &Snapshot, window: &UsageWindow, threshold: i64) {
        let _ = self.conn.execute(
            "DELETE FROM alerts WHERE provider=? AND account=? AND window_id=? AND resets=? AND threshold=?",
            params![snapshot.provider, snapshot.account_id, window.id, window.resets_at, threshold],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_delivery_is_deduplicated() {
        let dir = tempfile::tempdir().unwrap();
        let db = Store::open(dir.path()).unwrap();
        let mut snapshot = Snapshot::empty("codex", "ready", "");
        snapshot.account_id = Some("A".into());
        snapshot.windows = vec![UsageWindow {
            id: "5h".into(),
            label: "Five hours".into(),
            used_percent: Some(90.),
            resets_at: Some(now() + 1000),
            duration_mins: None,
        }];
        assert!(db.claim_alert(&snapshot, &snapshot.windows[0], 10).unwrap());
        assert!(!db.claim_alert(&snapshot, &snapshot.windows[0], 10).unwrap());
        assert!(!db.claim_alert(&snapshot, &snapshot.windows[0], 20).unwrap());
    }

    #[test]
    fn obsolete_usage_tables_are_deleted_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = Connection::open(dir.path().join("usage.sqlite3")).unwrap();
        legacy
            .execute_batch(
                "CREATE TABLE history (at INTEGER); CREATE TABLE history_cutoff (at INTEGER);",
            )
            .unwrap();
        drop(legacy);
        let db = Store::open(dir.path()).unwrap();
        let count: i64 = db.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('history','history_cutoff')",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 0);
    }
}

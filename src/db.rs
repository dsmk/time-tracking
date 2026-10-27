use std::path::Path;

use rusqlite::Connection;

use crate::error::{AppError, ErrorKind, Result};

/// Ordered schema migrations. Each runs exactly once, inside a transaction,
/// and its version is recorded in `schema_migrations`.
const MIGRATIONS: &[(i64, &str)] = &[(
    1,
    r#"
CREATE TABLE entries (
    id          TEXT    PRIMARY KEY NOT NULL,
    start_at    TEXT    NOT NULL,
    end_at      TEXT,
    description TEXT    NOT NULL,
    project     TEXT,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL,
    revision    INTEGER NOT NULL DEFAULT 1,
    deleted_at  TEXT,
    timezone    TEXT    NOT NULL,
    -- 1 while this entry is the live running timer, NULL otherwise. SQLite
    -- treats NULLs in a unique index as distinct, so the index below allows
    -- any number of stopped or deleted entries but only one running timer,
    -- even across concurrent processes.
    running_marker INTEGER GENERATED ALWAYS AS (
        CASE WHEN end_at IS NULL AND deleted_at IS NULL THEN 1 END
    ) VIRTUAL,
    CHECK (length(trim(description)) > 0),
    CHECK (end_at IS NULL OR end_at > start_at),
    CHECK (revision >= 1)
);

CREATE UNIQUE INDEX entries_one_running ON entries (running_marker);
CREATE INDEX entries_start_at ON entries (start_at);
CREATE INDEX entries_live_span ON entries (deleted_at, start_at, end_at);
CREATE INDEX entries_project ON entries (project COLLATE NOCASE);

CREATE TABLE entry_tags (
    entry_id TEXT NOT NULL REFERENCES entries (id) ON DELETE CASCADE,
    tag      TEXT NOT NULL,
    PRIMARY KEY (entry_id, tag),
    CHECK (length(trim(tag)) > 0)
);

-- A tag may appear only once per entry, compared case-insensitively.
CREATE UNIQUE INDEX entry_tags_unique_ci ON entry_tags (entry_id, tag COLLATE NOCASE);
CREATE INDEX entry_tags_tag ON entry_tags (tag COLLATE NOCASE);
"#,
)];

/// Open (creating if needed) the database at `path` and bring the schema up to
/// date. Opening never deletes or truncates an existing file.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::new(
                ErrorKind::Database,
                format!("cannot create database directory {}: {e}", parent.display()),
            )
            .with_hint("choose a writable location with --db PATH or TT_DB")
        })?;
    }

    let conn = Connection::open(path).map_err(|e| {
        AppError::new(
            ErrorKind::Database,
            format!("cannot open database {}: {e}", path.display()),
        )
        .with_hint("check that the path exists and is writable, or pass --db PATH")
    })?;

    conn.pragma_update(None, "foreign_keys", "ON")?;
    // WAL plus a busy timeout lets concurrent CLI invocations queue instead of
    // failing outright, without any process holding a lock between commands.
    let _: String =
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    conn.pragma_update(None, "busy_timeout", "5000")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;

    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    INTEGER PRIMARY KEY,
             applied_at TEXT NOT NULL
         );",
    )?;

    for (version, sql) in MIGRATIONS {
        let already: i64 = conn.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = ?1",
            [version],
            |row| row.get(0),
        )?;
        if already > 0 {
            continue;
        }
        // Each migration, and the record that it ran, commit together.
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let applied = (|| -> rusqlite::Result<()> {
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                rusqlite::params![version, jiff::Timestamp::now().to_string()],
            )?;
            Ok(())
        })();
        match applied {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(err) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(AppError::new(
                    ErrorKind::Database,
                    format!("migration {version} failed: {err}"),
                ));
            }
        }
    }
    Ok(())
}

pub fn schema_version(conn: &Connection) -> Result<i64> {
    let version: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_the_schema_and_is_safe_to_repeat() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("tt.db");

        let conn = open(&path).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), 1);
        conn.execute(
            "INSERT INTO entries (id, start_at, end_at, description, created_at, updated_at, \
             revision, timezone) VALUES ('a', '2026-10-27T13:00:00Z', '2026-10-27T14:00:00Z', \
             'kept', '2026-10-27T13:00:00Z', '2026-10-27T13:00:00Z', 1, 'UTC')",
            [],
        )
        .unwrap();
        drop(conn);

        // Re-opening must migrate idempotently and preserve existing rows.
        let conn = open(&path).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), 1);
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1, "init must not discard existing data");
    }

    #[test]
    fn only_one_running_entry_is_possible() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("tt.db")).unwrap();
        let insert = "INSERT INTO entries (id, start_at, end_at, description, created_at, \
                      updated_at, revision, timezone) VALUES (?1, '2026-10-27T13:00:00Z', NULL, \
                      'running', '2026-10-27T13:00:00Z', '2026-10-27T13:00:00Z', 1, 'UTC')";
        conn.execute(insert, ["a"]).unwrap();
        assert!(conn.execute(insert, ["b"]).is_err(), "second running timer must be rejected");

        // Soft-deleting the first frees the slot.
        conn.execute("UPDATE entries SET deleted_at = '2026-10-27T15:00:00Z' WHERE id = 'a'", [])
            .unwrap();
        conn.execute(insert, ["b"]).unwrap();
    }

    #[test]
    fn check_constraints_reject_invalid_rows() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("tt.db")).unwrap();
        let blank = "INSERT INTO entries (id, start_at, end_at, description, created_at, \
                     updated_at, revision, timezone) VALUES ('x', '2026-10-27T13:00:00Z', NULL, \
                     '   ', '2026-10-27T13:00:00Z', '2026-10-27T13:00:00Z', 1, 'UTC')";
        assert!(conn.execute(blank, []).is_err(), "blank description must be rejected");

        let backwards = "INSERT INTO entries (id, start_at, end_at, description, created_at, \
                         updated_at, revision, timezone) VALUES ('y', '2026-10-27T14:00:00Z', \
                         '2026-10-27T13:00:00Z', 'backwards', '2026-10-27T13:00:00Z', \
                         '2026-10-27T13:00:00Z', 1, 'UTC')";
        assert!(conn.execute(backwards, []).is_err(), "end before start must be rejected");
    }

    #[test]
    fn tags_are_unique_per_entry_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("tt.db")).unwrap();
        conn.execute(
            "INSERT INTO entries (id, start_at, end_at, description, created_at, updated_at, \
             revision, timezone) VALUES ('a', '2026-10-27T13:00:00Z', '2026-10-27T14:00:00Z', \
             'x', '2026-10-27T13:00:00Z', '2026-10-27T13:00:00Z', 1, 'UTC')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO entry_tags (entry_id, tag) VALUES ('a', 'Incident')", [])
            .unwrap();
        assert!(
            conn.execute("INSERT INTO entry_tags (entry_id, tag) VALUES ('a', 'incident')", [])
                .is_err()
        );
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("tt.db")).unwrap();
        assert!(
            conn.execute("INSERT INTO entry_tags (entry_id, tag) VALUES ('ghost', 't')", [])
                .is_err()
        );
    }
}

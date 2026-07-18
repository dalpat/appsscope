//! SQLite schema and migration.
//!
//! The catalog is a *cache*, not a source of truth — if anything about it looks
//! wrong we delete it and re-ingest. That's why there's no migration ladder:
//! [`SCHEMA_VERSION`] bumps, the old database is dropped, and ingest re-runs.

use rusqlite::Connection;

/// Bump on any schema change. Mismatch wipes and rebuilds the catalog.
pub const SCHEMA_VERSION: i64 = 1;

/// FTS5 is the whole point of using SQLite here: search-as-you-type over ~3000
/// apps has to stay under a frame budget, which rules out scanning.
///
/// The FTS table is external-content (`content='app'`) so app text isn't stored
/// twice, kept in sync by the triggers below.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS app (
    id            INTEGER PRIMARY KEY,
    backend       TEXT NOT NULL,
    app_id        TEXT NOT NULL,
    origin        TEXT,
    name          TEXT NOT NULL,
    summary       TEXT,
    description   TEXT,
    developer     TEXT,
    license       TEXT,
    version       TEXT,
    icon_path     TEXT,
    categories    TEXT NOT NULL DEFAULT '',
    keywords      TEXT NOT NULL DEFAULT '',
    UNIQUE (backend, app_id, origin)
);

CREATE INDEX IF NOT EXISTS app_by_name ON app (name COLLATE NOCASE);

CREATE TABLE IF NOT EXISTS screenshot (
    app          INTEGER NOT NULL REFERENCES app(id) ON DELETE CASCADE,
    url          TEXT NOT NULL,
    caption      TEXT,
    is_default   INTEGER NOT NULL DEFAULT 0,
    position     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS screenshot_by_app ON screenshot (app, position);

CREATE TABLE IF NOT EXISTS release (
    app          INTEGER NOT NULL REFERENCES app(id) ON DELETE CASCADE,
    version      TEXT NOT NULL,
    timestamp    INTEGER,
    description  TEXT,
    position     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS release_by_app ON release (app, position);

CREATE VIRTUAL TABLE IF NOT EXISTS app_fts USING fts5 (
    name,
    summary,
    keywords,
    app_id,
    content='app',
    content_rowid='id',
    tokenize="unicode61 remove_diacritics 2"
);

CREATE TRIGGER IF NOT EXISTS app_fts_insert AFTER INSERT ON app BEGIN
    INSERT INTO app_fts (rowid, name, summary, keywords, app_id)
    VALUES (new.id, new.name, new.summary, new.keywords, new.app_id);
END;

CREATE TRIGGER IF NOT EXISTS app_fts_delete AFTER DELETE ON app BEGIN
    INSERT INTO app_fts (app_fts, rowid, name, summary, keywords, app_id)
    VALUES ('delete', old.id, old.name, old.summary, old.keywords, old.app_id);
END;

CREATE TRIGGER IF NOT EXISTS app_fts_update AFTER UPDATE ON app BEGIN
    INSERT INTO app_fts (app_fts, rowid, name, summary, keywords, app_id)
    VALUES ('delete', old.id, old.name, old.summary, old.keywords, old.app_id);
    INSERT INTO app_fts (rowid, name, summary, keywords, app_id)
    VALUES (new.id, new.name, new.summary, new.keywords, new.app_id);
END;

CREATE TABLE IF NOT EXISTS source (
    key           TEXT PRIMARY KEY,
    fingerprint   TEXT NOT NULL
);
"#;

/// Create the schema, wiping first if it was written by an older version.
pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    let found: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    if found != 0 && found != SCHEMA_VERSION {
        tracing::info!(found, expected = SCHEMA_VERSION, "catalog schema outdated, rebuilding");
        conn.execute_batch(
            "DROP TABLE IF EXISTS app_fts;
             DROP TABLE IF EXISTS screenshot;
             DROP TABLE IF EXISTS release;
             DROP TABLE IF EXISTS source;
             DROP TABLE IF EXISTS app;",
        )?;
    }

    conn.execute_batch(SCHEMA)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

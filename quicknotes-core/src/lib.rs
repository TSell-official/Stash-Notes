//! # quicknotes-core
//!
//! Shared storage engine for **QuickNotes**. One SQLite database (with an FTS5
//! full-text index kept in sync by triggers) backs every surface of the
//! project: the Tauri desktop app, the `quicknotes` CLI, and the
//! `quicknotes-mcp` Model Context Protocol server.
//!
//! Because all three binaries link this crate, a note created from the CLI is
//! instantly searchable in the desktop app and visible to AI agents over MCP.
//!
//! ## Board layout
//!
//! The sticky-note board layout (position / color / size per device) lives in
//! a companion JSON file next to the database — see [`board`] — so the
//! relational schema stays clean while drag-and-drop state persists locally.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod board;

/// A full note as stored in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    pub title: String,
    pub content: String,
    /// Comma-separated tag list, e.g. `"mcp,setup"`.
    pub tags: String,
    pub pinned: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A lightweight note summary used in lists and search results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteBrief {
    pub id: i64,
    pub title: String,
    pub snippet: String,
    pub tags: String,
    pub updated_at: i64,
}

/// Resolve the database path.
///
/// Priority:
/// 1. `QUICKNOTES_DATA_DIR` environment variable (used by tests and advanced users)
/// 2. Platform default app-data directory
///
/// | OS      | Default location                                   |
/// |---------|----------------------------------------------------|
/// | Windows | `%APPDATA%\QuickNotes\quicknotes.db`               |
/// | macOS   | `~/Library/Application Support/QuickNotes/…`       |
/// | Linux   | `~/.local/share/quicknotes/quicknotes.db`          |
///
/// Desktop, CLI and MCP all call this function, guaranteeing a single source
/// of truth.
pub fn default_db_path() -> PathBuf {
    default_data_dir().join("quicknotes.db")
}

/// Directory that holds the database and the board-layout JSON file.
pub fn default_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("QUICKNOTES_DATA_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
        PathBuf::from(base).join("QuickNotes")
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("QuickNotes")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        match std::env::var("XDG_DATA_HOME") {
            Ok(x) => PathBuf::from(x).join("quicknotes"),
            Err(_) => {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                PathBuf::from(home)
                    .join(".local")
                    .join("share")
                    .join("quicknotes")
            }
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    {
        PathBuf::from(".")
    }
}

/// Open (creating if necessary) the database at `path` and ensure the schema,
/// FTS5 virtual table and sync triggers exist.
pub fn open_db(path: &Path) -> anyhow::Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    init_schema(&conn)?;
    Ok(conn)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS notes (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            title       TEXT NOT NULL DEFAULT '',
            content     TEXT NOT NULL DEFAULT '',
            tags        TEXT NOT NULL DEFAULT '',
            pinned      INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL,
            deleted     INTEGER NOT NULL DEFAULT 0
        );

        CREATE INDEX IF NOT EXISTS idx_notes_updated ON notes(updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_notes_deleted ON notes(deleted);

        CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
            title,
            content,
            content='notes',
            content_rowid='id',
            tokenize='unicode61 remove_diacritics 2'
        );

        -- Keep the FTS index in sync via triggers, never application logic.
        CREATE TRIGGER IF NOT EXISTS notes_ai AFTER INSERT ON notes BEGIN
            INSERT INTO notes_fts(rowid, title, content)
            VALUES (new.id, new.title, new.content);
        END;

        CREATE TRIGGER IF NOT EXISTS notes_ad AFTER DELETE ON notes BEGIN
            INSERT INTO notes_fts(notes_fts, rowid, title, content)
            VALUES ('delete', old.id, old.title, old.content);
        END;

        CREATE TRIGGER IF NOT EXISTS notes_au AFTER UPDATE ON notes BEGIN
            INSERT INTO notes_fts(notes_fts, rowid, title, content)
            VALUES ('delete', old.id, old.title, old.content);
            INSERT INTO notes_fts(rowid, title, content)
            VALUES (new.id, new.title, new.content);
        END;
        "#,
    )
}

/// Build a short, highlighted snippet around the first occurrence of any of
/// the query's terms in `text`.
pub fn make_snippet(text: &str, terms: &[&str], radius: usize) -> String {
    let lower = text.to_lowercase();
    let mut best: Option<usize> = None;
    for t in terms {
        if t.is_empty() {
            continue;
        }
        if let Some(pos) = lower.find(&t.to_lowercase()) {
            if best.is_none_or(|b| pos < b) {
                best = Some(pos);
            }
        }
    }
    let chars: Vec<char> = text.chars().collect();
    match best {
        None => {
            let end = chars.len().min(radius * 4);
            let mut s: String = chars[..end].iter().collect();
            if end < chars.len() {
                s.push('…');
            }
            s
        }
        Some(pos) => {
            // Map byte offset -> char offset.
            let char_pos = text[..pos.min(text.len())].chars().count();
            let start = char_pos.saturating_sub(radius);
            let end = (char_pos + radius).min(chars.len());
            let mut out = String::new();
            if start > 0 {
                out.push('…');
            }
            out.extend(&chars[start..end]);
            if end < chars.len() {
                out.push('…');
            }
            out
        }
    }
}

/// Split a raw FTS5 query into plain terms usable for snippet building.
pub fn query_terms(query: &str) -> Vec<&str> {
    query
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .collect()
}

/// Create a note, returning its new id.
pub fn create_note(
    conn: &Connection,
    title: &str,
    content: &str,
    tags: &str,
) -> anyhow::Result<i64> {
    let ts = now();
    conn.execute(
        "INSERT INTO notes (title, content, tags, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![title, content, tags, ts],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Update an existing note's mutable fields.
pub fn update_note(
    conn: &Connection,
    id: i64,
    title: &str,
    content: &str,
    tags: &str,
) -> anyhow::Result<()> {
    let n = conn.execute(
        "UPDATE notes SET title=?1, content=?2, tags=?3, updated_at=?4 WHERE id=?5",
        params![title, content, tags, now(), id],
    )?;
    if n == 0 {
        anyhow::bail!("note {id} not found");
    }
    Ok(())
}

/// Toggle a note's pinned flag.
pub fn set_pinned(conn: &Connection, id: i64, pinned: bool) -> anyhow::Result<()> {
    let n = conn.execute(
        "UPDATE notes SET pinned=?1, updated_at=?2 WHERE id=?3",
        params![pinned as i64, now(), id],
    )?;
    if n == 0 {
        anyhow::bail!("note {id} not found");
    }
    Ok(())
}

/// Hard-delete a note (the FTS trigger removes it from the index too).
pub fn delete_note(conn: &Connection, id: i64) -> anyhow::Result<()> {
    let n = conn.execute("DELETE FROM notes WHERE id=?1", params![id])?;
    if n == 0 {
        anyhow::bail!("note {id} not found");
    }
    Ok(())
}

/// Fetch one full note by id.
pub fn get_note(conn: &Connection, id: i64) -> anyhow::Result<Option<Note>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, content, tags, pinned, created_at, updated_at
         FROM notes WHERE id=?1 AND deleted=0",
    )?;
    let mut rows = stmt.query_map(params![id], |row| {
        Ok(Note {
            id: row.get(0)?,
            title: row.get(1)?,
            content: row.get(2)?,
            tags: row.get(3)?,
            pinned: row.get::<_, i64>(4)? != 0,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    })?;
    Ok(rows.next().transpose()?)
}

/// Map one result row of the list/search SELECT shape into a [`NoteBrief`].
fn map_brief(row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteBrief> {
    Ok(NoteBrief {
        id: row.get(0)?,
        title: row.get(1)?,
        snippet: row.get(2)?,
        tags: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

/// List the most recently updated notes (pinned first).
pub fn list_notes(conn: &Connection, limit: i64) -> anyhow::Result<Vec<NoteBrief>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, substr(content, 1, 160), tags, updated_at
         FROM notes WHERE deleted=0
         ORDER BY pinned DESC, updated_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], map_brief)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Full-text search using FTS5 BM25 ranking. Returns briefs whose snippet is
/// built around the first matched term.
pub fn search_notes(conn: &Connection, query: &str, limit: i64) -> anyhow::Result<Vec<NoteBrief>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(vec![]);
    }
    // Sanitize: strip punctuation that could cause FTS5 syntax errors, then
    // prefix each term so "fts" matches whole tokens like "fts5" (the unicode61
    // tokenizer does not split on digits). Terms are OR-joined for recall and
    // ranked by BM25.
    let safe_query: String = trimmed
        .split_whitespace()
        .map(|w| {
            let cleaned: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if cleaned.is_empty() {
                None
            } else {
                Some(format!("\"{cleaned}\"*"))
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" OR ");
    if safe_query.is_empty() {
        return Ok(vec![]);
    }

    let mut stmt = conn.prepare(
        r#"
        SELECT notes.id, notes.title, notes.content, notes.tags, notes.updated_at
        FROM notes_fts
        JOIN notes ON notes.id = notes_fts.rowid
        WHERE notes_fts MATCH ?1 AND notes.deleted = 0
        ORDER BY bm25(notes_fts)
        LIMIT ?2
        "#,
    )?;
    let terms = query_terms(trimmed);
    let rows = stmt.query_map(params![safe_query, limit], |row| {
        let content: String = row.get(2)?;
        Ok(NoteBrief {
            id: row.get(0)?,
            title: row.get(1)?,
            snippet: make_snippet(&content, &terms, 60),
            tags: row.get(3)?,
            updated_at: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Count live notes (handy for status bars and tests).
pub fn count_notes(conn: &Connection) -> anyhow::Result<i64> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM notes WHERE deleted=0", [], |r| {
        r.get(0)
    })?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_conn(tag: &str) -> Connection {
        let dir = std::env::temp_dir().join(format!("qtest-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        open_db(&dir.join("test.db")).unwrap()
    }

    #[test]
    fn create_get_update_delete_roundtrip() {
        let conn = temp_conn("crud");
        let id = create_note(&conn, "Groceries", "buy milk and eggs", "shopping").unwrap();
        let note = get_note(&conn, id).unwrap().expect("note exists");
        assert_eq!(note.title, "Groceries");
        assert_eq!(note.tags, "shopping");

        update_note(
            &conn,
            id,
            "Groceries!",
            "buy bread and eggs",
            "shopping,food",
        )
        .unwrap();
        let note = get_note(&conn, id).unwrap().unwrap();
        assert_eq!(note.title, "Groceries!");

        delete_note(&conn, id).unwrap();
        assert!(get_note(&conn, id).unwrap().is_none());
    }

    #[test]
    fn fts_stays_in_sync_via_triggers() {
        let conn = temp_conn("fts");
        let id = create_note(
            &conn,
            "Tauri setup",
            "register global shortcut Ctrl+Shift+N",
            "rust",
        )
        .unwrap();
        let hits = search_notes(&conn, "shortcut", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, id);

        update_note(&conn, id, "Tauri setup", "tray icon and shortcuts", "rust").unwrap();
        assert!(search_notes(&conn, "Ctrl", 10).unwrap().is_empty());
        assert_eq!(search_notes(&conn, "tray", 10).unwrap().len(), 1);

        delete_note(&conn, id).unwrap();
        assert!(search_notes(&conn, "shortcut", 10).unwrap().is_empty());
    }

    #[test]
    fn snippet_highlights_term() {
        let text = "The quick brown fox jumps over the lazy dog";
        let snip = make_snippet(text, &["fox"], 8);
        assert!(snip.contains("fox"), "got: {snip}");
    }

    #[test]
    fn list_orders_pinned_first() {
        let conn = temp_conn("list");
        let a = create_note(&conn, "Old pinned", "a", "").unwrap();
        let _b = create_note(&conn, "Newer", "b", "").unwrap();
        set_pinned(&conn, a, true).unwrap();
        let items = list_notes(&conn, 10).unwrap();
        assert_eq!(items[0].id, a);
    }
}

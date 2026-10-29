//! Board layout persistence for the sticky-note canvas.
//!
//! Positions, colors and sizes of notes on the drag-and-drop board are
//! per-device presentation state, so they live in a small JSON file next to
//! the database rather than polluting the relational schema. The CLI and MCP
//! server ignore this file entirely; only the desktop frontend uses it.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Layout entry for one note on the board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteLayout {
    pub x: f64,
    pub y: f64,
    /// One of the four sticky colors: yellow/green/pink/blue hex value.
    pub color: String,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub z: Option<i64>,
}

/// Whole-board document, keyed by note id (as string for JSON safety).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BoardDoc {
    /// Map of note id -> layout.
    pub items: HashMap<String, NoteLayout>,
}

impl BoardDoc {
    /// Load from `path`; returns an empty doc if the file is missing or corrupt.
    pub fn load(path: &PathBuf) -> BoardDoc {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Atomically write the doc (tmp file + rename).
    pub fn save(&self, path: &PathBuf) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Path of the board file inside the shared data directory.
pub fn default_board_path() -> PathBuf {
    crate::default_data_dir().join("board.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_save_load() {
        let dir = std::env::temp_dir().join(format!("qboard-{}", std::process::id()));
        let path = dir.join("board.json");
        let mut doc = BoardDoc::default();
        doc.items.insert(
            "42".into(),
            NoteLayout {
                x: 10.5,
                y: 20.0,
                color: "#fff8b3".into(),
                width: None,
                height: None,
                z: Some(3),
            },
        );
        doc.save(&path).unwrap();
        let loaded = BoardDoc::load(&path);
        assert_eq!(loaded.items.get("42").unwrap().color, "#fff8b3");
    }

    #[test]
    fn load_missing_returns_empty() {
        let doc = BoardDoc::load(&PathBuf::from("/nonexistent/board.json"));
        assert!(doc.items.is_empty());
    }
}

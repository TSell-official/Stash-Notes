//! Tauri 2 desktop backend for QuickNotes.
//!
//! Exposes the `quicknotes-core` API as Tauri commands, registers a global
//! shortcut (Ctrl+Shift+N) that triggers quick-capture even when the window
//! is hidden, and persists board layout through `quicknotes_core::board`.

use quicknotes_core as core;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

/// Shared database handle managed by Tauri.
pub struct Db(pub Mutex<Connection>);

#[derive(Debug, Serialize, Deserialize)]
pub struct BoardEntry {
    pub x: f64,
    pub y: f64,
    pub color: String,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
    #[serde(default)]
    pub z: Option<i64>,
}

#[tauri::command]
fn create_note(
    state: State<'_, Db>,
    title: String,
    content: String,
    tags: String,
) -> Result<i64, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    core::create_note(&conn, &title, &content, &tags).map_err(|e| e.to_string())
}

#[tauri::command]
fn update_note(
    state: State<'_, Db>,
    id: i64,
    title: String,
    content: String,
    tags: String,
) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    core::update_note(&conn, id, &title, &content, &tags).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_note(state: State<'_, Db>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    core::delete_note(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_pinned(state: State<'_, Db>, id: i64, pinned: bool) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    core::set_pinned(&conn, id, pinned).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_note(state: State<'_, Db>, id: i64) -> Result<Option<core::Note>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    core::get_note(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_notes(state: State<'_, Db>, limit: i64) -> Result<Vec<core::NoteBrief>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    core::list_notes(&conn, limit).map_err(|e| e.to_string())
}

#[tauri::command]
fn search_notes(
    state: State<'_, Db>,
    query: String,
    limit: i64,
) -> Result<Vec<core::NoteBrief>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    core::search_notes(&conn, &query, limit).map_err(|e| e.to_string())
}

#[tauri::command]
fn count_notes(state: State<'_, Db>) -> Result<i64, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    core::count_notes(&conn).map_err(|e| e.to_string())
}

/// Load the sticky-board layout (note id -> position/color/size).
#[tauri::command]
fn board_load() -> Result<std::collections::HashMap<String, BoardEntry>, String> {
    let doc = core::board::BoardDoc::load(&core::board::default_board_path());
    Ok(doc
        .items
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                BoardEntry {
                    x: v.x,
                    y: v.y,
                    color: v.color,
                    width: v.width,
                    height: v.height,
                    z: v.z,
                },
            )
        })
        .collect())
}

/// Persist the sticky-board layout atomically.
#[tauri::command]
fn board_save(items: std::collections::HashMap<String, BoardEntry>) -> Result<(), String> {
    let doc = core::board::BoardDoc {
        items: items
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    core::board::NoteLayout {
                        x: v.x,
                        y: v.y,
                        color: v.color,
                        width: v.width,
                        height: v.height,
                        z: v.z,
                    },
                )
            })
            .collect(),
    };
    doc.save(&core::board::default_board_path())
        .map_err(|e| e.to_string())
}

/// Show + focus the main window (used by tray / shortcut handlers).
#[tauri::command]
fn show_main_window(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db_path = core::default_db_path();
    let conn = core::open_db(&db_path).expect("failed to open QuickNotes database");

    let builder = tauri::Builder::default()
        .manage(Db(Mutex::new(conn)))
        .invoke_handler(tauri::generate_handler![
            create_note,
            update_note,
            delete_note,
            set_pinned,
            get_note,
            list_notes,
            search_notes,
            count_notes,
            board_load,
            board_save,
            show_main_window
        ]);

    // Global shortcut plugin only exists on desktop targets.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_global_shortcut::Builder::new().build());

    builder
        .setup(|app| {
            #[cfg(desktop)]
            {
                use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};
                let handle = app.handle().clone();
                let shortcut =
                    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyN);
                app.global_shortcut()
                    .on_shortcut(shortcut, move |_app, _scut, event| {
                        if event.state() == ShortcutState::Pressed {
                            // Frontend listens for this event to open quick capture.
                            let _ = handle.emit("global-quick-capture", ());
                            if let Some(w) = handle.get_webview_window("main") {
                                let _ = w.unminimize();
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    })?;
            }
            let _ = app;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running QuickNotes");
}

// Bring the extension trait into scope for `app.global_shortcut()` in setup.
#[cfg(desktop)]
use tauri_plugin_global_shortcut::GlobalShortcutExt;

#[cfg(test)]
mod tests {
    //! Command functions take `tauri::State`, which is awkward to fabricate;
    //! instead we verify the underlying core calls behave for the desktop flow.
    use super::*;

    fn test_conn() -> Connection {
        let dir = std::env::temp_dir().join(format!("qdesktop-{}", std::process::id()));
        core::open_db(&dir.join("test.db")).unwrap()
    }

    #[test]
    fn desktop_commands_use_shared_store() {
        let conn = test_conn();
        let id = core::create_note(&conn, "from desktop", "hello", "ui").unwrap();
        assert_eq!(core::count_notes(&conn).unwrap(), 1);
        core::update_note(&conn, id, "from desktop", "hello world", "ui").unwrap();
        let hits = core::search_notes(&conn, "world", 5).unwrap();
        assert_eq!(hits[0].id, id);
    }

    #[test]
    fn board_roundtrip() {
        let dir = std::env::temp_dir().join(format!("qdesktop-board-{}", std::process::id()));
        std::env::set_var("QUICKNOTES_DATA_DIR", &dir);
        let path = core::board::default_board_path();
        let mut items = std::collections::HashMap::new();
        items.insert(
            "7".to_string(),
            BoardEntry {
                x: 1.0,
                y: 2.0,
                color: "#c7ffd8".into(),
                width: None,
                height: None,
                z: None,
            },
        );
        let doc = core::board::BoardDoc {
            items: items
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        core::board::NoteLayout {
                            x: v.x,
                            y: v.y,
                            color: v.color,
                            width: v.width,
                            height: v.height,
                            z: v.z,
                        },
                    )
                })
                .collect(),
        };
        doc.save(&path).unwrap();
        let loaded = core::board::BoardDoc::load(&path);
        assert_eq!(loaded.items.get("7").unwrap().color, "#c7ffd8");
    }
}

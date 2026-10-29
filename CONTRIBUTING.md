# Contributing to QuickNotes

Thanks for your interest! QuickNotes is a Rust workspace plus a Vite/TypeScript
frontend. This guide gets you from clone to green CI.

## Prerequisites

- [Rust](https://rustup.rs/) (stable, 1.77+)
- Node.js 18+ and npm
- Tauri 2 prerequisites for your OS: https://tauri.app/start/prerequisites/
  - **Windows**: Microsoft C++ Build Tools + WebView2 (preinstalled on Win 11)
  - **Linux**: `libwebkit2gtk-4.1-dev build-essential libssl-dev libayatana-appindicator3-dev librsvg2-dev`
  - **macOS**: Xcode command line tools

## Workspace layout

| Crate / dir          | What it is                                              |
|----------------------|---------------------------------------------------------|
| `quicknotes-core`    | SQLite + FTS5 storage engine, board layout persistence  |
| `quicknotes-cli`     | `quicknotes` binary (clap)                              |
| `quicknotes-mcp`     | MCP stdio server for AI agents (rust-mcp-sdk)           |
| `quicknotes-desktop` | Tauri 2 app with global shortcut + tray                 |
| `frontend/`          | Vite + TypeScript sticky-board UI                       |
| `.qwen/skills/`      | Qwen Code project skill                                 |

All four Rust surfaces share **one database file** resolved by
`quicknotes_core::default_db_path()`. Keep it that way.

## Development

```bash
# Fast loop: core + CLI only
cargo run -p quicknotes-cli -- list

# Frontend in browser mode (localStorage fallback, no Tauri needed)
cd frontend && npm install && npm run dev

# Full desktop app with hot reload
npm install -g @tauri-apps/cli
cd quicknotes-desktop && cargo tauri dev

# Test the MCP server interactively
npx @modelcontextprotocol/inspector cargo run -p quicknotes-mcp
```

## Before opening a PR

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd frontend && npx tsc --noEmit && npm run build
```

CI runs all of the above. The desktop crate is checked on Windows and Ubuntu
(Tauri dependencies installed); if your change touches `quicknotes-desktop`,
please attach a screenshot or recording of the app running.

## Design rules

1. **Storage logic lives in `quicknotes-core` only.** Commands, CLI args and
   MCP tools are thin wrappers. New capability = new core function, then
   surface it in each binary as appropriate.
2. **Never bypass the FTS triggers.** Don't write to `notes_fts` directly;
   insert/update/delete on `notes` and let the triggers sync the index.
3. **Sanitize user search input** before handing it to `MATCH` (see
   `search_notes`). FTS5 syntax errors from raw user text are a bug.
4. **Keep the frontend backend-agnostic.** Anything Tauri-specific goes behind
   the `Backend` interface in `frontend/src/main.ts` so browser mode keeps working.
5. Board layout (position/color/z) is per-device state — it belongs in the
   JSON sidecar (`quicknotes-core/src/board.rs`), not the database.

## Releasing

Tag `vX.Y.Z`; `.github/workflows/release.yml` builds installers for
Windows (msi/exe), Linux (deb/AppImage) and macOS (dmg/app) and attaches them
to a GitHub Release along with the `quicknotes` CLI and `quicknotes-mcp`
binaries.

Questions? Open an issue first for anything larger than a typo.

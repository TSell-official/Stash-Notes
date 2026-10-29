# QuickNotes

**A fast, local-first sticky-note board with voice capture — plus a Rust core that
gives you a desktop app (Tauri 2), a global CLI, and an MCP server for AI agents,
all sharing one SQLite + FTS5 database.**

![CI](https://github.com/your-name/quicknotes/actions/workflows/ci.yml/badge.svg)
![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.77%2B-orange.svg)
![Tauri](https://img.shields.io/badge/Tauri-2-ffc87c)

---

## Why this exists

It started as a single-file HTML sticky-note board with drag-and-drop notes, colors,
and Web-Speech dictation. Cute — but the notes lived in `localStorage`, invisible to
everything else on your machine.

QuickNotes keeps the exact same board UX and rebuilds the persistence layer on a
**single source of truth**: a SQLite database with an FTS5 full-text index. Four
surfaces read and write it concurrently:

| Surface        | What it is                                                    | Entry point                    |
| -------------- | ------------------------------------------------------------- | ------------------------------ |
| 🖥 Desktop app  | Tauri 2 sticky board, voice input, system-wide shortcut       | `cargo tauri dev`              |
| ⌨️ CLI          | `quicknotes new/list/search/export…` from any terminal         | `quicknotes-cli`               |
| 🤖 MCP server   | stdio JSON-RPC so Claude Desktop / Cursor / Qwen can use notes | `quicknotes-mcp`               |
| 🧠 Qwen Code    | Project skill + headless prompts                               | `.qwen/skills/note-assistant/` |

```
┌─────────────┐  ┌──────────┐  ┌─────────────┐
│ Tauri desk- │  │ quicknotes│  │ MCP client  │
│ top app     │  │ CLI       │  │ (AI agent)  │
└──────┬──────┘  └─────┬─────┘  └──────┬──────┘
       │ commands      │ clap          │ stdio JSON-RPC
       ▼               ▼               ▼
              ┌──────────────────────────┐
              │      quicknotes-core     │  ← schema, FTS5 triggers,
              │  (Rust crate, no deps    │     CRUD, search, board layout
              │   on any UI framework)   │
              └────────────┬─────────────┘
                           ▼
              SQLite (WAL) + FTS5 virtual table
              $APPDATA\QuickNotes\quicknotes.db (Windows)
              ~/.quicknotes/quicknotes.db        (macOS/Linux)
```

**Design guarantees**

- **One database file.** Every surface resolves the same path via
  `quicknotes_core::default_db_path()` (override with `QUICKNOTES_DATA_DIR`).
  A note created by an AI agent appears on your board instantly.
- **No index drift.** FTS5 stays in sync through *SQLite triggers*, not app logic.
- **Queries are sanitized** before hitting FTS5 — typos like stray quotes can't
  crash search.
- **Board layout persists too.** Sticky positions/colors go to a JSON sidecar next
  to the DB (atomic tmp+rename writes), so the double-tap-to-create, drag-anywhere
  UX survives restarts.
- **Voice still works.** Dictation uses the browser Web Speech API inside the
  Tauri webview (Chromium-based ⇒ supported).

---

## Features

### Desktop (Tauri 2)
- Drag-and-drop sticky board: yellow/green/pink/blue notes, resizable z-order,
  double-tap (or double-click) anywhere to create a note at that spot.
- 🎤 Per-note voice dictation with live interim results.
- Global shortcut **Ctrl+Shift+N** — works even when minimized; brings the window
  forward and opens quick-capture.
- Full-text search box (FTS5 + BM25 ranking) over every note.
- System tray icon.

### CLI (`quicknotes`)
```powershell
quicknotes new --title "Idea" --content "Ship QuickNotes" --tags "work,ideas"
echo "piped body" | quicknotes new --title "From stdin"
quicknotes list --limit 10
quicknotes search "fts5 triggers"
quicknotes show <id>
quicknotes edit <id> --content "fixed text"
quicknotes pin <id>
quicknotes delete <id>
quicknotes export > notes.md        # Markdown dump of everything
quicknotes path                     # where the DB lives
```
`list` and `search` support `--json` for scripting.

### MCP server (`quicknotes-mcp`)
Six tools over stdio: `note_create`, `note_search`, `note_list`, `note_get`,
`note_update`, `note_delete`.

Claude Desktop / Cursor config:
```json
{
  "mcpServers": {
    "quicknotes": {
      "command": "C:\\Users\\you\\.quicknotes\\bin\\quicknotes-mcp.exe"
    }
  }
}
```

Test locally with the MCP Inspector:
```bash
npx @modelcontextprotocol/inspector target/release/quicknotes-mcp
```

### Qwen Code
The repo ships a project skill at `.qwen/skills/note-assistant/SKILL.md`, so Qwen
Code automatically knows how to manage your notes. Headless examples:

```bash
qwen -p "Save a QuickNotes note titled 'MCP registration' with tags mcp,setup."
qwen -p "Search QuickNotes for 'MCP' and summarize the top 5 results."
qwen -p "Run quicknotes list --limit 20 and write a Markdown digest grouped by tags."
cat meeting.md | qwen -p "Extract every heading as a QuickNotes note using quicknotes new."
```

---

## Repository layout

```
quicknotes/
├── Cargo.toml                  # workspace (4 members)
├── quicknotes-core/            # SQLite schema, FTS5 triggers, CRUD/search, board store
├── quicknotes-desktop/         # Tauri 2 app + global shortcut + tray
│   ├── src/{main.rs,lib.rs}
│   └── tauri.conf.json
├── quicknotes-mcp/             # stdio MCP server (rust-mcp-sdk)
├── quicknotes-cli/             # clap CLI, binary name: quicknotes
├── frontend/                   # Vite + TypeScript sticky-board UI
│   ├── index.html
│   └── src/main.ts
├── .qwen/skills/note-assistant/SKILL.md
└── .github/workflows/ci.yml    # fmt/clippy/test ×3 OSes + frontend build + release artifacts
```

---

## Getting started

### Prerequisites
- [Rust](https://rustup.rs/) stable (1.77+)
- Node.js 18+
- [Tauri 2 system dependencies](https://tauri.app/start/prerequisites/) for your OS
  (Linux: `libwebkit2gtk-4.1-dev libgtk-3-dev …`; Windows/macOS: just the WebView2/Safari runtime)

### Build & run everything

```bash
# 1. All binaries
cargo build --release
# → target/release/quicknotes        (CLI)
# → target/release/quicknotes-mcp    (MCP server)

# 2. Desktop app (dev mode, hot reload)
cd quicknotes-desktop
npm install --prefix ../frontend && npm run --prefix ../frontend dev &
cargo tauri dev

# 3. Try the CLI
./target/release/quicknotes new --title "Hello" --content "World" --tags demo
./target/release/quicknotes search hello

# 4. Try the MCP server
npx @modelcontextprotocol/inspector ./target/release/quicknotes-mcp
```

### Install the CLI globally on Windows

```powershell
$bin = "$env:USERPROFILE\.quicknotes\bin"
New-Item -ItemType Directory -Force -Path $bin | Out-Null
Copy-Item .\target\release\quicknotes.exe $bin\quicknotes.exe -Force
Copy-Item .\target\release\quicknotes-mcp.exe $bin\quicknotes-mcp.exe -Force

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$bin*") {
  [Environment]::SetEnvironmentVariable("Path", "$userPath;$bin", "User")
}
```

On macOS/Linux: `cargo install --path quicknotes-cli --root ~/.quicknotes/bin`.

### Frontend without Tauri

`frontend/src/main.ts` detects whether it's running inside Tauri. Outside of it
(plain browser via `npm run dev`) it falls back to `localStorage`, so you can hack
on the board UI without a Rust toolchain.

---

## Testing

```bash
cargo test                      # core unit tests: CRUD, trigger sync, snippets, board store
cd frontend && npm run build    # type-checks main.ts and builds dist/
```

CI runs formatting, clippy (`-D warnings`), and the full test suite on
ubuntu/windows/macos, plus the frontend build, on every push and PR.

---

## Data & privacy

Everything is local. No telemetry, no network calls except the browser's own
Web Speech recognition (which, depending on platform, may send audio to the
platform vendor — disable the mic button if that matters to you). To wipe your
data: `quicknotes delete` per note, or remove the DB + `board.json` beside it
(`quicknotes path` prints the location).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Small, focused PRs are welcome — the core
crate is deliberately dependency-light so patches stay easy.

## License

MIT © [your name] — see [LICENSE](LICENSE).

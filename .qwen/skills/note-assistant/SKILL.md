---
name: note-assistant
description: >
  Create, search, and organize the user's local QuickNotes store through the
  `quicknotes` CLI or the QuickNotes MCP server. Use whenever the user asks to
  save a note, look something up in their notes, review recent notes, pin
  something important, or export notes to Markdown.
---

# QuickNotes Assistant

A skill for creating, searching, and organizing notes through the QuickNotes
CLI, MCP server, and SQLite + FTS5 database.

## Description

QuickNotes stores notes in a local SQLite database with full-text search
(FTS5). You can interact with it through three surfaces:

1. **CLI**: `quicknotes` (new, list, search, show, edit, pin, delete, export, path)
2. **MCP server**: `quicknotes-mcp` (note_create, note_search, note_list, note_get, note_update, note_delete)
3. **Desktop app**: Tauri commands (`create_note`, `search_notes`, etc.)

The database lives at `%APPDATA%\QuickNotes\quicknotes.db` on Windows,
`~/Library/Application Support/QuickNotes/quicknotes.db` on macOS, and
`~/.local/share/quicknotes/quicknotes.db` on Linux. Set
`QUICKNOTES_DATA_DIR` to override.

## Instructions

When the user asks you to work with notes:

1. **Prefer the CLI** for simple operations because it is fast and requires no
   MCP connection. Prefer MCP tools when a QuickNotes MCP connection is active.
2. **Use FTS5 search** with `quicknotes search "<query>"` for keyword lookup.
   Search terms are ANDed together and results are BM25-ranked.
3. **Create well-structured notes**: put a short title in `--title`, the body
   in `--content`, and comma-separated tags in `--tags`. For long bodies, pipe
   them in: `cat body.txt | quicknotes new --title "T" --stdin`.
4. **Never edit the SQLite database directly**; always use the CLI or MCP tools
   so the FTS5 triggers stay in sync. No index drift is tolerated.
5. If the user asks for a summary of recent work, run
   `quicknotes list --limit 10` (add `--json` if you need machine-readable
   output) and summarize the titles/snippets.
6. Use `quicknotes export --out notes.md` when the user wants a Markdown
   snapshot of everything.

## Tools

- `bash` — to run `quicknotes` CLI commands
- `read` — to read source files or generated Markdown exports
- `write_file` — to export notes to Markdown if the user asks

## Examples

### Example 1: Capture a note
User: "Save a note: the MCP server needs to be registered with Claude Desktop."
Assistant:
```bash
quicknotes new --title "MCP registration" \
  --content "Register quicknotes-mcp.exe in Claude Desktop config." \
  --tags "mcp,setup"
```

### Example 2: Search notes
User: "What did I write about FTS5?"
Assistant:
```bash
quicknotes search "FTS5"
```
Then summarize the returned snippets.

### Example 3: Daily review
User: "Show me my 5 most recent notes."
Assistant:
```bash
quicknotes list --limit 5
```

### Example 4: Read then edit
User: "Update the groceries note to add bread."
Assistant:
```bash
quicknotes search "groceries"        # find the id
quicknotes show <id>                 # read full content
quicknotes edit <id> --content "buy milk, eggs, bread"
```

## MCP tools

When the QuickNotes MCP server is connected, prefer these tools over shell
commands:

- `note_create(title, content, tags)` — create a note, returns its id
- `note_search(query, limit)` — FTS5 BM25-ranked search with snippets
- `note_list(limit)` — recent notes (pinned first)
- `note_get(id)` — one full note including complete body
- `note_update(id, title?, content?, tags?)` — partial update; omitted fields keep current values
- `note_delete(id)` — delete by id (also removes from the FTS index)

/**
 * QuickNotes frontend.
 *
 * Runs in two modes:
 *  - **Tauri desktop**: notes live in the shared SQLite + FTS5 database via
 *    `invoke()` commands; board layout persists through `board_load/board_save`.
 *  - **Plain browser** (`npm run dev` / `vite preview` without Tauri): falls
 *    back to a localStorage shim so the UI is fully usable for demos.
 */

/* ------------------------------ backend layer ----------------------------- */

type NoteBrief = {
  id: number;
  title: string;
  snippet: string;
  tags: string;
  updated_at: number;
};

type Note = { id: number; title: string; content: string; tags: string; pinned: boolean; created_at: number; updated_at: number };

type BoardEntry = { x: number; y: number; color: string; width?: number; height?: number; z?: number };

interface Backend {
  readonly name: "tauri" | "local";
  listNotes(limit: number): Promise<NoteBrief[]>;
  searchNotes(query: string, limit: number): Promise<NoteBrief[]>;
  createNote(title: string, content: string, tags: string): Promise<number>;
  updateNote(id: number, title: string, content: string, tags: string): Promise<void>;
  deleteNote(id: number): Promise<void>;
  getNote(id: number): Promise<Note | null>;
  setPinned(id: number, pinned: boolean): Promise<void>;
  boardLoad(): Promise<Record<string, BoardEntry>>;
  boardSave(items: Record<string, BoardEntry>): Promise<void>;
  onQuickCapture(cb: () => void): void;
}

const isTauri = "__TAURI_INTERNALS__" in window || "__TAURI__" in window;

async function makeTauriBackend(): Promise<Backend> {
  const { invoke } = await import("@tauri-apps/api/core");
  const { listen } = await import("@tauri-apps/api/event");
  return {
    name: "tauri",
    listNotes: (limit) => invoke("list_notes", { limit }),
    searchNotes: (query, limit) => invoke("search_notes", { query, limit }),
    createNote: (title, content, tags) => invoke("create_note", { title, content, tags }),
    updateNote: (id, title, content, tags) => invoke("update_note", { id, title, content, tags }),
    deleteNote: (id) => invoke("delete_note", { id }),
    getNote: (id) => invoke<Note | null>("get_note", { id }),
    setPinned: (id, pinned) => invoke("set_pinned", { id, pinned }),
    boardLoad: () => invoke("board_load"),
    boardSave: (items) => invoke("board_save", { items }),
    onQuickCapture: (cb) => {
      void listen("global-quick-capture", () => cb());
    },
  };
}

/* localStorage fallback — mirrors quicknotes-core semantics loosely. */
function makeLocalBackend(): Backend {
  const NOTES_KEY = "quicknotes.notes.v1";
  const BOARD_KEY = "quicknotes.board.v1";
  const load = <T,>(k: string, d: T): T => {
    try { return JSON.parse(localStorage.getItem(k) ?? "") as T; } catch { return d; }
  };
  const store = (k: string, v: unknown) => localStorage.setItem(k, JSON.stringify(v));
  let notes = load<Note[]>(NOTES_KEY, []);
  let nextId = load<number>("quicknotes.nextId", 1);

  const brief = (n: Note): NoteBrief => ({
    id: n.id, title: n.title, snippet: n.content.slice(0, 160), tags: n.tags, updated_at: n.updated_at,
  });

  return {
    name: "local",
    async listNotes(limit) {
      return [...notes]
        .sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updated_at - a.updated_at)
        .slice(0, limit)
        .map(brief);
    },
    async searchNotes(query, limit) {
      const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
      if (!terms.length) return [];
      return notes
        .filter((n) => terms.every((t) => (n.title + " " + n.content).toLowerCase().includes(t)))
        .slice(0, limit)
        .map(brief);
    },
    async createNote(title, content, tags) {
      const id = nextId++;
      const ts = Math.floor(Date.now() / 1000);
      notes.push({ id, title, content, tags, pinned: false, created_at: ts, updated_at: ts });
      store(NOTES_KEY, notes); store("quicknotes.nextId", nextId);
      return id;
    },
    async updateNote(id, title, content, tags) {
      const n = notes.find((x) => x.id === id);
      if (!n) throw new Error(`note ${id} not found`);
      Object.assign(n, { title, content, tags, updated_at: Math.floor(Date.now() / 1000) });
      store(NOTES_KEY, notes);
    },
    async getNote(id) { return notes.find((n) => n.id === id) ?? null; },
    async deleteNote(id) {
      notes = notes.filter((n) => n.id !== id);
      store(NOTES_KEY, notes);
      const board = load<Record<string, BoardEntry>>(BOARD_KEY, {});
      delete board[String(id)];
      store(BOARD_KEY, board);
    },
    async setPinned(id, pinned) {
      const n = notes.find((x) => x.id === id);
      if (n) { n.pinned = pinned; store(NOTES_KEY, notes); }
    },
    async boardLoad() { return load<Record<string, BoardEntry>>(BOARD_KEY, {}); },
    async boardSave(items) { store(BOARD_KEY, items); },
    onQuickCapture(cb) {
      // No OS-level shortcut in the browser; Ctrl+Shift+N works when focused.
      window.addEventListener("keydown", (e) => {
        if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "n") { e.preventDefault(); cb(); }
      });
    },
  };
}

/* --------------------------------- app ----------------------------------- */

const COLORS = ["#fff8b3", "#c7ffd8", "#ffd1dc", "#d0e7ff"];
const DEFAULT_COLOR = "#fff8b3";

const boardEl = document.getElementById("board")!;
const emptyEl = document.getElementById("empty")!;
const addBtn = document.getElementById("add") as HTMLButtonElement;
const colorSel = document.getElementById("color") as HTMLSelectElement;
const clearAllBtn = document.getElementById("clearAll") as HTMLButtonElement;
const searchInput = document.getElementById("search") as HTMLInputElement;
const toastEl = document.getElementById("toast") as HTMLDivElement;
const supportEl = document.getElementById("support") as HTMLSpanElement;

type SpeechRecognitionLike = {
  lang: string; interimResults: boolean; continuous: boolean;
  onresult: ((ev: any) => void) | null;
  onend: (() => void) | null;
  onerror: (() => void) | null;
  start(): void; stop(): void;
};
type SpeechCtor = new () => SpeechRecognitionLike;
const SR: SpeechCtor | undefined =
  (window as any).SpeechRecognition ?? (window as any).webkitSpeechRecognition;

let zCounter = 1;
let backend: Backend;

function ping(msg: string) {
  toastEl.textContent = msg;
  toastEl.classList.remove("hidden");
  clearTimeout((ping as any).t);
  (ping as any).t = setTimeout(() => toastEl.classList.add("hidden"), 1200);
}

/* In-memory mirror of the board state persisted via board_save. */
type Sticky = {
  id: number;
  el: HTMLElement;
  titleEl: HTMLInputElement;
  ta: HTMLTextAreaElement;
  pinBtn: HTMLButtonElement;
  pos: BoardEntry;
  saveTimer?: number;
};
const stickies = new Map<number, Sticky>();

function serializeBoard(): Record<string, BoardEntry> {
  const out: Record<string, BoardEntry> = {};
  for (const [id, s] of stickies) {
    out[String(id)] = { ...s.pos, z: s.el.style.zIndex ? Number(s.el.style.zIndex) : undefined };
  }
  return out;
}

let boardTimer: number | undefined;
function persistBoard() {
  clearTimeout(boardTimer);
  boardTimer = window.setTimeout(() => {
    void backend?.boardSave(serializeBoard());
  }, 400);
}

function raise(el: HTMLElement) {
  el.style.zIndex = String(++zCounter);
}

function createSticky(id: number, entry: BoardEntry, text: string, title: string) {
  if (stickies.has(id)) return;

  const el = document.createElement("div");
  el.className = "note";
  el.style.background = entry.color || DEFAULT_COLOR;
  el.style.left = `${entry.x}px`;
  el.style.top = `${entry.y}px`;
  if (entry.width) el.style.width = `${entry.width}px`;
  if (entry.height) el.style.height = `${entry.height}px`;
  el.style.zIndex = String(entry.z && entry.z > zCounter ? (zCounter = entry.z) : ++zCounter);

  /* toolbar */
  const toolbar = document.createElement("div");
  toolbar.className = "toolbar";
  const chip = document.createElement("div");
  chip.className = "chip";
  COLORS.forEach((c) => {
    const sw = document.createElement("button");
    sw.className = "color";
    sw.style.background = c;
    sw.title = "Set color";
    sw.addEventListener("click", () => {
      el.style.background = c;
      s.pos.color = c;
      persistBoard();
    });
    chip.appendChild(sw);
  });
  const mic = document.createElement("button");
  mic.className = "mini mic";
  mic.textContent = "🎤";
  mic.title = "Dictate into this note";
  const pin = document.createElement("button");
  pin.className = "mini pin";
  pin.textContent = "📌";
  pin.title = "Pin note";
  const del = document.createElement("button");
  del.className = "mini del";
  del.textContent = "✕";
  del.title = "Delete note";
  toolbar.append(chip, mic, pin, del);

  /* title + body */
  const titleEl = document.createElement("input");
  titleEl.className = "titlebar";
  titleEl.placeholder = "Title…";
  titleEl.value = title;
  const ta = document.createElement("textarea");
  ta.placeholder = "Type…";
  ta.value = text;

  el.append(toolbar, titleEl, ta);
  boardEl.appendChild(el);

  const s: Sticky = { id, el, titleEl, ta, pinBtn: pin, pos: { ...entry } };
  stickies.set(id, s);
  updateEmpty();

  /* debounce persistence of note content */
  const onEdit = () => {
    clearTimeout(s.saveTimer);
    s.saveTimer = window.setTimeout(() => {
      void backend.updateNote(id, titleEl.value, ta.value, "").catch(() => ping("Save failed"));
      ping("Saved");
    }, 500);
  };
  titleEl.addEventListener("input", onEdit);
  ta.addEventListener("input", onEdit);

  /* drag (pointer events cover mouse + touch + pen) */
  let dragging = false, sx = 0, sy = 0, ox = 0, oy = 0;
  el.addEventListener("pointerdown", (e) => {
    const t = e.target as HTMLElement;
    if (t.tagName === "TEXTAREA" || t.tagName === "INPUT" || t.tagName === "BUTTON") {
      raise(el);
      return;
    }
    dragging = true;
    sx = e.clientX; sy = e.clientY;
    ox = el.offsetLeft; oy = el.offsetTop;
    el.setPointerCapture(e.pointerId);
    raise(el);
    persistBoard();
  });
  el.addEventListener("pointermove", (e) => {
    if (!dragging) return;
    el.style.left = `${ox + e.clientX - sx}px`;
    el.style.top = `${oy + e.clientY - sy}px`;
  });
  el.addEventListener("pointerup", (e) => {
    if (!dragging) return;
    dragging = false;
    el.releasePointerCapture(e.pointerId);
    s.pos.x = el.offsetLeft;
    s.pos.y = el.offsetTop;
    s.pos.z = Number(el.style.zIndex);
    persistBoard();
  });

  /* pin */
  pin.addEventListener("click", async () => {
    const active = pin.classList.toggle("active");
    await backend.setPinned(id, active).catch(() => {});
    ping(active ? "Pinned" : "Unpinned");
  });

  /* delete */
  del.addEventListener("click", async () => {
    if (!confirm("Delete this note?")) return;
    await backend.deleteNote(id).catch(() => ping("Delete failed"));
    el.remove();
    stickies.delete(id);
    persistBoard();
    updateEmpty();
    ping("Deleted");
  });

  /* voice dictation */
  if (SR) {
    let rec: SpeechRecognitionLike | null = null;
    let listening = false;
    mic.addEventListener("click", () => {
      if (!listening) {
        rec = new SR!();
        rec.lang = "en-US";
        rec.interimResults = true;
        rec.continuous = true;
        rec.onresult = (ev: any) => {
          let chunk = "";
          for (let i = ev.resultIndex; i < ev.results.length; i++) chunk += ev.results[i][0].transcript;
          ta.value = (ta.value ? ta.value + " " : "") + chunk;
          onEdit();
        };
        rec.onend = () => { listening = false; mic.textContent = "🎤"; };
        rec.onerror = () => { listening = false; mic.textContent = "🎤"; };
        rec.start();
        listening = true;
        mic.textContent = "⏹";
        ping("Listening… speak now");
      } else {
        rec?.stop();
      }
    });
  } else {
    mic.disabled = true;
    mic.title = "Speech not supported in this browser";
  }

  el.addEventListener("pointerdown", () => raise(el));
}

/** Replace the truncated snippet on a fresh sticky with the full note body. */
async function hydrate(id: number) {
  const full = await backend.getNote(id).catch(() => null);
  const s = stickies.get(id);
  if (!full || !s || document.activeElement === s.ta) return;
  s.ta.value = full.content;
  s.titleEl.value = full.title;
  s.pinBtn.classList.toggle("active", full.pinned);
}

function updateEmpty() {
  emptyEl.classList.toggle("hidden", stickies.size > 0);
}

async function addNote(x?: number, y?: number, color?: string) {
  const c = color || colorSel.value || DEFAULT_COLOR;
  const id = await backend.createNote("", "", "");
  const rect = boardEl.getBoundingClientRect();
  const px = x ?? 20 + (stickies.size % 6) * 24;
  const py = y ?? 20 + (stickies.size % 6) * 24;
  createSticky(id, { x: Math.max(0, px), y: Math.max(0, py), color: c }, "", "");
  persistBoard();
  ping("New note");
}

async function refreshFromList(limit = 100) {
  const briefs = await backend.listNotes(limit);
  const layout = await backend.boardLoad();
  // Remove DOM notes that no longer exist server-side.
  for (const [id, s] of stickies) {
    if (!briefs.some((b) => b.id === id)) {
      s.el.remove();
      stickies.delete(id);
    }
  }
  let cascade = 0;
  for (const b of briefs) {
    if (stickies.has(b.id)) continue;
    const key = String(b.id);
    const entry: BoardEntry = layout[key] ?? {
      x: 20 + (cascade % 6) * 26,
      y: 20 + (cascade % 6) * 26,
      color: DEFAULT_COLOR,
    };
    createSticky(b.id, entry, b.snippet.replace(/…$/, ""), b.title);
    void hydrate(b.id);
    cascade++;
  }
  updateEmpty();
}

let searchTimer: number | undefined;
searchInput.addEventListener("input", () => {
  clearTimeout(searchTimer);
  const q = searchInput.value.trim();
  searchTimer = window.setTimeout(async () => {
    if (!q) { await refreshFromList(); return; }
    const results = await backend.searchNotes(q, 50);
    for (const [, s] of stickies) { s.el.remove(); }
    stickies.clear();
    const layout = await backend.boardLoad();
    for (const r of results) {
      createSticky(r.id, layout[String(r.id)] ?? { x: 20, y: 20, color: DEFAULT_COLOR }, r.snippet, r.title);
      void hydrate(r.id);
    }
    updateEmpty();
    ping(`${results.length} match${results.length === 1 ? "" : "es"}`);
  }, 250);
});

clearAllBtn.addEventListener("click", async () => {
  if (!confirm("Delete all notes? This cannot be undone.")) return;
  for (const [id] of stickies) await backend.deleteNote(id).catch(() => {});
  for (const [, s] of stickies) s.el.remove();
  stickies.clear();
  persistBoard();
  updateEmpty();
  ping("Board cleared");
});

addBtn.addEventListener("click", () => void addNote());

/* double-click / double-tap empty board area to add a note there */
let lastTap = 0;
boardEl.addEventListener("pointerdown", (e) => {
  if ((e.target as HTMLElement).closest(".note")) return;
  const now = Date.now();
  const dbl = e.detail >= 2 || now - lastTap < 300;
  lastTap = now;
  if (dbl) {
    const rect = boardEl.getBoundingClientRect();
    void addNote(e.clientX - rect.left - 90, e.clientY - rect.top - 20);
  }
});

/* ------------------------------- bootstrap -------------------------------- */

async function boot() {
  backend = isTauri ? await makeTauriBackend() : makeLocalBackend();
  supportEl.textContent = SR ? "🎙️ Voice ready" : "ℹ️ Voice not supported";
  if (backend.name === "local") {
    supportEl.textContent += " • browser mode (localStorage)";
  }
  await refreshFromList();

  backend.onQuickCapture(() => {
    void addNote();
    (document.querySelector(".note textarea") as HTMLTextAreaElement | null)?.focus();
  });
}

void boot();

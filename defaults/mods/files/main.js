// A lazy file browser for the focused pane's folder. Local files edit in place and save
// themselves (VS Code-style auto-save); remote and truncated files are read-only previews.
import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, Editor, EditorState } from "gpui-base";
import { spaces, panes, focused, agents, core, exec, readTextFile, writeTextFile, watchFolder, theme, settings, window, windowVisible, layout, nextCommand, openFile, shared, setShared } from "h0x";

// Whether the layout shows this panel (as the sessions panel asks): no hidden
// node on the way, and the selected child of each stack. Not in the layout: shown.
const placed = (n) => n?.content?.mod === "files" || (n?.children ?? []).some(placed);
function shown(root) {
  const walk = (n) => {
    if (!n || n.hidden) return false;
    if (n.content?.mod === "files") return true;
    const children = (n.children ?? []).filter((c) => !c.hidden);
    if (n.type === "stack") return walk(children.find((c) => c.id === n.active) ?? children[0]);
    return children.some(walk);
  };
  return !placed(root) || walk(root);
}

function location() {
  const space = spaces().find(s => s.active), f = focused();
  const pane = f?.pane ? panes().find(p => p.pane === f.pane) : null;
  const on = f?.pane === null ? (f.terminal?.includes("/") ? f.terminal.split("/")[0] : null) : space?.machine ?? null;
  return { cwd: pane?.cwd || f?.cwd || space?.folder, on, space: space?.id };
}
const key = () => JSON.stringify(location());
const LANG = { ts: "js", tsx: "js", mts: "js", cts: "js", jsx: "js", mjs: "js", toml: "yaml", jsonc: "json", svelte: "html", vue: "html" };
// The extensions the editor reads (links.rs TEXT). A file with any other real extension goes to
// openFile first, and the host has the last word: it answers "text" for one it would have us read.
const TEXT_EXT = new Set(["txt", "md", "markdown", "json", "jsonc", "toml", "yaml", "yml", "rs", "js", "mjs", "cjs", "ts", "tsx", "jsx", "py", "go", "java", "kt", "c", "h", "cpp", "hpp", "cs", "rb", "php", "sh", "bash", "zsh", "fish", "html", "htm", "css", "scss", "xml", "csv", "log", "ini", "cfg", "conf"]);
const nonText = path => {
  const name = path.split("/").pop(), dot = name.lastIndexOf("."), ext = dot > 0 ? name.slice(dot + 1) : "";
  return /^[A-Za-z0-9]+$/.test(ext) && !TEXT_EXT.has(ext.toLowerCase());
};
// What the card says about a file the system was handed (anything else is an error string).
const HOW = { opened: "Opened in its default app", revealed: "Shown in its folder" };
const PER_FOLDER = 500, TOTAL = 5000;
// The file a terminal link opened, kept in shared state: editing this mod reloads it, and the new
// instance opens that file again (at its line) instead of the folder. Only for a few minutes.
const LAST_LINK = "files.last-link", LAST_LINK_MS = 5 * 60 * 1000;
// Saves of one file this close together are one edit; a note lists at most LISTED files.
const BURST = 30000, LISTED = 5;
// A file name goes into text typed at an agent: control characters (a newline would
// submit a fake prompt line), line/paragraph separators and invisible or bidi format
// characters (they disguise a name) are dropped and it is capped at 200 characters.
// At most 120 UTF-16 units of the error; a character the cut would split is dropped whole.
const reason = e => { const s = String((e !== null && typeof e === "object" ? e.message : e) || "unknown error").slice(0, 120); return /[\ud800-\udbff]$/.test(s) ? s.slice(0, -1) : s; };
const noteName =name => [...name.replace(/[\u0000-\u001f\u007f-\u009f\u200b-\u200f\u2028-\u202e\u2060-\u2064\u2066-\u2069\ufeff]/g, "")].slice(0, 200).join("");
// A file is written back with the line endings it was read with: CRLF only when every line had it.
const eol = text => /\r\n/.test(text) && !/(^|[^\r])\n/.test(text) ? "\r\n" : "\n";
const text = doc => { const value = doc.state.value(); return doc.eol === "\r\n" ? value.replace(/\r?\n/g, "\r\n") : value; };

async function directory(path, on) {
  const windowsDirectory = async () => {
    // Windows PowerShell otherwise uses the console code page; exec decodes UTF-8.
    // Links keep directory attributes even when dangling; check their target.
    // Other reparse points (OneDrive's folders) have no LinkType and are folders.
    // PowerShell reads curly single quotes as quotes too, so they're doubled like '.
    const r = await exec("powershell", ["-NoProfile", "-Command", "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); & { param($p) @(Get-ChildItem -LiteralPath $p -Force -ErrorAction Stop | Where-Object Name -ne '.git' | Select-Object -First 501 | ForEach-Object { @{ name=$_.Name; dir=($_.PSIsContainer -and (-not ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -or -not $_.LinkType -or ($_.Target -and [IO.Directory]::Exists(([IO.Path]::Combine([IO.Path]::GetDirectoryName($_.FullName), $_.Target[0])))))) } }) | ConvertTo-Json -Compress } '" + path.replace(/['‘’‚‛]/g, "$&$&") + "'"], { on });
    if (r.code !== 0) throw new Error(r.stderr || "Cannot list this folder");
    const result = JSON.parse(r.stdout || "[]");
    return Array.isArray(result) ? result : [result];
  };
  if (!on && window()?.platform === "windows") return windowsDirectory();
  const script = 'n=0; for p in "$1"/* "$1"/.[!.]* "$1"/..?*; do [ -e "$p" ] || [ -L "$p" ] || continue; name=${p##*/}; [ "$name" = .git ] && continue; if [ -d "$p" ]; then kind=d; else kind=f; fi; printf "%s\\0%s\\0" "$kind" "$name"; n=$((n+1)); [ "$n" -gt 500 ] && break; done; test -d "$1"';
  let r;
  try { r = await exec("sh", ["-c", script, "files", path], { on }); }
  catch (e) {
    if (on && String(e).includes("cannot start sh: program not found")) return windowsDirectory();
    throw e;
  }
  if (r.code !== 0) throw new Error(r.stderr || "Cannot list this folder");
  const fields = r.stdout.split("\0"), out = [];
  for (let i = 0; i + 1 < fields.length; i += 2) out.push({ name: fields[i + 1], dir: fields[i] === "d" });
  return out;
}

export default class Files extends View {
  init(_props, cx) {
    this.saved = new Map(); this.drafts = new Map(); this.edits = new Map();
    this.entries = new Map(); this.folders = new Set(); this.pending = new Set(); this.marks = {};
    const last = shared(LAST_LINK);
    if (last?.full && Date.now() - last.at < LAST_LINK_MS) this.link = { full: last.full, line: last.line ?? null };
    this.refresh(cx);
    cx.timer.every(1000, cx => this.tick(cx));
    // `files.open:<path>` from a terminal link click.
    cx.spawn(async cx => {
      // `files.line:<n>` right after an open: the 1-based line to show.
      let opened = false;
      for (;;) {
        const c = await nextCommand();
        // One bad command never ends the loop: the panel keeps opening files.
        try {
          if (c.startsWith("open:")) { this.openFile(c.slice(5), cx); opened = true; continue; }
          if (opened && /^line:\d+$/.test(c)) this.setLine(Number(c.slice(5)));
        } catch (_e) { this.line = null; }
        opened = false;
      }
    });
  }

  // Every 10 s while shown. Hidden (another panel tab, a closed panel) or the window in the
  // background: no listing or git until shown or back, then at once; unsent notes still go out.
  tick(cx) {
    this.ticks = ((this.ticks ?? 0) + 1) % 10;
    if (!shown(layout()) || !windowVisible()) {
      this.missed = true;
      if (!this.ticks) this.flush(cx);
    } else if (!this.ticks || this.missed) {
      this.missed = false;
      this.refresh(cx);
    }
  }

  watch(root, cx) {
    if (this.on || this.watched === root) return;
    this.watched = root;
    const context = this.context;
    cx.spawn(async cx => {
      try { while (this.context === context) { await watchFolder(root); if (this.current(context)) this.refresh(cx); } }
      catch { if (this.context === context) this.watched = null; }
    });
  }

  current(context) {
    return this.context === context && key() === context.key;
  }

  async loadFolder(path, context) {
    const { on, cwd: root } = context;
    const entries = await directory(path ? `${root}/${path}` : root, on);
    if (!this.current(context) || (path && !this.folders.has(path))) return;
    const used = [...this.entries.entries()].filter(([p]) => p !== path).reduce((n, [, e]) => n + e.length, 0);
    this.entries.set(path, entries.slice(0, Math.max(0, Math.min(PER_FOLDER, TOTAL - used))).sort((a, b) => Number(b.dir) - Number(a.dir) || a.name.localeCompare(b.name)));
    if (entries.length > PER_FOLDER || used + entries.length > TOTAL) this.limited = true;
  }

  refresh(cx) {
    const { cwd, on, space } = location(), next = key();
    if (next !== this.context?.key) {
      if (this.context) this.saved.set(this.context.key, { folders: this.folders, file: this.file });
      const saved = this.saved.get(next);
      this.context = { key: next, cwd, on, space }; this.root = cwd; this.on = on; this.entries = new Map(); this.folders = new Set(saved?.folders);
      // A file opened from a terminal link stays up when the folder behind it settles (a new profile's first
      // look at the folder can change the context a moment after the click): its view is left alone, unless
      // it is still loading (that load belongs to the old context), then it is opened again.
      const kept = this.link;
      this.pending = new Set(); this.marks = {}; this.watched = null; this.error = null; this.limited = false;
      this.loading = false;
      if (!kept) this.show(null, cx);
      if (kept) {
        // A card ("opened in its default app") is already the answer: that app is never started a second time.
        if (!this.code && !this.fileError && !this.opened) { this.openFile(kept.full, cx); this.setLine(kept.line); }
      } else if (saved?.file) this.show(saved.file, cx, false, true);
    }
    this.check(cx);
    this.flush(cx);
    if (!cwd || this.loading) return;
    const context = this.context;
    this.loading = true;
    this.limited = false;
    cx.spawn(async cx => {
      try {
        for (const path of ["", ...this.folders]) {
          if (!this.current(context)) return;
          await this.loadFolder(path, context);
        }
        const top = await exec("git", ["rev-parse", "--show-toplevel"], { cwd, on }).catch(() => null);
        const marks = {};
        if (top?.code === 0) {
          // Writes nothing under .git (index lock, fsmonitor cookie): the folder watch would refresh again.
          const status = await exec("git", ["--no-optional-locks", "-c", "core.fsmonitor=false", "status", "--porcelain=v1", "-z", "--no-renames", "--untracked-files=all"], { cwd, on }).catch(() => null);
          if (status?.code === 0) for (const entry of status.stdout.split("\0").filter(Boolean).slice(0, TOTAL)) {
            const xy = entry.slice(0, 2);
            marks[`${top.stdout.trim()}/${entry.slice(3)}`] = xy === "??" || xy.includes("A") ? "A" : xy.includes("U") ? "U" : xy.includes("D") ? "D" : "M";
          }
        }
        if (!this.current(context)) return;
        this.marks = marks; this.error = null; this.watch(cwd, cx);
      } catch (e) { if (this.current(context)) this.error = String(e); }
      finally { if (this.context === context) this.loading = false; cx.notify(); }
    });
  }

  toggle(path, cx) {
    if (this.folders.has(path)) {
      for (const folder of [...this.folders]) if (folder === path || folder.startsWith(path + "/")) {
        this.folders.delete(folder);
        this.entries.delete(folder);
      }
    } else {
      this.folders.add(path);
      if (!this.entries.has(path) && !this.pending.has(path)) {
        const context = this.context;
        this.pending.add(path);
        cx.spawn(async cx => {
          try { await this.loadFolder(path, context); }
          catch (e) { if (this.current(context)) this.error = String(e); }
          finally { if (this.context === context) this.pending.delete(path); cx.notify(); }
        });
      }
    }
    cx.notify();
  }

  // A terminal link opens a file of this machine, by its full path, whatever machine the panel lists.
  openFile(full, cx) {
    const path = full.replaceAll("\\", "/"), root = this.root?.replaceAll("\\", "/");
    this.show(root && !this.on && path.startsWith(root + "/") ? path.slice(root.length + 1) : path, cx, true);
    this.link = { full, line: null };
    this.remember();
    cx.notify();
  }

  // `files.line:<n>`: the line to show once the file just opened is loaded (and again if it is reopened).
  setLine(n) {
    if (this.link) { this.link.line = n; this.remember(); }
    if (!n) return;
    this.line = n;
    this.reveal();
  }

  remember() {
    if (this.link) setShared(LAST_LINK, { full: this.link.full, line: this.link.line, at: Date.now() });
  }

  // Once the opened file is loaded: the pending line (clamped to the last one) is scrolled to.
  reveal() {
    if (!this.line || !this.code) return;
    const n = this.line;
    this.line = null;
    // Scrolling is a nicety: a failure never costs the file its display.
    try { this.code.set_cursor_line(Math.min(n, this.code.value().split("\n").length) - 1); } catch (_e) {}
  }

  // `restoring`: the folder's last file comes back by itself, not by a click. The system hands a file to its app
  // only for a click, a link or "Open again" (the card's buttons call the host themselves): a restored one shows its card.
  show(path, cx, local, restoring) {
    this.line = null;
    if (!local) { this.link = null; if (shared(LAST_LINK)) setShared(LAST_LINK, null); }
    const prev = this.doc;
    if (prev && cx) {
      // Leaving a file saves it; edits held back by a disk conflict wait for its reopening.
      if (prev.conflict && text(prev) !== prev.saved) { prev.draft = text(prev); this.drafts.set(prev.full, prev); }
      this.save(cx);
    }
    prev?.timer?.cancel();
    this.code?.release(); this.code = null; this.doc = null; this.file = path; this.fileError = null; this.opened = null;
    if (!path || !cx) return;
    const context = this.context, root = this.root, on = local ? null : this.on, full = /^(\/|[A-Za-z]:\/)/.test(path) ? path : `${root}/${path}`;
    cx.spawn(async cx => {
      try {
        // A video or a program is never read as text: the system opens it (or shows it in its folder).
        if (!on && nonText(path)) {
          if (restoring) { this.opened = { full, how: "opened" }; cx.notify(); return; }
          const how = openFile(full);
          if (this.file !== path || !this.current(context)) return;
          if (how !== "text") { this.opened = { full, how }; cx.notify(); return; }
        }
        const disk = await readTextFile(full, { on });
        if (this.file !== path || !this.current(context)) return;
        if (disk === null) { this.fileError = "This file is no longer available"; return; }
        if (disk.includes("\0")) {
          if (on) this.fileError = "Binary file · no text preview"; else this.opened = { full, how: "binary" };
          return;
        }
        const t = theme(), ext = path.includes(".") ? path.split(".").pop() : "txt";
        const large = disk.length > 400000, draft = on || large ? null : this.drafts.get(full);
        this.drafts.delete(full);
        this.code = EditorState.new({ language: LANG[ext] ?? ext, readonly: Boolean(on) || large,
          value: large ? disk.slice(0, 400000) + "\n… (file truncated)" : draft ? draft.draft : disk,
          font_family: settings().font_family, font_size: 12,
          syntax: { keyword: t.ansi5, string: t.ansi2, comment: t.dim, function: t.ansi4, type: t.ansi3, number: t.ansi1, constant: t.ansi6 } });
        this.reveal();
        if (on || large) { this.doc = { full, hint: on ? "read-only: remote machine" : "read-only: too large" }; return; }
        const saved = draft ? draft.saved : disk;
        const doc = this.doc = { full, path, space: context.space, state: this.code, editable: true, saved, disk, eol: eol(saved), version: 0,
          dirty: Boolean(draft), conflict: draft ? disk !== saved : false, error: draft?.error ?? null };
        doc.state.on("change", (_e, cx) => {
          doc.dirty = text(doc) !== doc.saved;
          doc.timer?.cancel();
          doc.timer = doc.dirty && !doc.conflict ? cx.timer.after(1000, cx => { if (this.doc === doc) this.save(cx); }) : null;
          cx.notify();
        });
        doc.state.on("blur", (_e, cx) => { if (this.doc === doc) this.save(cx); });
      } catch (e) { if (this.file === path && this.current(context)) this.fileError = `Cannot preview this file: ${reason(e)}`; }
      finally { cx.notify(); }
    });
  }

  // The one save site: writes the open file's edits over what it last read or wrote
  // (keep: over the changed disk text instead). Resolves when the write is done.
  save(cx, keep = false) {
    const doc = this.doc;
    if (!doc?.editable) return null;
    doc.timer?.cancel(); doc.timer = null;
    const next = text(doc);
    if (!keep && (doc.conflict || next === doc.saved)) return null;
    const before = doc.writing;
    doc.writing = new Promise(done => cx.spawn(async cx => {
      try {
        await before;
        if (!keep && (doc.conflict || next === doc.saved)) return;
        const expected = keep ? doc.disk : doc.saved;
        doc.busy = true; doc.version++;
        try {
          await writeTextFile(doc.full, next, expected);
          doc.saved = doc.disk = next; doc.conflict = false; doc.error = null;
          this.edited(doc, cx);
        } catch (e) {
          // Stale snapshot (the disk moved on) or a failed write: the edits stay in the buffer.
          const disk = await readTextFile(doc.full).catch(() => undefined);
          if (disk !== undefined && disk !== expected) { doc.conflict = true; doc.disk = disk; }
          else doc.error = `Not saved: ${e?.message ?? e}`;
          if (this.doc !== doc) { doc.draft = next; this.drafts.set(doc.full, doc); }
        } finally { doc.busy = false; }
        if (this.doc === doc) doc.dirty = text(doc) !== doc.saved;
      } finally { done(); cx.notify(); }
    }));
    return doc.writing;
  }

  // A saved file: the agents in its space hear of it once per burst of saves.
  edited(doc, cx) {
    const id = `${doc.space}
${doc.path}`, now = Date.now(), edit = this.edits.get(id);
    if (edit && now - edit.at < BURST) edit.at = now;
    else this.edits.set(id, { space: doc.space, path: doc.path, at: now, told: new Set() });
    this.flush(cx);
  }

  // One note per agent with the files edited in its space it hasn't heard of. A working or
  // blocked agent is skipped (typing would interrupt it or answer its dialog) and hears at a
  // later flush; `--if idle` makes the core type nothing if it got busy meanwhile.
  flush(cx) {
    if (this.flushing || !this.edits.size) return;
    this.flushing = true;
    cx.spawn(async () => {
      try {
        const live = agents(), now = Date.now();
        for (const [id, edit] of this.edits) {
          // Closed agents are no longer listed; a space without agents has no one to tell.
          if (now - edit.at >= BURST && live.every(a => a.space !== edit.space || edit.told.has(a.address))) this.edits.delete(id);
        }
        for (const agent of live) {
          if (agent.status === "working" || agent.status === "blocked") continue;
          const news = [...this.edits.values()].filter(e => e.space === agent.space && !e.told.has(agent.address));
          if (!news.length) continue;
          // A name that sanitises to nothing is left out; if none is left the note says "a file".
          const names = news.map(e => noteName(e.path)).filter(Boolean), more = names.length - LISTED;
          const list = names.length ? names.slice(0, LISTED).join(", ") + (more > 0 ? ` and ${more} more` : "") : "a file";
          const sent = await core(["send", agent.address, `HyperOx: the user edited ${list} in the editor. Re-read before changing it.`, "--if", "idle"]).catch(() => null);
          if (sent?.code === 0) for (const e of news) e.told.add(agent.address);
        }
      } finally { this.flushing = false; }
    });
  }

  // Compares the open file with the disk: unedited, it follows the disk; edited, it asks.
  check(cx) {
    const doc = this.doc;
    if (!doc?.editable || doc.checking || doc.busy) return;
    doc.checking = true;
    const version = doc.version;
    cx.spawn(async cx => {
      try {
        const disk = await readTextFile(doc.full);
        if (this.doc !== doc || doc.busy || doc.version !== version || (doc.conflict && disk === doc.disk)) return;
        if (disk === doc.saved) { doc.conflict = false; return; }
        if (doc.dirty || disk === null || disk.includes("\0") || disk.length > 400000) {
          doc.conflict = true; doc.disk = disk; doc.timer?.cancel(); doc.timer = null;
        } else {
          doc.saved = doc.disk = disk; doc.eol = eol(disk); doc.state.set_value(disk);
        }
      } catch { /* unreadable now; the next refresh looks again */ }
      finally { doc.checking = false; cx.notify(); }
    });
  }

  // Takes the disk text, dropping the edits.
  reload(cx) {
    const doc = this.doc;
    if (!doc) return;
    doc.timer?.cancel(); doc.state.release(); this.code = null; this.doc = null;
    this.show(this.file, cx);
  }

  rows(prefix, depth, out, t) {
    if (out.length >= TOTAL || depth > 40) return out;
    for (const entry of this.entries.get(prefix) ?? []) {
      if (out.length >= TOTAL) break;
      const path = prefix ? `${prefix}/${entry.name}` : entry.name, open = this.folders.has(path), mark = this.marks[`${this.root}/${path}`];
      out.push(h_flex().id(`file-${path}`).h(28).flex_none().pl(8 + depth * 18).pr(8).gap(7).items_center().rounded(7).cursor_pointer().hover(s => s.bg(t.hover))
        .on_click((_e, cx) => entry.dir ? this.toggle(path, cx) : this.show(path, cx))
        .child(entry.dir ? svg(`mods/files/chevron-${open ? "down" : "right"}.svg`).size(12).flex_none().text_color(t.dim) : div().w(12).flex_none())
        .child(svg(`mods/files/${entry.dir ? "folder" : "file"}.svg`).size(14).flex_none().text_color(t.muted))
        .child(div().flex_1().min_w(0).truncate().child(entry.name))
        .when(Boolean(mark), el => el.child(div().text_size(11).text_color(mark === "A" ? t.ansi2 : mark === "D" || mark === "U" ? t.ansi1 : t.blocked).child(mark))));
      if (entry.dir && open) this.rows(path, depth + 1, out, t);
    }
    return out;
  }

  render(cx) {
    if (key() !== this.context?.key) this.refresh(cx);
    const t = theme(), doc = this.doc;
    const button = (id, label, click) => div().id(id).px(8).py(2).rounded(5).cursor_pointer().border_1().border_color(t.border).hover(s => s.bg(t.hover)).on_click(click).child(label);
    return v_flex().id("files").size_full().min_h(0).rounded(10).overflow_hidden().bg(t.background).border_1().border_color(t.border).text_color(t.foreground).text_size(13)
      .child(h_flex().h(42).flex_none().px(14).gap(8).items_center().border_b(1).border_color(t.border)
        .when(Boolean(this.file), el => el.child(div().id("files-back").w(20).cursor_pointer().text_color(t.muted).on_click((_e, cx) => { this.show(null, cx); cx.notify(); }).child("←")))
        .child(div().font_semibold().child("Files"))
        .child(this.file
          // The folder shrinks (and is cut) before the file name does.
          ? h_flex().flex_1().min_w(0).justify_end().text_size(12)
            .child(div().min_w(0).truncate().text_color(t.dim).child(this.file.slice(0, this.file.lastIndexOf("/") + 1)))
            .child(div().flex_none().text_color(t.foreground).child((doc?.dirty ? "● " : "") + this.file.slice(this.file.lastIndexOf("/") + 1)))
          : div().flex_1().min_w(0).truncate().text_right().text_size(12).text_color(t.dim).child(this.root?.split(/[\\/]/).pop() ?? "")))
      .when(!this.file, el => el.child(v_flex().id("files-tree").flex_1().min_h(0).overflow_y_scrollbar().p(6).children(this.rows("", 0, [], t))
        .when(Boolean(this.error) || !this.entries.get("")?.length, el => el.child(div().p(8).text_size(12).text_color(t.dim).child(this.error || (this.loading ? "Loading files…" : this.root ? "No files" : "No folder"))))
        .when(this.limited, el => el.child(div().p(8).text_size(11).text_color(t.dim).child("File list limited to 500 entries per folder and 5,000 total")))))
      .when(Boolean(doc?.conflict), el => el.child(h_flex().flex_none().px(12).py(6).gap(8).items_center().border_b(1).border_color(t.border).text_size(12)
        .child(div().flex_1().min_w(0).truncate().text_color(t.blocked).child("Changed on disk"))
        .child(button("files-reload", "Reload", (_e, cx) => { this.reload(cx); cx.notify(); }))
        .child(button("files-keep", "Keep mine", (_e, cx) => { this.save(cx, true); cx.notify(); }))))
      .when(Boolean(doc?.error), el => el.child(div().flex_none().px(12).py(4).truncate().text_size(12).text_color(t.ansi1).child(doc.error)))
      .when(Boolean(doc?.hint), el => el.child(div().flex_none().px(12).py(4).text_size(12).text_color(t.dim).child(doc.hint)))
      .when(Boolean(this.file), el => el.child(this.code
        ? v_flex().id("files-editor").flex_1().min_h(0)
          // Ctrl+S / Cmd+S saves now; the key travels up from the focused editor.
          .on_key_down((e, cx) => {
            if (e.key !== "s" || !(e.modifiers.control || e.modifiers.platform) || e.modifiers.shift || e.modifiers.alt) return;
            this.save(cx); cx.stop_propagation(); cx.notify();
          })
          .child(Editor.new(this.code).flex_1().min_h(0))
        : this.opened ? this.openedCard(t, button)
        : div().p(12).text_color(t.dim).child(this.fileError || "Loading file…")));
  }

  // A file that is not text: what was done with it, and the two ways to do it again.
  openedCard(t, button) {
    const o = this.opened, name = o.full.split("/").pop();
    const say = o.how === "binary" ? "Binary file · no text preview" : HOW[o.how] ?? `Cannot open this file: ${reason(o.how)}`;
    const again = (id, label, options) => button(id, label, (_e, cx) => {
      const how = openFile(o.full, options);
      if (this.opened === o) { this.opened = { full: o.full, how }; cx.notify(); }
    });
    return v_flex().id("files-opened").flex_1().min_h(0).items_center().justify_center().gap(10).p(16)
      .child(div().max_w_full().truncate().font_semibold().child(name))
      .child(div().text_size(12).text_color(t.dim).child(say))
      .child(h_flex().gap(8).child(again("files-open-again", "Open again")).child(again("files-show-folder", "Show in folder", { reveal: true })));
  }

  // The panel or the app closes: write what's pending (bounded by the host's close budget).
  async close(cx) {
    await this.save(cx);
  }
}

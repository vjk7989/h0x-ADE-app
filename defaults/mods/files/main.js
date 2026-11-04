// A lazy file browser for the focused pane's folder. Local files edit in place and save
// themselves (VS Code-style auto-save); remote and truncated files are read-only previews.
import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, Editor, EditorState } from "gpui-base";
import { spaces, panes, focused, agents, core, exec, readTextFile, writeTextFile, watchFolder, theme, settings, window } from "h0x";

function location() {
  const space = spaces().find(s => s.active), f = focused();
  const pane = f?.pane ? panes().find(p => p.pane === f.pane) : null;
  const on = f?.pane === null ? (f.terminal?.includes("/") ? f.terminal.split("/")[0] : null) : space?.machine ?? null;
  return { cwd: pane?.cwd || f?.cwd || space?.folder, on, space: space?.id };
}
const key = () => JSON.stringify(location());
const LANG = { ts: "js", tsx: "js", mts: "js", cts: "js", jsx: "js", mjs: "js", toml: "yaml", jsonc: "json", svelte: "html", vue: "html" };
const PER_FOLDER = 500, TOTAL = 5000;
// Saves of one file this close together are one edit; a note lists at most LISTED files.
const BURST = 30000, LISTED = 5;
// A file is written back with the line endings it was read with: CRLF only when every line had it.
const eol = text => /\r\n/.test(text) && !/(^|[^\r])\n/.test(text) ? "\r\n" : "\n";
const text = doc => { const value = doc.state.value(); return doc.eol === "\r\n" ? value.replace(/\r?\n/g, "\r\n") : value; };

async function directory(path, on) {
  if (!on && window()?.platform === "windows") {
    // Windows PowerShell otherwise uses the console code page; exec decodes UTF-8.
    // Links keep directory attributes even when dangling; check their target.
    // Other reparse points (OneDrive's folders) have no LinkType and are folders.
    // PowerShell reads curly single quotes as quotes too, so they're doubled like '.
    const r = await exec("powershell", ["-NoProfile", "-Command", "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); & { param($p) @(Get-ChildItem -LiteralPath $p -Force -ErrorAction Stop | Where-Object Name -ne '.git' | Select-Object -First 501 | ForEach-Object { @{ name=$_.Name; dir=($_.PSIsContainer -and (-not ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -or -not $_.LinkType -or ($_.Target -and [IO.Directory]::Exists(([IO.Path]::Combine([IO.Path]::GetDirectoryName($_.FullName), $_.Target[0])))))) } }) | ConvertTo-Json -Compress } '" + path.replace(/['‘’‚‛]/g, "$&$&") + "'"]);
    if (r.code !== 0) throw new Error(r.stderr || "Cannot list this folder");
    const result = JSON.parse(r.stdout || "[]");
    return Array.isArray(result) ? result : [result];
  }
  const script = 'n=0; for p in "$1"/* "$1"/.[!.]* "$1"/..?*; do [ -e "$p" ] || [ -L "$p" ] || continue; name=${p##*/}; [ "$name" = .git ] && continue; if [ -d "$p" ]; then kind=d; else kind=f; fi; printf "%s\\0%s\\0" "$kind" "$name"; n=$((n+1)); [ "$n" -gt 500 ] && break; done; test -d "$1"';
  const r = await exec("sh", ["-c", script, "files", path], { on });
  if (r.code !== 0) throw new Error(r.stderr || "Cannot list this folder");
  const fields = r.stdout.split("\0"), out = [];
  for (let i = 0; i + 1 < fields.length; i += 2) out.push({ name: fields[i + 1], dir: fields[i] === "d" });
  return out;
}

export default class Files extends View {
  init(_props, cx) {
    this.saved = new Map(); this.drafts = new Map(); this.edits = new Map();
    this.entries = new Map(); this.folders = new Set(); this.pending = new Set(); this.marks = {};
    this.refresh(cx);
    cx.timer.every(10000, cx => this.refresh(cx));
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
      this.pending = new Set(); this.marks = {}; this.show(null, cx); this.watched = null; this.error = null; this.limited = false;
      this.loading = false;
      if (saved?.file) this.show(saved.file, cx);
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

  show(path, cx) {
    const prev = this.doc;
    if (prev && cx) {
      // Leaving a file saves it; edits held back by a disk conflict wait for its reopening.
      if (prev.conflict && text(prev) !== prev.saved) { prev.draft = text(prev); this.drafts.set(prev.full, prev); }
      this.save(cx);
    }
    prev?.timer?.cancel();
    this.code?.release(); this.code = null; this.doc = null; this.file = path; this.fileError = null;
    if (!path || !cx) return;
    const context = this.context, root = this.root, on = this.on, full = `${root}/${path}`;
    cx.spawn(async cx => {
      try {
        const disk = await readTextFile(full, { on });
        if (this.file !== path || !this.current(context)) return;
        if (disk === null) { this.fileError = "This file is no longer available"; return; }
        if (disk.includes("\0")) { this.fileError = "Binary file · no text preview"; return; }
        const t = theme(), ext = path.includes(".") ? path.split(".").pop() : "txt";
        const large = disk.length > 400000, draft = on || large ? null : this.drafts.get(full);
        this.drafts.delete(full);
        this.code = EditorState.new({ language: LANG[ext] ?? ext, readonly: Boolean(on) || large,
          value: large ? disk.slice(0, 400000) + "\n… (file truncated)" : draft ? draft.draft : disk,
          font_family: settings().font_family, font_size: 12,
          syntax: { keyword: t.ansi5, string: t.ansi2, comment: t.dim, function: t.ansi4, type: t.ansi3, number: t.ansi1, constant: t.ansi6 } });
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
      } catch { if (this.file === path && this.current(context)) this.fileError = "Cannot preview this file (binary, too large, or unreadable)"; }
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
          const more = news.length - LISTED;
          const list = news.slice(0, LISTED).map(e => e.path).join(", ") + (more > 0 ? ` and ${more} more` : "");
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
        .child(div().flex_1().min_w(0).truncate().text_right().text_size(12).text_color(t.dim).child(this.file ? (doc?.dirty ? "● " : "") + this.file : this.root?.split(/[\\/]/).pop() ?? "")))
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
        : div().p(12).text_color(t.dim).child(this.fileError || "Loading file…")));
  }

  // The panel or the app closes: write what's pending (bounded by the host's close budget).
  async close(cx) {
    await this.save(cx);
  }
}

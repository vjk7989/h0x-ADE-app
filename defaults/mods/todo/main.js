// To-do lists: Markdown files in the app folder (todo/<list>.md, todo/lists.md) shown as a
// simple task list. Tasks have steps; ticking the last open task offers a small celebration.
// The same mod draws the pop-up (props().celebrate) and the confetti (props().confetti).
import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { appHome, readTextFile, writeTextFile, removeAppFile, watchFolder, nextCommand, notify, openOverlay, closeOverlay, props, run, theme, windowVisible } from "h0x";
import { parseList, serializeList, parseIndex, serializeIndex, slugify, openCount, allDone, escapeTitle } from "./model.js";

const ACTIONS = ["new", "rename", "delete"];
const DEFAULT = { name: "Tasks", slug: "tasks" };
const UNDO_MS = 8000, CONFETTI_MS = 1500, CONFETTI_PIECES = 36;
const READ_FAIL = "This list can't be read (over 1 MB or unreadable). It is left untouched and read-only.";
const FULL = "This list is full (512 KB). Start a new list for more tasks.";
// The panel never writes a list past 512 KB (about 10,000 tasks): the host's readTextFile refuses
// files over 1 MiB, and this keeps every list we write well clear of it.
const MAX_BYTES = 512 * 1024;
// UTF-8 length by hand: the mod runtime has no TextEncoder.
const utf8Length = (text) => {
  let n = 0;
  for (const ch of text) {
    const c = ch.codePointAt(0);
    n += c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
  }
  return n;
};
const fits = (text) => text.length <= MAX_BYTES / 4 || utf8Length(text) <= MAX_BYTES;
const same = (a, b) => a && b && a.k === b.k && a.i === b.i && a.j === b.j;

// Mutations are replayed on fresh disk text after a stale write, so they find their task by
// title (near the old position) instead of trusting an index.
const findTask = (list, i, title) => (list.tasks[i]?.title === title ? i : list.tasks.findIndex((t) => t.title === title));
const findStep = (task, j, title) => (task.steps[j]?.title === title ? j : task.steps.findIndex((s) => s.title === title));
// fn(array, index, tasks, taskIndex) works on the array holding the task or the step.
const on = (ref, fn) => (list) => {
  const i = findTask(list, ref.i, ref.title);
  if (i < 0) return false;
  if (ref.j == null) return fn(list.tasks, i, list.tasks, i);
  const j = findStep(list.tasks[i], ref.j, ref.stepTitle);
  return j < 0 ? false : fn(list.tasks[i].steps, j, list.tasks, i);
};
const setDone = (ref, done) => on(ref, (a, k) => { a[k].done = done; });
const setTitle = (ref, title) => on(ref, (a, k) => { a[k].title = title; });
const remove = (ref) => on(ref, (a, k) => { a.splice(k, 1); });
// Moves among its own kind: a task past open tasks only, a step past steps.
const move = (ref, dir) => on(ref, (a, k) => {
  let to = k + dir;
  while (ref.j == null && a[to] && a[to].done !== a[k].done) to += dir;
  if (!a[to]) return false;
  [a[k], a[to]] = [a[to], a[k]];
});
// The task becomes the last step of `parent` ({ i, title }): the task shown above it.
const indent = (ref, parent) => on(ref, (a, k, tasks) => {
  const p = findTask({ tasks }, parent.i, parent.title);
  if (p < 0 || p >= k || a[k].steps.length) return false;
  const [t] = a.splice(k, 1);
  tasks[p].steps.push({ title: t.title, done: t.done, extra: t.extra });
});
const outdent = (ref) => on(ref, (steps, j, tasks, i) => {
  const [s] = steps.splice(j, 1);
  tasks.splice(i + 1, 0, { title: s.title, done: s.done, steps: [], extra: s.extra });
});

export default class Todo extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    const p = props() ?? {};
    if (p.confetti) {
      this.size = null;
      cx.timer.after(CONFETTI_MS, () => closeOverlay(p.overlay));
      return;
    }
    if (p.celebrate !== undefined) { this.handle.focus(); return; }
    this.dir = `${appHome()}/todo`;
    this.index = []; this.indexText = null; this.cur = null; this.list = parseList(""); this.text = ""; this.disk = "";
    this.pending = []; this.open = new Set(); this.showDone = false; this.sel = { k: "head" }; this.pos = 0;
    this.edit = null; this.naming = null; this.menu = false; this.menuAt = 0; this.confirming = false; this.hover = null;
    this.readonly = false; this.ready = false; this.error = null; this.shown = false; this.wantFocus = false; this.badIndex = null;
    this.add = this.makeAdd();
    this.load(cx);
    cx.timer.every(10000, (cx) => { if (windowVisible()) this.reload(cx); });
    cx.spawn(async (cx) => {
      for (;;) {
        const c = await nextCommand();
        if (c === "focus") { if (this.shown) this.handle.focus(); else this.wantFocus = true; }
        else if (c === "undo") this.undo(cx);
        cx.notify();
      }
    });
  }

  file(slug) { return `${this.dir}/${slug}.md`; }
  setText(text) { this.text = text; this.list = parseList(text); }
  name() { return this.index.find((e) => e.slug === this.cur)?.name ?? this.list.name; }

  // ---- storage -------------------------------------------------------------------------

  // The index and the selected list; first run (or a lost index) creates "Tasks".
  async load(cx) {
    try {
      const text = await readTextFile(`${this.dir}/lists.md`);
      let entries = parseIndex(text ?? "");
      if (!entries.length && (text ?? "").trim()) {
        // Lines we can't read: the file is never touched until the person clicks "Start a new index".
        this.badIndex = text; this.error = "lists.md has no lists HyperOx can read.";
        return cx.notify();
      }
      if (!entries.length) {
        entries = [DEFAULT];
        await writeTextFile(this.file(DEFAULT.slug), serializeList({ name: DEFAULT.name, head: [], tasks: [] }), null).catch(() => {});
        await writeTextFile(`${this.dir}/lists.md`, serializeIndex(entries), text);
      }
      this.badIndex = null; this.error = null;
      this.index = entries; this.indexText = serializeIndex(entries);
      const saved = localStorage.getItem("todo.list");
      await this.openList((entries.find((e) => e.slug === saved) ?? entries[0]).slug, cx);
      this.watch(cx);
    } catch (e) {
      this.error = `Can't open your lists: ${e?.message ?? e}`;
    }
    cx.notify();
  }

  async startNewIndex(cx) {
    const old = this.badIndex;
    this.badIndex = null;
    try {
      await writeTextFile(this.file(DEFAULT.slug), serializeList({ name: DEFAULT.name, head: [], tasks: [] }), null).catch(() => {});
      await writeTextFile(`${this.dir}/lists.md`, serializeIndex([DEFAULT]), old);
    } catch (e) { this.badIndex = old; this.error = `Can't start a new index: ${e?.message ?? e}`; return cx.notify(); }
    this.load(cx);
  }

  watch(cx) {
    if (this.watching) return;
    this.watching = true;
    cx.spawn(async (cx) => {
      try { for (;;) { await watchFolder(this.dir); if (windowVisible()) this.reload(cx); } } catch { /* the 10 s timer still looks */ }
    });
  }

  // Never a silent hang: anything unexpected while opening a list is shown, read-only.
  async openList(slug, cx) {
    try { await this.openListFrom(slug, cx); }
    catch (e) {
      if (this.cur !== slug) return;
      this.readonly = true; this.ready = true; this.error = `Couldn't open this list: ${e?.message ?? e}`;
      cx.notify();
    }
  }

  async openListFrom(slug, cx) {
    while (this.flushing) await cx.sleep(50);
    this.cur = slug; this.pending = []; this.edit = null; this.error = null; this.readonly = false;
    localStorage.setItem("todo.list", slug);
    let text;
    try { text = await readTextFile(this.file(slug)); } catch { text = undefined; }
    if (this.cur !== slug) return;
    if (text === undefined) {
      this.readonly = true; this.error = READ_FAIL; this.disk = ""; this.setText(serializeList({ name: this.name(), head: [], tasks: [] }));
    } else {
      if (text === null) {
        text = serializeList({ name: this.name(), head: [], tasks: [] });
        await writeTextFile(this.file(slug), text, null).catch(() => {});
      }
      this.disk = text; this.setText(text);
    }
    this.ready = true; this.pos = 0; this.sel = this.rows()[0] ?? { k: "head" };
    cx.notify();
  }

  // A look at the disk (watch event or 10 s): other editors' changes appear; our own writes and
  // anything while we have unsaved changes or an open edit are left alone.
  async reload(cx) {
    if (!this.ready || this.flushing || this.pending.length || this.edit || this.naming || this.reloading) return;
    this.reloading = true;
    const slug = this.cur;
    try {
      const idx = await readTextFile(`${this.dir}/lists.md`).catch(() => null);
      if (idx !== null && idx !== this.indexText) {
        const entries = parseIndex(idx);
        if (entries.length) { this.index = entries; this.indexText = idx; }
        if (!this.index.some((e) => e.slug === slug)) { this.reloading = false; return this.openList(this.index[0].slug, cx); }
      }
      let text;
      try { text = await readTextFile(this.file(slug)); }
      catch { if (this.cur === slug && !this.readonly) { this.readonly = true; this.error = READ_FAIL; } return; }
      if (this.cur !== slug || this.flushing || this.pending.length || this.edit || text === null || text === this.disk) return;
      this.disk = text; this.setText(text); this.readonly = false; this.error = null;
    } finally { this.reloading = false; cx.notify(); }
  }

  // One change to the open list. The screen shows it at once; the write follows.
  commit(mut, cx) {
    if (this.readonly || !this.ready) return false;
    const list = parseList(this.text);
    if (mut(list) === false) return false;
    const next = serializeList(list);
    if (!fits(next)) { this.error = FULL; cx.notify(); return false; }
    this.setText(next);
    this.pending.push(mut);
    this.flush(cx);
    cx.notify();
    return true;
  }

  // Writes the latest text over what we last read or wrote. A stale write re-reads the file and
  // re-applies our unsaved changes to it; a second failure keeps them and says so.
  flush(cx) {
    if (this.flushing) return;
    this.flushing = true;
    const slug = this.cur, path = this.file(slug);
    cx.spawn(async (cx) => {
      try {
        while (this.pending.length && this.cur === slug) {
          const batch = this.pending.slice(), next = this.text;
          try {
            try { await writeTextFile(path, next, this.disk); this.disk = next; }
            catch (first) {
              const fresh = await readTextFile(path).catch(() => undefined);
              if (fresh === undefined) throw first;
              const list = parseList(fresh ?? serializeList({ name: this.name(), head: [], tasks: [] }));
              for (const mut of batch) mut(list);
              const merged = serializeList(list);
              if (!fits(merged)) { this.pending = []; this.disk = fresh; this.setText(fresh); this.error = FULL; return; }
              await writeTextFile(path, merged, fresh);
              this.disk = merged;
              const rest = this.pending.slice(batch.length), after = parseList(merged);
              for (const mut of rest) mut(after);
              this.setText(serializeList(after));
            }
            this.pending.splice(0, batch.length); this.error = null;
          } catch (e) {
            this.error = `Not saved yet (${String(e?.message ?? e).slice(0, 100)}). Your changes are kept here and will be retried.`;
            break;
          }
        }
      } finally { this.flushing = false; cx.notify(); }
    });
  }

  // The lists file with one change, re-read and retried once if someone else wrote meanwhile.
  async editIndex(mut) {
    const path = `${this.dir}/lists.md`;
    for (let n = 0; ; n++) {
      const text = await readTextFile(path);
      const entries = parseIndex(text ?? "");
      if (mut(entries) === false) return false;
      const next = serializeIndex(entries);
      if (!fits(next)) { this.error = FULL; return false; }
      try { await writeTextFile(path, next, text); this.index = entries; this.indexText = next; return true; }
      catch (e) { if (n) throw e; }
    }
  }

  // ---- list switcher -------------------------------------------------------------------

  switchTo(slug, cx) {
    this.menu = false; this.confirming = false;
    if (slug !== this.cur) this.openList(slug, cx);
    this.handle.focus(); cx.notify();
  }

  askName(mode, cx) {
    this.menu = false;
    const value = mode === "rename" ? this.name() : "";
    const input = this.field(value, "List name", (v, cx) => this.finishName(v, cx), (v, cx) => { if (this.naming?.input === input) this.naming = null; cx.notify(); });
    this.naming = { mode, input };
    cx.notify();
  }

  async finishName(value, cx) {
    const { mode } = this.naming ?? {};
    this.naming = null; this.handle.focus();
    const name = escapeTitle(value).trim();
    if (!mode || !name) return cx.notify();
    try {
      if (mode === "new") {
        let slug;
        const added = await this.editIndex((entries) => {
          slug = slugify(name, entries.map((e) => e.slug));
          entries.push({ name, slug });
        });
        if (added === false) return cx.notify();
        await writeTextFile(this.file(slug), serializeList({ name, head: [], tasks: [] }), null);
        await this.openList(slug, cx);
      } else {
        await this.editIndex((entries) => { const e = entries.find((x) => x.slug === this.cur); if (!e) return false; e.name = name; });
        this.commit((list) => { list.name = name; }, cx);
      }
    } catch (e) { this.error = `Couldn't ${mode} the list: ${e?.message ?? e}`; }
    cx.notify();
  }

  async deleteList(cx) {
    this.confirming = false; this.menu = false;
    const slug = this.cur;
    try {
      while (this.flushing) await cx.sleep(50);
      this.pending = [];
      await this.editIndex((entries) => { const k = entries.findIndex((e) => e.slug === slug); if (k < 0) return false; entries.splice(k, 1); });
      await removeAppFile(this.file(slug));
      if (this.index.length) await this.openList(this.index[0].slug, cx);
      else await this.load(cx);
    } catch (e) { this.error = `Couldn't delete the list: ${e?.message ?? e}`; }
    cx.notify();
  }

  // ---- tasks ---------------------------------------------------------------------------

  // What is on screen, top to bottom. A row is { k: "task" | "step" | "add" | "head", i, j }.
  rows() {
    const out = [], tasks = this.list.tasks;
    const push = (i) => {
      out.push({ k: "task", i });
      if (!this.open.has(tasks[i].title)) return;
      tasks[i].steps.forEach((_s, j) => out.push({ k: "step", i, j }));
      out.push({ k: "add", i });
    };
    tasks.forEach((t, i) => { if (!t.done) push(i); });
    const done = tasks.map((t, i) => (t.done ? i : -1)).filter((i) => i >= 0);
    if (done.length) {
      out.push({ k: "head" });
      if (this.showDone) done.forEach(push);
    }
    return out;
  }

  // Keeps the selection on a row that exists (after a tick moved it, a delete, a reload).
  current() {
    const rows = this.rows(), at = rows.findIndex((r) => same(r, this.sel));
    if (at >= 0) this.pos = at;
    else this.sel = rows[Math.min(this.pos, rows.length - 1)] ?? { k: "head" };
    return rows;
  }

  item(row) { const t = this.list.tasks[row.i]; return row.j == null ? t : t?.steps[row.j]; }
  ref(row) {
    const t = this.list.tasks[row.i];
    return { i: row.i, title: t.title, j: row.j, stepTitle: row.j == null ? null : t.steps[row.j]?.title };
  }

  select(row, cx) { this.sel = row; this.handle.focus(); cx.notify(); }

  go(delta, cx) {
    const rows = this.current(), at = rows.findIndex((r) => same(r, this.sel));
    this.select(rows[Math.max(0, Math.min(rows.length - 1, at + delta))] ?? this.sel, cx);
  }

  tick(row, cx) {
    if (row.k === "head") { this.showDone = !this.showDone; return cx.notify(); }
    if (row.k === "add") return this.startStep(row.i, cx);
    this.select(row, cx);
    const item = this.item(row);
    if (!item) return;
    const done = !item.done;
    if (this.commit(setDone(this.ref(row), done), cx) && done && row.j == null && allDone(this.list)) this.celebrate();
  }

  // The tick that finishes the list (never a load, an add or a delete) offers the pop-up.
  celebrate() {
    openOverlay({ mod: "todo", kind: "dialog", width: 400, props: { celebrate: this.name() } });
  }

  fold(row, open, cx) {
    if (row.k === "head") this.showDone = open;
    else if (row.k === "task") {
      const title = this.item(row)?.title;
      if (open) this.open.add(title); else this.open.delete(title);
    } else if (row.k === "step" && !open) this.sel = { k: "task", i: row.i };
    cx.notify();
  }

  startEdit(row, cx) {
    const item = this.item(row);
    if (!item || this.readonly || (row.k !== "task" && row.k !== "step")) return;
    this.select(row, cx);
    const ref = this.ref(row);
    const input = this.field(item.title, "", (v, cx) => this.finishEdit(v, ref, cx), (v, cx) => { if (this.edit?.input === input) this.finishEdit(v, ref, cx); });
    this.edit = { row, input };
    cx.notify();
  }

  finishEdit(value, ref, cx) {
    this.edit = null; this.handle.focus();
    const title = escapeTitle(value).trim();
    if (title) this.commit(setTitle(ref, title), cx);
    cx.notify();
  }

  startStep(i, cx) {
    const task = this.list.tasks[i];
    if (!task || this.readonly) return;
    this.open.add(task.title); this.sel = { k: "add", i };
    const title = task.title;
    const input = this.field("", "Add step", (v, cx) => this.addStep(i, title, v, cx), (v, cx) => { if (this.edit?.input === input) { this.edit = null; this.addStep(i, title, v, cx, true); } });
    this.edit = { row: { k: "add", i }, input };
    cx.notify();
  }

  addStep(i, taskTitle, value, cx, leave = false) {
    const title = escapeTitle(value).trim();
    if (title) this.commit((list) => { const k = findTask(list, i, taskTitle); if (k < 0) return false; list.tasks[k].steps.push({ title, done: false, extra: [] }); }, cx);
    if (leave || !title) { this.edit = null; this.handle.focus(); } else this.startStep(i, cx);
    cx.notify();
  }

  addTask(value, cx) {
    const title = escapeTitle(value).trim();
    if (!title) return;
    // A refused add (a full list) keeps what was typed.
    if (!this.commit((list) => { list.tasks.unshift({ title, done: false, steps: [], extra: [] }); }, cx)) return cx.notify();
    this.sel = { k: "task", i: 0 };
    this.add.set_value("");
    cx.notify();
  }

  makeAdd() {
    const input = InputState.new({ placeholder: "Add a task" });
    input.on("submit", (_e, cx) => this.addTask(input.value(), cx));
    input.on("focus", (_e, cx) => { this.addFocus = true; cx.notify(); });
    input.on("blur", (_e, cx) => { this.addFocus = false; cx.notify(); });
    return input;
  }

  field(value, placeholder, submit, blur) {
    const input = InputState.new({ value, placeholder });
    if (value) input.set_selected_range({ start: 0, end: value.length });
    input.on("submit", (_e, cx) => submit(input.value(), cx));
    input.on("blur", (_e, cx) => blur(input.value(), cx));
    input.focus();
    return input;
  }

  delete(row, cx) {
    const item = this.item(row);
    if (!item || row.k === "add" || row.k === "head") return;
    const ref = this.ref(row), copy = JSON.parse(JSON.stringify(item)), slug = this.cur;
    if (!this.commit(remove(ref), cx)) return;
    const restore = (list) => {
      if (ref.j == null) return void list.tasks.splice(Math.min(ref.i, list.tasks.length), 0, copy);
      const k = findTask(list, ref.i, ref.title);
      if (k < 0) return false;
      list.tasks[k].steps.splice(Math.min(ref.j, list.tasks[k].steps.length), 0, copy);
    };
    this.undoing = { slug, restore };
    notify({ title: ref.j == null ? "Task deleted" : "Step deleted", body: copy.title, timeout: UNDO_MS, action: { name: "todo.undo" } });
  }

  undo(cx) {
    const u = this.undoing;
    this.undoing = null;
    if (u && u.slug === this.cur) this.commit(u.restore, cx);
  }

  reorder(row, dir, cx) {
    if (row.k !== "task" && row.k !== "step") return;
    const list = this.list, a = row.j == null ? list.tasks : list.tasks[row.i].steps, k = row.j ?? row.i;
    let to = k + dir;
    while (row.j == null && a[to] && a[to].done !== a[k].done) to += dir;
    if (this.commit(move(this.ref(row), dir), cx)) this.sel = row.j == null ? { k: "task", i: to } : { k: "step", i: row.i, j: to };
  }

  indentRow(row, cx) {
    if (row.k !== "task" || this.item(row).steps.length) return;
    // The task shown above it in the same part of the screen (never a hidden finished one).
    const rows = this.rows();
    let at = rows.findIndex((r) => same(r, row)) - 1;
    while (at >= 0 && rows[at].k !== "task" && rows[at].k !== "head") at--;
    if (at < 0 || rows[at].k !== "task") return;
    const parent = this.list.tasks[rows[at].i], pi = rows[at].i;
    if (this.commit(indent(this.ref(row), { i: pi, title: parent.title }), cx)) {
      this.open.add(parent.title);
      this.sel = { k: "step", i: pi, j: this.list.tasks[pi].steps.length - 1 };
    }
  }

  outdentRow(row, cx) {
    if (row.k === "step" && this.commit(outdent(this.ref(row)), cx)) this.sel = { k: "task", i: row.i + 1 };
  }

  // ---- keys ----------------------------------------------------------------------------

  key(e, cx) {
    const k = e.key, m = e.modifiers ?? {}, mod = m.control || m.platform;
    const typing = this.edit || this.naming || this.addFocus;
    if (k === "escape") {
      if (this.edit) { this.edit = null; this.handle.focus(); }
      else if (this.naming) { this.naming = null; this.handle.focus(); }
      else if (this.confirming) this.confirming = false;
      else if (this.menu) this.menu = false;
      else if (this.addFocus) this.handle.focus();
      else run("pane.go", { direction: "left" });
    } else if (typing) {
      return;
    } else if (this.confirming) {
      if (k === "enter") this.deleteList(cx); else return;
    } else if (this.menu) {
      this.menuKey(k, cx);
    } else if (mod && k === "l") {
      this.openMenu(cx);
    } else if (mod && k === "enter") {
      this.add.focus();
    } else if (!m.alt && !mod && !m.shift && k === "n") {
      this.add.focus();
    } else {
      this.listKey(k, m, cx);
    }
    cx.stop_propagation();
    cx.notify();
  }

  listKey(k, m, cx) {
    this.current();
    const row = this.sel;
    if (k === "up" || k === "down") return m.alt ? this.reorder(row, k === "up" ? -1 : 1, cx) : this.go(k === "up" ? -1 : 1, cx);
    if (k === "right") return this.fold(row, true, cx);
    if (k === "left") return this.fold(row, false, cx);
    if (k === "space" || k === " ") return this.tick(row, cx);
    if (k === "enter") return m.shift ? this.startStep(row.i, cx) : row.k === "head" ? this.fold(row, !this.showDone, cx) : row.k === "add" ? this.startStep(row.i, cx) : this.startEdit(row, cx);
    if (k === "tab") return m.shift ? this.outdentRow(row, cx) : this.indentRow(row, cx);
    if (k === "delete" || k === "backspace") this.delete(row, cx);
  }

  openMenu(cx) {
    this.menu = true; this.confirming = false;
    this.menuAt = Math.max(0, this.index.findIndex((l) => l.slug === this.cur));
    cx.notify();
  }

  menuKey(k, cx) {
    const n = this.index.length + ACTIONS.length;
    if (k === "down") this.menuAt = Math.min(n - 1, this.menuAt + 1);
    else if (k === "up") this.menuAt = Math.max(0, this.menuAt - 1);
    else if (k === "enter") this.menuPick(this.menuAt, cx);
  }

  menuPick(at, cx) {
    if (at < this.index.length) return this.switchTo(this.index[at].slug, cx);
    const action = ACTIONS[at - this.index.length];
    if (action === "delete") { this.menu = false; this.confirming = true; cx.notify(); }
    else this.askName(action, cx);
  }

  // ---- drawing -------------------------------------------------------------------------

  render(cx) {
    const p = props() ?? {}, t = theme();
    if (!p.confetti && p.celebrate === undefined && !this.shown) {
      this.shown = true;
      if (this.wantFocus) { this.wantFocus = false; cx.timer.after(0, () => this.handle.focus()); }
    }
    if (p.confetti) return this.confetti(t);
    if (p.celebrate !== undefined) return this.dialog(p, t);
    const rows = this.ready ? this.current() : [], busy = this.edit || this.naming;
    return v_flex().id("todo").size_full().min_h(0).rounded(10).overflow_hidden().bg(t.background).border_1().border_color(t.border).text_color(t.foreground).text_size(13)
      .track_focus(this.handle).on_key_down((e, cx) => this.key(e, cx))
      .child(this.header(t))
      .when(this.menu, (el) => el.child(this.switcher(t)))
      .when(this.confirming, (el) => el.child(this.confirmBar(t)))
      .when(Boolean(this.naming), (el) => el.child(this.nameBar(t)))
      .when(Boolean(this.error), (el) => el.child(div().flex_none().px(14).py(6).text_size(12).text_color(t.ansi1).child(this.error)))
      .when(Boolean(this.badIndex), (el) => el.child(this.newIndexBar(t)))
      .when(!this.readonly && this.ready, (el) => el.child(this.addBar(t)))
      .child(v_flex().id("todo-list").flex_1().min_h(0).overflow_y_scrollbar().p(6).gap(2)
        .children(rows.map((r) => this.row(r, t)))
        .when(this.ready && !this.readonly && !this.list.tasks.length, (el) => el.child(div().p(10).text_color(t.dim).child("No tasks yet. Add one above.")))
        .when(this.ready && this.list.tasks.length > 0 && openCount(this.list) === 0, (el) => el.child(div().px(10).py(8).text_color(t.dim).child("All done for today"))))
      .child(div().flex_none().px(14).py(8).border_t(1).border_color(t.border).text_size(11).text_color(t.dim).truncate()
        .child(busy ? "Enter save · Esc cancel" : "↑↓ move · Space done · Enter edit · N new · Del remove"));
  }

  header(t) {
    return h_flex().h(42).flex_none().px(10).gap(8).items_center().border_b(1).border_color(t.border)
      .child(h_flex().id("todo-switch").h(28).px(8).gap(6).items_center().rounded(7).cursor_pointer().hover((s) => s.bg(t.hover))
        .on_click((_e, cx) => { if (this.menu) { this.menu = false; this.handle.focus(); cx.notify(); } else this.openMenu(cx); })
        .child(div().font_semibold().max_w(220).truncate().child(this.ready ? this.name() : "To-do"))
        .child(svg("mods/workspace-chrome/chevron-down.svg").size(12).flex_none().text_color(t.dim)))
      .child(div().flex_1())
      .when(this.ready, (el) => el.child(div().text_size(11).text_color(t.dim).child(`${openCount(this.list)} open`)));
  }

  switcher(t) {
    const item = (id, label, at, check) => h_flex().id(id).h(30).px(10).gap(8).items_center().rounded(7).cursor_pointer()
      .when(this.menuAt === at, (el) => el.bg(t.selected)).hover((s) => s.bg(t.hover))
      .on_click((_e, cx) => this.menuPick(at, cx))
      .child(div().w(12).flex_none().child(check ? svg("mods/workspace-chrome/check.svg").size(12).text_color(t.accent) : ""))
      .child(div().flex_1().min_w(0).truncate().child(label));
    const n = this.index.length;
    return v_flex().flex_none().p(6).bg(t.raised).border_b(1).border_color(t.border)
      .children(this.index.map((l, at) => item(`list-${l.slug}`, l.name, at, l.slug === this.cur)))
      .child(div().h(1).my(4).bg(t.border))
      .child(item("list-new", "New list", n, false))
      .child(item("list-rename", "Rename list", n + 1, false))
      .child(item("list-delete", "Delete list", n + 2, false));
  }

  confirmBar(t) {
    const button = (id, label, primary, click) => div().id(id).px(10).py(3).rounded(6).cursor_pointer().border_1().border_color(primary ? t.ansi1 : t.border)
      .when(primary, (el) => el.text_color(t.ansi1)).hover((s) => s.bg(t.hover)).on_click(click).child(label);
    return h_flex().flex_none().px(14).py(8).gap(8).items_center().bg(t.raised).border_b(1).border_color(t.border).text_size(12)
      .child(div().flex_1().min_w(0).truncate().child(`Delete “${this.name()}” and its tasks?`))
      .child(button("todo-del-no", "Cancel", false, (_e, cx) => { this.confirming = false; this.handle.focus(); cx.notify(); }))
      .child(button("todo-del-yes", "Delete", true, (_e, cx) => this.deleteList(cx)));
  }

  newIndexBar(t) {
    return h_flex().flex_none().px(14).py(8).gap(8).items_center().bg(t.raised).border_b(1).border_color(t.border).text_size(12)
      .child(div().flex_1().min_w(0).truncate().child("Your lists.md is kept as it is."))
      .child(div().id("todo-new-index").px(10).py(3).rounded(6).cursor_pointer().border_1().border_color(t.border).hover((s) => s.bg(t.hover))
        .on_click((_e, cx) => this.startNewIndex(cx)).child("Start a new index"));
  }

  nameBar(t) {
    return h_flex().flex_none().px(10).py(8).gap(8).items_center().bg(t.raised).border_b(1).border_color(t.border)
      .child(div().text_size(12).text_color(t.dim).child(this.naming.mode === "new" ? "New list" : "Rename"))
      .child(Input.new(this.naming.input).flex_1().min_w(0).h(28).px(8).rounded(7).border_1().border_color(t.accent).bg(t.background).text_size(12.5));
  }

  addBar(t) {
    return h_flex().id("todo-add").flex_none().mx(10).mt(10).mb(4).h(36).px(10).gap(8).items_center().rounded(9).border_1().border_color(this.addFocus ? t.accent : t.border).bg(t.surface)
      .child(svg("mods/workspace-chrome/plus.svg").size(14).flex_none().text_color(t.accent))
      .child(Input.new(this.add).flex_1().min_w(0).h_full().text_size(12.8).text_color(t.foreground));
  }

  row(r, t) {
    if (r.k === "head") return this.headRow(r, t);
    if (r.k === "add") return this.addStepRow(r, t);
    return this.taskRow(r, t);
  }

  headRow(r, t) {
    const count = this.list.tasks.filter((x) => x.done).length, sel = same(r, this.sel);
    return h_flex().id("todo-done").h(30).flex_none().px(8).gap(8).items_center().rounded(7).cursor_pointer().text_color(t.muted)
      .when(sel, (el) => el.bg(t.selected)).hover((s) => s.bg(t.hover))
      .on_click((_e, cx) => { this.sel = r; this.showDone = !this.showDone; this.handle.focus(); cx.notify(); })
      .child(svg(`mods/workspace-chrome/chevron-${this.showDone ? "down" : "right"}.svg`).size(12).flex_none())
      .child(div().text_size(12).child(`Completed (${count})`));
  }

  addStepRow(r, t) {
    const editing = this.edit && same(this.edit.row, r), sel = same(r, this.sel);
    return h_flex().id(`todo-addstep-${r.i}`).min_h(30).flex_none().pl(42).pr(8).gap(8).items_center().rounded(7).cursor_pointer().text_color(t.dim)
      .when(sel && !editing, (el) => el.bg(t.selected)).hover((s) => s.bg(t.hover))
      .on_click((_e, cx) => this.startStep(r.i, cx))
      .child(svg("mods/workspace-chrome/plus.svg").size(12).flex_none())
      .child(editing ? Input.new(this.edit.input).flex_1().min_w(0).h(26).px(6).rounded(6).border_1().border_color(t.accent).bg(t.background).text_size(12.5).text_color(t.foreground)
        : div().text_size(12).child("Add step"));
  }

  taskRow(r, t) {
    const item = this.item(r);
    if (!item) return div();
    const step = r.k === "step", key = `${r.i}.${r.j ?? ""}`, sel = same(r, this.sel);
    const editing = this.edit && same(this.edit.row, r), shown = this.hover === key || sel;
    const stop = (_e, cx) => cx.stop_propagation();
    const circle = div().id(`todo-tick-${key}`).size(18).flex_none().rounded(9).flex().items_center().justify_center().cursor_pointer()
      .border_1().border_color(item.done ? t.accent : t.muted).when(item.done, (el) => el.bg(t.accent))
      .on_mouse_down("left", stop).on_click((_e, cx) => { cx.stop_propagation(); this.tick(r, cx); })
      .when(item.done, (el) => el.child(svg("mods/workspace-chrome/check.svg").size(11).text_color(t.accent_foreground)));
    const open = this.open.has(item.title), steps = step ? [] : item.steps;
    return h_flex().id(`todo-row-${key}`).min_h(34).flex_none().pl(step ? 30 : 8).pr(8).gap(8).items_center().rounded(7).cursor_pointer()
      .when(sel, (el) => el.bg(t.selected)).hover((s) => s.bg(t.hover))
      .on_hover((h, cx) => { this.hover = h ? key : (this.hover === key ? null : this.hover); cx.notify(); })
      .on_mouse_down("left", (e, cx) => { this.select(r, cx); if (e.click_count === 2) this.startEdit(r, cx); })
      .child(circle)
      .child(editing
        ? Input.new(this.edit.input).flex_1().min_w(0).h(26).px(6).rounded(6).border_1().border_color(t.accent).bg(t.background).text_size(12.5).text_color(t.foreground)
        : div().flex_1().min_w(0).truncate().text_color(item.done ? t.muted : t.foreground).child(item.title))
      .when(steps.length > 0 && !editing, (el) => el.child(div().text_size(11).text_color(t.dim).child(`${steps.filter((s) => s.done).length}/${steps.length}`)))
      .when(!step && !editing, (el) => el.child(div().id(`todo-fold-${key}`).w(16).flex_none().cursor_pointer()
        .on_mouse_down("left", stop).on_click((_e, cx) => { cx.stop_propagation(); this.select(r, cx); this.fold(r, !open, cx); })
        .child(svg(`mods/workspace-chrome/chevron-${open ? "down" : "right"}.svg`).size(12).text_color(t.dim))))
      .when(shown && !editing && !this.readonly, (el) => el.child(div().id(`todo-del-${key}`).w(16).flex_none().cursor_pointer().text_color(t.dim).hover((s) => s.text_color(t.ansi1))
        .on_mouse_down("left", stop).on_click((_e, cx) => { cx.stop_propagation(); this.delete(r, cx); })
        .child(svg("mods/pane/close.svg").size(12))));
  }

  // The pop-up when the last open task is ticked.
  dialog(p, t) {
    const close = () => closeOverlay(p.overlay);
    const celebrate = () => {
      close();
      if (windowVisible()) openOverlay({ mod: "todo", kind: "dialog", frame: "none", modal: true, width: 1, height: 1, props: { confetti: true } });
    };
    const button = (id, label, primary, click) => h_flex().id(id).h(32).px(14).items_center().rounded(8).border_1().border_color(primary ? t.solid : t.input)
      .bg(primary ? t.solid : t.raised).text_color(primary ? t.solid_foreground : t.foreground).text_size(12.5).font_medium().cursor_pointer()
      .hover((s) => s.opacity(0.88)).on_click(click).child(label);
    return v_flex().id("todo-congrats").track_focus(this.handle).w_full().min_h(0).p(22).gap(8).bg(t.popover).text_color(t.foreground)
      .on_key_down((e, cx) => {
        if (e.key === "escape") close(); else if (e.key === "enter") celebrate(); else return;
        cx.stop_propagation();
      })
      .child(div().text_size(18).font_semibold().child("Congratulations!"))
      .child(div().text_color(t.muted).child(`You finished every task in ${p.celebrate}.`))
      .child(h_flex().pt(14).gap(8).justify_end()
        .child(button("todo-close", "Close", false, close))
        .child(button("todo-celebrate", "Celebrate", true, celebrate)));
  }

  // Light confetti over the window: each piece falls on its own (no timer redraws), the
  // overlay closes itself after a moment.
  confetti(t) {
    const w = this.size?.width ?? 800, h = this.size?.height ?? 600;
    const colors = [t.accent, t.working, t.done, t.blocked, t.ansi2, t.ansi4, t.ansi5];
    const pieces = [];
    for (let n = 0; n < CONFETTI_PIECES; n++) {
      pieces.push(div().id(`todo-confetti-${n}`).absolute().left(((n * 53) % 100) / 100 * w).top(-20).size(6 + (n % 3) * 2).rounded(2).bg(colors[n % colors.length])
        .loop("top", { from: -20 - (n % 7) * 24, to: h + 20, period: 800 + ((n * 37) % 500) }));
    }
    return div().id("todo-confetti").size_full().relative().overflow_hidden().on_size_change((size, cx) => { this.size = size; cx.notify(); }).children(pieces);
  }

  async close(cx) {
    while (this.flushing) await cx.sleep(50);
  }
}

// To-do lists: Markdown files in the app folder (todo/<list>.md, todo/lists.md) shown as a
// simple task list. Tasks have steps (a task and its steps are ticked together); the tick that finishes the list offers a small celebration.
// The same mod draws the pop-up (props().celebrate) and the confetti (props().confetti).
import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { appHome, readTextFile, writeTextFile, removeAppFile, watchFolder, nextCommand, notify, openOverlay, closeOverlay, props, run, theme, windowVisible, fonts } from "h0x";
import { parseList, serializeList, parseIndex, serializeIndex, slugify, openCount, allDone, escapeTitle, setTaskDone, setStepDone } from "./model.js";

// Handwriting for task and step titles (SIL OFL, fonts/PatrickHand-Regular.ttf); the app font stands in when it is not loaded.
const HAND = "Patrick Hand";

const ACTIONS = ["new", "rename", "delete"];
const DEFAULT = { name: "Tasks", slug: "tasks" };
const DRAG_PX = 4;
const UNDO_MS = 6000, CONFETTI_MS = 1500, CONFETTI_PIECES = 36;
const READ_FAIL = "This list can't be read (over 1 MB or unreadable). It is left untouched and read-only.";
const FULL = "This list is full (64 KB, about 2,900 tasks). Start a new list for more tasks.";
// The panel never writes a list past 64 KB (about 2,900 tasks): it draws every row, so longer lists open slowly. The host readTextFile refuses
// files over 1 MiB, and this keeps every list we write well clear of it.
const MAX_BYTES = 64 * 1024;
// UTF-8 length by hand: the mod runtime has no TextEncoder.
// ASCII counts once; only the other characters are looked at one by one (fast for big plain lists).
const utf8Length = (text) => {
  let n = text.length;
  for (const ch of text.match(/[^\x00-\x7f]/gu) ?? []) {
    const c = ch.codePointAt(0);
    // Extra bytes beyond one per UTF-16 unit: 2-byte chars +1, 3-byte +2, 4-byte (a 2-unit pair) +2.
    n += c < 0x800 ? 1 : 2;
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
// A task carries its steps; the last step done makes the task done, any step undone makes it open.
const setDone = (ref, done) => on(ref, (a, k, tasks, i) => { if (ref.j == null) setTaskDone(a[k], done); else setStepDone(tasks[i], k, done); });
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
// Drag and drop. A task goes before/after another task (`to` = { i, title, after }); no-op when no open task lies between.
const placeTask = (ref, to) => on(ref, (a, k) => {
  const p = findTask({ tasks: a }, to.i, to.title);
  if (p < 0 || p === k) return false;
  const at = to.after ? p + 1 : p;
  if (a.slice(Math.min(k + 1, at), Math.max(k, at)).every((t) => t.done)) return false;
  const [t] = a.splice(k, 1);
  a.splice(at > k ? at - 1 : at, 0, t);
});
// A step goes into the steps of a task before the step titled `to.beforeTitle` (the end when null); it keeps its done state.
const placeStep = (ref, to) => on(ref, (steps, k, tasks, i) => {
  const p = findTask({ tasks }, to.i, to.title);
  if (p < 0) return false;
  const dest = tasks[p].steps;
  const at = to.beforeTitle == null ? dest.length : findStep(tasks[p], to.j, to.beforeTitle);
  if (at < 0 || (p === i && (at === k || at === k + 1))) return false;
  const [s] = steps.splice(k, 1);
  dest.splice(p === i && at > k ? at - 1 : at, 0, s);
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
    this.pending = []; this.finishing = new WeakSet(); this.open = new Set(); this.wide = new Set(); this.showDone = false; this.sel = { k: "head" }; this.pos = 0;
    this.press = null; this.drag = null; this.edit = null; this.naming = null; this.menu = false; this.menuAt = 0; this.confirming = false; this.hover = null;
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
    while (this.flushBusy(cx)) await cx.sleep(50);
    this.cur = slug; this.pending = []; this.wide.clear(); this.retryMs = 0; this.press = null; this.drag = null; this.edit = null; this.error = null; this.readonly = false;
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
    if (!this.ready || this.flushBusy(cx) || this.pending.length || this.edit || this.naming || this.reloading) return;
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
    // The sandbox cuts off a script call that runs too long (500 ms), uncatchably: all the slow
    // work (parsing a big list) is done before anything changes, and the change is queued before
    // it is shown, so a cut-off change is never shown without being queued for writing.
    const shown = parseList(next);
    this.pending.push(mut); this.text = next; this.list = shown;
    this.flush(cx);
    cx.notify();
    return true;
  }

  // Writes the latest text over what we last read or wrote. A stale write re-reads the file and
  // re-applies our unsaved changes to it; a second failure keeps them and says so.
  // True while a write loop runs. If the runtime cut the loop off before it could reset
  // `flushing` (nothing writing for 10 s, or no end to a write for 60 s), the flag is reset once and the unsaved changes retried.
  flushBusy(cx) {
    // A real write never takes 60 s; a loop cut off after `writing = true` is healed after that.
    if (this.flushing && Date.now() - this.flushAt > (this.writing ? 60000 : 10000)) {
      this.flushing = false; this.writing = false;
      if (!this.healed) { this.healed = true; console.log("todo: a stuck save was reset and is retried"); }
      if (this.pending.length) this.flush(cx);
    }
    return this.flushing;
  }

  // A failed save tries again by itself while changes are pending: after 5 s, then 10, 20, 40, 60 s.
  retry(cx) {
    if (this.retryTimer) return;
    this.retryMs = Math.min((this.retryMs || 2500) * 2, 60000);
    this.retryTimer = cx.timer.after(this.retryMs, (cx) => {
      this.retryTimer = null;
      if (!this.flushing && this.pending.length) this.flush(cx);
    });
  }

  flush(cx) {
    if (this.flushBusy(cx)) return;
    const slug = this.cur, path = this.file(slug);
    this.flushing = true; this.flushAt = Date.now();
    try { cx.spawn(async (cx) => {
      try {
        while (this.pending.length && this.cur === slug) {
          const batch = this.pending.slice(), next = this.text;
          // The finishing decision is made on the text that reached the disk, not the view.
          const ticked = batch.some((m) => this.finishing.has(m)), was = ticked && allDone(parseList(this.disk));
          let written = next, replaced = this.disk;
          this.writing = true;
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
              replaced = fresh;
              this.disk = merged; written = merged;
              const rest = this.pending.slice(batch.length), after = parseList(merged);
              for (const mut of rest) mut(after);
              this.setText(serializeList(after));
            }
            this.pending.splice(0, batch.length); this.error = null; this.retryMs = 0;
            if (ticked && !was && written !== replaced && allDone(parseList(written))) this.celebrate();
          } catch (e) {
            this.error = `Not saved yet (${String(e?.message ?? e).slice(0, 100)}). Your changes are kept here and will be retried.`;
            this.retry(cx);
            break;
          } finally { this.writing = false; }
        }
      } finally { this.flushing = false; cx.notify(); }
    }); } catch (e) { this.flushing = false; throw e; }
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
    if (slug !== this.cur) cx.spawn(async (cx) => this.openList(slug, cx));
    this.handle.focus(); cx.notify();
  }

  askName(mode, cx) {
    this.menu = false;
    const value = mode === "rename" ? this.name() : "";
    // The box keeps its own mode: the input's blur can clear `naming` before its submit runs.
    const input = this.field(value, "List name", (v, cx) => {
      if (this.naming?.input === input) this.naming = null;
      this.handle.focus(); cx.notify();
      cx.spawn(async (cx) => this.finishName(v, mode, cx));
    }, (v, cx) => { if (this.naming?.input === input) this.naming = null; cx.notify(); });
    this.naming = { mode, input };
    cx.notify();
  }

  async finishName(value, mode, cx) {
    const slug0 = this.cur;
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
        await this.editIndex((entries) => { const e = entries.find((x) => x.slug === slug0); if (!e) return false; e.name = name; });
        if (this.cur === slug0) this.commit((list) => { list.name = name; }, cx);
      }
    } catch (e) { this.error = `Couldn't ${mode} the list: ${e?.message ?? e}`; }
    cx.notify();
  }

  async deleteList(cx) {
    if (this.deleting) return;
    this.deleting = true;
    this.confirming = false; this.menu = false;
    const slug = this.cur;
    try {
      while (this.flushBusy(cx)) await cx.sleep(50);
      this.pending = [];
      // Already gone from lists.md: nothing to remove, just reopen what is left.
      const removed = await this.editIndex((entries) => { const k = entries.findIndex((e) => e.slug === slug); if (k < 0) return false; entries.splice(k, 1); });
      if (removed === false) await this.load(cx);
      else {
        await removeAppFile(this.file(slug));
        if (this.index.length) await this.openList(this.index[0].slug, cx);
        else await this.load(cx);
      }
    } catch (e) { this.error = `Couldn't delete the list: ${e?.message ?? e}`; }
    finally { this.deleting = false; }
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
    const done = !item.done, mut = setDone(this.ref(row), done);
    if (done) this.finishing.add(mut);
    this.commit(mut, cx);
  }

  // The write that finishes the list because of a tick (never a load, an add, a delete or an
  // untick) offers the pop-up: flush() decides it on the text that reached the disk.
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
    // The input propagates its Enter, so the panel's key() sees the same Enter right after.
    input.on("submit", (_e, cx) => {
      // The same Enter reaches the panel's key() next; ignore it until the next tick.
      this.justSubmitted = true;
      cx.timer.after(0, () => { this.justSubmitted = false; });
      submit(input.value(), cx);
    });
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
    notify({ title: ref.j == null ? "Task deleted" : "Step deleted", body: copy.title, timeout: UNDO_MS, actions: [{ label: "Undo", name: "todo.undo" }, { label: "OK" }] });
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

  // A press that moves DRAG_PX or more becomes a drag; the click toggle it already made is undone.
  dragMove(r, e, cx) {
    const p = this.press;
    if (!p) return;
    let changed = false;
    if (!this.drag) {
      if (Math.hypot(e.position.x - p.x, e.position.y - p.y) < DRAG_PX) return;
      if (p.toggled) { this.toggleWide(p.item, p.step); p.toggled = false; }
      this.drag = { src: p.ref, slot: null };
      changed = true;
    }
    const slot = this.slotFor(r, e);
    if (JSON.stringify(slot) !== JSON.stringify(this.drag.slot)) { this.drag.slot = slot; changed = true; }
    if (changed) cx.notify();
  }

  // Where a drop over row `r` would land: a task among the open tasks, a step in any open task (null when there is no place).
  slotFor(r, e) {
    const src = this.drag.src, task = this.list.tasks[r.i];
    if (r.k === "head" || !task || task.done) return null;
    const after = e.local_position.y > e.bounds.height / 2, title = task.title;
    if (src.j == null) return r.i === src.i ? null : { k: "task", i: r.i, title, after: r.k !== "task" || after };
    const end = { k: "step", i: r.i, title, j: task.steps.length, beforeTitle: null };
    if (r.k === "task") return r.i === src.i ? null : end;
    if (r.k === "add") return end;
    const j = after ? r.j + 1 : r.j;
    if (r.i === src.i && (j === src.j || j === src.j + 1)) return null;
    return j >= task.steps.length ? end : { ...end, j, beforeTitle: task.steps[j].title };
  }

  // "top" or "bottom" when the drop slot is at that edge of row `r`.
  lineOf(r) {
    const s = this.drag?.slot;
    if (!s || r.i !== s.i) return null;
    const open = this.open.has(s.title);
    if (s.k === "task") return s.after ? (r.k === (open ? "add" : "task") ? "bottom" : null) : (r.k === "task" ? "top" : null);
    if (s.beforeTitle != null) return r.k === "step" && r.j === s.j ? "top" : null;
    return open ? (r.k === "add" ? "top" : null) : (r.k === "task" ? "bottom" : null);
  }

  endDrag(cx) {
    const d = this.drag;
    this.press = null; this.drag = null;
    if (d?.slot && !this.readonly) {
      const { src, slot } = d;
      if (this.commit(src.j == null ? placeTask(src, slot) : placeStep(src, slot), cx)) {
        const tasks = this.list.tasks, i = tasks.findIndex((t) => t.title === (src.j == null ? src.title : slot.title));
        if (src.j == null) this.sel = { k: "task", i };
        else {
          this.open.add(slot.title);
          this.sel = { k: "step", i, j: tasks[i].steps.findIndex((s) => s.title === src.stepTitle) };
        }
      }
    }
    cx.notify();
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
    if (k === "enter" && this.justSubmitted) return;
    if (k === "escape") {
      if (this.drag) { this.press = null; this.drag = null; }
      else if (this.edit) { this.edit = null; this.handle.focus(); }
      else if (this.naming) { this.naming = null; this.handle.focus(); }
      else if (this.confirming) this.confirming = false;
      else if (this.menu) this.menu = false;
      else if (this.addFocus) this.handle.focus();
      else run("pane.go", { direction: "left" });
    } else if (typing) {
      return;
    } else if (this.confirming) {
      if (k === "enter") { this.confirming = false; this.menu = false; cx.spawn(async (cx) => this.deleteList(cx)); } else return;
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
    this.hand = fonts().includes(HAND);
    const rows = this.ready ? this.current() : [], busy = this.edit || this.naming;
    return v_flex().id("todo").size_full().min_h(0).rounded(10).overflow_hidden().bg(t.background).border_1().border_color(t.border).text_color(t.foreground).text_size(13)
      .track_focus(this.handle).on_key_down((e, cx) => this.key(e, cx))
      .on_mouse_up("left", (_e, cx) => { if (this.press) this.endDrag(cx); })
      .on_hover((h, cx) => { if (!h && this.press) { this.press = null; this.drag = null; cx.notify(); } })
      .child(this.header(t))
      .when(this.menu, (el) => el.child(this.switcher(t)))
      .when(this.confirming, (el) => el.child(this.confirmBar(t)))
      .when(Boolean(this.naming), (el) => el.child(this.nameBar(t)))
      .when(Boolean(this.error), (el) => el.child(div().flex_none().px(14).py(6).text_size(12).text_color(t.ansi1).child(this.error)))
      .when(Boolean(this.badIndex), (el) => el.child(this.newIndexBar(t)))
      .when(!this.readonly && this.ready, (el) => el.child(this.addBar(t)))
      .child(v_flex().id("todo-list").flex_1().min_h(0).overflow_y_scrollbar().px(10).py(6).gap(4)
        .on_hover((h, cx) => { if (!h && this.drag?.slot) { this.drag.slot = null; cx.notify(); } })
        .children(rows.map((r) => this.row(r, t)))
        .when(this.ready && !this.readonly && !this.list.tasks.length, (el) => el.child(this.emptyNote("square-pen", "No tasks yet. Add one above.", t)))
        .when(this.ready && this.list.tasks.length > 0 && openCount(this.list) === 0, (el) => el.child(this.emptyNote("circle-check", "All done for today", t))))
      .child(div().flex_none().px(14).py(8).border_t(1).border_color(t.border).text_size(11).text_color(t.dim).truncate()
        .child(busy ? "Enter save · Esc cancel" : "↑↓ move · Space done · Enter edit · N new · Del remove"));
  }

  header(t) {
    const total = this.list.tasks.length, done = total - openCount(this.list), ready = this.ready;
    return v_flex().flex_none().border_b(1).border_color(t.border)
      .child(h_flex().h(46).flex_none().px(10).gap(8).items_center()
        .child(h_flex().id("todo-switch").h(30).px(8).gap(6).items_center().rounded(8).cursor_pointer().hover((s) => s.bg(t.hover))
          .on_click((_e, cx) => { if (this.menu) { this.menu = false; this.handle.focus(); cx.notify(); } else this.openMenu(cx); })
          .child(div().text_size(14.5).font_semibold().max_w(220).truncate().child(ready ? this.name() : "To-do"))
          .child(svg("mods/workspace-chrome/chevron-down.svg").size(12).flex_none().text_color(t.dim)))
        .child(div().flex_1())
        .when(ready, (el) => el.child(div().text_size(11).text_color(t.dim).child(`${openCount(this.list)} open`))))
      .when(ready && total > 0, (el) => el.child(h_flex().flex_none().px(14).pb(10).gap(10).items_center()
        .child(div().flex_1().min_w(0).h(5).rounded(3).bg(t.hover).overflow_hidden()
          .child(div().h_full().rounded(3).bg(t.accent).w(`${Math.round((done / total) * 100)}%`)))
        .child(div().flex_none().text_size(11).text_color(t.dim).child(`${done} of ${total} done`))));
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
      .child(button("todo-del-yes", "Delete", true, (_e, cx) => { this.confirming = false; this.menu = false; cx.notify(); cx.spawn(async (cx) => this.deleteList(cx)); }));
  }

  newIndexBar(t) {
    return h_flex().flex_none().px(14).py(8).gap(8).items_center().bg(t.raised).border_b(1).border_color(t.border).text_size(12)
      .child(div().flex_1().min_w(0).truncate().child("Your lists.md is kept as it is."))
      .child(div().id("todo-new-index").px(10).py(3).rounded(6).cursor_pointer().border_1().border_color(t.border).hover((s) => s.bg(t.hover))
        .on_click((_e, cx) => { cx.spawn(async (cx) => this.startNewIndex(cx)); }).child("Start a new index"));
  }

  nameBar(t) {
    return h_flex().flex_none().px(10).py(8).gap(8).items_center().bg(t.raised).border_b(1).border_color(t.border)
      .child(div().text_size(12).text_color(t.dim).child(this.naming.mode === "new" ? "New list" : "Rename"))
      .child(Input.new(this.naming.input).flex_1().min_w(0).h(28).px(8).rounded(7).border_1().border_color(t.accent).bg(t.background).text_size(12.5));
  }

  // Handwriting for task titles and the add box when the font is loaded; the app font otherwise.
  face(el) { return this.hand ? el.font_family(HAND) : el; }

  addBar(t) {
    return h_flex().id("todo-add").flex_none().mx(12).mt(12).mb(6).h(40).px(12).gap(8).items_center().rounded(12).border_1()
      .border_color(this.addFocus ? t.accent : t.border).bg(t.surface)
      .when(!this.addFocus, (el) => el.hover((s) => s.bg(t.hover)))
      .child(svg("mods/workspace-chrome/plus.svg").size(16).flex_none().text_color(t.accent))
      .child(this.face(Input.new(this.add).flex_1().min_w(0).h_full().text_size(this.hand ? 15.5 : 13).text_color(t.foreground)));
  }

  row(r, t) {
    if (r.k === "head") return this.headRow(r, t);
    if (r.k === "add") return this.addStepRow(r, t);
    return this.taskRow(r, t);
  }

  headRow(r, t) {
    const count = this.list.tasks.filter((x) => x.done).length, sel = same(r, this.sel);
    return h_flex().id("todo-done").h(32).flex_none().mt(6).px(8).gap(8).items_center().rounded(8).cursor_pointer().text_color(t.muted)
      .when(sel, (el) => el.bg(t.selected)).hover((s) => s.bg(t.hover))
      .on_click((_e, cx) => { this.sel = r; this.showDone = !this.showDone; this.handle.focus(); cx.notify(); })
      .child(svg(`mods/workspace-chrome/chevron-${this.showDone ? "down" : "right"}.svg`).size(12).flex_none())
      .child(div().text_size(12).font_medium().child("Completed"))
      .child(div().flex_none().min_w(20).h(18).px(6).rounded(9).bg(t.hover).flex().items_center().justify_center().text_size(10.5).text_color(t.dim).child(String(count)));
  }

  // The guide line that ties a step to its task.
  stepWrap(row, t) {
    return h_flex().flex_none().ml(20).pl(8).border_l(1).border_color(t.border).child(row);
  }

  addStepRow(r, t) {
    const editing = this.edit && same(this.edit.row, r), sel = same(r, this.sel);
    return this.stepWrap(h_flex().id(`todo-addstep-${r.i}`).relative().flex_1().min_w(0).min_h(30).px(8).gap(8).items_center().rounded(8).cursor_pointer().text_color(t.dim)
      .when(sel && !editing, (el) => el.bg(t.selected)).hover((s) => s.bg(t.hover))
      .on_click((_e, cx) => this.startStep(r.i, cx))
      .on_mouse_move((e, cx) => this.dragMove(r, e, cx))
      .when(this.lineOf(r) !== null, (el) => el.child(this.dropLine(r, t)))
      .child(svg("mods/workspace-chrome/plus.svg").size(12).flex_none())
      .child(editing ? this.face(Input.new(this.edit.input).flex_1().min_w(0).h(28).px(6).rounded(6).border_1().border_color(t.accent).bg(t.background).text_size(this.hand ? 14 : 12.5).text_color(t.foreground))
        : div().text_size(12).child("Add step")), t);
  }

  dropLine(r, t) {
    const at = this.lineOf(r);
    return at && div().absolute().left(0).right(0).h(2).bg(t.accent).when(at === "top", (el) => el.top(0)).when(at === "bottom", (el) => el.bottom(0));
  }

  wideKey(item, step) { return `${step ? "s" : "t"}:${item.title}`; }

  // A click toggles the full wrapped title; the second click of a double click undoes it before editing.
  toggleWide(item, step) {
    const k = this.wideKey(item, step);
    if (this.wide.has(k)) this.wide.delete(k); else this.wide.add(k);
  }

  // Done text is struck through by a line over it (the style kit has no text decoration) and dimmer; a wrapped (wide) title is only dimmer.
  titleEl(item, step, t) {
    const wide = this.wide.has(this.wideKey(item, step));
    const text = this.face(div().min_w(0).when(wide, (el) => el.flex_1()).when(!wide, (el) => el.truncate()).text_size(this.hand ? (step ? 14.5 : 16) : (step ? 12.5 : 13.5))
      .text_color(item.done ? t.muted : t.foreground).child(item.title));
    return div().relative().min_w(0).flex().when(wide, (el) => el.flex_1()).child(text)
      .when(item.done && !wide, (el) => el.child(div().absolute().inset(0).flex().items_center().child(div().w_full().h(1).bg(t.muted))));
  }

  taskRow(r, t) {
    const item = this.item(r);
    if (!item) return div();
    const step = r.k === "step", key = `${r.i}.${r.j ?? ""}`, sel = same(r, this.sel);
    const editing = this.edit && same(this.edit.row, r), shown = this.hover === key || sel;
    const stop = (_e, cx) => cx.stop_propagation();
    const circle = div().id(`todo-tick-${key}`).size(step ? 17 : 21).flex_none().rounded(step ? 9 : 11).flex().items_center().justify_center().cursor_pointer()
      .border_1().border_color(item.done ? t.accent : t.muted).when(item.done, (el) => el.bg(t.accent))
      .hover((s) => s.border_color(t.accent))
      .on_mouse_down("left", stop).on_click((_e, cx) => { cx.stop_propagation(); this.tick(r, cx); })
      .when(item.done, (el) => el.child(svg("mods/workspace-chrome/check.svg").size(step ? 10 : 13).text_color(t.accent_foreground)));
    const open = this.open.has(item.title), wide = this.wide.has(this.wideKey(item, step)), steps = step ? [] : item.steps;
    const dragging = this.drag && this.drag.src.i === r.i && (this.drag.src.j ?? null) === (r.j ?? null);
    const row = h_flex().id(`todo-row-${key}`).relative().min_h(step ? 32 : 44).px(10).gap(10).items_center().cursor_pointer().when(wide, (el) => el.items_start().py(8))
      .when(step, (el) => el.flex_1().min_w(0).rounded(8))
      .when(!step, (el) => el.flex_none().rounded(12).border_1().border_color(t.border).bg(t.surface))
      .when(sel, (el) => el.bg(t.selected)).hover((s) => s.bg(t.hover))
      .on_hover((h, cx) => { this.hover = h ? key : (this.hover === key ? null : this.hover); cx.notify(); })
      .when(dragging, (el) => el.opacity(0.5))
      .when(this.lineOf(r) !== null, (el) => el.child(this.dropLine(r, t)))
      .on_mouse_move((e, cx) => this.dragMove(r, e, cx))
      .on_mouse_down("left", (e, cx) => {
        // The row being edited leaves presses to its edit box: selecting, word picks and drags stay in the text.
        if (editing) return;
        this.select(r, cx);
        const toggled = e.click_count <= 2;
        if (toggled) this.toggleWide(item, step);
        if (e.click_count === 2) this.startEdit(r, cx);
        const open = step ? !this.list.tasks[r.i]?.done : !item.done;
        this.drag = null;
        this.press = e.click_count === 1 && !this.readonly && open ? { ref: this.ref(r), x: e.position.x, y: e.position.y, item, step, toggled } : null;
      })
      .child(circle)
      .child(editing
        ? this.face(Input.new(this.edit.input).flex_1().min_w(0).h(28).px(6).rounded(6).border_1().border_color(t.accent).bg(t.background).text_size(this.hand ? 15 : 12.5).text_color(t.foreground))
        : this.titleEl(item, step, t))
      .when(!editing && !wide, (el) => el.child(div().flex_1()))
      .when(steps.length > 0 && !editing, (el) => el.child(div().flex_none().h(18).px(7).rounded(9).bg(t.hover).flex().items_center().text_size(10.5).text_color(t.dim)
        .child(`${steps.filter((s) => s.done).length}/${steps.length}`)))
      .when(!step && !editing, (el) => el.child(div().id(`todo-fold-${key}`).w(16).flex_none().cursor_pointer()
        .on_mouse_down("left", stop).on_click((_e, cx) => { cx.stop_propagation(); this.select(r, cx); this.fold(r, !open, cx); })
        .child(svg(`mods/workspace-chrome/chevron-${open ? "down" : "right"}.svg`).size(12).text_color(t.dim))))
      .when(shown && !editing && !this.readonly, (el) => el.child(div().id(`todo-del-${key}`).w(16).flex_none().cursor_pointer().text_color(t.dim).hover((s) => s.text_color(t.ansi1))
        .on_mouse_down("left", stop).on_click((_e, cx) => { cx.stop_propagation(); this.delete(r, cx); })
        .child(svg("mods/pane/close.svg").size(12))));
    return step ? this.stepWrap(row, t) : row;
  }

  // A friendly line with a quiet icon when there is nothing to show.
  emptyNote(icon, text, t) {
    return v_flex().flex_none().items_center().gap(8).py(28).text_color(t.dim)
      .child(svg(`mods/workspace-chrome/${icon}.svg`).size(28).text_color(t.dim))
      .child(div().text_size(12.5).child(text));
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
    while (this.flushBusy(cx)) await cx.sleep(50);
  }
}

// Sessions: the agent sessions that ran in the shown space's folder, from
// h0x-core (`session list`), newest first with pinned ones on top. Pinned,
// hidden and renamed live in shared("sessions"), keyed by machine, folder,
// agent and id; shared("sessions.count") keeps each folder's visible count for
// the spaces list. A list is asked for when the space changes or the window
// comes back, never per frame. The ⋯ menu (mod "menu", tab "session") and the
// delete dialog (mod "confirm" with buttons) send "sessions.<command>" here.
// Saved layouts (a space's tabs and splits, each agent pane with its session)
// live in shared("sessions") too, under the space's machine and folder.

import { View, div, svg, image } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { spaces, panes, terminals, tree, core, exec, home, knownAgents, run, openOverlay, notify, nextCommand, shared, setShared, theme, window } from "h0x";

// A space's folder as the spaces list keys its count.
const where = (s) => JSON.stringify([s.machine ?? null, s.folder]);

// One `h0x-core session <verb>`: its JSON, or { error: { kind, message } }.
// An older core without `session` answers kind "usage".
async function ask(on, verb, rest) {
  const args = ["session", verb, "--json", ...(on ? ["--on", on] : []), ...(rest.length ? ["--", ...rest] : [])];
  let r;
  try {
    r = await core(args);
  } catch (e) {
    return { error: { kind: "unavailable", message: String(e?.message ?? e) } };
  }
  if (r.code === 0) {
    try {
      return JSON.parse(r.stdout);
    } catch {
      return { error: { kind: "unreadable", message: "h0x-core answered something unreadable." } };
    }
  }
  try {
    const error = JSON.parse(r.stderr || r.stdout).error;
    if (error?.kind) return { error };
  } catch {}
  const text = (r.stderr || r.stdout || "").trim();
  return { error: { kind: /unrecognized subcommand/i.test(text) ? "usage" : "failed", message: text || "h0x-core failed." } };
}

// "now", "5m", "3h", "2d", "4mo", "1y".
export function ago(ms, now) {
  const m = Math.floor(Math.max(0, now - ms) / 60000);
  if (m < 1) return "now";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  const d = Math.floor(h / 24);
  if (d < 30) return `${d}d`;
  return d < 365 ? `${Math.floor(d / 30)}mo` : `${Math.floor(d / 365)}y`;
}

// Its words as a person would type them.
const shell = (argv) => argv.map((w) => (w === "" || /[\s"'$`\\]/.test(w) ? JSON.stringify(w) : w)).join(" ");

// A tab's pane tree as a layout keeps it: a pane `leaf(terminal)` describes, or
// { split: "right" | "down", children }. Mod panes are left out.
function shape(node, leaf) {
  if (node?.type === "block") return node.content?.terminal ? leaf(node.content.terminal) : null;
  const children = (node?.children ?? []).map((c) => shape(c, leaf)).filter(Boolean);
  if (children.length < 2) return children[0] ?? null;
  return { split: node.type === "column" ? "down" : "right", children };
}

const first = (node) => (node.children ? first(node.children[0]) : node);
const leaves = (node) => (node.children ? node.children.flatMap(leaves) : [node]);
const used = (agent, id) => `${agent}\n${id}`;

// Gives each agent pane the newest session of its agent in its folder (`base`
// when it has none) that no earlier pane took, else null. Known limit: two
// panes of one agent in one folder may get each other's sessions.
async function assign(on, base, items) {
  const lists = new Map();
  const taken = new Set();
  for (const item of items) {
    const folder = item.cwd || base;
    if (!lists.has(folder)) {
      const r = await ask(on, "list", folder ? [folder] : []);
      lists.set(folder, Array.isArray(r.sessions) ? r.sessions : []);
    }
    const x = lists.get(folder).find((x) => x.agent === item.agent && !taken.has(used(x.agent, x.id)));
    if (x) taken.add(used(x.agent, x.id));
    item.session = x?.id ?? null;
  }
}

// What an agent pane starts: its session's resume argv, once per session id;
// else (no session, gone, an older core) `fresh`.
async function launch(on, item, fresh, started) {
  if (item.session && !started.has(used(item.agent, item.session))) {
    started.add(used(item.agent, item.session));
    const r = await ask(on, "resume", [item.agent, item.session]);
    if (!r.error && Array.isArray(r.argv) && r.argv.length) return r.argv;
  }
  return fresh?.length ? fresh : undefined;
}

// Commands apply after run() returns: polls (50 ms, at most 10 s) until `find` answers.
async function until(cx, find) {
  for (let i = 0; i < 200; i++) {
    const found = find();
    if (found) return found;
    await cx.sleep(50);
  }
  return null;
}

export default class Sessions extends View {
  init(_props, cx) {
    this.list = [];
    this.state = "loading"; // "ready" | "old" (core without `session`) | "error"
    this.programs = [];
    this.refresh(cx);
    cx.spawn(async (cx) => {
      this.programs = await knownAgents().catch(() => []);
      cx.notify();
    });
    cx.timer.every(1000, (cx) => this.check(cx));
    cx.spawn(async (cx) => {
      for (;;) {
        const command = await nextCommand();
        this.command(command, cx);
        cx.notify();
      }
    });
  }

  // Another space, or the window back in front: ask again.
  check(cx) {
    const s = spaces().find((x) => x.active);
    const active = Boolean(window()?.active);
    const back = active && this.active === false;
    this.active = active;
    if ((s ? where(s) : null) !== this.key || back) this.refresh(cx);
  }

  refresh(cx) {
    const s = spaces().find((x) => x.active);
    const key = s ? where(s) : null;
    if (key !== this.key) {
      this.key = key;
      this.list = [];
      this.state = "loading";
      this.renaming = null;
    }
    if (!s) {
      this.state = "ready";
      return;
    }
    const on = s.machine ?? null;
    // Empty: the machine's home (`--on` without a folder means that already).
    const folder = s.folder || (on ? "" : home());
    cx.spawn(async (cx) => {
      const r = await ask(on, "list", folder ? [folder] : []);
      if (this.key !== key) return;
      this.space = { id: s.id, on, folder };
      if (r.error) {
        this.state = r.error.kind === "usage" ? "old" : "error";
        this.error = r.error.message;
        this.list = [];
      } else {
        this.state = "ready";
        this.list = (Array.isArray(r.sessions) ? r.sessions : []).map((x) => ({ ...x, mark: JSON.stringify([on, s.folder, x.agent, x.id]) }));
      }
      cx.notify();
    });
  }

  // Shown ones, pinned first, each with its HyperOx-side state as `mine`.
  rows() {
    const marks = shared("sessions") ?? {};
    const shown = this.list.map((x) => ({ ...x, mine: marks[x.mark] ?? {} })).filter((x) => !x.mine.hidden);
    return [...shown.filter((x) => x.mine.pinned), ...shown.filter((x) => !x.mine.pinned)];
  }

  // Merges into (null: forgets) a session's HyperOx-side state.
  setMark(x, change) {
    const all = { ...(shared("sessions") ?? {}) };
    const next = change && { ...(all[x.mark] ?? {}), ...change };
    for (const k of Object.keys(next ?? {})) if (!next[k]) delete next[k];
    if (next && Object.keys(next).length) all[x.mark] = next;
    else delete all[x.mark];
    setShared("sessions", all);
  }

  program(x) {
    return this.programs.find((p) => p.key === x.agent);
  }

  title(x) {
    return x.mine?.name || x.title || `${this.program(x)?.name ?? x.agent} session`;
  }

  fail(title, message) {
    notify({ title, body: message ?? "", level: "error" });
  }

  menu(x) {
    this.target = x;
    openOverlay({ mod: "menu", kind: "popup", width: 220, height: 300, props: { tab: "session", target: { pinned: Boolean(x.mine?.pinned), local: !this.space?.on } } });
  }

  command(c, cx) {
    if (c === "overwrite") return this.pending && this.save(this.pending, cx);
    const x = this.target;
    if (!x) return;
    if (c === "resume" || c === "resume-tab") this.resume(x, c === "resume-tab", cx);
    else if (c === "pin" || c === "unpin") this.setMark(x, { pinned: c === "pin" });
    else if (c === "rename") this.rename(x);
    else if (c === "copy") this.withArgv(x, cx, (argv, cx) => {
      cx.write_to_clipboard(shell(argv));
      notify({ title: "Copied the resume command", body: shell(argv), level: "success" });
    });
    else if (c === "reveal") this.reveal(x, cx);
    else if (c === "delete") this.askDelete(x);
    else if (c === "hide") this.setMark(x, { hidden: true });
    else if (c === "trash") this.trash(x, cx);
  }

  withArgv(x, cx, then) {
    const on = this.space?.on ?? null;
    cx.spawn(async (cx) => {
      const r = await ask(on, "resume", [x.agent, x.id]);
      if (r.error || !Array.isArray(r.argv) || !r.argv.length) return this.fail("Couldn't resume the session", r.error?.message);
      then(r.argv, cx);
      cx.notify();
    });
  }

  // In this tab: beside the focused pane. Either way in the folder it ran in.
  resume(x, newTab, cx) {
    const { id, folder } = this.space ?? {};
    this.withArgv(x, cx, (argv) => {
      const at = folder ? { folder } : {};
      if (newTab) run("tab.new", { space: id, ...at, argv });
      else run("terminal.open", { argv, ...at, place: "right" });
    });
  }

  rename(x) {
    this.renaming = x.mark;
    this.input = InputState.new({ value: this.title(x) });
    this.input.set_selected_range({ start: 0, end: this.title(x).length });
    this.input.on("submit", (_e, cx) => {
      this.setMark(x, { name: this.input.value().trim() });
      this.renaming = null;
      cx.notify();
    });
    this.input.focus();
  }

  // This computer's file manager, with the file selected where it can be.
  reveal(x, cx) {
    const platform = window()?.platform;
    const [command, args] =
      platform === "windows" ? ["explorer", [`/select,${x.path.replace(/\//g, "\\")}`]]
      : platform === "macos" ? ["open", ["-R", x.path]]
      : ["xdg-open", [x.path.replace(/\/[^/]*$/, "") || "/"]];
    cx.spawn(async () => {
      // Explorer exits 1 even when it showed the file: only not starting counts.
      await exec(command, args).catch((e) => this.fail("Couldn't show the file", String(e?.message ?? e)));
    });
  }

  askDelete(x) {
    const bin = this.space?.on ? `the Recycle Bin or Trash on ${this.space.on}` : window()?.platform === "windows" ? "the Recycle Bin" : "the Trash";
    openOverlay({
      mod: "confirm",
      kind: "dialog",
      width: 460,
      height: 210,
      props: {
        title: "Delete session?",
        detail: `“${this.title(x)}”. Remove from HyperOx hides it here; the agent keeps it. Delete completely moves its files to ${bin}.`,
        buttons: [
          { label: "Remove from HyperOx", command: { name: "sessions.hide" }, primary: true },
          { label: "Delete completely", command: { name: "sessions.trash" } },
        ],
      },
    });
  }

  trash(x, cx) {
    const on = this.space?.on ?? null;
    cx.spawn(async (cx) => {
      const r = await ask(on, "trash", [x.agent, x.id]);
      if (r.error?.kind === "unsupported")
        notify({ title: "This session can only be removed from HyperOx", body: `${this.program(x)?.name ?? x.agent} keeps it where HyperOx can't delete it. Use Remove from HyperOx to hide it.`, level: "warning" });
      else if (r.error && r.error.kind !== "not_found") this.fail("Couldn't delete the session", r.error.message);
      // Gone: its HyperOx-side state goes too.
      else this.setMark(x, null);
      this.refresh(cx);
    });
  }

  // The shown space's saved layouts, by name.
  layouts() {
    const s = spaces().find((x) => x.active);
    return (s && shared("sessions")?.[where(s)]?.layouts) ?? {};
  }

  // Stores (null: forgets) one of a space's layouts.
  setLayout(key, name, layout) {
    const all = { ...(shared("sessions") ?? {}) };
    const layouts = { ...(all[key]?.layouts ?? {}) };
    if (layout) layouts[name] = layout;
    else delete layouts[name];
    if (Object.keys(layouts).length) all[key] = { ...all[key], layouts };
    else delete all[key];
    setShared("sessions", all);
  }

  askName() {
    this.naming = true;
    this.input = InputState.new({ value: "" });
    this.input.on("submit", (_e, cx) => {
      this.naming = false;
      this.named(this.input.value().trim(), cx);
      cx.notify();
    });
    this.input.focus();
  }

  // A name taken here already is replaced only after asking.
  named(name, cx) {
    if (!name) return;
    if (!this.layouts()[name]) return this.save(name, cx);
    this.pending = name;
    openOverlay({
      mod: "confirm",
      kind: "dialog",
      width: 420,
      height: 180,
      props: { title: "Replace layout?", detail: `“${name}” is saved here already. Replace it with this space's tabs?`, command: { name: "sessions.overwrite" } },
    });
  }

  // The shown space's tabs and splits under `name`: each agent pane with its
  // agent (label h0x.agent), session, argv and folder; a shell pane its folder.
  save(name, cx) {
    this.pending = null;
    const s = spaces().find((x) => x.active);
    const t = tree();
    if (!s || t?.id !== s.id) return;
    const ps = panes();
    const ts = terminals();
    const leaf = (address) => {
      const cwd = ps.find((p) => p.terminal === address)?.cwd ?? null;
      const r = ts.find((x) => x.address === address);
      const agent = r?.labels?.["h0x.agent"];
      return agent ? { agent, session: null, argv: r.argv ?? [], cwd } : { cwd };
    };
    const tabs = (t.tabs ?? []).map((x) => ({ name: x.name ?? "", root: shape(x.root, leaf) })).filter((x) => x.root);
    if (!tabs.length) return notify({ title: "Nothing to save", body: "This space has no terminals.", level: "warning" });
    const on = s.machine ?? null;
    const key = where(s);
    cx.spawn(async (cx) => {
      await assign(on, s.folder || (on ? "" : home()), tabs.flatMap((x) => leaves(x.root)).filter((x) => x.agent));
      this.setLayout(key, name, { tabs });
      notify({ title: `Saved layout “${name}”`, body: "", level: "success" });
      cx.notify();
    });
  }

  // Its tabs as new tabs in the shown space, each agent resumed in its session.
  restore(name, cx) {
    const s = spaces().find((x) => x.active);
    const layout = this.layouts()[name];
    if (!s || !layout || this.restoring) return;
    this.restoring = true;
    const on = s.machine ?? null;
    const started = new Set();
    const start = async (leaf) => {
      const folder = leaf.cwd || s.folder;
      const argv = leaf.agent ? await launch(on, leaf, leaf.argv?.length ? leaf.argv : this.program(leaf)?.run, started) : undefined;
      return { ...(folder ? { folder } : {}), ...(argv ? { argv } : {}) };
    };
    cx.spawn(async (cx) => {
      const done = await this.rebuild(s.id, layout.tabs ?? [], start, cx);
      this.restoring = false;
      if (!done) this.fail("Couldn't restore the layout", `“${name}” stopped before all its panes started.`);
      cx.notify();
    });
  }

  async rebuild(space, tabs, start, cx) {
    for (const tab of tabs) {
      const at = await start(first(tab.root));
      const before = new Set(panes().map((p) => p.tab));
      run("tab.new", { space, ...(tab.name ? { name: tab.name } : {}), ...at });
      const pane = await until(cx, () => panes().find((p) => !before.has(p.tab))?.pane);
      if (!pane || !(await this.grow(tab.root, pane, start, cx))) return false;
    }
    return true;
  }

  // `pane` holds the node's first pane: its later children split off one after
  // another, then each child grows its own splits.
  async grow(node, pane, start, cx) {
    if (!node.children) return true;
    const ids = [pane];
    for (const child of node.children.slice(1)) {
      const at = await start(first(child));
      const seen = new Set(panes().map((p) => p.pane));
      const tab = panes().find((p) => p.pane === ids.at(-1))?.tab;
      run("pane.focus", { pane: ids.at(-1) });
      run("terminal.open", { ...at, place: node.split });
      const id = await until(cx, () => panes().find((p) => p.tab === tab && !seen.has(p.pane))?.pane);
      if (!id) return false;
      ids.push(id);
    }
    for (const [i, child] of node.children.entries()) if (!(await this.grow(child, ids[i], start, cx))) return false;
    return true;
  }

  // The shown space's kept agent panes whose agent no longer runs (a reboot
  // left a shell, or it ended).
  stale() {
    const s = spaces().find((x) => x.active);
    const tabs = new Set((s?.tabs ?? []).map((t) => t.id));
    const ts = terminals();
    return panes().flatMap((p) => {
      const t = tabs.has(p.tab) && p.terminal ? ts.find((x) => x.address === p.terminal) : null;
      const agent = t?.labels?.["h0x.agent"];
      return agent && (t.state !== "live" || !t.program?.agent) ? [{ pane: p.pane, agent, cwd: p.cwd, argv: this.program({ agent })?.run ?? t.argv }] : [];
    });
  }

  // Each stale pane gives way to its agent resumed in its place.
  resumeAll(cx) {
    const s = spaces().find((x) => x.active);
    const items = this.stale();
    if (!s || !items.length) return;
    const on = s.machine ?? null;
    cx.spawn(async (cx) => {
      await assign(on, s.folder || (on ? "" : home()), items);
      const started = new Set();
      for (const item of items) {
        const argv = await launch(on, item, item.argv, started);
        if (!argv) continue;
        run("pane.split", { direction: "right", pane: item.pane, argv });
        run("pane.close", { pane: item.pane });
      }
      cx.notify();
    });
  }

  layoutsView(t) {
    const names = Object.keys(this.layouts());
    const stale = this.stale();
    const action = (id, label, onClick) => h_flex().id(id).px(8).h(24).flex_none().items_center().rounded(6).text_size(12).text_color(t.muted)
      .hover((s) => s.bg(t.hover).text_color(t.foreground)).on_click(onClick).child(label);
    return v_flex().px(6).pb(6).gap(1).flex_none()
      .child(h_flex().gap(4)
        .child(action("sessions-save-layout", "Save layout…", (_e, cx) => { this.askName(); cx.notify(); }))
        .when(stale.length > 0, (el) => el.child(action("sessions-resume-all", "Resume all agents", (_e, cx) => this.resumeAll(cx)))))
      .when(Boolean(this.naming), (el) => el.child(div().px(2)
        .on_key_down((e, cx) => {
          if (e.key !== "escape") return;
          this.naming = false;
          cx.stop_propagation();
          cx.notify();
        })
        .child(Input.new(this.input).h(26).px(6).rounded(6).border_1().border_color(t.accent).bg(t.background).text_size(12.5))))
      .children(names.map((name, i) => h_flex().id(`layout-${i}`).h(28).px(8).gap(8).items_center().rounded(7).text_size(12.8).text_color(t.foreground)
        .hover((s) => s.bg(t.hover)).on_click((_e, cx) => this.restore(name, cx))
        .child(div().flex_1().min_w(0).truncate().child(name))
        .child(div().id(`layout-delete-${i}`).flex_none().px(4).rounded(4).text_color(t.dim).hover((s) => s.bg(t.raised).text_color(t.foreground))
          .on_click((_e, cx) => { cx.stop_propagation(); this.forget(name); cx.notify(); }).child("×"))));
  }

  forget(name) {
    const s = spaces().find((x) => x.active);
    if (s) this.setLayout(where(s), name, null);
  }

  row(x, i, t, now) {
    const p = this.program(x);
    const title = this.title(x);
    const icon = p?.icon ? (p.tint ? svg(p.icon).size(14).text_color(t.muted) : image(p.icon).size(14)) : div().size(14);
    const name = this.renaming === x.mark
      ? div().flex_1().min_w(0)
          .on_key_down((e, cx) => {
            if (e.key !== "escape") return;
            this.renaming = null;
            cx.stop_propagation();
            cx.notify();
          })
          .child(Input.new(this.input).h(24).px(6).rounded(6).border_1().border_color(t.accent).bg(t.background).text_size(12.5))
      : div().flex_1().min_w(0).truncate().child(title);
    return h_flex().id(`session-${i}`).h(30).px(8).gap(8).flex_none().items_center().rounded(7).text_size(12.8).text_color(t.foreground)
      .hover((s) => s.bg(t.hover))
      .on_mouse_down("right", (_e, cx) => { cx.stop_propagation(); this.menu(x); })
      .child(div().flex_none().child(icon))
      .child(name)
      .when(Boolean(x.mine.pinned), (el) => el.child(svg("mods/workspace-chrome/pin.svg").size(12).flex_none().text_color(t.dim)))
      .child(div().flex_none().text_size(11).text_color(t.dim).child(ago(x.last_used, now)))
      .child(div().id(`session-more-${i}`).flex_none().px(4).rounded(4).text_color(t.dim).hover((s) => s.bg(t.raised).text_color(t.foreground))
        .on_click((_e, cx) => { cx.stop_propagation(); this.menu(x); }).child("⋯"));
  }

  render() {
    const t = theme();
    const rows = this.rows();
    // The spaces list's count, written only when it changes.
    if (this.state === "ready" && this.key) {
      const counts = shared("sessions.count") ?? {};
      if (counts[this.key] !== rows.length) setShared("sessions.count", { ...counts, [this.key]: rows.length });
    }
    const note = (text) => div().px(12).py(10).text_size(12.5).text_color(t.dim).child(text);
    const body =
      this.state === "old" ? note("Update h0x-core to see sessions")
      : this.state === "error" ? note(this.error || "Couldn't list sessions")
      : this.state === "loading" ? note("Loading…")
      : !rows.length ? note("No agent sessions in this folder yet")
      : v_flex().gap(1).children(rows.map((x, i) => this.row(x, i, t, Date.now())));
    return v_flex().id("sessions").size_full().min_h(0).bg(t.chrome)
      .child(h_flex().h(34).flex_none().px(12).items_center().text_size(11.5).text_color(t.dim).child(div().flex_1().child("Sessions"))
        .when(this.state === "ready" && rows.length > 0, (el) => el.child(String(rows.length))))
      .when(Boolean(this.key), (el) => el.child(this.layoutsView(t)))
      .child(v_flex().id("sessions-list").flex_1().min_h(0).px(6).pb(10).overflow_y_scrollbar().child(body));
  }
}

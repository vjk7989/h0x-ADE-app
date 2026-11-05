// Sessions: the agent sessions that ran in the shown space's folder, from
// h0x-core (`session list`), newest first with pinned ones on top. Pinned,
// hidden and renamed live in shared("sessions"), keyed by machine, folder,
// agent and id; shared("sessions.count") keeps each folder's visible count for
// the spaces list. A list is asked for when the space changes or the window
// comes back, never per frame. The ⋯ menu (mod "menu", tab "session") and the
// delete dialog (mod "confirm" with buttons) send "sessions.<command>" here.

import { View, div, svg, image } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { spaces, core, exec, home, knownAgents, run, openOverlay, notify, nextCommand, shared, setShared, theme, window, layout } from "h0x";

const placed = (n) => n?.content?.mod === "sessions" || (n?.children ?? []).some(placed);

// Whether the layout shows this panel: no hidden node on the way, and the
// selected child of each stack. Not in the layout (an overlay, a pane): shown.
function shown(root) {
  const walk = (n) => {
    if (!n || n.hidden) return false;
    if (n.content?.mod === "sessions") return true;
    const children = (n.children ?? []).filter((c) => !c.hidden);
    if (n.type === "stack") return walk(children.find((c) => c.id === n.active) ?? children[0]);
    return children.some(walk);
  };
  return !placed(root) || walk(root);
}

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

export default class Sessions extends View {
  init(_props, cx) {
    this.list = [];
    this.state = "loading"; // "ready" | "old" (core without `session`) | "error"
    this.programs = [];
    this.check(cx);
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
  // Hidden (another panel tab, a closed panel): asks nothing until shown again.
  check(cx) {
    const s = spaces().find((x) => x.active);
    const active = Boolean(window()?.active);
    const back = active && this.active === false;
    this.active = active;
    if (!shown(layout())) {
      this.missed ||= back;
      return;
    }
    if ((s ? where(s) : null) !== this.key || back || this.missed) {
      this.missed = false;
      this.refresh(cx);
    }
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
      .child(v_flex().id("sessions-list").flex_1().min_h(0).px(6).pb(10).overflow_y_scrollbar().child(body));
  }
}

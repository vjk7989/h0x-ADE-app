// First-run welcome over the live app; records finished/skipped in frame.welcomed.

import { View, div, svg, image } from "gpui-kit";
import { v_flex, h_flex, Scrollbar } from "gpui-base";
import { spaces, run, theme, props, closeOverlay, setShared, shared, core, appHome, home, readTextFile, writeTextFile, settings, layout, keys, knownAgents, installOpenCode } from "h0x";

const I = (name) => `frame/welcome/${name}.svg`;
const PAGES = 4;
// The guide sits in the app's own folder (HYPEROX_HOME may move it): shown with ~ when it is under the home folder.
function askText() {
  const dir = appHome().replaceAll("\\", "/"), h = home().replaceAll("\\", "/").replace(/\/$/, "");
  return `Read ${h && dir.startsWith(h + "/") ? "~" + dir.slice(h.length) : dir}/AGENTS.md, then tell me what you can change in HyperOx.`;
}
const MIGRATE = "Read `h0x-ade --skill`, then migrate my keybinds and settings from ~/.config/h0x/config.toml";
const APPEARANCES = [
  [null, "System"],
  ["light", "Light"],
  ["dark", "Dark"],
];
// The panels the layout offers (its "panel-body" stack), as the tab bar's buttons.
const PANELS = [
  { id: "files", icon: "folder-tree", title: "Files", about: "The folder you're in as a tree. Open a file to read it, highlighted." },
  { id: "changes", icon: "git-compare", title: "Changes", about: "What your agents changed: every file with its diff, and a commit box." },
  { id: "sessions", icon: "history", title: "Sessions", about: "Your agents' past sessions in this folder. Resume, pin, rename or delete one." },
  { id: "todo", icon: "list-checks", title: "To-do", about: "Your to-do lists, kept as Markdown files. Tick tasks off with Space." },
];

// The key bound to a command, as people press it: "Ctrl+Shift+P".
function bound(action) {
  const k = keys().find((b) => b.section === "global" && b.action === action);
  return k ? k.keys.split("-").map((p) => (p === "cmd" ? "\u2318" : p.charAt(0).toUpperCase() + p.slice(1))).join("+") : null;
}

function find(node, id) {
  if (!node) return null;
  if (node.id === id) return node;
  for (const c of node.children ?? []) {
    const f = find(c, id);
    if (f) return f;
  }
  return null;
}

export default class Welcome extends View {
  init(_props, cx) {
    this.page = 1;
    this.machines = [];
    this.copied = null;
    this.fold = false; // "Coming from h0x?" open
    this.appearance = null;
    this.foundVia = true; // settings.json found_via; null (not yet asked) shows the question
    this.openCodeInstalled = false;
    this.openCodeStarted = false;
    this.handle = cx.focus_handle();
    this.handle.focus();
    // It opens while the window is still starting, which takes the keyboard
    // back for its first pane: take it again once that's done.
    cx.timer.after(600, () => this.handle.focus());
    cx.spawn(async (cx) => {
      try {
        const saved = JSON.parse((await readTextFile(`${appHome()}/settings.json`)) ?? "{}");
        this.appearance = saved.appearance ?? null;
        this.foundVia = saved.found_via ?? null;
      } catch {}
      await this.loadMachines();
      cx.notify();
      try {
        if ((await knownAgents()).some((a) => a.run?.[0] === "opencode" && a.installed)) this.openCodeInstalled = true;
      } catch {}
      cx.notify();
    });
    // Machines added meanwhile (the Add a machine dialog) appear.
    cx.timer.every(3000, (cx) => {
      if (this.page !== 2) return;
      cx.spawn(async (cx) => {
        await this.loadMachines();
        cx.notify();
      });
    });
  }

  async loadMachines() {
    const r = await core(["machine", "ls", "--json"]).catch(() => null);
    try {
      this.machines = r?.code === 0 ? (JSON.parse(r.stdout).machines ?? []).filter((m) => !m.temporary) : [];
    } catch {
      this.machines = [];
    }
  }

  done(outcome) {
    setShared("frame.welcomed", outcome);
    closeOverlay(props().overlay);
  }

  // The OpenCode card's "Yes" starts the install once the person moves on: it runs in the
  // app, never holds up this screen, and says how it went in a toast.
  installOpenCodeIfWanted() {
    if (shared("welcome.opencode") !== true || this.openCodeStarted || this.openCodeInstalled) return;
    this.openCodeStarted = true;
    try {
      installOpenCode();
    } catch {}
  }

  start() {
    this.installOpenCodeIfWanted();
    this.done("finished");
    if (spaces().length === 0) run("space.new");
  }

  setAppearance(value, cx) {
    this.appearance = value;
    this.edit((p) => {
      if (value === null) delete p.appearance;
      else p.appearance = value;
    }, cx);
    cx.notify();
  }

  // The one-time "How did you find HyperOx?": the answer (or "skip") is kept, and it is not asked again.
  setFoundVia(value, cx) {
    this.foundVia = value;
    this.edit((p) => { p.found_via = value; }, cx);
    cx.notify();
  }

  // Changes settings.json: read, change, write back only if nobody changed it meanwhile.
  edit(change, cx) {
    cx.spawn(async (cx) => {
      const file = `${appHome()}/settings.json`;
      for (let i = 0; i < 3; i++) {
        const raw = await readTextFile(file);
        let p = {};
        try {
          p = raw ? JSON.parse(raw.replace(/^\uFEFF/, "")) : {};
          if (p === null || typeof p !== "object" || Array.isArray(p)) return;
        } catch {
          return;
        }
        change(p);
        try {
          await writeTextFile(file, JSON.stringify(p, null, 2) + "\n", raw);
          break;
        } catch {}
      }
      cx.notify();
    });
  }

  // The panels picked: the tab bar shows a button for each.
  panels() {
    const stored = shared("workspace.panels");
    return Array.isArray(stored) ? stored : PANELS.map((p) => p.id);
  }

  togglePanel(id) {
    const on = this.panels().includes(id);
    const next = on ? this.panels().filter((x) => x !== id) : [...this.panels(), id];
    setShared("workspace.panels", next);
    // The ids the tab bar migration knows (defaults/mods/tabs/main.js enabledPanels): a panel turned off stays off.
    setShared("workspace.panels.added", ["sessions", "todo"]);
    const L = layout();
    if (!find(L, "panel")) return;
    if (on) {
      if (find(L, "panel-body")?.active === id || next.length === 0) run("layout.hide", { id: "panel" });
    } else {
      run("layout.select", { stack: "panel-body", child: id });
      run("layout.show", { id: "panel" });
    }
  }

  // Pieces -------------------------------------------------------------

  // A paragraph. The command name `h0x-core` (and `ls`, `attach t3` after it) is set in the mono font, so the
  // text goes out word by word, wrapping like a line of text.
  para(t, text) {
    const plain = div().w_full().text_size(13).line_height("20px").text_color(t.muted);
    if (!text.includes("h0x-core")) return plain.child(text);
    const words = text.split(" "), pieces = [];
    for (let i = 0; i < words.length; i++) {
      const m = /^(h0x-core)([.,;:]?)$/.exec(words[i]);
      const space = i < words.length - 1 ? "\u00a0" : "";
      if (!m) { pieces.push([words[i] + space, false]); continue; }
      let code = m[1];
      // `h0x-core ls` and `h0x-core attach t3` are whole commands.
      if (words[i + 1] === "ls" && !m[2]) code += "\u00a0" + words[++i];
      else if (words[i + 1] === "attach" && words[i + 2] === "t3" && !m[2]) { code += "\u00a0attach\u00a0t3"; i += 2; }
      pieces.push([code, true]);
      if (m[2] || space) pieces.push([m[2] + space, false]);
    }
    return plain.flex().flex_wrap().children(
      pieces.map(([word, code]) => (code ? div().font_family(this.mono()).text_size(12).text_color(t.muted).child(word) : div().child(word))),
    );
  }

  title(t, text, sub) {
    return v_flex()
      .gap(6)
      .child(div().text_size(18).font_semibold().text_color(t.foreground).child(text))
      .when(Boolean(sub), (el) => el.child(this.para(t, sub)));
  }

  mono() {
    return settings().font_family;
  }

  // [word]: brackets in dim. A toggle (`on`/`off`) takes the accent when on; a status (`info`) is plain text.
  stateTag(t, kind, word) {
    const color = kind === "on" ? t.accent : kind === "off" ? t.muted : kind === "gone" ? t.muted : t.foreground;
    return h_flex()
      .flex_none()
      .font_family(this.mono())
      .text_size(12)
      .line_height("18px")
      .child(div().text_color(t.dim).child("["))
      .child(div().text_color(color).when(kind === "on" || kind === "off", (el) => el.font_medium()).child(word))
      .child(div().text_color(t.dim).child("]"));
  }

  // A key cap.
  kbd(t, label) {
    return div().flex_none().h(18).px(5).flex().items_center().rounded(4).border_1().border_color(t.border).border_b(2).bg(t.background)
      .font_family(this.mono()).text_size(10.5).text_color(t.muted).child(label);
  }

  hint(t, key, word) {
    return h_flex().gap(6).items_center().child(this.kbd(t, key)).child(div().font_family(this.mono()).text_size(11).text_color(t.muted).child(word));
  }

  button(t, id, label, primary, onClick, icon) {
    return h_flex()
      .id(id)
      .h(30)
      .px(12)
      .gap(6)
      .items_center()
      .rounded(6)
      .cursor_pointer()
      .text_size(12.5)
      .font_medium()
      .when(primary, (el) => el.bg(t.solid).text_color(t.solid_foreground).hover((s) => s.opacity(0.88)))
      .when(!primary, (el) => el.text_color(t.muted).hover((s) => s.text_color(t.foreground).bg(t.hover)))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click(onClick)
      .child(label)
      .when(Boolean(icon), (el) => el.child(svg(I(icon)).size(14)));
  }

  segment(t, items, now, onPick) {
    return h_flex()
      .p(2)
      .gap(2)
      .rounded(7)
      .border_1()
      .border_color(t.border)
      .bg(t.chrome)
      .children(
        items.map(([value, label]) =>
          div()
            .id(`w-seg-${label}`)
            .px(11)
            .py(4)
            .rounded(5)
            .text_size(12.5)
            .cursor_pointer()
            .when(now === value, (el) => el.bg(t.popover).text_color(t.foreground).border_1().border_color(t.border))
            .when(now !== value, (el) => el.text_color(t.muted).hover((s) => s.text_color(t.foreground)))
            .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
            .on_click((_e, cx) => onPick(value, cx))
            .child(label),
        ),
      );
  }

  sizeControl(t) {
    const step = (id, label, cmd) =>
      div()
        .id(id)
        .w(28)
        .h(26)
        .rounded(6)
        .flex()
        .items_center()
        .justify_center()
        .text_size(14)
        .text_color(t.muted)
        .cursor_pointer()
        .hover((s) => s.bg(t.hover).text_color(t.foreground))
        .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
        .on_click(() => run(cmd))
        .child(label);
    return h_flex()
      .p(2)
      .gap(2)
      .items_center()
      .rounded(7)
      .border_1()
      .border_color(t.border)
      .bg(t.chrome)
      .child(step("w-zoom-out", "\u2212", "app.zoom_out"))
      .child(div().w(44).font_family(this.mono()).text_size(12).text_color(t.foreground).flex().justify_center().child(`${Math.round((settings().zoom ?? 1) * 100)}%`))
      .child(step("w-zoom-in", "+", "app.zoom_in"));
  }

  feature(t, icon, title, body) {
    return h_flex()
      .gap(12)
      .items_start()
      .child(div().flex_none().w(16).h(20).flex().items_center().child(svg(I(icon)).size(16).text_color(t.accent)))
      .child(v_flex().flex_1().min_w(0).gap(3).child(div().text_size(13.5).line_height("20px").font_semibold().text_color(t.foreground).child(title)).child(this.para(t, body)));
  }

  copyLine(t, text, id = "w-copy") {
    const copied = this.copied === id;
    return v_flex()
      .gap(10)
      .p(12)
      .rounded(6)
      .bg(t.background)
      .border_1()
      .border_color(t.input)
      .child(div().text_size(13).line_height("20px").text_color(t.foreground).child(text))
      .child(
        h_flex().child(
          div()
            .id(id)
            .px(12)
            .py(5)
            .rounded(6)
            .cursor_pointer()
            .text_size(12.5)
            .font_medium()
            .bg(copied ? t.ansi2 + "22" : t.raised)
            .text_color(copied ? t.ansi2 : t.foreground)
            .border_1()
            .border_color(t.border)
            .hover((s) => s.bg(t.hover))
            .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
            .on_click((_e, cx) => {
              cx.write_to_clipboard(text);
              this.copied = id;
              cx.notify();
            })
            .child(copied ? "Copied" : "Copy"),
        ),
      );
  }

  comingFromH0x(t) {
    return v_flex()
      .child(
        h_flex()
          .id("w-coming")
          .py(10)
          .gap(8)
          .items_center()
          .border_t(1)
          .when(!this.fold, (el) => el.border_b(1))
          .border_color(t.border)
          .cursor_pointer()
          .hover((s) => s.bg(t.hover))
          .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
          .on_click((_e, cx) => { this.fold = !this.fold; cx.notify(); })
          .child(svg(I(this.fold ? "chevron-down" : "chevron-right")).size(14).text_color(t.muted))
          .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child("Coming from h0x?")),
      )
      .when(this.fold, (el) =>
        el.child(
          v_flex()
            .gap(12)
            .pb(14)
            .border_b(1)
            .border_color(t.border)
            .child(this.para(t, "This is a new app on a new core. Your h0x workspaces stay where they are; nothing moves over."))
            .child(this.para(t, "Same idea as before: spaces, tabs and terminals."))
            .child(this.para(t, "Want your keybinds and settings carried over? Give your agent this line:"))
            .child(this.copyLine(t, MIGRATE, "w-migrate-copy")),
        ),
      );
  }

  // Pages ---------------------------------------------------------------

  pageOne(t) {
    return v_flex()
      .gap(18)
      .child(this.title(t, "HyperOx runs your terminals in the background", "The terminals and agents you open here don't live in this window. They run in h0x-core, a small background program on your computer. Close HyperOx, update it, or let it crash, and your agents keep working. Open it again and everything is where you left it, as long as the computer is on."))
      .child(this.feature(t, "bot", "Agents tell you when they need you", "A ring while one works, \u201cNeeds you\u201d when it asks something, a sound when it finishes."))
      .child(this.feature(t, "git-fork", "Agents can start helpers", "They show up under the agent that started them. Click one to watch it."))
      .child(this.feature(t, "server", "Your other machines, in the same window", "A server or a Mac mini runs its own h0x-core, so its agents keep going even while this laptop sleeps."))
      .child(div().h(1).bg(t.border))
      .child(h_flex().gap(10).items_center().child(div().w(84).text_size(13).text_color(t.muted).child("Appearance")).child(this.segment(t, APPEARANCES, this.appearance, (v, cx) => this.setAppearance(v, cx))))
      .child(h_flex().gap(10).items_center().child(div().w(84).text_size(13).text_color(t.muted).child("Size")).child(this.sizeControl(t)).child(div().ml(6).font_family(this.mono()).text_size(11).text_color(t.muted).child("ctrl +/\u2212")))
      .child(this.comingFromH0x(t));
  }

  machineRow(t, icon, name, note, ok) {
    return h_flex()
      .h(40)
      .px(12)
      .gap(10)
      .items_center()
      .border_b(1)
      .border_color(t.border)
      .child(svg(I(icon)).size(15).text_color(t.muted))
      .child(div().flex_1().min_w(0).truncate().text_size(13).text_color(t.foreground).child(name))
      .child(this.stateTag(t, ok ? "info" : "gone", note));
  }

  pageTwo(t) {
    return v_flex()
      .gap(18)
      .child(this.title(t, "Add your other machines", "Pick a host from your ssh config. HyperOx connects, installs h0x-core there if it's missing, and lists that machine in the sidebar with its own spaces."))
      .child(
        v_flex()
          .rounded(6)
          .border_1()
          .border_color(t.border)
          .bg(t.background)
          .overflow_hidden()
          .child(this.machineRow(t, "monitor", "This computer", "here", true))
          .children(this.machines.map((m) => this.machineRow(t, "server", m.name, m.state === "connected" ? "connected" : (m.state ?? "offline").replace(/_/g, " "), m.state === "connected")))
          .child(
            h_flex()
              .id("w-add-machine")
              .h(40)
              .px(12)
              .gap(10)
              .items_center()
              .cursor_pointer()
              .text_size(13)
              .text_color(t.accent)
              .hover((s) => s.bg(t.hover))
              .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
              .on_click(() => run("mod.open", { name: "machine-add" }))
              .child(svg(I("plus")).size(15))
              .child("Add a machine"),
          ),
      )
      .child(this.para(t, "Not now? Add machine is at the bottom of the sidebar too."));
  }

  panelCard(t, p, last) {
    const on = this.panels().includes(p.id);
    return h_flex()
      .id(`w-panel-${p.id}`)
      .items_stretch()
      .when(!last, (el) => el.border_b(1).border_color(t.border))
      .cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click((_e, cx) => {
        this.togglePanel(p.id);
        cx.notify();
      })
      // The 2 px rule on the left edge of a row that is on.
      .child(div().flex_none().w(2).h_full().bg(on ? t.accent : "#00000000"))
      .child(
        h_flex()
          .flex_1()
          .min_w(0)
          .p(11)
          .gap(12)
          .items_start()
          .child(div().flex_none().mt(2).child(svg(I(p.icon)).size(16).text_color(on ? t.accent : t.muted)))
          .child(
            v_flex()
              .flex_1()
              .min_w(0)
              .gap(2)
              .child(
                h_flex()
                  .gap(8)
                  .items_center()
                  .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child(p.title))
                  .child(div().font_family(this.mono()).text_size(11).text_color(t.dim).child(`mods/${p.id}`)),
              )
              .child(div().w_full().text_size(12.5).line_height("18px").text_color(t.muted).child(p.about)),
          )
          .child(v_flex().flex_none().h_full().justify_center().child(this.stateTag(t, on ? "on" : "off", on ? "on" : "off"))),
      );
  }

  openCodeCard(t) {
    const yes = shared("welcome.opencode") === true;
    return v_flex()
      .id("w-opencode")
      .rounded(6)
      .border_1()
      .border_color(t.border)
      .child(
        h_flex()
          .p(11)
          .gap(12)
          .items_start()
          .child(div().flex_none().mt(2).child(svg(I("square-terminal")).size(16).text_color(t.muted)))
          .child(
            v_flex()
              .flex_1()
              .min_w(0)
              .gap(2)
              .child(
                h_flex()
                  .gap(6)
                  .items_center()
                  .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child("OpenCode CLI"))
                  .child(div().text_size(13.5).text_color(t.muted).child("(free)"))
                  .child(div().ml(2).font_family(this.mono()).text_size(11).text_color(t.dim).child("opencode")),
              )
              .child(div().w_full().text_size(12.5).line_height("18px").text_color(t.muted).child("A free coding agent for your terminal: pick one of its free models, or bring your own keys. HyperOx installs it in the background.")),
          )
          .child(
            v_flex().flex_none().h_full().justify_center().child(
              this.openCodeInstalled
                ? this.stateTag(t, "info", "installed")
                : this.segment(
                    t,
                    [
                      [false, "No"],
                      [true, "Yes"],
                    ],
                    yes,
                    (v, cx) => {
                      setShared("welcome.opencode", v);
                      cx.notify();
                    },
                  ),
            ),
          ),
      );
  }

  pageThree(t) {
    return v_flex()
      .gap(14)
      .child(this.title(t, "Choose your panels", "HyperOx comes with a couple of panels. Turn one on and it appears beside your terminals. Drag this window aside to see it. Later, the buttons in the tab bar open and close them."))
      .child(
        v_flex()
          .rounded(6)
          .border_1()
          .border_color(t.border)
          .bg(t.background)
          .overflow_hidden()
          .children(PANELS.map((p, i) => this.panelCard(t, p, i === PANELS.length - 1))),
      )
      .child(this.openCodeCard(t))
      .child(div().w_full().text_size(13).line_height("20px").text_color(t.dim).child("Ask your agent for a board of your agents, your pull requests, a cost meter, or the sidebar on the right. Not sure what's possible? Ask it that too."));
  }

  pageFour(t) {
    const pal = bound("app.palette");
    const found = (id, label, value) => this.button(t, id, label, false, (_e, cx) => this.setFoundVia(value, cx));
    return v_flex()
      .gap(16)
      // Asked once, first on this page: answer or skip, then it is gone for good.
      .when(this.foundVia === null, (el) =>
        el.child(
          v_flex()
            .gap(8)
            .p(12)
            .rounded(10)
            .bg(t.raised)
            .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child("How did you find HyperOx?"))
            .child(
              h_flex()
                .gap(4)
                .flex_wrap()
                .child(found("w-found-friend", "Friend", "friend"))
                .child(found("w-found-x", "X / Twitter", "x"))
                .child(found("w-found-github", "GitHub", "github"))
                .child(found("w-found-search", "Search", "search"))
                .child(found("w-found-other", "Other", "other"))
                .child(found("w-found-skip", "Skip", "skip")),
            ),
        ),
      )
      .when(Boolean(pal), (el) =>
        el.child(
          h_flex()
            .gap(12)
            .items_center()
            .p(12)
            .rounded(10)
            .border_1()
            .border_color(t.accent)
            .bg(t.accent + "10")
            .child(div().flex_none().px(8).py(3).rounded(6).border_1().border_color(t.border).bg(t.raised).text_size(13).font_semibold().text_color(t.foreground).child(pal))
            .child(
              v_flex()
                .flex_1()
                .min_w(0)
                .gap(2)
                .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child("One key opens everything"))
                .child(this.para(t, "It opens the command palette, with every action by name and its key. It works the same in every layout.")),
            ),
        ),
      )
      .child(this.title(t, "Make it yours", "Every part of this window is a layout or a mod: plain files your agent can change or add to. Give it this line to start:"))
      .child(this.copyLine(t, askText()))
      .child(this.para(t, "Then ask for anything: \u201cmove the sidebar to the right\u201d, \u201ca panel with my pull requests\u201d."))
      .child(
        v_flex()
          .gap(6)
          .p(12)
          .rounded(10)
          .bg(t.raised)
          .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child("Your terminals, outside this window too"))
          .child(this.para(t, "h0x-core is the runtime's command line. In any terminal, h0x-core ls lists everything running and h0x-core attach t3 jumps into one. Your agents can use it to start and watch other agents for you. Tell them to read h0x-core's help, or make it a skill.")),
      )
      .child(
        v_flex()
          .gap(10)
          .p(12)
          .rounded(10)
          .bg(t.raised)
          .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child("More examples in Settings"))
          .child(this.para(t, "Settings \u203a Look has more layouts to try or start from, HyperOx 98 among them. Your agent can copy any of them and change anything.")),
      )
      .child(this.para(t, "Something off or missing? Settings \u203a Send feedback. We read every one."));
  }

  render() {
    const t = theme();
    const go = (page, cx) => {
      this.page = Math.max(1, Math.min(PAGES, page));
      cx.notify();
    };
    // The primary button's label, also the word in the Enter hint.
    const next = this.page < PAGES ? (this.page === 2 && !this.machines.length ? "Not now" : "Next") : spaces().length ? "Done" : "Open a space";
    const dots = h_flex()
      .gap(4)
      .children([...Array(PAGES)].map((_, i) => i + 1).map((p) => div().w(p === this.page ? 16 : 6).h(6).rounded(3).bg(p === this.page ? t.foreground : t.input)));
    return v_flex()
      .id("welcome")
      .track_focus(this.handle)
      .size_full()
      .bg(t.popover)
      .on_key_down((e, cx) => {
        const k = e.key;
        const m = e.modifiers;
        if (k === "escape") return this.done("skipped");
        if (m.control || m.alt || m.platform) return;
        if (k === "enter" || k === "right") {
          if (this.page === PAGES) return k === "enter" ? this.start() : undefined;
          go(this.page + 1, cx);
        } else if (k === "left") go(this.page - 1, cx);
        else return;
        cx.stop_propagation();
      })
      .child(
        h_flex()
          .flex_none()
          .h(48)
          // Only the header drags the window, and only it shows the grab hand.
          .cursor_grab()
          .pl(18)
          .pr(12)
          .gap(10)
          .items_center()
          .border_b(1)
          .border_color(t.border)
          .child(image("frame/welcome/hyperox.png").w(22).h(22).rounded(5).flex_none())
          .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child("Welcome to HyperOx"))
          .child(div().px(5).h(18).flex().items_center().rounded(4).border_1().border_color(t.input).text_color(t.muted).font_family(this.mono()).text_size(10.5).child("beta"))
          .child(div().flex_1())
          .child(div().text_size(11).text_color(t.dim).child("Drag to move"))
          .child(
            div()
              .id("w-close")
              .w(26)
              .h(26)
              .rounded(6)
              .flex()
              .items_center()
              .justify_center()
              .text_color(t.muted)
              .cursor_pointer()
              .hover((s) => s.bg(t.hover).text_color(t.foreground))
              .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
              .on_click(() => this.done("skipped"))
              .child(svg(I("x")).size(14)),
          ),
      )
      // A bar that always shows: a page taller than the window should look it.
      .child(
        div()
          .relative()
          .flex_1()
          .min_h(0)
          .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
          .child(
            div()
              .id(`w-body-${this.page}`)
              .size_full()
              .overflow_y_scroll()
              // Padding inside, not on the scroll area: there it counts as
              // content and a page that fits still shows a bar.
              .child(div().px(22).pt(22).pb(20).child([this.pageOne, this.pageTwo, this.pageThree, this.pageFour][this.page - 1].call(this, t))),
          )
          .child(Scrollbar.vertical(`w-body-${this.page}`).mode("always").absolute().inset_0()),
      )
      .child(
        h_flex()
          .flex_none()
          .h(54)
          .pl(20)
          .pr(14)
          .gap(6)
          .items_center()
          .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
          .border_t(1)
          .border_color(t.border)
          .child(dots)
          .child(div().flex_1())
          .child(h_flex().gap(14).mr(12).child(this.hint(t, "Enter", next.toLowerCase())).child(this.hint(t, "Esc", "close")))
          .when(this.page > 1, (el) => el.child(this.button(t, "w-back", "Back", false, (_e, cx) => go(this.page - 1, cx))))
          .child(
            this.page < PAGES
              ? this.button(t, "w-next", next, true, (_e, cx) => {
                if (this.page === 3) this.installOpenCodeIfWanted();
                go(this.page + 1, cx);
              }, "arrow-right")
              : this.button(t, "w-start", next, true, () => this.start(), "arrow-right"),
          ),
      );
  }
}

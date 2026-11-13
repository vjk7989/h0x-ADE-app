// App-owned settings mod. Writes changes immediately; no Save step.

import { View, div, svg, image, Background } from "gpui-kit";
import { v_flex, h_flex, Scrollbar, InputState, Input } from "gpui-base";
import { core, run, theme, themes, fonts, updates, settings, layouts, layout, shared, setShared, appHome, readTextFile, writeTextFile, removeAppFile, playSound, openLink, copyText, notify, props, closeOverlay, window, account, accountSignIn, accountSwitch, accountSignOut, accountRemove } from "h0x";

const I = (name) => `frame/settings/${name}.svg`;

const SECTIONS = [
  { id: "look", title: "Look", icon: "palette" },
  ...(window()?.platform === "macos" ? [{ id: "keyboard", title: "Keyboard", icon: "keyboard" }] : []),
  { id: "machines", title: "Machines", icon: "server" },
  { id: "remote", title: "Remote", icon: "shield" },
  { id: "alerts", title: "Alerts", icon: "bell" },
  { id: "mods", title: "Mods", icon: "shield" },
  { id: "updates", title: "Updates", icon: "refresh" },
  { id: "info", title: "Info", icon: "shield" },
];

const LOOKS = { main: "HyperOx", terminals: "Terminals", win98: "HyperOx 98" };
const APPEARANCES = [
  [null, "System"],
  ["light", "Light"],
  ["dark", "Dark"],
];

// What the app uses when settings.json names no font (settings.rs).
const TOKEN_SHAPE = /^\d+:[A-Za-z0-9_-]{20,}$/; // the core's rule for a bot token
const DEFAULT_FONT = "JetBrains Mono";
const DEFAULT_UI_FONT = "Geist";

const PROVIDERS = { "google.com": "Google", "github.com": "GitHub", password: "Email" };
const accountName = (a) => a.name || a.email || "Account";

export default class Settings extends View {
  init(_props, cx) {
    const asked = props()?.section;
    this.section = SECTIONS.some((s) => s.id === asked) ? asked : "look";
    this.picking = null; // "font" | "ui_font" while its list is open
    this.filter = "";
    this.fontInput = InputState.new({ placeholder: "Type to search fonts" });
    this.fontInput.on("change", (_e, cx) => { this.filter = this.fontInput.value(); cx.notify(); });
    this.renameInput = InputState.new();
    this.renameInput.on("focus", (_e, cx) => { this.renameFocused = true; cx.notify(); });
    this.renameInput.on("blur", (_e, cx) => { this.renameFocused = false; cx.notify(); });
    this.renameInput.on("change", (_e, cx) => { if (this.machineEdit?.action === "rename") this.machineEdit.name = this.renameInput.value(); cx.notify(); });
    this.renameInput.on("submit", (_e, cx) => {
      const edit = this.machineEdit;
      if (edit?.action === "rename") this.machineAction(edit.machine, "rename", cx, edit.name);
    });
    this.person = {};
    this.analyticsAbout = false;
    this.acctMenu = null; // "switch" | "more" while its list is open
    this.acctConfirm = null; // uid whose removal awaits a yes
    this.acctBusy = false;
    this.raw = null;
    this.unreadable = false;
    this.error = null;
    this.previews = {}; // layout name -> its "preview" picture
    this.previewsOf = "";
    this.machineList = null;
    this.machineLoading = false;
    this.machineError = null;
    this.machineBusy = {};
    this.machineResults = {};
    this.machineEdit = null;
    this.machineNotice = null;
    this.tokenInput = InputState.new({ placeholder: "Paste the bot token" });
    this.tokenInput.set_masked(true);
    this.tokenAt = 0; // when the token field last changed; 0 = nothing waiting to be saved
    this.tokenInput.on("change", (_e, cx) => { this.tokenAt = Date.now(); cx.notify(); });
    this.tokenInput.on("submit", (_e, cx) => { this.tokenAt = 0; this.saveToken(cx, true); }); // Enter saves any text; the core judges it
    this.changingToken = false;
    this.showHelp = false;
    this.remote = null; // bridge ls
    this.remoteLog = [];
    this.remoteTerms = [];
    this.remoteError = null;
    this.remoteNotice = null;
    this.remoteProblem = null;
    this.remoteBusy = {};
    this.remoteLoading = false;
    this.pairing = null; // {code, expires_ms}
    this.webLink = null; // {url, pair}
    this.webQr = null; // {text, size, rows}
    this.reach = "lan"; // "lan" | "tunnel" | "vps"
    this.via = "";
    this.publicHost = InputState.new({ placeholder: "Public host, like vps.example.com" });
    this.tick = 0;
    this.handle = cx.focus_handle();
    this.handle.focus();
    cx.spawn(async (cx) => {
      await this.load();
      if (this.section === "machines") await this.loadMachines();
      cx.notify();
    });
    if (this.section === "remote") cx.spawn(async (cx) => { await this.loadRemote(); cx.notify(); });
    cx.timer.every(1000, (cx) => {
      if (this.section !== "remote") return;
      if (++this.tick % 3 === 0) {
        cx.spawn(async (cx) => {
          await this.loadRemote();
          cx.notify();
        });
      } else cx.notify(); // the pairing countdown
    });
    cx.timer.every(200, (cx) => {
      if (this.section !== "remote" || !this.tokenAt || Date.now() - this.tokenAt < 400) return;
      this.tokenAt = 0;
      if (TOKEN_SHAPE.test(this.tokenInput.value().trim())) this.saveToken(cx);
    });
    cx.timer.every(3000, (cx) => {
      if (this.section !== "machines") return;
      cx.spawn(async (cx) => {
        await this.loadMachines();
        cx.notify();
      });
    });
  }

  // After "Check now": follow the look (the answer comes from the app), then
  // "Up to date" for a moment; the timer runs only until then.
  follow(cx) {
    this.following?.cancel();
    this.following = cx.timer.every(300, (cx) => {
      if (updates().checking) this.asked = Date.now();
      else if (Date.now() - this.asked >= 3000) {
        this.asked = null;
        this.following.cancel();
        this.following = null;
      }
      cx.notify();
    });
  }

  file() {
    return `${appHome()}/settings.json`;
  }

  // Each layout file may name a picture of itself (`"preview"` on its root).
  loadPreviews(names, cx) {
    this.previewsOf = names.join("\n");
    cx.spawn(async (cx) => {
      const found = {};
      for (const name of names) {
        try {
          const preview = JSON.parse((await readTextFile(`${appHome()}/layouts/${name}.json`)) ?? "{}").preview;
          if (typeof preview === "string" && preview) found[name] = preview;
        } catch {}
      }
      this.previews = found;
      cx.notify();
    });
  }

  async load() {
    this.raw = await readTextFile(this.file());
    // A missing file is empty (it may be created); one that is there but can't be read as an
    // object is never written over: that would drop everything else in it.
    this.unreadable = false;
    this.person = {};
    if (this.raw === null || this.raw === undefined) return;
    try {
      const person = JSON.parse(this.raw.replace(/^\uFEFF/, ""));
      if (person === null || typeof person !== "object" || Array.isArray(person)) throw new Error("not an object");
      this.person = person;
    } catch {
      this.unreadable = true;
    }
  }

  // Changes settings.json the way an agent would: read, change, write back
  // only if nobody changed it meanwhile (else read again and redo).
  change(edit, cx) {
    cx.spawn(async (cx) => {
      for (let attempt = 0; attempt < 3; attempt++) {
        await this.load();
        if (this.unreadable) {
          this.error = "settings.json can't be read (empty or not valid JSON), so nothing was changed. Fix or delete the file, then try again.";
          break;
        }
        this.error = null;
        const next = JSON.parse(JSON.stringify(this.person));
        edit(next);
        try {
          await writeTextFile(this.file(), JSON.stringify(next, null, 2) + "\n", this.raw);
          this.person = next;
          break;
        } catch {}
      }
      cx.notify();
    });
  }

  close() {
    closeOverlay(props().overlay);
  }

  // Pieces ---------------------------------------------------------------

  heading(t, text) {
    return div().pt(6).pb(8).text_size(12).font_semibold().text_color(t.dim).child(text.toUpperCase());
  }

  // One setting: what it is on the left, the control on the right.
  row(t, title, about, control) {
    return h_flex()
      .w_full()
      .gap(16)
      .py(12)
      .items_center()
      .border_b(1)
      .border_color(t.border)
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(3)
          .child(div().text_size(14).text_color(t.foreground).child(title))
          .when(Boolean(about), (el) => el.child(div().text_size(12.5).line_height("18px").text_color(t.dim).child(about))),
      )
      .child(div().flex_none().child(control));
  }

  button(t, id, label, onClick) {
    return div()
      .id(id)
      .px(12)
      .py(5)
      .rounded(7)
      .border_1()
      .border_color(t.border)
      .bg(t.raised)
      .cursor_pointer()
      .text_size(13)
      .text_color(t.foreground)
      .hover((s) => s.bg(t.hover))
      .on_click(onClick)
      .child(label);
  }

  // A button that can't be pressed now.
  disabledButton(t, id, label) {
    return div().id(id).px(12).py(5).rounded(7).border_1().border_color(t.border).bg(t.raised).opacity(0.45).text_size(13).text_color(t.dim).child(label);
  }

  // − value +
  stepper(t, id, value, onMinus, onPlus) {
    const b = (k, label, f) =>
      div()
        .id(`${id}-${k}`)
        .w(28)
        .h(28)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_size(15)
        .text_color(t.muted)
        .hover((s) => s.bg(t.hover).text_color(t.foreground))
        .on_click(f)
        .child(label);
    return h_flex()
      .items_center()
      .rounded(7)
      .border_1()
      .border_color(t.border)
      .bg(t.raised)
      .overflow_hidden()
      .child(b("minus", "\u2212", onMinus))
      .child(div().w(54).text_size(13).text_color(t.foreground).flex().justify_center().child(value))
      .child(b("plus", "+", onPlus));
  }

  toggle(t, id, on, onClick) {
    return div()
      .id(id)
      .w(36)
      .h(20)
      .rounded(10)
      .p(2)
      .cursor_pointer()
      .bg(on ? t.accent : t.hover)
      .flex()
      .when(on, (el) => el.justify_end())
      .on_click(onClick)
      .child(div().w(16).h(16).rounded(8).bg(on ? t.accent_foreground : t.muted));
  }

  // The To-do button and panel: "todo" in the shared panel list (the tab bar's own default
  // includes it). The lists' files stay on disk either way.
  setTodoPanel(on) {
    const stored = shared("workspace.panels");
    const list = Array.isArray(stored) ? stored : ["files", "changes", "sessions", "todo"];
    const next = on ? [...list.filter((x) => x !== "todo"), "todo"] : list.filter((x) => x !== "todo");
    setShared("workspace.panels", next);
    // The ids the tab bar migration knows (defaults/mods/tabs/main.js enabledPanels): a list written here is the person's choice, never migrated.
    const added = Array.isArray(shared("workspace.panels.added")) ? shared("workspace.panels.added") : [];
    setShared("workspace.panels.added", [...added, ...["sessions", "todo"].filter((id) => !added.includes(id))]);
    const body = (function find(node) {
      if (node?.id === "panel-body") return node;
      for (const child of node?.children ?? []) {
        const found = find(child);
        if (found) return found;
      }
      return null;
    })(layout());
    if (!on && body?.active === "todo") run("layout.hide", { id: "panel" });
  }

  // Look -----------------------------------------------------------------

  // A small drawing of each layout, in the theme's colors (as the welcome draws them).
  preview(t, name) {
    const row = () => div().flex().flex_row();
    const col = () => div().flex().flex_col();
    const term = (bg = t.background, edge = t.border) => div().flex_1().rounded(2).bg(bg).border_1().border_color(edge);
    const line = (w, c = t.dim) => div().h(3).w(w).flex_none().rounded(2).bg(c);
    const lines = (n, c) => col().gap(4).p(5).children([...Array(n)].map((_, i) => line(i % 2 ? 22 : 32, c)));
    const frame = (el) => el.size_full().rounded(6).overflow_hidden().border_1().border_color(t.border).bg(t.surface);
    if (name === "main")
      return frame(row())
        .child(col().w(42).border_r(1).border_color(t.border).child(lines(4)))
        .child(
          col()
            .flex_1()
            .child(row().h(11).gap(2).p(2).border_b(1).border_color(t.border).child(div().w(20).rounded(2).bg(t.accent)).child(div().w(16).rounded(2).bg(t.raised)).child(div().w(16).rounded(2).bg(t.raised)))
            .child(row().flex_1().p(4).gap(3).child(term()).child(term())),
        );
    if (name === "terminals") {
      const card = () => col().flex_1().rounded(4).border_1().border_color("#2c2f34").bg("#17191c").child(row().h(7).px(3).items_center().border_b(1).border_color("#25282c").child(line(14, "#5d6168")));
      return frame(col())
        .bg("#131619")
        .p(4)
        .gap(4)
        .child(row().h(10).gap(3).p(1).rounded(4).bg("#1d1f22").child(div().w(30).rounded(3).bg("#34373c").border_1().border_color("#4a4d53")).child(div().w(30)))
        .child(row().flex_1().gap(4).child(card()).child(card()));
    }
    // A layout of the person's own: a plain frame.
    return frame(col()).child(row().h(11).border_b(1).border_color(t.border)).child(row().flex_1().p(4).child(term()));
  }

  layoutPicker(t, cx) {
    const current = shared("layout") ?? "main";
    // Shipped looks in the welcome's order, then the person's own; new files show up by themselves.
    const shipped = Object.keys(LOOKS);
    const rank = (n) => (shipped.includes(n) ? shipped.indexOf(n) : shipped.length);
    const names = [...layouts()].sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
    if (names.join("\n") !== this.previewsOf) this.loadPreviews(names, cx);
    return h_flex()
      .w_full()
      .flex_wrap()
      .gap(8)
      .children(
        names.map((name) => {
          const on = name === current;
          return v_flex()
            .id(`st-l-${name}`)
            .w(124)
            .gap(6)
            .p(6)
            .rounded(10)
            .border_2()
            .border_color(on ? t.accent : "#00000000")
            .bg(on ? t.raised : "#00000000")
            .cursor_pointer()
            .hover((s) => s.bg(t.raised))
            .on_click((_e, cx) => {
              run("layout.switch", { name });
              cx.notify();
            })
            .child(
              div()
                .h(62)
                .w_full()
                .overflow_hidden()
                .child(
                  this.previews[name]
                    ? image(this.previews[name]).w_full().h(62).rounded(6).border_1().border_color(t.border)
                    : this.preview(t, name),
                ),
            )
            .child(
              h_flex()
                .gap(5)
                .items_center()
                .child(div().text_size(12.5).text_color(on ? t.foreground : t.muted).child(LOOKS[name] ?? name))
                .when(on, (el) => el.child(svg(I("check")).size(12).text_color(t.accent))),
            );
        }),
      );
  }

  appearancePicker(t, cx) {
    const now = this.person.appearance ?? null;
    return h_flex()
      .p(2)
      .gap(2)
      .rounded(8)
      .border_1()
      .border_color(t.border)
      .bg(t.background)
      .children(
        APPEARANCES.map(([value, label]) =>
          div()
            .id(`st-ap-${label}`)
            .px(12)
            .py(4)
            .rounded(6)
            .text_size(13)
            .cursor_pointer()
            .when(now === value, (el) => el.bg(t.raised).text_color(t.foreground).border_1().border_color(t.border))
            .when(now !== value, (el) => el.text_color(t.muted).hover((s) => s.text_color(t.foreground)))
            .on_click((_e, cx) =>
              this.change((p) => {
                if (value === null) delete p.appearance;
                else p.appearance = value;
              }, cx),
            )
            .child(label),
        ),
      );
  }

  themePicker(t, light) {
    const s = settings();
    const current = light ? s.theme_light : s.theme_dark;
    return h_flex()
      .w_full()
      .flex_wrap()
      .gap(8)
      .children(
        // A theme named like a layout (themes/win98.json) is that layout's own.
        themes()
          .filter((x) => Boolean(x.light) === light && (x.name === shared("layout") || !layouts().includes(x.name)))
          .map(({ name, colors: c }) => {
          const on = name === current;
          const label = name.replace(/-/g, " ");
          const dot = (color) => div().w(8).h(8).rounded(4).bg(color);
          return v_flex()
            .id(`st-t-${name}`)
            .w(124)
            .gap(6)
            .p(6)
            .rounded(10)
            .border_2()
            .border_color(on ? t.accent : "#00000000")
            .cursor_pointer()
            .hover((s) => s.bg(t.raised))
            .on_click((_e, cx) => {
              run("app.theme", { name });
              cx.notify();
            })
            .child(
              v_flex()
                .h(46)
                .w_full()
                .p(7)
                .gap(6)
                .rounded(6)
                .border_1()
                .border_color(t.border)
                .bg(c.background)
                .child(div().h(4).w(44).rounded(2).bg(c.terminal_foreground))
                .child(h_flex().gap(4).child(dot(c.accent)).child(dot(c.ansi1)).child(dot(c.ansi2)).child(dot(c.ansi3)).child(dot(c.ansi5))),
            )
            .child(
              h_flex()
                .w_full()
                .min_w(0)
                .gap(5)
                .items_center()
                .child(div().min_w(0).truncate().tooltip(label).text_size(12.5).text_color(on ? t.foreground : t.muted).child(label))
                .when(on, (el) => el.child(svg(I("check")).size(12).flex_none().text_color(t.accent))),
            );
        }),
      );
  }

  // The font in use, written in itself; a click opens the list.
  fontButton(t, which, family) {
    const open = this.picking === which;
    return div()
      .id(`st-f-${which}`)
      .w(220)
      .px(12)
      .py(6)
      .rounded(7)
      .border_1()
      .border_color(open ? t.accent : t.border)
      .bg(t.raised)
      .cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .on_click((_e, cx) => {
        this.picking = open ? null : which;
        this.filter = "";
        this.fontInput.set_value("");
        if (this.picking) this.fontInput.focus();
        else this.handle.focus();
        cx.notify();
      })
      .child(div().font_family(family).text_size(13.5).text_color(t.foreground).child(family));
  }

  fontList(t, which, def) {
    const q = this.filter.toLowerCase();
    const matches = fonts().filter((f) => f.toLowerCase().includes(q)).slice(0, 200);
    const choose = (name, cx) => {
      this.change((p) => {
        if (name === null) delete p[which];
        else p[which] = name;
      }, cx);
      this.picking = null;
      this.handle.focus();
      cx.notify();
    };
    const item = (id, label, family, onClick) =>
      div()
        .id(id)
        .px(10)
        .py(5)
        .rounded(6)
        .cursor_pointer()
        .hover((s) => s.bg(t.hover))
        .on_click(onClick)
        .child(div().font_family(family).text_size(13.5).text_color(t.foreground).child(label));
    return v_flex()
      .w_full()
      .mb(8)
      .rounded(8)
      .border_1()
      .border_color(t.border)
      .bg(t.background)
      // Let the child's native scroll run before stopping the page's wheel event.
      .on_scroll_wheel((_e, cx) => cx.stop_propagation())
      .child(
        div()
          .px(12)
          .py(8)
          .border_b(1)
          .border_color(t.border)
          .text_size(13)
          .child(Input.new(this.fontInput).h(28).text_color(t.foreground)),
      )
      .child(
        div()
          .id(`st-fl-${which}`)
          .h(200)
          .p(4)
          .overflow_y_scrollbar()
          .when(!q, (el) => el.child(item(`st-fd-${which}`, `Default (${def})`, def, (_e, cx) => choose(null, cx))))
          .children(matches.map((f, i) => item(`st-fi-${which}-${i}`, f, f, (_e, cx) => choose(f, cx)))),
      );
  }

  look(t, cx) {
    const s = settings();
    const p = this.person;
    const clipboardOn = p.clipboard_write === true;
    const panels = shared("workspace.panels");
    const todoOn = Array.isArray(panels) ? panels.includes("todo") : true;
    // The settings file holds any number; the app uses it only inside this range, else 10,000.
    const stops = [1000, 2000, 5000, 10000, 20000, 50000, 100000];
    const lines = Number.isInteger(p.scrollback_lines) && p.scrollback_lines >= 1000 && p.scrollback_lines <= 100000 ? p.scrollback_lines : 10000;
    const stopAt = (dir) => (dir < 0 ? [...stops].reverse().find((n) => n < lines) : stops.find((n) => n > lines)) ?? lines;
    const own = (key, composed, def) => (p[key] ?? def) !== composed;
    const size = (key, value, d, cx) => this.change((q) => (q[key] = Math.max(8, Math.min(32, (q[key] ?? value) + d))), cx);
    const fontRow = (which, sizeKey, title, family, def, sizeNow) =>
      v_flex()
        .w_full()
        .child(
          this.row(
            t,
            title,
            own(which, family, def) ? "This layout sets its own; yours shows in the others." : null,
            h_flex()
              .gap(8)
              .items_center()
              .child(this.fontButton(t, which, family))
              // The panels' text follows zoom: they size their rows to it.
              .when(Boolean(sizeKey), (el) => el.child(this.stepper(t, `st-${sizeKey}`, `${Math.round(sizeNow)}`, (_e, cx) => size(sizeKey, sizeNow, -1, cx), (_e, cx) => size(sizeKey, sizeNow, 1, cx)))),
          ),
        )
        .when(this.picking === which, (el) => el.child(this.fontList(t, which, def)));
    return v_flex()
      .w_full()
      .child(this.heading(t, "Layout"))
      .child(this.layoutPicker(t, cx))
      .child(div().h(18))
      .child(this.heading(t, "Theme"))
      .child(this.row(t, "Appearance", "Light or dark, or as your system is now.", this.appearancePicker(t, cx)))
      .child(div().pt(12).pb(8).text_size(12.5).text_color(t.dim).child("For the layout you're in: each layout keeps its own light and dark theme."))
      .child(div().pb(6).text_size(12.5).font_medium().text_color(t.muted).child("Dark"))
      .child(this.themePicker(t, false))
      .child(div().pt(10).pb(6).text_size(12.5).font_medium().text_color(t.muted).child("Light"))
      .child(this.themePicker(t, true))
      .child(div().h(18))
      .child(this.heading(t, "Size and fonts"))
      .child(
        this.row(
          t,
          "Zoom",
          "Everything: terminals, panels, text.",
          h_flex()
            .gap(8)
            .items_center()
            .child(this.stepper(t, "st-zoom", `${Math.round(s.zoom * 100)}%`, () => run("app.zoom_out"), () => run("app.zoom_in")))
            .child(this.button(t, "st-zoom-reset", "Reset", () => run("app.zoom_reset"))),
        ),
      )
      .child(fontRow("font", "font_size", "Terminal font", s.font_family, DEFAULT_FONT, s.font_size))
      .child(fontRow("ui_font", null, "Interface font", s.ui_font_family, DEFAULT_UI_FONT))
      .child(div().h(18))
      .child(this.heading(t, "Panels"))
      .child(this.row(t, "To-do list", "Show the To-do button and panel.", this.toggle(t, "st-todo", todoOn, () => this.setTodoPanel(!todoOn))))
      .child(div().h(18))
      .child(this.heading(t, "Terminal"))
      .child(
        this.row(
          t,
          "Let programs copy to the clipboard",
          "Terminal programs, including ones on remote machines, can replace what you copied. Off unless you turn it on.",
          this.toggle(t, "st-clipboard-write", clipboardOn, (_e, cx) => this.change((p) => { p.clipboard_write = !clipboardOn; }, cx)),
        ),
      )
      .child(
        this.row(
          t,
          "Scrollback lines per pane",
          "How far back each terminal pane keeps its output (1,000 to 100,000). Wide panes or long wrapped lines keep fewer. Applies to panes opened after the change.",
          this.stepper(t, "st-scrollback", String(lines).replace(/\B(?=(\d{3})+(?!\d))/g, ","), (_e, cx) => this.change((p) => { p.scrollback_lines = stopAt(-1); }, cx), (_e, cx) => this.change((p) => { p.scrollback_lines = stopAt(1); }, cx)),
        ),
      );
  }

  keyboard(t) {
    const choices = [
      ["auto", "Automatic (Alt on US keyboards)"],
      ["on", "Alt"],
      ["off", "Types characters"],
      ["left", "Left Option is Alt"],
      ["right", "Right Option is Alt"],
    ];
    return v_flex()
      .w_full()
      .child(this.heading(t, "Option key"))
      .child(div().pb(12).text_size(13).text_color(t.dim).child("Use Option for terminal shortcuts or to type accents and special characters."))
      .children(choices.map(([value, label]) => {
        const on = (this.person.option_as_alt ?? "auto") === value;
        return h_flex()
          .id(`st-option-${value}`)
          .gap(10)
          .px(12)
          .py(10)
          .rounded(7)
          .items_center()
          .cursor_pointer()
          .hover((s) => s.bg(t.hover))
          .on_click((_e, cx) => this.change((p) => { p.option_as_alt = value; }, cx))
          .child(div().text_color(on ? t.accent : t.muted).child(on ? "◉" : "○"))
          .child(div().text_size(14).text_color(t.foreground).child(label));
      }));
  }

  // Machines -------------------------------------------------------------

  async loadMachines() {
    if (this.machineLoading) return;
    this.machineLoading = true;
    try {
      const r = await core(["machine", "ls", "--json"]);
      if (r.code !== 0) throw new Error(r.stderr || r.stdout || "Couldn't read machines.");
      const list = JSON.parse(r.stdout).machines;
      if (!Array.isArray(list)) throw new Error("Couldn't read machines.");
      this.machineList = list.filter((m) => !m.temporary);
      this.machineError = null;
    } catch (e) {
      this.machineError = String(e);
    } finally {
      this.machineLoading = false;
    }
  }

  machineAction(machine, action, cx, name) {
    if (this.machineBusy[machine]) return;
    if (action === "rename" && (!name?.trim() || name.trim() === machine)) return;
    this.machineBusy[machine] = action;
    delete this.machineResults[machine];
    this.machineEdit = null;
    this.handle.focus();
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const args = ["machine", action, machine];
        if (action === "rename") args.push(name.trim());
        const r = await core(args);
        let payload = null;
        try { payload = JSON.parse(r.stdout || r.stderr); } catch {}
        if (r.code !== 0) throw new Error(payload?.error?.message || (r.stderr || r.stdout || "It didn't work.").trim());
        const message = action === "update"
          ? `Updated ${machine}${payload?.version ? ` to h0x-core ${payload.version}` : ""}; its terminals kept running.`
          : action === "rename" ? `Renamed ${machine} to ${name.trim()}.` : `Removed ${machine}; its terminals keep running there.`;
        if (action === "update") this.machineResults[machine] = { message, ok: true };
        else this.machineNotice = message;
      } catch (e) {
        this.machineResults[machine] = { message: String(e), ok: false };
      } finally {
        delete this.machineBusy[machine];
      }
      await this.loadMachines();
      cx.notify();
    });
  }

  olderCore(remote, local) {
    const parse = (v) => /^(\d+)\.(\d+)\.(\d+)(?:-([^+]+))?(?:\+.*)?$/.exec(v ?? "");
    const a = parse(remote), b = parse(local);
    if (!a || !b) return false;
    for (let i = 1; i <= 3; i++) {
      if (+a[i] !== +b[i]) return +a[i] < +b[i];
    }
    if (!a[4] || !b[4]) return Boolean(a[4] && !b[4]);
    const x = a[4].split("."), y = b[4].split(".");
    for (let i = 0; i < Math.max(x.length, y.length); i++) {
      if (x[i] === y[i]) continue;
      if (x[i] === undefined || y[i] === undefined) return x[i] === undefined;
      const xn = /^\d+$/.test(x[i]), yn = /^\d+$/.test(y[i]);
      return xn && yn ? +x[i] < +y[i] : xn !== yn ? xn : x[i] < y[i];
    }
    return false;
  }

  machineRow(t, m, localVersion) {
    const busy = this.machineBusy[m.name];
    const edit = this.machineEdit?.machine === m.name ? this.machineEdit : null;
    const state = String(m.state ?? "unknown").replace(/[_-]/g, " ");
    const color = ["connected", "online"].includes(m.state) ? t.ansi2 : ["connecting", "reconnecting"].includes(m.state) ? t.working : t.blocked;
    const result = this.machineResults[m.name];
    const controls = h_flex().gap(6).items_center();
    if (busy) controls.child(svg("mods/workspace-chrome/spinner.svg").size(12).text_color(t.working).spin(900)).child(div().text_size(12).text_color(t.muted).child(busy === "update" ? "Updating…" : busy === "rename" ? "Renaming…" : "Removing…"));
    else {
      if (this.olderCore(m.version, localVersion)) controls.child(this.button(t, `st-m-update-${m.name}`, "Update", (_e, cx) => this.machineAction(m.name, "update", cx)));
      for (const [action, label] of [["rename", "Rename"], ["rm", "Remove"]]) {
        controls.child(this.button(t, `st-m-${action}-${m.name}`, label, (_e, cx) => {
          this.machineEdit = { machine: m.name, action, name: m.name };
          if (action === "rename") {
            this.renameInput.set_value(m.name);
            this.renameInput.set_selected_range({ start: 0, end: encodeURIComponent(m.name).replace(/%[0-9A-F]{2}/g, "_").length });
            this.renameFocused = true;
            this.renameInput.focus();
          }
          cx.notify();
        }));
      }
    }
    const row = v_flex().w_full().py(12).gap(8).border_b(1).border_color(t.border)
      .child(h_flex().gap(10).items_center()
        .child(div().w(7).h(7).flex_none().rounded(4).bg(color))
        .child(v_flex().flex_1().min_w(0).gap(3)
          .child(div().text_size(14).text_color(t.foreground).child(m.name))
          .child(div().text_size(12.5).text_color(t.dim).child(`${m.target ?? ""} · ${state} · ${m.version ? `h0x-core ${m.version}` : "h0x-core version unknown"}`)))
        .child(controls.flex_none()));
    if (busy === "update") row.child(div().text_size(12.5).text_color(t.dim).child("Copying this computer's h0x-core. Its terminals keep running."));
    if (edit && !busy) {
      const cancel = this.button(t, `st-m-cancel-${m.name}`, "Cancel", (_e, cx) => { this.machineEdit = null; this.handle.focus(); cx.notify(); });
      if (edit.action === "rm") row.child(v_flex().gap(8)
        .child(div().text_size(12.5).text_color(t.muted).child(`Remove ${m.name}? Its terminals keep running there.`))
        .child(h_flex().gap(6).child(cancel).child(this.button(t, `st-m-confirm-${m.name}`, "Remove machine", (_e, cx) => this.machineAction(m.name, "rm", cx)))));
      else row.child(v_flex().gap(6)
        .child(div().text_size(12).text_color(t.dim).child("New name (addresses using the old name stop working)"))
        .child(h_flex().gap(6).items_center()
          .child(Input.new(this.renameInput).flex_1().min_w(0).h(32).px(10).rounded(7).border_1().border_color(this.renameFocused ? t.accent : t.input).bg(t.background).text_size(13).text_color(t.foreground))
          .child(cancel).child(this.button(t, `st-m-save-${m.name}`, "Save", (_e, cx) => this.machineAction(m.name, "rename", cx, edit.name)))));
    }
    if (result) row.child(div().text_size(12.5).text_color(result.ok ? t.ansi2 : t.blocked).child(result.message));
    return row;
  }

  machines(t) {
    const version = updates().core.version;
    return v_flex().w_full()
      .child(this.heading(t, "Machines"))
      .child(div().pb(8).text_size(12.5).line_height("18px").text_color(t.dim).child("Your computers, reached over ssh. Their terminals keep running when you disconnect."))
      .child(this.row(t, "This computer", `Connected · ${version ? `h0x-core ${version}` : "h0x-core version unknown"}`, div().w(7).h(7).rounded(4).bg(t.ansi2)))
      .children((this.machineList ?? []).map((m) => this.machineRow(t, m, version)))
      .when(this.machineList === null && !this.machineError, (el) => el.child(h_flex().py(14).gap(8).items_center().child(svg("mods/workspace-chrome/spinner.svg").size(12).text_color(t.working).spin(900)).child(div().text_size(12.5).text_color(t.dim).child("Reading machines…"))))
      .when(this.machineList?.length === 0, (el) => el.child(div().py(14).text_size(12.5).text_color(t.dim).child("No other machines yet.")))
      .when(Boolean(this.machineError), (el) => el.child(div().py(10).text_size(12.5).text_color(t.blocked).child(this.machineError)))
      .when(Boolean(this.machineNotice), (el) => el.child(div().py(10).text_size(12.5).text_color(t.ansi2).child(this.machineNotice)))
      .child(h_flex().pt(16).child(this.button(t, "st-m-add", "Add a machine", () => run("mod.open", { name: "machine-add" }))));
  }

  // Remote ---------------------------------------------------------------

  // Runs a core command; JSON answers come back parsed. A core that doesn't know the command says to update.
  async coreJson(args) {
    const r = await core(args);
    let payload = null;
    try { payload = JSON.parse(r.stdout || r.stderr); } catch {}
    if (r.code !== 0) {
      const text = payload?.error?.message || (r.stderr || r.stdout || "It didn't work.").trim();
      throw new Error(/unrecogni[sz]ed|unknown (sub)?command|invalid subcommand|no such command/i.test(text) ? "Update h0x-core to use remote control." : text);
    }
    return payload;
  }

  termRunning(name) {
    return this.remoteTerms.some((t) => t.name === name && t.state !== "finished");
  }

  async loadRemote() {
    if (this.remoteLoading) return;
    this.remoteLoading = true;
    try {
      const ls = await this.coreJson(["bridge", "ls", "--json"]);
      if (!Array.isArray(ls?.identities)) throw new Error("Couldn't read remote control.");
      this.remote = ls;
      this.remoteError = null;
      const log = await this.coreJson(["bridge", "log", "-n", "20", "--json"]).catch(() => null);
      this.remoteLog = Array.isArray(log?.entries) ? log.entries.slice(-20).reverse() : [];
      const terms = await this.coreJson(["ls", "--json"]).catch(() => null);
      this.remoteTerms = Array.isArray(terms?.terminals) ? terms.terminals : [];
      await this.loadMachines();
      if (!this.termRunning("h0x-bridge-web")) {
        this.webLink = null;
        this.webQr = null;
      } else if (!this.webLink) {
        const r = await core(["read", "h0x-bridge-web"]);
        for (const line of String(r.stdout ?? "").split("\n")) {
          let found = null;
          try { found = JSON.parse(line.trim()); } catch {}
          if (found && (found.url || found.pair)) { this.webLink = found; break; }
        }
      }
      if (this.webLink?.pair && this.webQr?.text !== this.webLink.pair) {
        const q = await this.coreJson(["bridge", "qr", this.webLink.pair, "--json"]).catch((e) => { this.remoteError = e.message ?? String(e); return null; });
        const ok = Array.isArray(q?.rows) && q.rows.every((r) => typeof r === "string") && Number.isFinite(q.size);
        this.webQr = { text: this.webLink.pair, size: q?.size, rows: ok ? q.rows : null };
      }
    } catch (e) {
      this.remoteError = e.message ?? String(e);
    } finally {
      this.remoteLoading = false;
    }
  }

  remoteAction(key, work, cx) {
    if (this.remoteBusy[key]) return;
    this.remoteBusy[key] = true;
    this.remoteProblem = null;
    this.remoteNotice = null;
    cx.notify();
    cx.spawn(async (cx) => {
      try { await work(); } catch (e) { this.remoteProblem = e.message ?? String(e); } finally { delete this.remoteBusy[key]; }
      await this.loadRemote();
      cx.notify();
    });
  }

  // The token goes through a file under the app folder (never an argument) and the file is
  // deleted afterwards, whatever the result; a leftover from a crash is deleted first.
  saveToken(cx, force = false) {
    const token = this.tokenInput.value().trim();
    if (!token || (!force && !TOKEN_SHAPE.test(token))) return;
    const path = `${appHome()}/telegram-token.tmp`;
    this.remoteAction("token", async () => {
      try {
        await removeAppFile(path);
        await writeTextFile(path, token, null);
        await this.coreJson(["bridge", "telegram", "token", "--file", path]);
        this.tokenInput.set_value("");
        this.tokenAt = 0; // clearing the field is not a new token
        this.changingToken = false;
        notify({ title: "Bot token saved", level: "success" });
      } catch (e) {
        notify({ title: "Couldn't save the bot token", body: e.message ?? String(e), level: "error" });
        throw e;
      } finally {
        await removeAppFile(path).catch(() => {});
      }
    }, cx);
  }

  startArgs(name, label, command) {
    return ["run", "--bg", "--keep", "--no-parent", "--outlive-parent", "--name", name, "--label", label, "--", "h0x-core", ...command];
  }

  startWeb(cx) {
    const host = this.publicHost.value().trim();
    const options = this.reach === "tunnel" ? ["--tunnel", "--yes"] : this.reach === "vps" ? ["--via", this.via, "--public-host", host] : ["--lan"];
    this.remoteAction("web", async () => {
      if (this.reach === "vps" && (!this.via || !host)) throw new Error("Pick a machine and type its public host.");
      this.webLink = null;
      this.webQr = null;
      await this.coreJson(this.startArgs("h0x-bridge-web", "h0x-bridge=web", ["bridge", "web", "--json", ...options]));
    }, cx);
  }

  remoteButton(t, id, label, work) {
    return this.button(t, id, this.remoteBusy[id] ? "…" : label, (_e, cx) => this.remoteAction(id, work, cx));
  }

  qrGrid(q) {
    const px = 5, quiet = 4, pad = (q.size + quiet * 2) * px;
    const grid = v_flex().w(pad).h(pad).flex_none().bg("#ffffff").p(quiet * px);
    for (const row of q.rows) {
      const line = h_flex().h(px).flex_none();
      for (const run of row.match(/0+|1+/g) ?? []) line.child(div().w(run.length * px).h(px).flex_none().bg(run[0] === "1" ? "#000000" : "#ffffff"));
      grid.child(line);
    }
    return grid;
  }

  remoteStatus(t) {
    const r = this.remote;
    const when = (ms) => (ms ? new Date(ms).toLocaleDateString() : "");
    return v_flex().w_full()
      .child(this.heading(t, "Status"))
      .child(this.row(t, `Remote control: ${r.locked ? "Locked" : "On"}`, r.locked ? "Paired devices can't run commands until you unlock." : "Paired devices can reach your terminals.",
        this.remoteButton(t, "st-r-lock", r.locked ? "Unlock" : "Lock", () => this.coreJson(["bridge", r.locked ? "unlock" : "lock"]))))
      .children(r.identities.map((i) => this.row(t, i.label || i.user_id || i.id, `${i.transport} · paired ${when(i.paired_at_ms)}`,
        this.remoteButton(t, `st-r-unpair-${i.id}`, "Remove", () => this.coreJson(["bridge", "unpair", i.id])))))
      .when(r.identities.length === 0, (el) => el.child(div().py(10).text_size(12.5).text_color(t.dim).child("No paired devices yet.")));
  }

  remoteTelegram(t) {
    const r = this.remote;
    const on = this.termRunning("h0x-bridge-telegram");
    const left = this.pairing ? Math.max(0, Math.ceil((this.pairing.expires_ms - Date.now()) / 1000)) : 0;
    const field = Input.new(this.tokenInput).flex_1().min_w(0).h(32).px(10).rounded(7).border_1().border_color(t.input).bg(t.background).text_size(13).text_color(t.foreground);
    const paired = r.identities.filter((i) => i.transport === "telegram").length;
    const saved = r.telegram_token === "set";
    const status = paired > 0 ? `Paired with ${paired} account${paired === 1 ? "" : "s"}` : saved ? "Token saved. Now pair your Telegram." : "Not set up";
    const editing = !saved || this.changingToken;
    const pairButton = !account().current ? this.disabledButton(t, "st-r-pair", "Pair my Telegram") : this.remoteButton(t, "st-r-pair", "Pair my Telegram", async () => {
      const p = await this.coreJson(["bridge", "pair", "--transport", "telegram", "--json"]);
      this.pairing = { code: p.code, expires_ms: p.expires_ms };
    });
    const live = Boolean(this.pairing) && left > 0;
    const command = this.pairing ? `/pair ${this.pairing.code}` : "";
    const commands = [
      ["/menu", "Show buttons for sessions, projects and help."],
      ["/sessions", "List your terminals and pick one."],
      ["/projects", "List your project folders."],
      ["/help", "Show this list in Telegram."],
      ["/agents", "List the coding agents that are running."],
      ["/read", "Read what a terminal shows."],
      ["/send", "Type text into a terminal."],
      ["/lock", "Stop paired devices from running commands."],
    ];
    return v_flex().w_full()
      .child(this.heading(t, "Telegram"))
      .child(div().py(6).text_size(13).font_semibold().text_color(paired > 0 ? t.ansi2 : t.muted).child(status))
      .child(editing
        ? h_flex().w_full().py(10).gap(6).items_center().border_b(1).border_color(t.border)
          .child(field)
          .child(div().flex_none().child(pairButton))
        : h_flex().w_full().py(10).gap(10).items_center().border_b(1).border_color(t.border)
          .child(div().flex_1().text_size(13.5).text_color(t.foreground).child("Bot token ✓ saved"))
          .child(this.button(t, "st-r-token-change", "Change", (_e, cx) => { this.changingToken = true; cx.notify(); }))
          .child(div().flex_none().child(pairButton)))
      .when(editing, (el) => el.child(div().py(6).text_size(12.5).text_color(t.dim).child("Get a token from @BotFather on Telegram. It saves by itself once you paste it.")))
      .when(live, (el) => el.child(v_flex().w_full().py(10).gap(6)
        .child(div().text_size(13).text_color(t.muted).child("Send this to your bot in Telegram:"))
        .child(h_flex().gap(12).items_center()
          .child(div().text_size(24).font_semibold().text_color(t.accent).child(command))
          .child(this.button(t, "st-r-pair-copy", "Copy", () => {
            const why = copyText(command);
            notify(why ? { title: "Couldn't copy", body: why, level: "error" } : { title: "Copied", level: "success" });
          })))
        .child(div().text_size(12.5).text_color(t.dim).child(`The code works for ${left} s.`))))
      .when(!live, (el) => el.child(div().py(6).text_size(12.5).text_color(t.dim).child("Press Pair my Telegram for a one-time code, then send it to your bot.")))
      .child(this.row(t, "Bot", on ? "Running" : "Stopped", on
        ? this.remoteButton(t, "st-r-bot-stop", "Stop bot", () => this.coreJson(["kill", "h0x-bridge-telegram"]))
        : this.remoteButton(t, "st-r-bot-start", "Start bot", () => this.coreJson(this.startArgs("h0x-bridge-telegram", "h0x-bridge=telegram", ["bridge", "run"])))))
      .child(this.button(t, "st-r-help", this.showHelp ? "Hide how to use" : "How to use", (_e, cx) => { this.showHelp = !this.showHelp; cx.notify(); }))
      .when(this.showHelp, (el) => el.child(v_flex().w_full().py(8).gap(4)
        .children(commands.map(([name, what]) => h_flex().gap(10)
          .child(div().w(90).flex_none().text_size(12.5).font_semibold().text_color(t.foreground).child(name))
          .child(div().flex_1().min_w(0).text_size(12.5).text_color(t.muted).child(what))))));
  }

  remoteWeb(t) {
    const on = this.termRunning("h0x-bridge-web");
    const choice = (value, label) => h_flex().id(`st-r-reach-${value}`).gap(8).py(4).items_center().cursor_pointer()
      .on_click((_e, cx) => { this.reach = value; cx.notify(); })
      .child(div().text_color(this.reach === value ? t.accent : t.muted).child(this.reach === value ? "◉" : "○"))
      .child(div().text_size(13.5).text_color(t.foreground).child(label));
    const machines = (this.machineList ?? []).map((m) => this.button(t, `st-r-via-${m.name}`, `${this.via === m.name ? "◉ " : "○ "}${m.name}`, (_e, cx) => { this.via = m.name; cx.notify(); }));
    return v_flex().w_full()
      .child(this.heading(t, "Phone / web panel"))
      .child(v_flex().w_full().py(8).gap(2)
        .child(choice("lan", "Same Wi-Fi"))
        .child(choice("tunnel", "From anywhere (Cloudflare)"))
        .child(choice("vps", "Through my VPS")))
      .when(this.reach === "tunnel", (el) => el.child(div().pb(8).text_size(12.5).text_color(t.dim).child("Traffic passes through Cloudflare.")))
      .when(this.reach === "vps", (el) => el.child(v_flex().pb(8).gap(6)
        .child(h_flex().gap(6).children(machines))
        .when(machines.length === 0, (e) => e.child(div().text_size(12.5).text_color(t.dim).child("Add a machine first (Machines page)."))
        )
        .child(Input.new(this.publicHost).w_full().h(32).px(10).rounded(7).border_1().border_color(t.input).bg(t.background).text_size(13).text_color(t.foreground))))
      .child(this.row(t, "Phone / web panel", on ? "Running" : "Stopped", on
        ? this.remoteButton(t, "st-r-web-stop", "Stop", () => this.coreJson(["kill", "h0x-bridge-web"]))
        : !account().current ? this.disabledButton(t, "st-r-web-start", "Start")
        : this.button(t, "st-r-web-start", this.remoteBusy.web ? "…" : "Start", (_e, cx) => this.startWeb(cx))))
      .when(on && !this.webLink, (el) => el.child(div().py(10).text_size(12.5).text_color(t.dim).child("Starting…")))
      .when(on && Boolean(this.webLink), (el) => el.child(v_flex().py(10).gap(8)
        .when(Boolean(this.webQr?.rows), (e) => e.child(this.qrGrid(this.webQr)))
        .when(Boolean(this.webQr) && !this.webQr.rows, (e) => e.child(div().text_size(12.5).text_color(t.dim).child("Cannot draw the QR code; use the link below.")))
        .child(div().text_size(12.5).text_color(t.muted).child("Scan with your phone"))
        .when(typeof this.webLink?.warning === "string" && this.webLink.warning !== "", (e) => e.child(div().id("st-r-warning").text_size(12.5).line_height("18px").text_color(t.ansi3).child(this.webLink.warning)))
        .child(div().id("st-r-link").text_size(12).text_color(t.dim).child(this.webLink?.pair ?? this.webLink?.url ?? ""))));
  }

  remoteActivity(t) {
    const names = Object.fromEntries((this.remote?.identities ?? []).map((i) => [i.id, i.label || i.user_id || i.id]));
    return v_flex().w_full()
      .child(this.heading(t, "Activity"))
      .children(this.remoteLog.map((e) => h_flex().w_full().py(5).gap(12).border_b(1).border_color(t.border)
        .child(div().w(80).flex_none().text_size(12).text_color(t.dim).child(e.time_ms ? new Date(e.time_ms).toLocaleTimeString() : ""))
        .child(div().w(110).flex_none().text_size(12.5).text_color(t.muted).child(names[e.identity] ?? e.identity ?? ""))
        .child(div().flex_1().min_w(0).text_size(12.5).text_color(t.foreground).child([e.command, e.what, e.result].filter(Boolean).join(" · ")))))
      .when(this.remoteLog.length === 0, (el) => el.child(div().py(10).text_size(12.5).text_color(t.dim).child("Nothing yet.")));
  }

  remotePage(t) {
    return v_flex().w_full()
      .child(div().pb(8).text_size(12.5).line_height("18px").text_color(t.dim).child("Control your terminals from your phone, through Telegram or a web panel."))
      .when(this.remote !== null && !account().current, (el) => el.child(h_flex().id("st-r-signin-note").w_full().py(10).gap(12).items_center()
        .child(div().flex_1().text_size(12.5).text_color(t.muted).child("Sign in to pair a phone or Telegram."))
        .child(this.button(t, "st-r-signin", "Sign in", (_e, cx) => this.accountDo(() => accountSignIn(), cx)))))
      .when(Boolean(this.remoteError), (el) => el.child(div().py(10).text_size(12.5).text_color(t.blocked).child(this.remoteError)))
      .when(Boolean(this.remoteProblem), (el) => el.child(div().py(10).text_size(12.5).text_color(t.blocked).child(this.remoteProblem)))
      .when(Boolean(this.remoteNotice), (el) => el.child(div().py(10).text_size(12.5).text_color(t.ansi2).child(this.remoteNotice)))
      .when(this.remote === null && !this.remoteError, (el) => el.child(div().py(14).text_size(12.5).text_color(t.dim).child("Reading remote control…")))
      .when(this.remote !== null, (el) => el
        .child(this.remoteStatus(t))
        .child(this.remoteTelegram(t))
        .child(this.remoteWeb(t))
        .child(this.remoteActivity(t)));
  }

  // Alerts ---------------------------------------------------------------

  alerts(t) {
    const sounds = this.person.sounds ?? {};
    const on = sounds.enabled !== false;
    const file = (f) => (f ? f.split(/[\\/]/).pop() : "HyperOx's sound");
    return v_flex()
      .w_full()
      .child(this.heading(t, "Sounds"))
      .child(
        this.row(
          t,
          "Play sounds",
          "When an agent needs you, and when one finishes while you're not looking. A note shows in the corner either way.",
          this.toggle(t, "st-sounds", on, (_e, cx) => this.change((p) => (p.sounds = { ...(p.sounds ?? {}), enabled: !on }), cx)),
        ),
      )
      .child(this.row(t, "An agent needs you", file(sounds.blocked), this.button(t, "st-play-blocked", "Play", () => playSound("blocked"))))
      .child(this.row(t, "An agent finished", file(sounds.done), this.button(t, "st-play-done", "Play", () => playSound("done"))))
      .child(div().pt(12).text_size(12.5).line_height("18px").text_color(t.dim).child("Want your own sounds? Ask your agent to set sounds.done and sounds.blocked in settings.json to an MP3."));
  }

  // Mods -----------------------------------------------------------------

  // Mods that did not ship with HyperOx, as answered when they asked (ADR-048).
  mods(t) {
    const list = Object.entries(this.person.mods ?? {}).sort(([a], [b]) => a.localeCompare(b));
    return v_flex()
      .w_full()
      .child(this.heading(t, "Mods you were asked about"))
      .child(div().pb(6).text_size(12.5).line_height("18px").text_color(t.dim).child("Mods that did not ship with HyperOx run only once you allow them. They can run programs, read and write files in your home folder and your spaces, and talk to agents. A mod whose code changes asks again."))
      .children(
        list.length === 0
          ? [div().py(12).text_size(13).text_color(t.muted).child("No mod has asked yet.")]
          : list.map(([id, choice]) =>
              this.row(
                t,
                id,
                choice?.allow ? "Allowed" : "Not allowed",
                this.button(t, `st-mod-${id}`, choice?.allow ? "Don't allow" : "Allow", (_e, cx) =>
                  this.change((p) => {
                    p.mods = { ...(p.mods ?? {}), [id]: { ...(p.mods?.[id] ?? {}), allow: !choice?.allow } };
                  }, cx),
                ),
              ),
            ),
      );
  }

  // Updates --------------------------------------------------------------

  updates(t) {
    const u = updates();
    const at = (secs) => {
      const d = new Date(secs * 1000);
      return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
    };
    const checked = u.checked ? ` Checked at ${at(u.checked)}.` : "";
    const result = u.checking ? "Checking\u2026" : u.checked
      ? `Up to date.${checked}`
      : u.tried
        ? "No successful check. Check your connection and try again."
        : "Not checked yet.";
    // Pressed: "Checking\u2026" while it looks, then "Up to date" for a moment
    // (a find changes the row instead). One per row: an element shows once.
    const asked = this.asked && Date.now() - this.asked < 3000;
    const check = (id) =>
      !u.checks
        ? null
        : u.checking && this.asked
          ? div().px(12).py(5).text_size(13).text_color(t.dim).child("Checking\u2026")
          : asked && u.checked && !u.checking
            ? div().px(12).py(5).text_size(13).text_color(t.ansi2).child("\u2713 Up to date")
            : this.button(t, id, "Check now", (_e, cx) => {
                this.asked = Date.now();
                this.follow(cx);
                run("app.check_updates");
                cx.notify();
              });
    const primary = (id, label, f) =>
      div().id(id).px(12).py(5).rounded(7).cursor_pointer().bg(t.accent).text_color(t.accent_foreground).text_size(13).font_semibold().hover((s) => s.opacity(0.9)).on_click(f).child(label);
    const app = !u.app.updates
      ? ["A build from source: it updates with git, not here.", null]
      : u.app.ready
        ? [`${u.app.ready} is ready. Restart to switch now; it also switches when you quit.`, primary("st-app-update", "Restart to update", () => run("app.update"))]
        : [result, check("st-check-app")];
    const c = u.core;
    const coreRow = c.managed
      ? [`Managed by ${c.managed}: update it there.`, null]
      : c.updating
        ? ["Updating\u2026 your terminals keep running.", null]
        : c.error
          ? [`The update failed: ${c.error}`, primary("st-core-retry", "Try again", () => run("app.update_core"))]
          : c.ready
            ? [`${c.ready} is ready. Your terminals keep running through it.`, primary("st-core-update", "Update h0x-core", () => run("app.update_core"))]
            : u.checks
              ? [result, check("st-check-core")]
              : ["Not checked: a build from source looks for no updates.", null];
    const autoOn = this.person.check_updates !== false;
    return v_flex()
      .w_full()
      .child(this.heading(t, "Checking"))
      .child(
        this.row(
          t,
          "Check for updates automatically",
          "Look for HyperOx and h0x-core updates every hour. Off: they are looked for only when you press Check now.",
          this.toggle(t, "st-check-updates", autoOn, (_e, cx) => this.change((p) => { p.check_updates = !autoOn; }, cx)),
        ),
      )
      .child(div().h(18))
      .child(this.heading(t, "HyperOx"))
      .child(this.row(t, `HyperOx ${u.app.version}`, app[0], h_flex().gap(8).flex_none()
        .child(this.button(t, "st-whats-new", "What's new", () => { closeOverlay(props()?.overlay); run("app.whats_new"); }))
        .when(Boolean(app[1]), (el) => el.child(app[1]))))
      .child(div().h(18))
      .child(this.heading(t, "h0x-core"))
      .child(this.row(t, `h0x-core ${c.version ?? ""}`, coreRow[0], coreRow[1] ?? div()))
      .child(div().pt(12).text_size(12.5).line_height("18px").text_color(t.dim).child("HyperOx looks for both every hour (unless automatic checks are off) and tells you in the corner. h0x-core runs your terminals, so they keep running through its updates."));
  }

  // Runs one account change: the menus close, the buttons wait, and a refusal is shown.
  accountDo(work, cx) {
    this.acctMenu = null;
    this.acctConfirm = null;
    this.acctBusy = true;
    const mine = (this.acctRun = (this.acctRun ?? 0) + 1);
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const why = await work();
        // A sign-in that a newer one replaced ends "cancelled": only the newest says so, and only it frees the buttons.
        if (mine !== this.acctRun) return;
        if (why === "cancelled") notify({ title: "Sign-in cancelled", level: "info" });
        else if (why) notify({ title: "Account", body: why, level: "error" });
      } catch (e) {
        if (mine === this.acctRun) notify({ title: "Account", body: e.message ?? String(e), level: "error" });
      }
      if (mine === this.acctRun) this.acctBusy = false;
      cx.notify();
    });
  }

  // The account on the banner's right: who is signed in and what can be done to it.
  accountCard(t, acct) {
    const cur = acct.current;
    const small = (id, label, onClick) => div().id(id).px(10).py(4).rounded(7).border_1().border_color(t.border).bg(t.raised).cursor_pointer()
      .text_size(12.5).text_color(t.foreground).hover((s) => s.bg(t.hover)).on_click(onClick).child(label);
    const switchButton = small("st-acct-switch", "Switch ▾", (_e, cx) => { this.acctMenu = this.acctMenu === "switch" ? null : "switch"; this.acctConfirm = null; cx.notify(); });
    if (this.acctBusy) return div().text_size(12.5).text_color(t.muted).child("Waiting…");
    if (!cur) {
      return v_flex().gap(6).items_end()
        .child(div().text_size(13).text_color(t.muted).child("Not signed in"))
        .child(h_flex().gap(6)
          .when(acct.accounts.length > 0, (el) => el.child(switchButton))
          .child(small("st-acct-signin", "Sign in", (_e, cx) => this.accountDo(() => accountSignIn(), cx))));
    }
    return h_flex().gap(10).items_center()
      .child(this.avatar(t, cur, 38))
      .child(v_flex().gap(2).min_w(0).max_w(260)
        .child(h_flex().gap(6).items_center().min_w(0)
          .child(div().id("st-acct-name").max_w(170).truncate().text_size(14).font_semibold().text_color(t.foreground).child(accountName(cur)))
          .child(this.providerBadge(t, cur)))
        .when(Boolean(cur.email), (el) => el.child(div().id("st-acct-email").max_w(220).truncate().text_size(12).text_color(t.dim).child(cur.email)))
        .child(h_flex().gap(6).pt(3)
          .child(switchButton)
          .child(small("st-acct-signout", "Sign out", (_e, cx) => this.accountDo(() => accountSignOut(cur.uid), cx)))
          .child(small("st-acct-more", "⋯", (_e, cx) => { this.acctMenu = this.acctMenu === "more" ? null : "more"; this.acctConfirm = null; cx.notify(); }))));
  }

  avatar(t, a, size) {
    return div().w(size).h(size).rounded_full().flex_none().flex().items_center().justify_center()
      .bg(Background.linear_gradient(135, t.accent, t.ansi4)).text_size(Math.round(size * 0.42)).font_semibold().text_color(t.accent_foreground)
      .child(accountName(a).slice(0, 1).toUpperCase());
  }

  providerBadge(t, a) {
    return div().px(6).rounded(5).bg(t.hover).text_size(11).text_color(t.muted).child(PROVIDERS[a.provider] ?? a.provider);
  }

  // The Switch and ⋯ menus, and the remove confirmation, open under the banner.
  accountPanel(t, acct) {
    const cur = acct.current;
    const row = (id, onClick, ...kids) => h_flex().id(id).h(30).px(10).gap(8).items_center().rounded(7).cursor_pointer().hover((s) => s.bg(t.hover)).on_click(onClick).children(kids);
    const panel = (...kids) => h_flex().w_full().justify_end().child(v_flex().w(280).p(5).rounded(9).border_1().border_color(t.border).bg(t.popover).text_size(12.8).children(kids));
    if (this.acctMenu === "switch") {
      return panel(
        ...acct.accounts.map((a) => row(`st-acct-pick-${a.uid}`, (_e, cx) => { if (!cur || a.uid !== cur.uid) this.accountDo(() => accountSwitch(a.uid), cx); },
          div().w(14).text_color(t.accent).child(cur && a.uid === cur.uid ? "✓" : ""),
          div().flex_1().min_w(0).text_color(t.foreground).child(accountName(a)),
          this.providerBadge(t, a))),
        row("st-acct-add", (_e, cx) => this.accountDo(() => accountSignIn(), cx), div().w(14).text_color(t.muted).child("+"), div().text_color(t.foreground).child("Add account")),
      );
    }
    if (this.acctMenu === "more" && cur) {
      if (this.acctConfirm === cur.uid) {
        const choice = (id, label, color, onClick) => div().id(id).px(12).py(5).rounded(7).border_1().border_color(color).cursor_pointer().text_size(12.5).text_color(color).hover((s) => s.bg(t.hover)).on_click(onClick).child(label);
        return h_flex().w_full().justify_end().child(v_flex().id("st-acct-confirm").w(340).gap(10).p(12).rounded(9).border_1().border_color(t.border).bg(t.popover)
          .child(div().text_size(13).line_height("19px").text_color(t.foreground).child(`Remove ${accountName(cur)} from this computer? Your local profile data for this account is deleted; your online account stays.`))
          .child(h_flex().gap(8).justify_end()
            .child(choice("st-acct-remove-no", "Cancel", t.muted, (_e, cx) => { this.acctConfirm = null; this.acctMenu = null; cx.notify(); }))
            .child(choice("st-acct-remove-yes", "Remove", t.blocked, (_e, cx) => this.accountDo(() => accountRemove(cur.uid), cx)))));
      }
      return panel(row("st-acct-remove", (_e, cx) => { this.acctConfirm = cur.uid; cx.notify(); }, div().text_color(t.blocked).child("Remove from this computer")));
    }
    return null;
  }

  info(t) {
    const u = updates();
    const acct = account();
    const open = (url) => () => {
      const why = openLink(url);
      if (why) notify({ title: "Can't open the link", body: why, level: "error" });
    };
    const link = (id, url, label) => div().id(id).cursor_pointer().text_color(t.accent).on_click(open(url)).child(`${label} ↗`);
    const card = (title, ...kids) => v_flex().flex_1().min_w(0).gap(8).p(16).rounded(10).border_1().border_color(t.border).bg(t.raised)
      .child(div().text_size(11.5).font_semibold().text_color(t.dim).child(title.toUpperCase()))
      .children(kids);
    const partner = (name, who, id, url, site) => h_flex().w_full().gap(12).items_center()
      .child(v_flex().flex_1().min_w(0).gap(1)
        .child(div().text_size(12.5).text_color(t.dim).child(name))
        .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child(who)))
      .child(div().flex_none().text_size(13).child(link(id, url, site)));
    const panel = this.accountPanel(t, acct);
    return v_flex()
      .w_full()
      .gap(12)
      .child(h_flex().w_full().gap(16).p(18).items_center().rounded(12).border_1().border_color(t.border)
        .bg(Background.linear_gradient(120, t.accent, t.raised).opacity(0.16))
        .child(image("frame/welcome/hyperox.png").w(52).h(52).rounded(10).flex_none())
        .child(v_flex().flex_1().min_w(0).gap(3)
          .child(h_flex().gap(8).items_center()
            .child(div().text_size(24).font_semibold().text_color(t.foreground).child("HyperOx"))
            .child(div().id("st-info-version").px(7).rounded(5).bg(t.hover).text_size(11).text_color(t.muted).child(`v${u.app.version}`)))
          .child(div().text_size(13).text_color(t.muted).child("Agentic development environment")))
        .child(div().flex_none().child(this.accountCard(t, acct))))
      .when(Boolean(panel), (el) => el.child(panel))
      .child(h_flex().w_full().gap(12).items_stretch()
        .child(card("About",
          div().text_size(13.5).font_semibold().text_color(t.foreground).child("The desktop workspace for AI coding agents."),
          div().text_size(13).line_height("20px").text_color(t.muted).child("A secure space to build with AI coding agents. Your code and your conversations stay on your own machine."),
          link("st-info-learn", "https://pavii.tech", "Learn more")))
        .child(card("Resources",
          this.button(t, "st-info-install", "Installation guide", open("https://www.pavii.tech/products/dev-tools/docs/")),
          this.button(t, "st-info-docs", "Documentation", open("https://www.pavii.tech/products/dev-tools/docs/")),
          this.button(t, "st-info-feedback", "Send feedback", () => notify({ title: "The feedback forum is coming soon.", level: "info" })))))
      .child(h_flex().w_full().gap(12).items_stretch()
        .child(card("Partners",
          partner("Built by", "Team Pavii", "st-info-pavii", "https://pavii.tech", "pavii.tech"),
          div().h(1).bg(t.border),
          partner("Security & blockchain partner", "Buckleson", "st-info-buckleson", "https://buckleson.com", "buckleson.com")))
        .child(card("Privacy", this.analytics(t, open))))
      .child(h_flex().w_full().gap(12).pt(6).justify_between().text_size(12).text_color(t.dim)
        .child(div().child("Currently closed source (open source soon)"))
        .child(div().child(`Version ${u.app.version} · Core ${u.core.version || "unknown"} · Open-source notices`)));
  }

  // "Share usage analytics": the switch (settings.json share_analytics), and an "i" that opens what is sent.
  analytics(t, open) {
    const on = this.person.share_analytics !== false;
    const para = (text) => div().w_full().text_size(12.5).line_height("18px").text_color(t.muted).child(text);
    return v_flex()
      .w_full()
      .gap(8)
      .child(h_flex().w_full().gap(10).items_center()
        .child(div().flex_1().text_size(13.5).font_semibold().text_color(t.foreground).child("Share usage analytics"))
        .child(div().id("st-analytics-info").w(18).h(18).rounded(9).flex().items_center().justify_center().border_1().border_color(t.border)
          .text_size(11).font_semibold().text_color(this.analyticsAbout ? t.accent : t.dim).cursor_pointer().hover((s) => s.bg(t.hover))
          .on_click((_e, cx) => { this.analyticsAbout = !this.analyticsAbout; cx.notify(); })
          .child("i"))
        .child(this.toggle(t, "st-analytics", on, (_e, cx) => this.change((p) => { if (on) p.share_analytics = false; else delete p.share_analytics; }, cx))))
      .when(Boolean(this.analyticsAbout), (el) => el.child(v_flex().w_full().gap(8).p(12).rounded(9).border_1().border_color(t.border).bg(t.background)
        .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child("How HyperOx handles your data (beta)"))
        .child(para("During the beta, HyperOx sends usage data and failure reports so we can fix problems and improve the app. You are identified by a random install ID, not your name or email. We never send what you type, your prompts, agent replies, terminal contents, file or folder names, or your keys. IP addresses are not stored; your approximate location (country and city) is."))
        .child(para("When you're signed in, your usage is linked to your HyperOx account ID (not your email)."))
        .child(para("After the beta we plan to move data collection to zero-knowledge proofs."))
        .child(para("You can turn usage analytics off any time. Failure reports stay on so we can fix crashes and update problems."))
        .child(h_flex().child(this.button(t, "st-analytics-more", "Click here", open("https://pavii.tech"))))));
  }

  // ----------------------------------------------------------------------

  render(cx) {
    const t = theme();
    const u = updates();
    const waiting = Boolean(u.app.ready || (u.core.ready && !u.core.managed));
    const nav = v_flex()
      .flex_none()
      .w(190)
      .p(10)
      .gap(2)
      .border_r(1)
      .border_color(t.border)
      .child(div().px(10).pt(6).pb(12).text_size(16).font_semibold().text_color(t.foreground).child("Settings"))
      .children(
        SECTIONS.map((s) => {
          const on = s.id === this.section;
          return h_flex()
            .id(`st-s-${s.id}`)
            .gap(10)
            .px(10)
            .py(7)
            .items_center()
            .rounded(7)
            .cursor_pointer()
            .bg(on ? t.raised : "#00000000")
            .hover((st) => st.bg(t.raised))
            .on_click((_e, cx) => {
              this.section = s.id;
              this.picking = null;
              this.machineEdit = null;
              this.handle.focus();
              if (s.id === "machines") cx.spawn(async (cx) => { await this.loadMachines(); cx.notify(); });
              if (s.id === "remote") cx.spawn(async (cx) => { await this.loadRemote(); cx.notify(); });
              cx.notify();
            })
            .child(svg(I(s.icon)).size(15).text_color(on ? t.accent : t.dim))
            .child(div().flex_1().text_size(13.5).text_color(on ? t.foreground : t.muted).child(s.title))
            .when(s.id === "updates" && waiting, (el) => el.child(div().w(7).h(7).rounded(4).bg(t.accent)));
        }),
      );
    return h_flex()
      .id("settings")
      .track_focus(this.handle)
      .w_full()
      .h(620)
      .min_h(0)
      .items_start()
      .on_key_down((e, cx) => {
        const k = e.key;
        if (this.machineEdit) {
          if (k === "escape") this.machineEdit = null;
          else return;
          this.handle.focus();
          cx.stop_propagation();
          cx.notify();
          return;
        }
        if (k === "escape") {
          if (this.picking) this.picking = null;
          else return this.close();
          this.handle.focus();
        } else return;
        cx.stop_propagation();
        cx.notify();
      })
      .child(nav.h_full().child(div().flex_1()).child(this.button(t, "st-feedback", "Send feedback", () => { closeOverlay(props()?.overlay); run("app.feedback"); })))
      .child(
        // A bar that always shows: the pages are taller than the dialog and
        // people should see that they scroll.
        div()
          .relative()
          .flex_1()
          .min_w(0)
          .h_full()
          .child(
            div()
              .id("st-body")
              .size_full()
              .overflow_y_scroll()
              // Padding inside, not on the scroll area: there it counts as
              // content and a page that fits still shows a bar.
              .child(
                div()
                  .px(28)
                  .py(22)
                  .when(Boolean(this.error), (el) => el.child(div().pb(12).text_size(12.5).text_color(t.blocked).child(this.error)))
                  .child(
                    this.section === "look"
                      ? this.look(t, cx)
                      : this.section === "keyboard"
                        ? this.keyboard(t)
                      : this.section === "machines"
                        ? this.machines(t)
                      : this.section === "remote"
                        ? this.remotePage(t)
                      : this.section === "alerts"
                        ? this.alerts(t)
                      : this.section === "mods"
                        ? this.mods(t)
                        : this.section === "info"
                          ? this.info(t)
                          : this.updates(t),
                  ),
              ),
          )
          .child(Scrollbar.vertical("st-body").mode("always").absolute().inset_0()),
      );
  }
}

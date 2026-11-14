// Workspaces: up to five saved snapshots of a space (its tabs and splits, each agent with its session), opened from
// the tab bar's button as a modal dialog. A click on a row restores it alongside what is open and closes the dialog
// when it is done (it stays open until then: a closed dialog stops its running restore). "Save over" and the bin
// ask a second click. "New…" saves the shown space under a name. The engine is snapshot.js.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { props, closeOverlay, notify, theme } from "h0x";
import { MAX, read, counts, saveNew, saveOver, remove, restore, importOld } from "./snapshot.js";

// "now", "5m", "3h", "2d", "4mo", "1y".
function ago(ms, now) {
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms <= 0) return "";
  const m = Math.floor(Math.max(0, now - ms) / 60000);
  if (m < 1) return "now";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  const d = Math.floor(h / 24);
  if (d < 30) return `${d}d`;
  return d < 365 ? `${Math.floor(d / 30)}mo` : `${Math.floor(d / 365)}y`;
}

const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

export default class Workspaces extends View {
  init(_props, cx) {
    this.busy = null; // "Restoring…" | "Saving…": every button is off meanwhile
    this.confirm = null; // { kind: "over" | "delete", id }: waits for the second click
    this.naming = false;
    this.handle = cx.focus_handle();
    this.handle.focus();
    // Sessions' older saved layouts come over once.
    try {
      const r = importOld();
      if (r.skipped > 0)
        notify({ title: `${plural(r.skipped, "saved layout")} not imported`, body: `Workspaces keeps ${MAX}. Delete one, then ask your agent to copy the rest from shared.json.`, level: "warning" });
    } catch {}
  }

  say(title, body = "", level = "info") {
    notify({ title, body, level });
  }

  // Runs a task that holds the buttons off; `label` shows while it runs. The dialog closes after `close` ones.
  work(label, cx, task, close) {
    if (this.busy) return;
    this.busy = label;
    this.confirm = null;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        await task(cx);
      } catch (e) {
        this.say("Workspaces stopped", String(e?.message ?? e), "error");
      }
      this.busy = null;
      if (close) closeOverlay(props().overlay);
      cx.notify();
    });
  }

  open(item, cx) {
    this.work("Restoring…", cx, async (task) => {
      const r = await restore(task, item);
      if (r.state === "already") this.say("Already open", `“${item.name}” is open in its space.`);
      else if (r.state === "timeout") this.say("Couldn't finish restoring", `“${item.name}” stopped before all its panes started.`, "warning");
    }, true);
  }

  // The second click of "Save over" / the bin.
  ask(kind, id, cx) {
    const item = read().find((x) => x.id === id);
    if (!item || this.busy) return;
    if (this.confirm?.kind !== kind || this.confirm.id !== id) {
      this.confirm = { kind, id };
      return cx.notify();
    }
    this.confirm = null;
    if (kind === "delete") {
      remove(id);
      return cx.notify();
    }
    this.work("Saving…", cx, async () => {
      const r = await saveOver(id);
      if (r === "empty") this.say("Nothing to save", "The shown space has no terminals.", "warning");
      else if (r === "missing") this.say("That workspace is gone", "", "warning");
      else this.say(`Saved “${item.name}”`, "", "success");
    }, false);
  }

  save(name, cx) {
    this.naming = false;
    this.work("Saving…", cx, async () => {
      const r = await saveNew(name);
      if (r === "full") this.say(`${MAX} workspaces are saved`, "Delete one or save over one.", "warning");
      else if (r === "empty") this.say("Nothing to save", "The shown space has no terminals.", "warning");
      else this.say(`Saved “${r.name}”`, "", "success");
    }, false);
  }

  askName() {
    this.naming = true;
    this.confirm = null;
    this.input = InputState.new({ placeholder: "Name" });
    this.input.on("submit", (_e, cx) => this.save(this.input.value(), cx));
    this.input.focus();
  }

  render(cx) {
    const t = theme();
    const items = read();
    const full = items.length >= MAX;
    const now = Date.now();
    const link = (id, label, color, onClick) =>
      div().id(id).flex_none().px(7).h(24).flex().items_center().rounded(6).text_size(12).text_color(color).cursor_pointer()
        .hover((s) => s.bg(t.hover).text_color(t.foreground)).on_mouse_down("left", (_e, cx) => cx.stop_propagation()).on_click((e, cx) => { cx.stop_propagation(); onClick(e, cx); }).child(label);
    const row = (it) => {
      const c = counts(it);
      const over = this.confirm?.kind === "over" && this.confirm.id === it.id;
      const del = this.confirm?.kind === "delete" && this.confirm.id === it.id;
      return h_flex().id(`ws-row-${it.id}`).h(46).px(10).gap(8).items_center().border_b(1).border_color(t.border)
        .when(!this.busy, (el) => el.cursor_pointer().hover((s) => s.bg(t.hover)).on_click((_e, cx) => this.open(it, cx)))
        .child(
          v_flex().flex_1().min_w(0).gap(1)
            .child(div().truncate().text_size(13.5).font_semibold().text_color(t.foreground).child(it.name))
            .child(div().truncate().text_size(11.5).text_color(t.dim).child([ago(it.saved, now), plural(c.tabs, "tab"), plural(c.panes, "pane")].filter(Boolean).join(" · "))),
        )
        .when(!this.busy, (el) => el
          .child(link(`ws-over-${it.id}`, over ? "Replace?" : "Save over", over ? t.accent : t.muted, (_e, cx) => this.ask("over", it.id, cx)))
          .child(link(`ws-del-${it.id}`, del ? "Delete?" : svg("mods/workspace-chrome/x.svg").size(13), del ? t.blocked : t.muted, (_e, cx) => this.ask("delete", it.id, cx))));
    };
    return v_flex()
      .id("workspaces")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (e.key !== "escape") return;
        // While it works, Esc must not close the dialog: that would stop the task.
        cx.stop_propagation();
        if (this.busy) return;
        if (this.naming) this.naming = false;
        else if (this.confirm) this.confirm = null;
        else return closeOverlay(props().overlay);
        this.handle.focus();
        cx.notify();
      })
      // The dialog is as tall as what is in it (it has no height): nothing here may be flex_1 or it collapses.
      .w_full()
      .min_h(0)
      .p(16)
      .gap(10)
      .rounded(8)
      .bg(t.popover)
      .text_size(13)
      .child(h_flex().items_center().child(div().flex_1().text_size(15).font_semibold().text_color(t.foreground).child("Workspaces")).child(div().text_size(11).text_color(t.dim).child(this.busy ?? "Esc")))
      .child(
        v_flex().max_h(300).overflow_y_scrollbar().border_t(1).border_color(t.border)
          .children(items.map(row))
          .when(items.length === 0, (el) => el.child(div().py(14).px(10).text_size(12.5).line_height("18px").text_color(t.muted).child("No workspaces yet. New… saves the shown space: its tabs, splits and each agent's session."))),
      )
      .child(
        h_flex().h(30).gap(10).items_center()
          .when(this.naming && !this.busy, (el) => el.child(Input.new(this.input).flex_1().min_w(0).h(30).px(10).rounded(6).border_1().border_color(t.input).bg(t.background).text_size(12.5).text_color(t.foreground))
            .child(link("ws-name-save", "Save", t.accent, (_e, cx) => this.save(this.input.value(), cx)))
            .child(link("ws-name-cancel", "Cancel", t.muted, (_e, cx) => { this.naming = false; cx.notify(); })))
          .when(!this.naming && !full && !this.busy, (el) => el.child(link("ws-new", "New…", t.foreground, (_e, cx) => { this.askName(); cx.notify(); })))
          .when(!this.naming && full, (el) => el.child(div().id("ws-full").text_size(12).text_color(t.dim).child(`${MAX} saved: delete one or save over one`)))
          .when(!this.naming && this.busy, (el) => el.child(div().id("ws-busy").text_size(12).text_color(t.dim).child(this.busy))),
      );
  }
}

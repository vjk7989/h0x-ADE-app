// Asks once whether a mod that did not ship with HyperOx may run (ADR-048).
// Opened by the app as a dialog with props { id, folder, hash }. The answer
// goes to settings.json `mods.<id>`, for this code (its hash); the app loads
// the mod when it reads an allow. Esc is "Don't allow"; Settings, Mods changes it.

import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { appHome, readTextFile, writeTextFile, props, closeOverlay, theme } from "h0x";

export default class Consent extends View {
  init(_props, cx) {
    this.error = null;
    this.handle = cx.focus_handle();
    this.handle.focus();
  }

  // Read, change, write back only if nobody changed settings.json meanwhile.
  answer(allow, cx) {
    const p = props() ?? {};
    if (!p.id || !p.hash) return closeOverlay(p.overlay);
    const file = `${appHome()}/settings.json`;
    cx.spawn(async (cx) => {
      let failed = null;
      for (let attempt = 0; attempt < 3; attempt++) {
        try {
          const raw = await readTextFile(file);
          const next = raw ? JSON.parse(raw) : {};
          next.mods = { ...(next.mods ?? {}), [p.id]: { hash: p.hash, allow } };
          await writeTextFile(file, JSON.stringify(next, null, 2) + "\n", raw);
          return closeOverlay(p.overlay);
        } catch (e) {
          failed = String(e?.message ?? e);
        }
      }
      this.error = `Could not save your answer to settings.json: ${failed}`;
      cx.notify();
    });
  }

  render(cx) {
    const t = theme();
    const p = props() ?? {};
    const button = (id, label, bg, fg, onClick) =>
      h_flex().id(id).h(32).px(14).items_center().rounded(8).border_1().border_color(id === "consent-no" ? t.input : bg).bg(bg).text_color(fg).cursor_pointer().hover((s) => s.opacity(0.88)).on_click(onClick).child(label);
    return v_flex()
      .id("consent")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (e.key === "escape") this.answer(false, cx);
        else return;
        cx.stop_propagation();
      })
      .size_full()
      .min_h(0)
      .p(18)
      .gap(10)
      .bg(t.popover)
      .text_size(13)
      .child(div().text_size(15).font_semibold().text_color(t.foreground).child(`Allow the mod ${p.id ?? ""} to run?`))
      .child(div().text_size(12).text_color(t.dim).child(p.folder ?? ""))
      .child(
        div()
          .flex_1()
          .min_h(0)
          .text_color(t.muted)
          .child("It did not ship with HyperOx. It can run programs, read and write files in your home folder, and talk to agents. Allow it only if you trust where it came from."),
      )
      .when(Boolean(this.error), (el) => el.child(div().text_size(12).text_color(t.blocked).child(this.error ?? "")))
      .child(
        h_flex()
          .justify_end()
          .gap(8)
          .text_size(12)
          .child(button("consent-no", "Don't allow", t.raised, t.foreground, (_e, cx) => this.answer(false, cx)))
          .child(button("consent-yes", "Allow", t.solid, t.solid_foreground, (_e, cx) => this.answer(true, cx))),
      );
  }
}

// How notifications look. Anything can post one with notify({ title, body,
// level, icon, action, actions }); replace this mod to restyle them all. Click to
// run its action (e.g. go to the agent) and dismiss it; `actions` is a list of
// { label, name, args? } buttons that each run one command and dismiss it.

import { View, div, image } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { props, closeOverlay, run, theme } from "h0x";

const DOT = { error: "blocked", warning: "working", success: "done", info: "accent" };

export default class Toast extends View {
  render(cx) {
    const t = theme();
    const p = props() ?? {};
    const buttons = Array.isArray(p.actions) ? p.actions.filter((a) => a?.label) : [];
    return h_flex()
      .id("toast")
      .size_full()
      .px(18)
      .gap(12)
      .items_center()
      .bg(t.popover)
      .cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .on_click(() => {
        if (p.action?.name) run(p.action.name, p.action.args ?? {});
        closeOverlay(p.overlay);
      })
      .child(p.icon ? image(p.icon).w(18).h(18).flex_none() : div().size(8).rounded(4).flex_none().bg(t[DOT[p.level] ?? "accent"]))
      .child(
        v_flex()
          .flex_1()
          .overflow_hidden()
          .child(div().truncate().font_semibold().text_size(13).text_color(t.foreground).child(p.title ?? ""))
          .when(Boolean(p.body), (el) => el.child(div().truncate().text_size(12).text_color(t.muted).child(p.body ?? ""))),
      )
      .children(
        buttons.map((a, i) =>
          div()
            .id(`toast-action-${i}`)
            .flex_none()
            .px(8)
            .py(3)
            .rounded(4)
            .text_size(12)
            .font_semibold()
            .text_color(t.foreground)
            .cursor_pointer()
            .hover((s) => s.bg(t.hover))
            .on_click((_e, cx) => {
              cx.stop_propagation();
              if (a.name) run(a.name, a.args ?? {});
              closeOverlay(p.overlay);
            })
            .child(a.label),
        ),
      );
  }
}

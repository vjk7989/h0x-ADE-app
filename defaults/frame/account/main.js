// App-owned account menu, opened from the logo (workspace-chrome) as a popup. Only app frames may use the
// account calls, so the menu lives here and the user-editable mod only opens it.

import { View, div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { theme, run, props, closeOverlay, notify, account, accountSignIn, accountSwitch, accountSignOut } from "h0x";

const PROVIDERS = { "google.com": "Google", "github.com": "GitHub", password: "Email" };
const accountName = (a) => a.name || a.email || "Account";

export default class AccountMenu extends View {
  init() {
    this.busy = false;
  }

  // Runs one account change, shows a refusal (a sign-in another window replaced is only "cancelled"), then closes the menu.
  change(work, cx) {
    this.busy = true;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const why = await work();
        if (why === "cancelled") notify({ title: "Sign-in cancelled", level: "info" });
        else if (why) notify({ title: "Account", body: why, level: "error" });
      } catch (e) {
        notify({ title: "Account", body: e.message ?? String(e), level: "error" });
      }
      closeOverlay(props().overlay);
    });
  }

  row(id, label, hint, action) {
    const t = theme();
    return h_flex().id(id).h(31).px(9).gap(8).rounded(7).items_center().flex_none().text_color(t.foreground).hover((s) => s.bg(t.selected))
      .on_click(action).child(div().flex_1().min_w(0).truncate().child(label)).child(div().text_size(11).text_color(t.dim).child(hint));
  }

  render() {
    const t = theme();
    const { current, accounts } = account();
    const frame = v_flex().id("account-menu").w_full().bg(t.popover).p(5).text_size(12.8);
    if (this.busy) return frame.child(div().px(9).py(8).text_color(t.muted).child("Waiting\u2026"));
    return frame
      .child(v_flex().id("account-current").px(9).py(6).gap(1)
        .child(div().min_w(0).truncate().font_semibold().text_color(t.foreground).child(current ? accountName(current) : "Not signed in"))
        .when(Boolean(current), (el) => el.child(div().text_size(11).text_color(t.dim).child(PROVIDERS[current.provider] ?? current.provider))))
      .children(accounts.filter((a) => a.uid !== current?.uid).map((a) => this.row(`account-switch-${a.uid}`, accountName(a), PROVIDERS[a.provider] ?? a.provider, (_e, cx) => this.change(() => accountSwitch(a.uid), cx))))
      .child(div().h(1).mx(4).my(4).bg(t.border))
      .child(this.row("account-add", "+ Add account", "", (_e, cx) => this.change(() => accountSignIn(), cx)))
      .child(this.row("account-settings", "Account settings", "", () => { closeOverlay(props().overlay); run("app.settings", { section: "info" }); }))
      .when(Boolean(current), (el) => el.child(this.row("account-signout", "Sign out", "", (_e, cx) => this.change(() => accountSignOut(current.uid), cx))));
  }
}

import { View, div, image, svg } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { theme, run, props, window, shared, setShared, updates, layout, account, accountSignIn, accountSwitch, accountSignOut, openOverlay, closeOverlay, notify } from "h0x";

function find(node, id) {
  if (node?.id === id) return node;
  for (const child of node?.children ?? []) {
    const found = find(child, id);
    if (found) return found;
  }
  return null;
}

// The sidebar's rail goes by its width, not a flag: dragging its edge narrows it too.
const narrow = () => (find(layout(), "sidebar")?.size?.px ?? Infinity) < 160;

// To the rail (on macOS wide enough for the traffic lights), or back to the width it had.
function toggleSidebar() {
  const width = find(layout(), "sidebar")?.size?.px;
  if (!narrow()) setShared("sidebar.width", width);
  // Mods edited before the rail went by width still read the flag.
  setShared("sidebar.collapsed", !narrow());
  run("layout.resize", { id: "sidebar", px: narrow() ? (shared("sidebar.width") ?? 268) : window()?.platform === "macos" ? 76 : 44 });
}

const PROVIDERS = { "google.com": "Google", "github.com": "GitHub", password: "Email" };
const accountName = (a) => a.name || a.email || "Account";

// The logo menu: an overlay of this same mod (props.menu), so it may use the account calls.
function openAccountMenu() {
  const { current, accounts } = account();
  const others = accounts.filter((a) => a.uid !== current?.uid).length;
  openOverlay({ mod: "workspace-chrome", kind: "popup", width: 250, height: 50 + others * 31 + (current ? 3 : 2) * 31 + 16, props: { menu: "account" } });
}

export default class WorkspaceChrome extends View {
  // Closes the menu, then runs one account change and shows a refusal.
  async change(work) {
    closeOverlay(props().overlay);
    try {
      const why = await work();
      if (why) notify({ title: "Account", body: why, level: "error" });
    } catch (e) {
      notify({ title: "Account", body: e.message ?? String(e), level: "error" });
    }
  }
  menuRow(id, label, hint, action) {
    const t = theme();
    return h_flex().id(id).h(31).px(9).gap(8).rounded(7).items_center().flex_none().text_color(t.foreground).hover(s => s.bg(t.selected))
      .on_click(action).child(div().flex_1().min_w(0).child(label)).child(div().text_size(11).text_color(t.dim).child(hint));
  }
  accountMenu() {
    const t = theme();
    const { current, accounts } = account();
    return v_flex().id("account-menu").w_full().bg(t.popover).p(5).text_size(12.8)
      .child(v_flex().id("account-current").px(9).py(6).gap(1)
        .child(div().font_semibold().text_color(t.foreground).child(current ? accountName(current) : "Not signed in"))
        .when(Boolean(current), el => el.child(div().text_size(11).text_color(t.dim).child(PROVIDERS[current.provider] ?? current.provider))))
      .children(accounts.filter((a) => a.uid !== current?.uid).map((a) => this.menuRow(`account-switch-${a.uid}`, accountName(a), PROVIDERS[a.provider] ?? a.provider, () => this.change(() => accountSwitch(a.uid)))))
      .child(div().h(1).mx(4).my(4).bg(t.border))
      .child(this.menuRow("account-add", "+ Add account", "", () => this.change(() => accountSignIn())))
      .child(this.menuRow("account-settings", "Account settings", "", () => { closeOverlay(props().overlay); run("app.settings", { section: "info" }); }))
      .when(Boolean(current), el => el.child(this.menuRow("account-signout", "Sign out", "", () => this.change(() => accountSignOut(current.uid)))));
  }
  button(id, label, action) {
    const t = theme();
    return div().id(id).size(26).flex_none().flex().items_center().justify_center().rounded(6).text_color(t.muted).hover(s => s.bg(t.hover).text_color(t.foreground))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation()).on_click(action).child(label);
  }
  render() {
    if (props()?.menu === "account") return this.accountMenu();
    const t = theme();
    const collapsed = narrow();
    if (props()?.footer) {
      const row = (id, icon, label, dim, action) => h_flex().id(id).h(28).px(8).gap(9).items_center().rounded(8)
        .when(collapsed, el => el.relative().w(32).px(0).justify_center().tooltip(label))
        .text_color(dim ? t.dim : t.muted).hover(s => s.bg(t.hover).text_color(t.foreground)).cursor_pointer()
        .on_click(action).child(svg(`mods/workspace-chrome/${icon}.svg`).size(14))
        .when(!collapsed, el => el.child(label));
      const ready = div().size(6).rounded_full().bg(t.accent);
      return v_flex().size_full().bg(t.chrome).border_t(1).border_color(t.border).px(8).pt(4).pb(8).text_size(13)
        .when(collapsed, el => el.px(0).items_center())
        .child(row("settings", "settings", "Settings", false, () => run("app.settings"))
          .when(Boolean(updates().app.ready), el => el.child(collapsed ? ready.absolute().top(5).right(6) : ready)))
        .child(row("machine", "plus", "Add machine", true, () => run("mod.open", { name: "machine-add" })));
    }
    return h_flex().size_full().bg(t.chrome).pl(collapsed ? 6 : 14).pr(10).gap(4).items_center().text_size(13.5)
      .when(window()?.platform === "macos" && !window()?.fullscreen && !collapsed, el => el.pl(76))
      .when(!collapsed, el => el.child(h_flex().id("brand").flex_1().gap(8).items_center().cursor_pointer().on_mouse_down("left", (_e, cx) => cx.stop_propagation()).on_click(openAccountMenu).child(svg("mods/workspace-chrome/hyperox-mark.svg").size(20).text_color(t.foreground)).child(div().font_semibold().text_color(t.foreground).child("HyperOx"))
        .child(div().id("brand-beta").flex_none().px(5).rounded(4).border(1).border_color(t.border).text_size(10).font_medium().text_color(t.muted).child("Beta"))))
      .when(!collapsed, el => el.child(this.button("search", svg("mods/workspace-chrome/search.svg").size(15), () => run("mod.open", { name: "picker" }))))
      // On the macOS rail the traffic lights fill the row; the agents list's » unfolds it.
      .when(!(collapsed && window()?.platform === "macos"), el => el.child(this.button("sidebar-toggle", svg("mods/workspace-chrome/sidebar.svg").size(15), toggleSidebar)));
  }
}

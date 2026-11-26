// HyperOx Worktrees dialog: { space, action: "new" | "open" | "merge" | "remove" } or
// { folder, machine } for a repo without a space; optional { branch, tab, argv }.

import { View, div } from "gpui-kit";
import { v_flex, h_flex, InputState, Input, Scrollbar } from "gpui-base";
import { run, props, closeOverlay, theme, spaces, panes, core, home, window } from "h0x";

// Where a space is now: its first terminal's live folder, else where it was made.
function place(s, all = panes()) {
  if (!s) return null;
  const tabs = new Set((s.tabs ?? []).map((c) => c.id));
  return all.find((p) => tabs.has(p.tab) && p.cwd)?.cwd || s.folder || null;
}

// Home as ~ (git says C:/Users/x where Windows says C:\Users\x). A remote home is unknown: paths stay whole.
const tilde = (path, userHome) => {
  if (!userHome) return path;
  const [p, h] = [path, userHome].map((s) => s.replace(/\\/g, "/"));
  return p.startsWith(h) ? `~${p.slice(h.length)}` : path;
};
const ago = (s) => {
  const d = Date.now() / 1000 - s;
  return d < 3600 ? `${Math.max(1, Math.round(d / 60))}m` : d < 86400 ? `${Math.round(d / 3600)}h` : d < 86400 * 60 ? `${Math.round(d / 86400)}d` : `${Math.round(d / 86400 / 30)}mo`;
};

// One `h0x-core worktree <verb>`: its JSON, or { error: { kind, message } }.
// An older core without `worktree` answers kind "usage".
async function ask(on, verb, flags = [], rest = []) {
  const args = ["worktree", verb, "--json", ...(on ? (on.startsWith("-") ? [`--on=${on}`] : ["--on", on]) : []), ...flags, ...(rest.length ? ["--", ...rest] : [])];
  let r;
  try {
    r = await core(args);
  } catch (e) {
    return { error: { kind: "unavailable", message: String(e?.message ?? e) } };
  }
  if (r.code === 0) {
    try {
      const reply = JSON.parse(r.stdout);
      if (reply === null || typeof reply !== "object") throw new Error("not an object");
      return reply;
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

// What a failed call says to the person.
function fail(error, on) {
  if (error.kind === "usage" || error.kind === "missing_tool") return `Update h0x-core on ${on ?? "this computer"} to use Worktrees`;
  if (error.kind === "conflict") return `Can't merge: ${error.message}. Nothing was changed.`;
  return error.message || "h0x-core failed.";
}

// The repo of a listing (`worktree list`, main checkout first) seen from a folder: its main
// checkout and name, and the checkout the folder is in (`top`; the main one when it is in
// none) with whether that is a linked worktree. null for an empty listing.
function repoOf(worktrees, folder) {
  const main = worktrees.find((w) => w.main) ?? worktrees[0];
  if (!main) return null;
  const at = folder.replace(/\\/g, "/").replace(/\/+$/, "");
  const here = worktrees.filter((w) => at === w.path || at.startsWith(w.path + "/")).sort((a, b) => b.path.length - a.path.length)[0] ?? main;
  // A bare repo hidden beside its worktrees (<project>/.bare) is named after its project.
  const parts = main.path.split("/");
  const name = parts.at(-1) === ".bare" ? parts.at(-2) : parts.at(-1);
  return { top: here.path, main: main.path, name, linked: !here.main };
}

// The space showing a checkout: its first terminal is in it (the deepest checkout wins).
function spaceOf(path, all, on, spaceList, paneList) {
  const inside = (p, dir) => p === dir || p.startsWith(dir + "/");
  return spaceList.find((s) => (s.machine ?? null) === (on ?? null) && (() => {
    const at = place(s, paneList)?.replace(/\\/g, "/");
    if (!at || !inside(at, path)) return false;
    return !all.some((c) => c.path !== path && c.path.length > path.length && inside(at, c.path));
  })());
}

// A checkout's state beside the default branch: "dirty · ↑2 ↓1 · merged".
const status = (c) => [c.dirty ? "dirty" : "", c.ahead || c.behind ? `\u2191${c.ahead} \u2193${c.behind}` : "", c.integrated && !c.main ? "merged" : ""].filter(Boolean).join(" \u00b7 ");

export default class Worktree extends View {
  init(_props, cx) {
    this.on = this.space()?.machine;
    this.userHome = this.on ? null : home();
    this.opening = props()?.action === "open";
    this.text = props()?.branch ?? "";
    this.list = undefined; // the checkouts, once read
    this.defaultBranch = null;
    this.merging = null; // the checkout the Merge confirmation is about
    this.squash = true;
    this.at = 0;
    this.input = InputState.new({ value: this.text, placeholder: this.opening ? "filter worktrees" : "branch name" });
    this.input.on("change", (_e, cx) => { this.text = this.input.value(); this.error = ""; this.at = 0; cx.notify(); });
    this.input.on("submit", (_e, cx) => this.go(cx));
    this.inputFocused = true;
    this.input.on("focus", (_e, cx) => { this.inputFocused = true; cx.notify(); });
    this.input.on("blur", (_e, cx) => { this.inputFocused = false; cx.notify(); });
    this.handle = cx.focus_handle();
    this.handle.focus(); // the root is mounted from the first frame: keep the keyboard here while loading
    const typing = !["remove", "merge"].includes(props()?.action);
    if (typing) this.input.focus();
    this.busy = false;
    this.error = "";
    this.repo = undefined; // undefined: looking, null: not in git
    this.dirty = false;
    this.unpushed = [];
    cx.spawn(async (cx) => {
      try {
        this.folder = props()?.folder || place(this.space()) || (this.on ? "." : this.userHome);
        // The repo is set last: Enter does nothing until the warnings it consents to are known.
        const listing = await ask(this.on, "list", [], [this.folder]);
        let repo = null;
        if (listing.error) {
          // Not in git is the answer, not a failure.
          if (listing.error.kind !== "not_git") this.error = fail(listing.error, this.on);
        } else {
          repo = repoOf(listing.worktrees ?? [], this.folder);
          this.list = listing.worktrees ?? [];
          this.defaultBranch = listing.default_branch ?? null;
          const here = this.list.find((c) => c.path === repo?.top);
          if (repo && props().action === "remove") this.dirty = Boolean(here?.dirty);
          if (repo?.linked && props().action === "merge") this.merging = here;
        }
        this.repo = repo;
        cx.notify();
        // The list (and with it the filter input) only mounts once loading is
        // done, so the keyboard must move there now — a focus call made while
        // loading lands on nothing and the dialog stays keyboard-dead.
        this.handle.focus();
        if (typing) cx.timer.after(100, () => this.input.focus());
        if (props().branch) await this.work(cx);
      } catch (error) { this.repo = null; this.error = String(error); cx.notify(); }
    });
  }

  space() {
    const p = props() ?? {};
    return spaces().find((s) => s.id === p.space) ?? (p.folder ? { folder: p.folder, machine: p.machine ?? null, name: "" } : undefined);
  }

  close() {
    closeOverlay(props().overlay);
  }

  go(cx) {
    if (this.busy || !this.repo) return;
    cx.spawn(async (cx) => this.work(cx));
  }

  async work(cx) {
    const action = props()?.action;
    if (this.busy || !this.repo || (action === "remove" && !this.repo.linked) || (action === "merge" && !this.merging)) return;
    this.busy = true;
    this.error = "";
    cx.notify();
    try { this.error = this.merging ? await this.merge() : action === "remove" ? await this.remove() : await this.create(); }
    catch (error) { this.error = String(error); }
    this.busy = false;
    if (!this.error) this.close();
    cx.notify();
  }

  async create() {
    const branch = this.text.trim();
    if (!branch) return "Type a branch name.";
    const r = await ask(this.on, "create", [`--folder=${this.repo.top}`], [branch]);
    if (r.error) return fail(r.error, this.on);
    if (typeof r.path !== "string" || !r.path) return "h0x-core did not say where the worktree is";
    const p = props();
    // Leave spaces unnamed so they follow the checkout folder and nest under the repo.
    run("space.new", { folder: r.path, machine: this.on ?? "", tab: p.tab, argv: p.argv });
    return "";
  }

  shown() {
    const f = this.text.trim().toLowerCase();
    return (this.list ?? []).filter((c) => !f || `${c.branch ?? ""} ${c.path}`.toLowerCase().includes(f));
  }

  open(c) {
    if (!c) return;
    this.close();
    const s = spaceOf(c.path, this.list, this.on, spaces(), panes());
    if (s) run("space.select", { id: s.id });
    else run("space.new", { folder: c.path, machine: this.on ?? "" });
  }

  // Ask before merging a checkout (the main one has nothing to merge).
  askMerge(c) {
    if (!c || c.main || !c.branch) return;
    this.merging = c;
    this.squash = true;
    this.error = "";
    this.handle?.focus();
  }

  // Merge the checkout into the default branch, then close the space showing it as `remove` does.
  // A conflict changes nothing (core puts everything back), so the space stays.
  async merge() {
    const c = this.merging;
    if (c.dirty) return "It has uncommitted changes. Commit or stash them first.";
    const r = await ask(this.on, "merge", this.squash ? [] : ["--no-squash"], [c.path]);
    if (r.error) return fail(r.error, this.on);
    const s = spaceOf(c.path, this.list, this.on, spaces(), panes());
    if (s?.id) run("space.close", { id: s.id, confirmed: true });
    return "";
  }

  async remove() {
    const s = this.space();
    // The popup already showed what's at risk (changes it listed); the core refuses a risk that
    // appeared since, and one more Enter removes it anyway.
    const force = this.dirty || this.unpushed.length > 0;
    // Git on a remote Windows machine reports a drive-letter or UNC path.
    const windows = this.on ? /^(?:[A-Za-z]:\/|\/\/)/.test(this.repo.top) : window().platform === "windows";
    const closeSpace = () => { if (s?.id) run("space.close", { id: s.id, confirmed: true }); };
    // A shell sitting in the folder can keep it busy on Windows.
    if (windows) closeSpace();
    const r = await ask(this.on, "remove", force ? ["--force"] : [], [this.repo.top]);
    if (r.error) {
      if (r.error.kind === "dirty") {
        this.dirty = true;
        return "It changed since this opened. Enter again removes it anyway.";
      }
      if (r.error.kind === "unpushed") {
        this.unpushed = r.error.message.split(": ").slice(1).join(": ").split(", ").filter(Boolean);
        return `Submodule ${this.unpushed.join(", ")} has commits that aren't pushed anywhere. Enter again removes it anyway.`;
      }
      return fail(r.error, this.on);
    }
    if (!windows) closeSpace();
    return "";
  }

  key(e, cx) {
    if (this.merging) {
      if (e.key === "escape") {
        // From the list, back to it; from the space's menu, out.
        if (!this.opening || this.busy) return this.close();
        this.merging = null;
        this.error = "";
      } else if (e.key === "enter") return this.go(cx);
      else if (e.key === "tab") this.squash = !this.squash;
      else return;
      cx.stop_propagation();
      return cx.notify();
    }
    if (e.key === "escape") return this.close();
    if (this.opening) {
      const n = this.shown().length;
      if (e.key === "enter") this.open(this.shown()[this.at]);
      else if ((e.key === "down" || e.key === "up") && n) this.at = (this.at + (e.key === "down" ? 1 : n - 1)) % n;
      else if (e.key === "m" && e.modifiers?.control) this.askMerge(this.shown()[this.at]);
      else return;
      cx.stop_propagation();
      return cx.notify();
    }
    if (e.key === "enter" && props()?.action === "remove") return this.go(cx);
  }

  renderOpen(t) {
    const r = this.repo;
    const rows = this.shown();
    const all = this.list ?? [];
    const spaceList = spaces();
    const paneList = panes();
    const row = (c, i) => {
      const on = i === this.at;
      const there = spaceOf(c.path, all, this.on, spaceList, paneList);
      const state = status(c);
      return v_flex()
        .id(`wt-${i}`)
        .flex_none()
        .h(44)
        .px(10)
        .justify_center()
        .rounded(7)
        .cursor_pointer()
        .when(on, (el) => el.bg(t.selected))
        .when(!on, (el) => el.hover((s) => s.bg(t.hover)))
        .on_click(() => this.open(c))
        .child(
          h_flex()
            .gap(8)
            .items_center()
            .child(div().flex_1().min_w(0).truncate().text_color(t.foreground).child(c.branch ?? "detached"))
            .when(c.main, (el) => el.child(div().flex_none().text_size(11).text_color(t.dim).child("main checkout")))
            .when(Boolean(state), (el) => el.child(div().flex_none().text_size(11).text_color(t.dim).child(state)))
            .when(Boolean(there), (el) => el.child(div().flex_none().text_size(11).text_color(t.accent).child("open")))
            .when(!there && c.at > 0, (el) => el.child(div().flex_none().text_size(11).text_color(t.dim).child(ago(c.at))))
            .when(!c.main && Boolean(c.branch), (el) =>
              el.child(
                div()
                  .id(`wt-merge-${i}`)
                  .flex_none()
                  .px(6)
                  .rounded(4)
                  .text_size(11)
                  .text_color(t.accent)
                  .hover((s) => s.bg(t.hover))
                  .on_click((_e, cx) => { cx?.stop_propagation?.(); this.askMerge(c); cx?.notify?.(); })
                  .child("Merge"),
              ),
            ),
        )
        .child(div().truncate().text_size(11.5).text_color(t.dim).child(tilde(c.path, this.userHome)));
    };
    const count = rows.length === all.length ? `${all.length} checkouts` : `${rows.length}/${all.length} checkouts`;
    return v_flex()
      .id("worktree")
      .track_focus(this.handle)
      .on_key_down((e, cx) => this.key(e, cx))
      .size_full()
      .p(16)
      .gap(10)
      .bg(t.popover)
      .text_size(13)
      .child(h_flex().child(div().flex_1().font_semibold().text_color(t.foreground).child(`HyperOx Worktrees${r ? ` \u00b7 ${r.name}` : ""}`)).child(div().text_size(11).text_color(t.dim).child(r ? count : "")))
      .when(r === undefined || (r && this.list === undefined), (el) => el.child(div().text_color(t.muted).child("\u2026")))
      .when(r === null, (el) => el.child(div().text_color(t.muted).child(`${this.folder ? tilde(this.folder, this.userHome) : "This folder"} is not in a git repository.`)))
      .when(Boolean(this.error), (el) => el.child(div().text_color(t.blocked).child(this.error)))
      .when(Boolean(r && this.list), (el) =>
        el
          .child(Input.new(this.input).h(32).px(10).items_center().rounded(6).border(1).border_color(this.inputFocused ? t.accent : t.input).bg(t.background).text_color(t.foreground))
          .child(
            div()
              .relative()
              .flex_1()
              .min_h(0)
              .child(
                v_flex()
                  .id("wt-list")
                  .size_full()
                  .overflow_y_scroll()
                  .children(rows.map(row))
                  .when(rows.length === 0, (el) => el.child(div().p(10).text_color(t.dim).child("No matching worktrees."))),
              )
              .child(Scrollbar.vertical("wt-list").absolute().inset_0()),
          ),
      )
      .child(h_flex().justify_end().text_size(11).text_color(t.dim).child("\u2191\u2193 choose \u00b7 \u21b5 open \u00b7 ^m merge \u00b7 esc close"));
  }

  renderMerge(t) {
    const c = this.merging;
    const into = this.defaultBranch ?? "the default branch";
    const n = c.ahead === 1 ? "1 commit" : `${c.ahead} commits`;
    const how = this.squash ? `Squashes its ${n} into one on ${into}` : `Rebases its ${n} onto ${into} and fast-forwards`;
    return v_flex()
      .id("worktree")
      .track_focus(this.handle)
      .on_key_down((e, cx) => this.key(e, cx))
      .size_full()
      .p(16)
      .gap(10)
      .bg(t.popover)
      .text_size(13)
      .child(div().text_size(11).text_color(t.dim).child("HyperOx Worktrees"))
      .child(div().font_semibold().text_color(t.foreground).child(`Merge ${c.branch} into ${into}?`))
      .child(div().text_color(t.muted).child(`${how}, then removes the worktree ${tilde(c.path, this.userHome)} and the branch.`))
      .when(c.dirty, (el) => el.child(div().text_color(t.blocked).child("It has uncommitted changes. Commit or stash them first.")))
      .when(Boolean(this.error), (el) => el.child(div().text_color(t.blocked).child(this.error)))
      .child(div().flex_1())
      .child(
        h_flex()
          .justify_end()
          .text_size(11)
          .text_color(t.dim)
          .gap(6)
          .when(this.busy, (el) => el.child(div().frames(["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"], 90)))
          .child(this.busy ? "working…" : `\u21b5 merge \u00b7 tab ${this.squash ? "rebase instead" : "squash instead"} \u00b7 esc ${this.opening ? "back" : "cancel"}`),
      );
  }

  render(cx) {
    const t = theme();
    if (this.merging) return this.renderMerge(t);
    if (this.opening) return this.renderOpen(t);
    const action = props()?.action;
    const remove = action === "remove";
    const merge = action === "merge";
    const s = this.space();
    const r = this.repo;
    const title = remove ? `Remove worktree ${s?.name ?? ""}?` : merge ? `Merge worktree ${s?.name ?? ""}` : `New worktree${r ? ` of ${r.name}` : ""}`;
    const detail =
      r === undefined
        ? "…"
        : r === null
          ? `${this.folder ? tilde(this.folder, this.userHome) : "This folder"} is not in a git repository.`
          : remove
            ? !r.linked
              ? "This is the repository's main checkout, not a worktree."
              : `Closes the space and deletes ${tilde(r.top, this.userHome)}. The branch stays unless its work is in ${this.defaultBranch ?? "the default branch"}.`
            : merge
              ? "This is the repository's main checkout, not a worktree."
              : `A new space on a checkout of ${r.name}. A branch that exists is checked out; a new one starts from ${this.defaultBranch ?? "the default branch"}.`;
    const blocked = !r || ((remove || merge) && !r.linked);
    return v_flex()
      .id("worktree")
      .track_focus(this.handle)
      .on_key_down((e, cx) => this.key(e, cx))
      .size_full()
      .p(16)
      .gap(10)
      .bg(t.popover)
      .text_size(13)
      .child(div().text_size(11).text_color(t.dim).child("HyperOx Worktrees"))
      .child(div().font_semibold().text_color(t.foreground).child(title))
      .child(div().text_color(t.muted).child(detail))
      .when(remove && this.dirty, (el) => el.child(div().text_color(t.blocked).child("It has uncommitted changes: they will be lost.")))
      .when(remove && this.unpushed.length > 0, (el) => el.child(div().text_color(t.blocked).child(`Submodule ${this.unpushed.join(", ")} has commits that aren't pushed anywhere: they will be lost.`)))
      .when(!remove && !merge && !blocked, (el) =>
        el.child(
          Input.new(this.input)
            .h(32)
            .px(10)
            .items_center()
            .rounded(6)
            .border(1)
            .border_color(this.inputFocused ? t.accent : t.input)
            .bg(t.background)
            .text_color(t.foreground),
        ),
      )
      .when(Boolean(this.error), (el) => el.child(div().text_color(t.blocked).child(this.error)))
      .child(div().flex_1())
      .child(
        h_flex()
          .justify_end()
          .text_size(11)
          .text_color(t.dim)
          .gap(6)
          .when(this.busy, (el) => el.child(div().frames(["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"], 90)))
          .child(this.busy ? "working…" : blocked ? "esc close" : remove ? "↵ remove · esc cancel" : "↵ create · esc cancel"),
      );
  }
}

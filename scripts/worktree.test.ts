import { test, expect } from "bun:test";
import { readFile } from "node:fs/promises";
import { readFileSync } from "node:fs";

const source = (await readFile(new URL("../defaults/mods/worktree/main.js", import.meta.url), "utf8"))
  .replace(/^import .*;\n/gm, "").replace("export default class Worktree", "class Worktree");

// Execute conditional builders and retain text, without native GPUI layout.
function element() {
  const node = { id: "", children: [] as any[] };
  const el: any = new Proxy(node, { get: (_, method) => {
    if (method === "node") return node;
    return (...args: any[]) => {
      if (method === "id") node.id = args[0];
      if (method === "child") node.children.push(args[0]?.node ?? args[0]);
      if (method === "children") node.children.push(...args[0].map((child: any) => child?.node ?? child));
      if (method === "when" && args[0]) args[1](el);
      return el;
    };
  } });
  return el;
}
function textOf(node: any): string {
  return typeof node === "string" ? node : (node?.children ?? []).map(textOf).join(" ");
}
function rowsOf(node: any): any[] {
  return node?.id?.match(/^wt-\d+$/) ? [node] : (node?.children ?? []).flatMap(rowsOf);
}

const json = (value: unknown) => ({ code: 0, stdout: JSON.stringify(value), stderr: "" });
const refuse = (kind: string, message: string) => ({ code: 1, stdout: "", stderr: JSON.stringify({ error: { kind, message } }) });
const wt = (path: string, branch: string | null, extra: Record<string, unknown> = {}) =>
  ({ path, branch, main: false, dirty: false, ahead: 0, behind: 0, integrated: null, at: 0, ...extra });
const listing = (...rest: any[]) => ({ default_branch: "main", worktrees: [wt("/repos/main", "main", { main: true }), ...rest] });

function harness(focusedMachine: string | null = null, overrides: Record<string, unknown> = {}) {
  let spaceList: any[] = [];
  let paneList: any[] = [];
  const reads = { spaces: 0, panes: 0 };
  const writes: unknown[] = [];
  const destinations: (string | null)[] = [];
  const globals = {
    View: class {}, div: element, v_flex: element, h_flex: element,
    Input: { new: element }, Scrollbar: { vertical: element }, theme: () => ({}),
    spaces: () => { reads.spaces++; return structuredClone(spaceList); },
    panes: () => { reads.panes++; return structuredClone(paneList); },
    props: () => ({ overlay: "overlay" }), closeOverlay: () => {},
    run: (command: string, args: any) => {
      writes.push([command, args]);
      if (command === "space.new") destinations.push(args.machine === "" ? null : args.machine ?? focusedMachine);
    },
    core: async () => json({}),
    ...overrides,
  };
  const Worktree = new Function(...Object.keys(globals), `${source}; return Worktree;`)(...Object.values(globals));
  const worktree = new Worktree();
  Object.assign(worktree, { opening: true, text: "", at: 0, repo: { name: "repo" }, userHome: "/home/user" });
  return { worktree, reads, writes, destinations, snapshot: (spaces: any[], panes: any[]) => { spaceList = spaces; paneList = panes; } };
}

// A harness whose core answers with `answer` and records the argv it was called with.
function withCore(answer: (args: string[]) => unknown, on: string | null = null) {
  const calls: string[][] = [];
  const h = harness(null, { core: async (args: string[]) => { calls.push(args); return answer(args); } });
  Object.assign(h.worktree, { on });
  return { ...h, calls };
}

function pick(name: string) {
  const globals = { View: class {}, div: element, v_flex: element, h_flex: element, Input: { new: element }, Scrollbar: { vertical: element }, theme: () => ({}) };
  return new Function(...Object.keys(globals), `${source}; return ${name};`)(...Object.values(globals));
}

test("open render reads spaces and panes once and refreshes both on the next render", () => {
  const { worktree, reads, snapshot } = harness();
  worktree.list = Array.from({ length: 80 }, (_, i) => ({ path: `/repo/wt-${i}`, branch: `branch-${i}`, at: 0 }));
  const spaces = worktree.list.map((c: any, i: number) => ({ id: `s${i}`, tabs: [{ id: `tab${i}` }] }));
  const panes = worktree.list.map((c: any, i: number) => ({ tab: `tab${i}`, cwd: `${c.path}/src` }));
  snapshot(spaces, panes);
  const first = rowsOf(worktree.render({}).node);
  expect(reads).toEqual({ spaces: 1, panes: 1 });
  expect(first).toHaveLength(80);
  for (let i = 0; i < first.length; i++) {
    expect(textOf(first[i])).toContain(`branch-${i}`);
    expect(textOf(first[i])).toContain(" open ");
  }
  snapshot(spaces.slice(0, -1), panes.map((p: any, i: number) => i === 0 ? { ...p, cwd: "/elsewhere" } : p));
  const second = rowsOf(worktree.render({}).node);
  expect(reads).toEqual({ spaces: 2, panes: 2 });
  expect(textOf(second[0])).not.toContain(" open ");
  expect(textOf(second[79])).not.toContain(" open ");
  expect(textOf(second[1])).toContain(" open ");
});

test("open clicks read fresh snapshots once and select an existing space or create one", () => {
  const { worktree, reads, writes, snapshot } = harness();
  const checkout = { path: "/repo/worktree", branch: "branch", at: 0 };
  worktree.list = [checkout];
  snapshot([], []);
  worktree.render({});
  snapshot([{ id: "existing", tabs: [{ id: "tab" }] }], [{ tab: "tab", cwd: checkout.path }]);
  worktree.open(checkout);
  expect(reads).toEqual({ spaces: 2, panes: 2 });
  expect(writes).toEqual([["space.select", { id: "existing" }]]);
  snapshot([], []);
  worktree.open(checkout);
  expect(reads).toEqual({ spaces: 3, panes: 3 });
  expect(writes[1]).toEqual(["space.new", { folder: checkout.path, machine: "" }]);
  worktree.open(undefined);
  expect(reads).toEqual({ spaces: 3, panes: 3 });
});

for (const on of [undefined, null, "server"]) {
  for (const focused of [null, "server"]) {
    test(`create and open stay on ${on ?? "local"} while ${focused ?? "local"} is focused`, async () => {
      const calls: string[][] = [];
      const { worktree, destinations } = harness(focused, {
        core: async (args: string[]) => { calls.push(args); return json({ path: "/wt/feature", branch: "feature", new_branch: true }); },
      });
      Object.assign(worktree, { on, text: "feature", repo: { name: "repo", main: "/repo", top: "/repo" }, list: [] });
      expect(await worktree.create()).toBe("");
      expect(calls).toEqual([["worktree", "create", "--json", ...(on ? ["--on", on] : []), "--folder=/repo", "--", "feature"]]);
      worktree.open({ path: "/repo/existing" });
      expect(destinations).toEqual([on ?? null, on ?? null]);
    });
  }
}

test("create opens a space on the folder the core made, keeping the first tab", async () => {
  const { worktree, writes } = harness(null, {
    props: () => ({ overlay: "overlay", tab: "t1", argv: ["claude"] }),
    core: async () => json({ path: "/home/me/.h0x/worktrees/repo/feature", branch: "feature", new_branch: true }),
  });
  Object.assign(worktree, { text: "feature", repo: { name: "repo", main: "/repo", top: "/repo" } });
  expect(await worktree.create()).toBe("");
  expect(writes).toEqual([["space.new", { folder: "/home/me/.h0x/worktrees/repo/feature", machine: "", tab: "t1", argv: ["claude"] }]]);
});

test("a machine name or branch that looks like a flag stays an argument", async () => {
  const { worktree, calls } = withCore(() => json({ path: "/wt/x", branch: "-x", new_branch: true }), "-odd");
  Object.assign(worktree, { text: "-x", repo: { name: "repo", main: "/repo", top: "/repo" } });
  expect(await worktree.create()).toBe("");
  expect(calls[0]).toEqual(["worktree", "create", "--json", "--on=-odd", "--folder=/repo", "--", "-x"]);
});

test("an empty or blank branch name is refused before the core runs", async () => {
  for (const text of ["", "   ", "\t\n"]) {
    const { worktree, calls, writes } = withCore(() => json({}));
    Object.assign(worktree, { text, repo: { name: "repo", main: "/repo", top: "/repo" } });
    expect(await worktree.create()).toBe("Type a branch name.");
    expect(calls).toEqual([]);
    expect(writes).toEqual([]);
  }
});

test("a refused create reports the core's words and opens no space", async () => {
  const { worktree, writes } = withCore(() => refuse("checked_out", "that branch is already checked out at /wt/topic"));
  Object.assign(worktree, { text: "topic", repo: { name: "repo", main: "/repo", top: "/repo" } });
  expect(await worktree.create()).toContain("already checked out");
  expect(writes).toEqual([]);
});

test("an older core says to update it, locally and on a machine", async () => {
  const older = [
    () => ({ code: 2, stdout: "", stderr: "error: unrecognized subcommand 'worktree'" }),
    () => refuse("usage", "unrecognized subcommand"),
    () => refuse("missing_tool", "h0x-wt is not installed next to h0x-core"),
  ];
  for (const answer of older) {
    for (const on of [null, "server"]) {
      const { worktree, writes } = withCore(answer, on);
      Object.assign(worktree, { text: "topic", repo: { name: "repo", main: "/repo", top: "/repo" } });
      expect(await worktree.create()).toBe(`Update h0x-core on ${on ?? "this computer"} to use Worktrees`);
      expect(writes).toEqual([]);
    }
  }
});

test("a core that cannot run or answers nonsense is reported, not thrown", async () => {
  const gone = withCore(() => { throw new Error("spawn failed"); });
  Object.assign(gone.worktree, { text: "topic", repo: { name: "repo", main: "/repo", top: "/repo" } });
  expect(await gone.worktree.create()).toContain("spawn failed");
  const garbled = withCore(() => ({ code: 0, stdout: "not json", stderr: "" }));
  Object.assign(garbled.worktree, { text: "topic", repo: { name: "repo", main: "/repo", top: "/repo" } });
  expect(await garbled.worktree.create()).toContain("unreadable");
});

test("space matching keeps deepest checkout, machine, live-folder and fallback rules", () => {
  const { worktree, reads, writes, snapshot } = harness();
  const paths = ["C:/repo", "C:/repo/nested", "C:/repo/fallback", "C:/repo-other"];
  worktree.list = paths.map(path => ({ path }));
  snapshot([
    { id: "remote", machine: "server", folder: paths[0] },
    { id: "nested", folder: paths[0], tabs: [{ id: "tab" }] },
    { id: "fallback", folder: paths[2] },
    { id: "other", folder: paths[3] },
  ], [
    { tab: "tab" },
    { tab: "tab", cwd: "C:\\repo\\nested\\src" },
    { tab: "tab", cwd: paths[0] },
  ]);
  for (const checkout of worktree.list) worktree.open(checkout);
  expect(writes).toEqual([
    ["space.new", { folder: paths[0], machine: "" }],
    ["space.select", { id: "nested" }],
    ["space.select", { id: "fallback" }],
    ["space.select", { id: "other" }],
  ]);
  worktree.on = "server";
  worktree.open(worktree.list[0]);
  expect(writes[4]).toEqual(["space.select", { id: "remote" }]);
  expect(reads).toEqual({ spaces: 5, panes: 5 });
});

test("the open list shows dirty, ahead/behind and merged per worktree", () => {
  const { worktree, snapshot } = harness();
  worktree.list = [
    wt("/repos/main", "main", { main: true, integrated: "same_commit" }),
    wt("/wt/a", "a", { dirty: true, ahead: 2, behind: 1 }),
    wt("/wt/b", "b", { integrated: "merge_adds_nothing" }),
    wt("/wt/c", "c"),
  ];
  snapshot([], []);
  const rows = rowsOf(worktree.render({}).node).map(textOf);
  expect(rows[0]).not.toContain("merged");
  expect(rows[1]).toContain("dirty");
  expect(rows[1]).toContain("\u21912 \u21931");
  expect(rows[2]).toContain("merged");
  expect(rows[3]).not.toMatch(/dirty|merged|\u2191/);
  expect(rows[0]).not.toContain("Merge");
  expect(rows.slice(1).every(r => r.includes("Merge"))).toBe(true);
});

test("the dialog says HyperOx Worktrees in every mode", () => {
  const { worktree } = harness();
  Object.assign(worktree, { list: [], repo: { name: "repo", main: "/repo", top: "/repo" } });
  expect(textOf(worktree.render({}).node)).toContain("HyperOx Worktrees");
  worktree.opening = false;
  expect(textOf(worktree.render({}).node)).toContain("HyperOx Worktrees");
});

test("repoOf reads a listing from any folder in it", () => {
  const repoOf = pick("repoOf");
  const list = listing(wt("/home/me/wt/topic", "topic")).worktrees;
  expect(repoOf(list, "/home/me/wt/topic")).toEqual({ top: "/home/me/wt/topic", main: "/repos/main", name: "main", linked: true });
  expect(repoOf(list, "/home/me/wt/topic/src/")).toMatchObject({ top: "/home/me/wt/topic", linked: true });
  expect(repoOf(list, "/repos/main/src")).toMatchObject({ top: "/repos/main", linked: false });
  expect(repoOf(list, "/home/me/wt/topic-other")).toMatchObject({ top: "/repos/main", linked: false });
  expect(repoOf(list, "/nowhere")).toMatchObject({ top: "/repos/main", linked: false });
  expect(repoOf([], "/x")).toBeNull();
});

test("repoOf handles Windows folders and a bare repo hidden beside its worktrees", () => {
  const repoOf = pick("repoOf");
  const windows = [wt("C:/Users/me/My Proj", "main", { main: true }), wt("C:/Users/me/wt/topic", "topic")];
  expect(repoOf(windows, "C:\\Users\\me\\wt\\topic\\src")).toEqual({ top: "C:/Users/me/wt/topic", main: "C:/Users/me/My Proj", name: "My Proj", linked: true });
  const bare = [wt("/home/user/project/.bare", null, { main: true }), wt("/home/user/project/topic", "topic")];
  expect(repoOf(bare, "/home/user/project/topic")).toEqual({ top: "/home/user/project/topic", main: "/home/user/project/.bare", name: "project", linked: true });
});

// The dialog as the app opens it, with a core that answers `answer`.
function dialog(action: string, { folder, answer, platform = "linux", on = null, spaces = true }: {
  folder: string; answer: (args: string[]) => unknown; platform?: string; on?: string | null; spaces?: boolean;
}) {
  const calls: string[][] = [];
  const events: string[] = [];
  const closedOverlays: string[] = [];
  const { worktree, snapshot } = harness(null, {
    home: () => "/home/me",
    window: () => ({ platform }),
    props: () => ({ space: "target", action, overlay: "dialog" }),
    InputState: { new: () => ({ on() {}, focus() {}, value: () => "" }) },
    core: async (args: string[]) => { calls.push(args); events.push(`core ${args[1]}`); return answer(args); },
    run: (command: string, args: any) => events.push(`${command} ${args.id ?? args.folder}`),
    closeOverlay: (id: string) => closedOverlays.push(id),
  });
  snapshot(spaces ? [{ id: "target", name: "topic", folder, machine: on }] : [], []);
  let pending: Promise<void> = Promise.resolve();
  const cx: any = {
    notify() {}, stop_propagation() {}, focus_handle: () => ({ focus() {} }), timer: { after() {} },
    spawn: (task: (context: unknown) => Promise<void>) => { pending = task(cx); },
  };
  worktree.init({}, cx);
  const press = async (key: string, modifiers: Record<string, boolean> = {}) => { worktree.key({ key, modifiers }, cx); await pending; };
  return { worktree, calls, events, closedOverlays, cx, ready: () => pending, press, text: () => textOf(worktree.render({}).node) };
}

const answers = (list: unknown, other: (args: string[]) => unknown = () => json({})) => (args: string[]) => args[1] === "list" ? list : other(args);

test("opening reads the listing of the folder, local or on a machine", async () => {
  for (const on of [null, "winbox"]) {
    const d = dialog("open", { folder: "/wt/topic", on, answer: answers(json(listing(wt("/wt/topic", "topic")))) });
    await d.ready();
    expect(d.calls).toEqual([["worktree", "list", "--json", ...(on ? ["--on", on] : []), "--", "/wt/topic"]]);
    expect(d.worktree.repo).toMatchObject({ top: "/wt/topic", linked: true, name: "main" });
    expect(d.worktree.list).toHaveLength(2);
  }
});

test("worktrees at the old ~/.h0x/worktrees path still open", async () => {
  const old = "/home/me/.h0x/worktrees/repo/feature-x";
  const d = dialog("open", { folder: "/repos/main", spaces: false, answer: answers(json(listing(wt(old, "feature/x")))) });
  await d.ready();
  expect(d.text()).toContain("feature/x");
  d.worktree.open(d.worktree.list[1]);
  expect(d.events.at(-1)).toBe(`space.new ${old}`);
});

test("a folder outside git says so, without an error", async () => {
  const d = dialog("open", { folder: "/tmp", answer: () => refuse("not_git", "not a git repository") });
  await d.ready();
  expect(d.worktree.repo).toBeNull();
  expect(d.worktree.error).toBe("");
  expect(d.text()).toContain("is not in a git repository");
});

test("an older core on a machine says to update it when the dialog opens", async () => {
  const d = dialog("open", { folder: "C:/wt", on: "winbox", answer: () => refuse("usage", "unrecognized subcommand") });
  await d.ready();
  expect(d.worktree.repo).toBeNull();
  expect(d.text()).toContain("Update h0x-core on winbox to use Worktrees");
});

for (const [where, top, on, platform, first] of [
  ["a remote Windows drive", "C:/Users/me/wt/topic", "winbox", "macos", true],
  ["a remote UNC share", "//server/share/wt/topic", "winbox", "linux", true],
  ["a remote Linux machine seen from Windows", "/home/me/wt/topic", "linuxbox", "windows", false],
  ["this Windows computer", "C:/Users/J\u00fcrgen \u00c4/wt/\u00fcn\u00ef topic", null, "windows", true],
  ["this Mac", "/Users/me/wt/topic", null, "macos", false],
] as [string, string, string | null, string, boolean][]) {
  test(`remove on ${where} closes the space ${first ? "before" : "after"} the core removes the checkout`, async () => {
    const d = dialog("remove", { folder: top, on, platform, answer: answers(json(listing(wt(top, "topic"))), () => json({ path: top })) });
    await d.ready();
    await d.press("enter");
    expect(d.events).toEqual(first ? ["core list", "space.close target", "core remove"] : ["core list", "core remove", "space.close target"]);
    expect(d.calls[1]).toEqual(["worktree", "remove", "--json", ...(on ? ["--on", on] : []), "--", top]);
    expect(d.closedOverlays).toEqual(["dialog"]);
  });
}

test("a dirty checkout is shown as such and removed with --force on the first Enter", async () => {
  const d = dialog("remove", { folder: "/wt/topic", answer: answers(json(listing(wt("/wt/topic", "topic", { dirty: true })))) });
  await d.ready();
  expect(d.text()).toContain("It has uncommitted changes");
  await d.press("enter");
  expect(d.calls[1]).toEqual(["worktree", "remove", "--json", "--force", "--", "/wt/topic"]);
});

test("a checkout that became dirty asks once more, then removes with --force", async () => {
  let refused = false;
  const d = dialog("remove", { folder: "/wt/topic", answer: answers(json(listing(wt("/wt/topic", "topic"))), () => {
    if (refused) return json({});
    refused = true;
    return refuse("dirty", "uncommitted changes");
  }) });
  await d.ready();
  await d.press("enter");
  expect(d.worktree.error).toBe("It changed since this opened. Enter again removes it anyway.");
  expect(d.events).toEqual(["core list", "core remove"]);
  expect(d.closedOverlays).toEqual([]);
  await d.press("enter");
  expect(d.calls[2]).toContain("--force");
  expect(d.events).toEqual(["core list", "core remove", "core remove", "space.close target"]);
});

test("unpushed submodules are named, and one more Enter removes anyway", async () => {
  let refused = false;
  const d = dialog("remove", { folder: "/wt/topic", answer: answers(json(listing(wt("/wt/topic", "topic"))), () => {
    if (refused) return json({});
    refused = true;
    return refuse("unpushed", "submodules with unpushed commits: libs/a, b");
  }) });
  await d.ready();
  await d.press("enter");
  expect(d.worktree.error).toContain("libs/a, b");
  expect(d.worktree.unpushed).toEqual(["libs/a", "b"]);
  expect(d.text()).toContain("they will be lost");
  await d.press("enter");
  expect(d.calls[2]).toContain("--force");
  expect(d.closedOverlays).toEqual(["dialog"]);
});

test("a refused removal reports the core's words and keeps the space where it can", async () => {
  for (const platform of ["windows", "linux"]) {
    const d = dialog("remove", { folder: "/wt/topic", platform, answer: answers(json(listing(wt("/wt/topic", "topic"))), () => refuse("failed", "This worktree is locked: busy")) });
    await d.ready();
    await d.press("enter");
    expect(d.worktree.error).toContain("This worktree is locked: busy");
    // On Windows the space had to close first (a shell in the folder keeps it busy); elsewhere it stays.
    expect(d.events.filter(e => e.startsWith("space.close"))).toEqual(platform === "windows" ? ["space.close target"] : []);
    expect(d.closedOverlays).toEqual([]);
  }
});

test("a refused removal with no message still says what failed", async () => {
  const d = dialog("remove", { folder: "/wt/topic", answer: answers(json(listing(wt("/wt/topic", "topic"))), () => refuse("failed", "")) });
  await d.ready();
  await d.press("enter");
  expect(d.worktree.error).toBe("h0x-core failed.");
});

test("the main checkout is never removed or merged, even when Enter is pressed", async () => {
  for (const action of ["remove", "merge"]) {
    const d = dialog(action, { folder: "/repos/main/src", answer: answers(json(listing(wt("/wt/topic", "topic")))) });
    await d.ready();
    await d.press("enter");
    expect(d.events).toEqual(["core list"]);
    expect(d.text()).toContain("This is the repository's main checkout");
    expect(d.closedOverlays).toEqual([]);
  }
});

test("merge asks first, then merges with squash, and closes the space and dialog", async () => {
  const d = dialog("merge", { folder: "/wt/topic", answer: answers(json(listing(wt("/wt/topic", "topic", { ahead: 3 }))), () => json({ target: "main", merged: true, removed: { path: "/wt/topic" } })) });
  await d.ready();
  expect(d.text()).toContain("Merge topic into main?");
  expect(d.text()).toContain("Squashes its 3 commits into one");
  expect(d.text()).toContain("HyperOx Worktrees");
  expect(d.calls).toHaveLength(1);
  await d.press("enter");
  expect(d.calls[1]).toEqual(["worktree", "merge", "--json", "--", "/wt/topic"]);
  expect(d.events).toEqual(["core list", "core merge", "space.close target"]);
  expect(d.closedOverlays).toEqual(["dialog"]);
});

test("merge on a machine, and tab switches to rebase", async () => {
  const d = dialog("merge", { folder: "/wt/topic", on: "box", answer: answers(json(listing(wt("/wt/topic", "topic", { ahead: 1 }))), () => json({ target: "main", merged: true })) });
  await d.ready();
  await d.press("tab");
  expect(d.text()).toContain("Rebases its 1 commit onto main");
  await d.press("enter");
  expect(d.calls[1]).toEqual(["worktree", "merge", "--json", "--on", "box", "--no-squash", "--", "/wt/topic"]);
});

test("a merge conflict shows the files and leaves the space and dialog as they were", async () => {
  const d = dialog("merge", { folder: "/wt/topic", answer: answers(json(listing(wt("/wt/topic", "topic", { ahead: 1 }))), () => refuse("conflict", "merge conflict in: src/a.rs, b.txt")) });
  await d.ready();
  await d.press("enter");
  expect(d.worktree.error).toContain("src/a.rs, b.txt");
  expect(d.worktree.error).toContain("Nothing was changed");
  expect(d.events).toEqual(["core list", "core merge"]);
  expect(d.closedOverlays).toEqual([]);
  expect(d.text()).toContain("Nothing was changed");
});

test("a dirty worktree is not merged", async () => {
  const d = dialog("merge", { folder: "/wt/topic", answer: answers(json(listing(wt("/wt/topic", "topic", { dirty: true })))) });
  await d.ready();
  await d.press("enter");
  expect(d.worktree.error).toContain("uncommitted changes");
  expect(d.events).toEqual(["core list"]);
});

test("a merge by an older core says to update it", async () => {
  const d = dialog("merge", { folder: "/wt/topic", on: "box", answer: answers(json(listing(wt("/wt/topic", "topic"))), () => refuse("usage", "unrecognized subcommand")) });
  await d.ready();
  await d.press("enter");
  expect(d.worktree.error).toBe("Update h0x-core on box to use Worktrees");
});

test("merging from the open list: ctrl+m asks, esc goes back to the list, Enter merges", async () => {
  const d = dialog("open", { folder: "/repos/main", spaces: false, answer: answers(json(listing(wt("/wt/a", "a", { ahead: 2 }))), () => json({ target: "main", merged: true })) });
  await d.ready();
  await d.press("m", { control: true }); // the main checkout: nothing to merge
  expect(d.worktree.merging).toBeNull();
  d.worktree.at = 1;
  await d.press("m"); // without ctrl it is typing
  expect(d.worktree.merging).toBeNull();
  await d.press("m", { control: true });
  expect(d.worktree.merging).toMatchObject({ path: "/wt/a" });
  expect(d.text()).toContain("Merge a into main?");
  await d.press("escape");
  expect(d.worktree.merging).toBeNull();
  expect(d.closedOverlays).toEqual([]);
  expect(rowsOf(d.worktree.render({}).node)).toHaveLength(2);
  d.worktree.askMerge(d.worktree.list[1]);
  await d.press("enter");
  expect(d.calls.at(-1)).toEqual(["worktree", "merge", "--json", "--", "/wt/a"]);
  expect(d.closedOverlays).toEqual(["dialog"]);
});

test("esc on the merge question of a space's own dialog closes it", async () => {
  const d = dialog("merge", { folder: "/wt/topic", answer: answers(json(listing(wt("/wt/topic", "topic")))) });
  await d.ready();
  await d.press("escape");
  expect(d.closedOverlays).toEqual(["dialog"]);
  expect(d.events).toEqual(["core list"]);
});

// ---- Edge cases (WT-A1/A2): core replies, error kinds, odd names, cancel, double Enter. ----

// Every string drawn in a rendered tree.
function leaves(node: any): string[] {
  return typeof node === "string" ? [node] : (node?.children ?? []).flatMap(leaves);
}
const ok = (stdout: string) => ({ code: 0, stdout, stderr: "" });
const topic = () => json(listing(wt("/wt/topic", "topic", { ahead: 1 })));

test("an empty, non-JSON or truncated reply to the listing is reported, not thrown", async () => {
  for (const stdout of ["", "   \n", "not json", "{\"default_branch\":\"main\",\"worktrees\":[{\"path\":\"/wt/a\"", "<html>"]) {
    const d = dialog("open", { folder: "/wt/topic", answer: () => ok(stdout) });
    await d.ready();
    expect(d.worktree.repo).toBeNull();
    expect(d.worktree.error).toBe("h0x-core answered something unreadable.");
    expect(d.text()).toContain("h0x-core answered something unreadable.");
    expect(d.events).toEqual(["core list"]);
  }
});

test("a JSON null or bare number as the listing is reported, not shown as a script error", async () => {
  for (const stdout of ["null", "42"]) {
    const d = dialog("open", { folder: "/wt/topic", answer: () => ok(stdout) });
    await d.ready();
    expect(d.worktree.repo).toBeNull();
    expect(d.worktree.error).not.toMatch(/TypeError|Cannot read|undefined|null/);
    expect(d.closedOverlays).toEqual([]);
  }
});

test("an unreadable reply to merge, create or remove leaves everything as it was", async () => {
  for (const stdout of ["", "{\"target\":\"ma", "not json"]) {
    const m = dialog("merge", { folder: "/wt/topic", answer: answers(topic(), () => ok(stdout)) });
    await m.ready();
    await m.press("enter");
    expect(m.worktree.error).toBe("h0x-core answered something unreadable.");
    expect(m.events).toEqual(["core list", "core merge"]);
    expect(m.closedOverlays).toEqual([]);
    const r = dialog("remove", { folder: "/wt/topic", answer: answers(topic(), () => ok(stdout)) });
    await r.ready();
    await r.press("enter");
    expect(r.worktree.error).toBe("h0x-core answered something unreadable.");
    expect(r.events).toEqual(["core list", "core remove"]);
    expect(r.closedOverlays).toEqual([]);
    const c = withCore(() => ok(stdout));
    Object.assign(c.worktree, { text: "topic", repo: { name: "repo", main: "/repo", top: "/repo" } });
    expect(await c.worktree.create()).toBe("h0x-core answered something unreadable.");
    expect(c.writes).toEqual([]);
  }
});

test("a failed call with no words, plain words or a JSON error on stdout is told apart", async () => {
  const cases: [unknown, string][] = [
    [{ code: 1, stdout: "", stderr: "" }, "h0x-core failed."],
    [{ code: 1, stdout: "  \n", stderr: "  " }, "h0x-core failed."],
    [{ code: 128, stdout: "", stderr: "fatal: boom\n" }, "fatal: boom"],
    [{ code: 1, stdout: JSON.stringify({ error: { kind: "usage", message: "x" } }), stderr: "" }, "Update h0x-core on this computer to use Worktrees"],
    [{ code: 2, stdout: "", stderr: "error: Unrecognized subcommand 'worktree'\n\nUsage: h0x-core <COMMAND>" }, "Update h0x-core on this computer to use Worktrees"],
  ];
  for (const [answer, said] of cases) {
    const { worktree, writes } = withCore(() => answer);
    Object.assign(worktree, { text: "topic", repo: { name: "repo", main: "/repo", top: "/repo" } });
    expect(await worktree.create()).toBe(said);
    expect(writes).toEqual([]);
  }
});

test("extra and unknown fields in a listing are ignored, missing status fields draw nothing", async () => {
  const extra = {
    default_branch: "main", schema: 9, future: { a: 1 },
    worktrees: [
      { ...wt("/repos/main", "main", { main: true }), locked: "why", prunable: null },
      { path: "/wt/bare-minimum", branch: "bare-minimum", main: false, hooks: ["x"] },
    ],
  };
  const d = dialog("open", { folder: "/repos/main", spaces: false, answer: answers(json(extra)) });
  await d.ready();
  expect(d.worktree.error).toBe("");
  expect(d.worktree.repo).toEqual({ top: "/repos/main", main: "/repos/main", name: "main", linked: false });
  const rows = rowsOf(d.worktree.render({}).node).map(textOf);
  expect(rows).toHaveLength(2);
  expect(rows[1]).toContain("bare-minimum");
  for (const row of rows) expect(row).not.toMatch(/undefined|NaN|null|\[object/);
});

test("extra fields in a create reply are ignored: the space opens on its path only", async () => {
  const { worktree, writes } = withCore(() => json({ path: "/wt/x", branch: "x", new_branch: false, hooks_ran: 0, extra: { y: 1 } }));
  Object.assign(worktree, { text: "x", repo: { name: "repo", main: "/repo", top: "/repo" } });
  expect(await worktree.create()).toBe("");
  expect(writes).toEqual([["space.new", { folder: "/wt/x", machine: "", tab: undefined, argv: undefined }]]);
});

test("a create reply without a path opens no space on an undefined folder", async () => {
  for (const reply of [{}, { branch: "x", new_branch: true }, { path: "" }]) {
    const { worktree, writes } = withCore(() => json(reply));
    Object.assign(worktree, { text: "x", repo: { name: "repo", main: "/repo", top: "/repo" } });
    const said = await worktree.create();
    expect(writes.filter((w: any) => w[0] === "space.new" && !w[1].folder)).toEqual([]);
    expect(said).not.toBe("");
  }
});

test("an integration reason the app does not know still shows merged (integrated, reason unknown)", () => {
  const { worktree, snapshot } = harness();
  worktree.list = [
    wt("/repos/main", "main", { main: true }),
    wt("/wt/u", "u", { integrated: "unknown" }),
    wt("/wt/f", "f", { integrated: "a_reason_from_a_newer_tool" }),
    wt("/wt/n", "n", { integrated: null }),
  ];
  snapshot([], []);
  const rows = rowsOf(worktree.render({}).node).map(textOf);
  expect(rows[1]).toContain("merged");
  expect(rows[2]).toContain("merged");
  expect(rows[3]).not.toMatch(/unknown|merged/);
});

test("every known integration reason shows merged on a linked worktree, never on the main checkout", () => {
  const reasons = ["same_commit", "ancestor", "no_added_changes", "trees_match", "merge_adds_nothing", "patch_id_match"];
  const { worktree, snapshot } = harness();
  worktree.list = [wt("/repos/main", "main", { main: true, integrated: "same_commit" }), ...reasons.map((r) => wt(`/wt/${r}`, r, { integrated: r }))];
  snapshot([], []);
  const rows = rowsOf(worktree.render({}).node).map(textOf);
  expect(rows[0]).not.toContain("merged");
  for (let i = 1; i <= reasons.length; i++) expect(rows[i]).toContain("merged");
});

test("not_git on a later call shows the core's words", async () => {
  const d = dialog("merge", { folder: "/wt/topic", answer: answers(topic(), () => refuse("not_git", "not a git repository: /wt/topic")) });
  await d.ready();
  await d.press("enter");
  expect(d.worktree.error).toBe("not a git repository: /wt/topic");
  expect(d.closedOverlays).toEqual([]);
});

test("checked_out names the path where the branch already is", async () => {
  const at = "/home/me/wt/my t\u00f4pic \u65e5\u672c";
  const { worktree, writes } = withCore(() => refuse("checked_out", `that branch is already checked out at ${at}`));
  Object.assign(worktree, { text: "topic", repo: { name: "repo", main: "/repo", top: "/repo" } });
  expect(await worktree.create()).toBe(`that branch is already checked out at ${at}`);
  expect(writes).toEqual([]);
});

test("dirty from the core on merge (it changed since the list) changes nothing", async () => {
  const d = dialog("merge", { folder: "/wt/topic", answer: answers(topic(), () => refuse("dirty", "/wt/topic has uncommitted changes")) });
  await d.ready();
  await d.press("enter");
  expect(d.worktree.error).toBe("/wt/topic has uncommitted changes");
  expect(d.events).toEqual(["core list", "core merge"]);
  expect(d.closedOverlays).toEqual([]);
});

test("a conflict lists files with spaces and non-ASCII names exactly", async () => {
  const files = "src/my file.rs, d\u00efr/\u00fcn\u00ef.txt, \u65e5\u672c\u8a9e.md, a, b.txt";
  const d = dialog("merge", { folder: "/wt/topic", answer: answers(topic(), () => refuse("conflict", `merge conflict in: ${files}`)) });
  await d.ready();
  await d.press("enter");
  expect(d.worktree.error).toBe(`Can't merge: merge conflict in: ${files}. Nothing was changed.`);
  expect(leaves(d.worktree.render({}).node)).toContain(d.worktree.error);
  expect(d.events).toEqual(["core list", "core merge"]);
  expect(d.closedOverlays).toEqual([]);
  expect(d.worktree.merging).toMatchObject({ path: "/wt/topic" });
});

test("main_checkout from the core on remove keeps the space and the dialog", async () => {
  const d = dialog("remove", { folder: "/wt/topic", answer: answers(topic(), () => refuse("main_checkout", "the main checkout cannot be removed")) });
  await d.ready();
  await d.press("enter");
  expect(d.worktree.error).toBe("the main checkout cannot be removed");
  expect(d.events).toEqual(["core list", "core remove"]);
  expect(d.closedOverlays).toEqual([]);
});

test("an unknown error kind shows its message", async () => {
  const { worktree } = withCore(() => refuse("invalid_params", "branch name is not valid"));
  Object.assign(worktree, { text: "a..b", repo: { name: "repo", main: "/repo", top: "/repo" } });
  expect(await worktree.create()).toBe("branch name is not valid");
});

for (const kind of ["usage", "missing_tool"]) {
  for (const on of [null, "my box", "-odd", "b\u00fcro"]) {
    test(`${kind} on ${on ?? "this computer"} says to update h0x-core for list, create, merge and remove`, async () => {
      const said = `Update h0x-core on ${on ?? "this computer"} to use Worktrees`;
      const older = () => refuse(kind, "anything");
      const l = dialog("open", { folder: "/wt/topic", on, answer: older });
      await l.ready();
      expect(l.worktree.error).toBe(said);
      const c = dialog("new", { folder: "/wt/topic", on, answer: answers(topic(), older) });
      await c.ready();
      c.worktree.text = "feature";
      c.worktree.go(c.cx);
      await c.ready();
      expect(c.worktree.error).toBe(said);
      const m = dialog("merge", { folder: "/wt/topic", on, answer: answers(topic(), older) });
      await m.ready();
      await m.press("enter");
      expect(m.worktree.error).toBe(said);
      const r = dialog("remove", { folder: "/wt/topic", on, answer: answers(topic(), older) });
      await r.ready();
      await r.press("enter");
      expect(r.worktree.error).toBe(said);
      for (const d of [l, c, m, r]) expect(d.closedOverlays).toEqual([]);
    });
  }
}

test("a machine name with spaces is one argument; one starting with - is glued to --on", async () => {
  for (const [on, flag] of [["my box", ["--on", "my box"]], ["-odd", ["--on=-odd"]], ["-", ["--on=-"]], ["--on", ["--on=--on"]]] as [string, string[]][]) {
    const m = dialog("merge", { folder: "/wt/topic", on, answer: answers(topic(), () => json({ target: "main", merged: true })) });
    await m.ready();
    await m.press("enter");
    expect(m.calls).toEqual([["worktree", "list", "--json", ...flag, "--", "/wt/topic"], ["worktree", "merge", "--json", ...flag, "--", "/wt/topic"]]);
    const r = dialog("remove", { folder: "/wt/topic", on, answer: answers(topic(), () => json({ path: "/wt/topic" })) });
    await r.ready();
    await r.press("enter");
    expect(r.calls[1]).toEqual(["worktree", "remove", "--json", ...flag, "--", "/wt/topic"]);
    const c = withCore(() => json({ path: "/wt/x", branch: "x", new_branch: true }), on);
    Object.assign(c.worktree, { text: "x", repo: { name: "repo", main: "/repo", top: "/repo" } });
    expect(await c.worktree.create()).toBe("");
    expect(c.calls[0]).toEqual(["worktree", "create", "--json", ...flag, "--folder=/repo", "--", "x"]);
    expect(c.writes[0]).toEqual(["space.new", { folder: "/wt/x", machine: on, tab: undefined, argv: undefined }]);
  }
});

test("branch names with spaces, unicode or a leading dash go after -- as one trimmed argument", async () => {
  for (const [typed, sent] of [
    ["my topic", "my topic"],
    ["  f\u00ebature/\u00fcn\u00ef \u65e5\u672c  ", "f\u00ebature/\u00fcn\u00ef \u65e5\u672c"],
    ["-x", "-x"],
    ["--force", "--force"],
    ["--", "--"],
  ]) {
    const { worktree, calls } = withCore(() => json({ path: "/wt/x", branch: sent, new_branch: true }));
    Object.assign(worktree, { text: typed, repo: { name: "repo", main: "/repo", top: "/repo" } });
    expect(await worktree.create()).toBe("");
    expect(calls).toEqual([["worktree", "create", "--json", "--folder=/repo", "--", sent]]);
  }
});

test("a worktree folder with spaces, unicode or a leading dash is passed whole to merge", async () => {
  for (const top of ["/wt/my topic", "/wt/-x", "C:/Users/J\u00fcrgen/wt/\u65e5\u672c topic"]) {
    const m = dialog("merge", { folder: top, answer: answers(json(listing(wt(top, "f\u00ebature x", { ahead: 1 }))), () => json({ target: "main", merged: true })) });
    await m.ready();
    expect(m.text()).toContain("Merge f\u00ebature x into main?");
    await m.press("enter");
    expect(m.calls[1]).toEqual(["worktree", "merge", "--json", "--", top]);
  }
});

test("a Windows folder with backslashes is listed as given and found in a forward-slash listing", async () => {
  const d = dialog("open", { folder: "C:\\Users\\me\\wt\\my topic\\", on: "winbox", answer: answers(json({ default_branch: "main", worktrees: [wt("C:/Users/me/repo", "main", { main: true }), wt("C:/Users/me/wt/my topic", "my topic")] })) });
  await d.ready();
  expect(d.calls[0]).toEqual(["worktree", "list", "--json", "--on", "winbox", "--", "C:\\Users\\me\\wt\\my topic\\"]);
  expect(d.worktree.repo).toEqual({ top: "C:/Users/me/wt/my topic", main: "C:/Users/me/repo", name: "repo", linked: true });
});

test("remove of the main checkout is never offered, wherever main sits in the listing", async () => {
  const listed = { default_branch: "main", worktrees: [wt("/wt/topic", "topic"), wt("/repos/main", "main", { main: true })] };
  for (const folder of ["/repos/main", "/repos/main/", "/repos/main/deep/src"]) {
    const d = dialog("remove", { folder, answer: answers(json(listed)) });
    await d.ready();
    expect(d.worktree.repo).toMatchObject({ top: "/repos/main", linked: false });
    const shown = leaves(d.worktree.render({}).node);
    expect(shown).toContain("esc close");
    expect(shown.some((s) => s.includes("\u21b5 remove"))).toBe(false);
    await d.press("enter");
    await d.press("enter");
    expect(d.events).toEqual(["core list"]);
  }
});

test("the space menu offers Merge and Remove worktree only on a linked worktree", () => {
  const menuSource = readFileSync(new URL("../defaults/mods/menu/main.js", import.meta.url), "utf8")
    .replace(/^import .*;\n/gm, "").replace("export default class", "class");
  const globals = { View: class {}, div: element, svg: element, v_flex: element, h_flex: element, run() {}, props: () => ({}), closeOverlay() {}, openOverlay() {}, theme: () => ({}), keys: () => [], terminals: () => [], spaces: () => [], panes: () => [], exec: async () => ({ code: 1 }) };
  const items = new Function(...Object.keys(globals), `${menuSource}; return items;`)(...Object.values(globals));
  const labels = (git: string | null | undefined) => items({ tab: "space", target: { id: "s" } }, git).map((i: any) => i[0]);
  for (const git of ["main", null, undefined, ""]) {
    expect(labels(git)).not.toContain("Remove worktree");
    expect(labels(git)).not.toContain("Merge worktree");
  }
  expect(labels("linked")).toContain("Remove worktree");
  expect(labels("linked")).toContain("Merge worktree");
  const merge = items({ tab: "space", target: { id: "s" } }, "linked").find((i: any) => i[0] === "Merge worktree");
  expect(merge).toEqual(["Merge worktree", "worktree", { space: "s", action: "merge" }]);
});

test("cancelling the merge question calls no core merge", async () => {
  // From a space's own dialog: esc (also after tab) closes with no merge.
  for (const keys of [["escape"], ["tab", "escape"], ["tab", "tab", "escape"]]) {
    const d = dialog("merge", { folder: "/wt/topic", answer: answers(topic(), () => json({ target: "main", merged: true })) });
    await d.ready();
    for (const k of keys) await d.press(k);
    expect(d.calls.map((c) => c[1])).toEqual(["list"]);
    expect(d.closedOverlays).toEqual(["dialog"]);
  }
  // From the open list: the Merge button, then esc, then Enter opens instead of merging.
  const o = dialog("open", { folder: "/repos/main", spaces: false, answer: answers(json(listing(wt("/wt/a", "a", { ahead: 2 }))), () => json({ target: "main", merged: true })) });
  await o.ready();
  o.worktree.askMerge(o.worktree.list[1]);
  await o.press("escape");
  expect(o.worktree.merging).toBeNull();
  o.worktree.at = 1;
  await o.press("enter");
  expect(o.calls.map((c) => c[1])).toEqual(["list"]);
  expect(o.events.at(-1)).toBe("space.new /wt/a");
});

test("the main checkout and a detached checkout have no merge question", async () => {
  const d = dialog("open", { folder: "/repos/main", spaces: false, answer: answers(json(listing(wt("/wt/d", null)))) });
  await d.ready();
  d.worktree.askMerge(d.worktree.list[0]);
  expect(d.worktree.merging).toBeNull();
  d.worktree.askMerge(d.worktree.list[1]);
  expect(d.worktree.merging).toBeNull();
  d.worktree.askMerge(undefined);
  expect(d.worktree.merging).toBeNull();
  const rows = rowsOf(d.worktree.render({}).node).map(textOf);
  expect(rows[1]).toContain("detached");
  expect(rows[1]).not.toContain("Merge");
});

test("a double Enter sends one merge", async () => {
  for (const on of [null, "box"]) {
    const d = dialog("merge", { folder: "/wt/topic", on, answer: answers(topic(), () => json({ target: "main", merged: true })) });
    await d.ready();
    d.worktree.key({ key: "enter", modifiers: {} }, d.cx);
    d.worktree.key({ key: "enter", modifiers: {} }, d.cx);
    await d.ready();
    expect(d.calls.filter((c) => c[1] === "merge")).toHaveLength(1);
    expect(d.events.filter((e) => e.startsWith("space.close"))).toEqual(["space.close target"]);
  }
});

test("a double Enter sends one merge even while a slow core has not answered", async () => {
  let release: (v: unknown) => void = () => {};
  const d = dialog("merge", { folder: "/wt/topic", answer: answers(topic(), () => new Promise((r) => { release = r; })) });
  await d.ready();
  d.worktree.key({ key: "enter", modifiers: {} }, d.cx);
  const first = d.ready();
  await Promise.resolve();
  d.worktree.key({ key: "enter", modifiers: {} }, d.cx);
  expect(d.worktree.busy).toBe(true);
  expect(d.text()).toContain("working\u2026");
  release(json({ target: "main", merged: true }));
  await first;
  await d.ready();
  expect(d.calls.filter((c) => c[1] === "merge")).toHaveLength(1);
  expect(d.closedOverlays).toEqual(["dialog"]);
});

test("a double Enter sends one remove and one create", async () => {
  const r = dialog("remove", { folder: "/wt/topic", answer: answers(topic(), () => json({ path: "/wt/topic" })) });
  await r.ready();
  r.worktree.key({ key: "enter", modifiers: {} }, r.cx);
  r.worktree.key({ key: "enter", modifiers: {} }, r.cx);
  await r.ready();
  expect(r.calls.filter((c) => c[1] === "remove")).toHaveLength(1);
  const c = dialog("new", { folder: "/wt/topic", answer: answers(topic(), () => json({ path: "/wt/x", branch: "x", new_branch: true })) });
  await c.ready();
  c.worktree.text = "x";
  c.worktree.go(c.cx);
  c.worktree.go(c.cx);
  await c.ready();
  expect(c.calls.filter((a) => a[1] === "create")).toHaveLength(1);
});

test("Enter before the listing arrives sends nothing", async () => {
  for (const action of ["merge", "remove", "new"]) {
    const d = dialog(action, { folder: "/wt/topic", answer: answers(topic()) });
    d.worktree.key({ key: "enter", modifiers: {} }, d.cx);
    d.worktree.go(d.cx);
    await d.ready();
    expect(d.calls.map((c) => c[1])).toEqual(["list"]);
  }
});

test("the dialog title is exactly HyperOx Worktrees in new, merge and remove, and leads the open list", async () => {
  for (const action of ["new", "merge", "remove"]) {
    const d = dialog(action, { folder: "/wt/topic", answer: answers(topic()) });
    await d.ready();
    const shown = leaves(d.worktree.render({}).node);
    expect(shown.filter((s) => s === "HyperOx Worktrees")).toHaveLength(1);
    expect(shown.some((s) => /herdr/i.test(s))).toBe(false);
  }
  const o = dialog("open", { folder: "/wt/topic", answer: answers(topic()) });
  await o.ready();
  expect(leaves(o.worktree.render({}).node)).toContain("HyperOx Worktrees \u00b7 main");
  const loading = dialog("open", { folder: "/wt/topic", answer: answers(topic()) });
  expect(leaves(loading.worktree.render({}).node)).toContain("HyperOx Worktrees");
  await loading.ready();
});

test("the worktree mod runs no git of its own: every call goes through the core", () => {
  const raw = readFileSync(new URL("../defaults/mods/worktree/main.js", import.meta.url), "utf8");
  expect(raw).not.toMatch(/\bexec\b/);
  expect(raw).not.toMatch(/["'`]git["'`]/);
  expect(raw).not.toMatch(/\bgit\s*\(/);
  expect(raw).not.toMatch(/\b(?:sub)?modules\s*\(/);
  expect(raw).not.toMatch(/rev-parse|worktree add|worktree remove|--porcelain/);
  expect(raw).toMatch(/import \{[^}]*\bcore\b[^}]*\} from "h0x"/);
  expect(raw).not.toMatch(/\r\n/);
});

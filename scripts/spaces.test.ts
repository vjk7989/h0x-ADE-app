import { test, expect, spyOn } from "bun:test";
import { readFile } from "node:fs/promises";
import { mkdtempSync, mkdirSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
const source = (await readFile(new URL("../defaults/mods/spaces/main.js", import.meta.url), "utf8"))
  .replace(/^import .*;\n/gm, "").replace("export default class Spaces", "class Spaces").replace(/^export /gm, "");

type Space = { id: string; folder: string; machine?: string | null; active?: boolean };
function harness(replies: unknown[] = [], storage = new Map<string, string>(), snapshot: Space[] = [], execGit?: (...args: any[]) => Promise<unknown>) {
  const calls: string[][] = [];
  const tasks: Promise<unknown>[] = [];
  const availability = [{ name: null as string | null, layout: "ready" }];
  const moves: { id: string; index: number }[] = [];
  const handlers: Record<string, (...args: any[]) => void> = {};
  const win = { visible: true };
  const env = { layout: null as unknown };
  // The list's scroll position: the requests the view makes are what the sidebar would scroll by.
  const ScrollHandle = { new: () => ({ calls: [] as number[], scroll_to_item(index: number) { this.calls.push(index); } }) };
  const theme = () => new Proxy({}, { get: () => "#000000" });
  const tracked: unknown[] = [];
  const revision = { n: 0 };
  const reads = { spaces: 0, panes: 0 };
  const element: any = new Proxy({}, { get: (_, method) => (...args: any[]) => {
    if (String(method).startsWith("on_")) handlers[String(method)] = args.at(-1);
    if (method === "track_scroll") tracked.push(args[0]);
    return element;
  } });
  const Spaces = new Function("View", "core", "localStorage", "spaces", "panes", "run", "v_flex", "h_flex", "div", "svg", "shared", "machines", "windowVisible", "revision", "exec", "ScrollHandle", "theme", "layout", "setShared", `${source}; return Spaces;`)(
    class {},
    async (args: string[]) => {
      calls.push(args);
      const reply = replies.shift();
      if (reply instanceof Error) throw reply;
      return reply ?? { code: 0, stdout: "{}", stderr: "" };
    },
    { setItem(key: string, value: string) { storage.set(key, value); }, getItem(key: string) { return storage.get(key) ?? null; } },
    () => { reads.spaces++; return snapshot; }, () => { reads.panes++; return []; },
    (name: string, args: { id: string; index: number }) => {
      expect(name).toBe("space.move");
      moves.push(args);
    },
    ...Array(4).fill(() => element), () => null, () => availability, () => win.visible, () => revision.n, execGit,
    ScrollHandle, theme, () => env.layout, () => {},
  );
  const view = new Spaces();
  view.reconnecting = {};
  view.folded = new Set(["cloud"]);
  view.refreshMachines = () => {};
  const cx = { timer: { every() {} }, notify() {}, spawn(fn: (cx: unknown) => Promise<unknown>) { tasks.push(fn(cx)); } };
  return { view, calls, tasks, cx, storage, moves, handlers, snapshot, availability, win, revision, reads, env, tracked };
}

test("git grouping reads refs for normal, detached, unborn and bare worktrees", async () => {
  // Git answers with real paths, and the temp folder is behind a link on macOS (/var -> /private/var).
  const root = realpathSync(mkdtempSync(join(tmpdir(), "h0x-spaces-")));
  const git = (cwd: string, ...args: string[]) => {
    const result = Bun.spawnSync(["git", ...args], { cwd, env: { ...process.env, GIT_CONFIG_GLOBAL: "/dev/null", GIT_CONFIG_NOSYSTEM: "1", GIT_AUTHOR_NAME: "Test", GIT_AUTHOR_EMAIL: "test@example.com", GIT_COMMITTER_NAME: "Test", GIT_COMMITTER_EMAIL: "test@example.com" } });
    if (result.exitCode) throw new Error(result.stderr.toString());
    return result.stdout.toString().trim();
  };
  const execGit = async (_command: string, args: string[], { cwd }: { cwd: string }) => {
    const result = Bun.spawnSync(["git", ...args], { cwd });
    return { code: result.exitCode, stdout: result.stdout.toString(), stderr: result.stderr.toString() };
  };
  const h = harness([], new Map(), [], execGit);
  Object.assign(h.view, { git: {}, checked: {}, unsure: new Set() });
  h.view.watch = () => {};
  const check = async (folder: string) => {
    const key = JSON.stringify([null, folder]);
    await h.view.check(key, h.cx);
    return h.view.git[key];
  };
  try {
    const repo = join(root, "repo");
    mkdirSync(repo);
    git(repo, "init", "-b", "main");
    expect(await check(repo)).toMatchObject({ branch: "main", ahead: 0, behind: 0, linked: false });
    git(repo, "commit", "--allow-empty", "-m", "base");
    git(repo, "branch", "upstream");
    git(repo, "branch", "--set-upstream-to=upstream");
    git(repo, "commit", "--allow-empty", "-m", "ahead");
    git(repo, "checkout", "upstream");
    git(repo, "commit", "--allow-empty", "-m", "behind");
    git(repo, "checkout", "main");
    git(repo, "tag", "main");
    expect(await check(repo)).toMatchObject({ name: "repo", branch: "main", ahead: 1, behind: 1, linked: false });
    const tree = join(root, "tree");
    git(repo, "worktree", "add", "-b", "topic", tree);
    expect(await check(tree)).toMatchObject({ branch: "topic", ahead: 0, behind: 0, linked: true, repo: JSON.stringify([null, join(repo, ".git")]) });
    git(tree, "checkout", "--detach");
    expect(await check(tree)).toMatchObject({ branch: "(detached)", ahead: 0, behind: 0 });
    expect(await check(root)).toBeNull();
    expect(h.view.unsure.has(JSON.stringify([null, root]))).toBe(false);
    const bare = join(root, "bare.git"), bareTree = join(root, "bare-tree");
    git(root, "clone", "--bare", repo, bare);
    expect(await check(bare)).toBeNull();
    git(bare, "worktree", "add", "-b", "bare-topic", bareTree);
    expect(await check(bareTree)).toMatchObject({ name: "bare", branch: "bare-topic", linked: true, repo: JSON.stringify([null, bare]) });
    // A bare repo hidden beside its worktrees (<project>/.bare) is named after its project.
    const project = join(root, "project"), hidden = join(project, ".bare"), projectTree = join(project, "topic");
    git(root, "clone", "--bare", repo, hidden);
    git(hidden, "worktree", "add", "-b", "project-topic", projectTree);
    expect(await check(projectTree)).toMatchObject({ name: "project", branch: "project-topic", linked: true, repo: JSON.stringify([null, hidden]) });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test.each(["--show-toplevel", "--git-common-dir"])("a failed %s keeps worktrees nested until git can answer again", async (failedRead) => {
  let failure: string | null = null;
  const snapshot = [{ id: "main", folder: "/repo" }, { id: "tree", folder: "/tree" }];
  const h = harness([], new Map(), snapshot, async (_command: string, args: string[], { cwd }: { cwd: string }) => {
    if (cwd === "/repo" && args.includes(failedRead) && failure) return { code: 128, stdout: "", stderr: failure };
    const stdout = args.includes("--show-toplevel") ? cwd + "\n"
      : args.includes("--git-common-dir") ? "/repo/.git\n"
      : args[0] === "symbolic-ref" ? "refs/heads/main\n" : "0 0\n";
    return { code: 0, stdout, stderr: "" };
  });
  h.view.init({}, h.cx);
  h.view.watch = () => {};
  const main = JSON.stringify([null, "/repo"]);
  const tree = JSON.stringify([null, "/tree"]);
  await h.view.check(main, h.cx);
  await h.view.check(tree, h.cx);
  const nesting = () => h.view.ordered().map((r: any) => [r.w.id, r.tree]);
  expect(nesting()).toEqual([["main", null], ["tree", "└─"]]);
  const cached = h.view.git[main];

  failure = "fatal: unable to create thread: Too many open files";
  await h.view.check(main, h.cx);
  expect(nesting()).toEqual([["main", null], ["tree", "└─"]]);
  expect(h.view.git[main]).toBe(cached);
  expect(h.view.unsure.has(main)).toBe(true);
  expect(h.view.due(Date.now() + 60001, snapshot, []).has(main)).toBe(true);

  failure = null;
  await h.view.check(main, h.cx);
  expect(h.view.unsure.has(main)).toBe(false);
  expect(nesting()).toEqual([["main", null], ["tree", "└─"]]);

  failure = "fatal: not a git repository (or any of the parent directories): .git";
  await h.view.check(main, h.cx);
  expect(h.view.git[main]).toBeNull();
  expect(h.view.unsure.has(main)).toBe(false);
  expect(nesting()).toEqual([["main", null], ["tree", null]]);
});


const machine = { name: "cloud", id: "old", target: "user@host", session: null, via: false };

test("Connect removes then pairs the same SSH machine, ignoring repeat clicks", async () => {
  const { view, calls, tasks, cx } = harness();
  view.reconnect(machine, cx);
  view.reconnect(machine, cx);
  await Promise.all(tasks);
  expect(calls).toEqual([
    ["machine", "rm", "cloud", "--json"],
    ["machine", "add", "user@host", "--name", "cloud", "--json"],
  ]);
  expect(view.reconnecting).toEqual({});
  expect(view.folded.has("cloud")).toBe(false);
});

test("Connect preserves a via command and nondefault core session", async () => {
  const { view, calls, tasks, cx } = harness();
  view.reconnect({ ...machine, via: true, target: "ssh -p 2222 host", session: "work" }, cx);
  await Promise.all(tasks);
  expect(calls[1]).toEqual([
    "machine", "add", "--via", "ssh -p 2222 host", "--name", "cloud", "--session", "work", "--json",
  ]);
});

test("a failed removal never adds, and a failed add remains retryable without another removal", async () => {
  const failure = { code: 1, stdout: "", stderr: '{"error":{"message":"no connection"}}' };
  const removed = harness([failure]);
  removed.view.reconnect(machine, removed.cx);
  await Promise.all(removed.tasks);
  expect(removed.calls).toHaveLength(1);
  expect(removed.view.reconnecting.cloud.error).toBe("no connection");
  expect(removed.view.reconnecting.cloud.removed).toBe(false);

  const added = harness([{ code: 0 }, failure]);
  added.view.reconnect(machine, added.cx);
  await Promise.all(added.tasks);
  expect(added.view.reconnecting.cloud.machine).toEqual(machine);
  expect(added.view.reconnecting.cloud.busy).toBe(false);
  added.view.reconnect(machine, added.cx);
  await Promise.all(added.tasks);
  expect(added.calls.map(c => c[1])).toEqual(["rm", "add", "add"]);
  expect(added.view.reconnecting).toEqual({});
});

test("temporary reconnects remain temporary", async () => {
  const { view, calls, tasks, cx } = harness();
  view.reconnect({ ...machine, temporary: true }, cx);
  await Promise.all(tasks);
  expect(calls[1]).toContain("--temp");
});

test("failed pairing survives a sidebar restart and retries without removal", async () => {
  const failed = harness([{ code: 0 }, { code: 1, stderr: "core too old" }]);
  failed.view.reconnect(machine, failed.cx);
  await Promise.all(failed.tasks);
  const restarted = harness([], failed.storage);
  restarted.view.init({}, restarted.cx);
  expect(restarted.view.reconnecting.cloud.machine).toEqual(machine);
  expect(restarted.view.reconnecting.cloud.error).toBe("core too old");
  expect(restarted.view.reconnecting.cloud.busy).toBe(false);
  restarted.view.reconnect(machine, restarted.cx);
  await Promise.all(restarted.tasks);
  expect(restarted.calls.map(c => c[1])).toEqual(["add"]);
  const completed = harness([], failed.storage);
  completed.view.init({}, completed.cx);
  expect(completed.view.reconnecting).toEqual({});
});

test("unexpected transport failures clear the busy state", async () => {
  const { view, tasks, cx } = harness([new Error("offline")]);
  view.reconnect(machine, cx);
  await Promise.all(tasks);
  expect(view.reconnecting.cloud.busy).toBe(false);
  expect(view.reconnecting.cloud.error).toBe("offline");
});

function orderedHarness(ids: string[]) {
  const h = harness([], new Map(), ids.map(id => ({ id, folder: "/" + id })));
  h.view.git = {};
  h.view.shown = new Set(ids);
  h.view.born = {};
  h.view.pressed = null;
  h.view.over = null;
  const git = (id: string, repo: string, linked: boolean) => {
    const s = h.snapshot.find(s => s.id === id)!;
    h.view.git[JSON.stringify([s.machine ?? null, s.folder])] = { repo, linked };
  };
  const flush = () => {
    for (const { id, index } of h.moves.splice(0)) {
      const from = h.snapshot.findIndex(s => s.id === id);
      h.snapshot.splice(index - 1, 0, h.snapshot.splice(from, 1)[0]);
    }
  };
  const rows = () => h.view.ordered();
  const order = () => h.snapshot.map(s => s.id);
  return { ...h, git, flush, rows, order };
}

test("the sidebar reconciles a full batch once, then the core echo is a no-op", () => {
  const h = orderedHarness(["other", "a", "main", "tail", "b"]);
  h.git("main", "repo", false);
  h.git("a", "repo", true);
  h.git("b", "repo", true);
  expect(h.rows().map((r: any) => r.w.id)).toEqual(["other", "main", "a", "b", "tail"]);
  expect(h.moves).toEqual([{ id: "main", index: 2 }, { id: "b", index: 4 }]);
  h.rows();
  expect(h.moves).toHaveLength(2);
  h.flush();
  h.rows();
  expect(h.moves).toEqual([]);
  expect(h.order()).toEqual(["other", "main", "a", "b", "tail"]);
});

test("unshown spaces are preserved and orphan worktrees stay in model order", () => {
  const h = orderedHarness(["new", "orphan", "a", "main"]);
  h.view.shown.delete("new");
  h.view.born.new = Date.now();
  h.git("orphan", "elsewhere", true);
  h.git("main", "repo", false);
  h.git("a", "repo", true);
  expect(h.rows().map((r: any) => r.w.id)).toEqual(["orphan", "main", "a"]);
  h.flush();
  expect(h.order()).toEqual(["new", "orphan", "main", "a"]);
});

test("machines and duplicate main-checkout spaces keep their own order", () => {
  const h = orderedHarness(["a", "main", "main2", "remote-a", "remote-main"]);
  for (const s of h.snapshot) if (s.id.startsWith("remote-")) s.machine = "cloud";
  h.git("main", "local-repo", false);
  h.git("main2", "local-repo", false);
  h.git("a", "local-repo", true);
  h.git("remote-main", "remote-repo", false);
  h.git("remote-a", "remote-repo", true);
  h.rows();
  h.flush();
  expect(h.order()).toEqual(["main", "a", "main2", "remote-main", "remote-a"]);
  expect(h.rows().filter((r: any) => r.tree).map((r: any) => r.w.id)).toEqual(["a", "remote-a"]);
  expect(h.moves).toEqual([]);
});

test("dragging a main checkout moves the right id and its children follow", () => {
  const h = orderedHarness(["other", "topic", "main", "last"]);
  h.git("main", "repo", false);
  h.git("topic", "repo", true);
  const first = h.rows();
  h.flush();
  // The row was built before reconciliation: neither its old index nor the
  // dragged row's old index may be used to resolve the drop.
  h.view.pressed = "main";
  h.view.over = "last";
  h.view.row(first.find((r: any) => r.w.id === "last"), {});
  h.handlers.on_mouse_up({}, h.cx);
  expect(h.moves).toEqual([{ id: "main", index: 4 }]);
  h.flush();
  h.rows();
  h.flush();
  expect(h.order()).toEqual(["other", "last", "main", "topic"]);
  h.rows();
  expect(h.moves).toEqual([]);
});

test("a machine becoming ready retries a previously rejected batch", () => {
  const h = orderedHarness(["topic", "main"]);
  h.git("main", "repo", false);
  h.git("topic", "repo", true);
  h.availability[0].layout = "unavailable";
  h.rows();
  h.moves.splice(0); // The unavailable machine rejected the moves.
  h.rows();
  expect(h.moves).toEqual([]);
  h.availability[0].layout = "ready";
  h.rows();
  expect(h.moves).toEqual([{ id: "main", index: 1 }]);
  h.flush();
  h.rows();
  expect(h.moves).toEqual([]);
});

test("a lost write retries after settling even without an intermediate ordered render", () => {
  const now = spyOn(Date, "now").mockReturnValue(10000);
  try {
    const h = orderedHarness(["topic", "main"]);
    h.git("main", "repo", false);
    h.git("topic", "repo", true);
    h.rows();
    h.flush();
    // A mirror conflict restores the original same-ID order before rendering.
    h.snapshot.reverse();
    now.mockReturnValue(10999);
    h.rows();
    expect(h.moves).toEqual([]);
    now.mockReturnValue(11000);
    let notified = false;
    h.view.due = () => new Set();
    h.view.checkNew({ notify() { notified = true; } });
    expect(notified).toBe(true);
    h.rows();
    expect(h.moves).toEqual([{ id: "main", index: 1 }]);
    h.rows();
    expect(h.moves).toHaveLength(1);
    h.flush();
    h.rows();
    now.mockReturnValue(12000);
    h.rows();
    expect(h.moves).toEqual([]);
    expect(h.view.ordering).toBeNull();
  } finally {
    now.mockRestore();
  }
});

test("settled retries wait for the affected machine to be ready", () => {
  const now = spyOn(Date, "now").mockReturnValue(10000);
  try {
    const h = orderedHarness(["topic", "main"]);
    h.git("main", "repo", false);
    h.git("topic", "repo", true);
    h.availability[0].layout = "unavailable";
    h.rows();
    h.moves.splice(0);
    now.mockReturnValue(12000);
    expect(h.view.orderingDue(Date.now())).toBe(false);
    h.rows();
    expect(h.moves).toEqual([]);
    h.availability[0].layout = "ready";
    h.rows();
    expect(h.moves).toEqual([{ id: "main", index: 1 }]);
  } finally {
    now.mockRestore();
  }
});

test("no reconciliation without a rendered sidebar", () => {
  const h = orderedHarness(["topic", "main"]);
  h.git("main", "repo", false);
  h.git("topic", "repo", true);
  expect(h.moves).toEqual([]);
  h.rows();
  h.flush();
  expect(h.order()).toEqual(["main", "topic"]);
});

// checkNew runs every 500 ms: it must cost nothing while the host's revision stands still, and
// keep reacting to events (a new space, a finished command) while the window is in the background.
function watching(active = true) {
  const h = harness([], new Map(), [{ id: "a", folder: "/a" }]);
  const timers: Record<number, (cx: unknown) => void> = {};
  const cx = { timer: { every(ms: number, f: (cx: unknown) => void) { timers[ms] = f; } }, notify() {}, spawn() {} };
  const refreshes = { machines: 0, git: 0 };
  h.view.refreshMachines = () => { refreshes.machines++; };
  h.view.refreshGit = () => { refreshes.git++; };
  h.view.git = {};
  h.win.visible = active;
  h.view.init({}, cx);
  const started = refreshes.machines;
  refreshes.machines = 0;
  const due: Set<string>[] = [];
  h.view.due = () => { const d = new Set<string>(); due.push(d); return d; };
  return { ...h, timers, refreshes, due, cx, started };
}

test("checkNew reads no snapshot while the revision is unchanged, then looks again", () => {
  const h = watching();
  h.view.checkNew(h.cx);
  expect(h.due).toHaveLength(1);
  h.view.shown = new Set(["a"]);
  h.view.wake = Infinity;
  const reads = h.reads.spaces + h.reads.panes;
  for (let i = 0; i < 20; i++) h.view.checkNew(h.cx);
  expect(h.due).toHaveLength(1);
  expect(h.reads.spaces + h.reads.panes).toBe(reads);
  h.revision.n++;
  h.view.checkNew(h.cx);
  expect(h.due).toHaveLength(2);
  h.view.checkNew(h.cx);
  expect(h.due).toHaveLength(2);
});

test("a revision that goes down or wraps still counts as a change", () => {
  const h = watching();
  h.revision.n = 5;
  h.view.checkNew(h.cx);
  h.view.shown = new Set(["a"]);
  h.view.wake = Infinity;
  h.revision.n = 0; // the host restarted its count
  h.view.checkNew(h.cx);
  expect(h.due).toHaveLength(2);
  h.revision.n = Number.MAX_SAFE_INTEGER;
  h.view.checkNew(h.cx);
  expect(h.due).toHaveLength(3);
  h.revision.n = 0; // wrapped
  h.view.checkNew(h.cx);
  expect(h.due).toHaveLength(4);
});

test("a space that is not yet shown keeps being looked at without a new revision", () => {
  const h = watching();
  h.view.checkNew(h.cx);
  h.view.wake = Infinity;
  expect(h.view.ids).toEqual(["a"]);
  h.view.shown = new Set(); // still waiting for its git
  h.view.checkNew(h.cx);
  expect(h.due).toHaveLength(2);
});

test("a new space is noticed while the window is in the background", () => {
  const h = watching();
  h.view.checkNew(h.cx);
  h.view.shown = new Set(["a"]);
  h.view.wake = Infinity;
  h.win.visible = false;
  h.snapshot.push({ id: "b", folder: "/b" });
  h.revision.n++;
  h.view.checkNew(h.cx);
  expect(h.due).toHaveLength(2);
  expect(h.view.ids).toEqual(["a", "b"]);
});

test("machines are not refreshed while the window is in the background, and catch up at once when it is back", () => {
  const h = watching();
  h.win.visible = false;
  h.timers[15000](h.cx);
  h.timers[500](h.cx);
  expect(h.refreshes.machines).toBe(0);
  h.win.visible = true;
  h.revision.n++; // the window snapshot changed
  h.timers[500](h.cx);
  expect(h.refreshes.machines).toBe(1);
  h.timers[500](h.cx);
  h.timers[15000](h.cx);
  expect(h.refreshes.machines).toBe(2); // the regular timer is back to normal
});

test("the machines timer refreshes as before while the window is active", () => {
  const h = watching();
  h.timers[15000](h.cx);
  h.timers[15000](h.cx);
  expect(h.refreshes.machines).toBe(2);
});

test("started while the window is in the background: machines are asked once at start and once when it is back", () => {
  const h = watching(false);
  expect(h.started).toBe(1);
  for (let i = 0; i < 4; i++) h.timers[15000](h.cx);
  h.timers[500](h.cx);
  expect(h.refreshes.machines).toBe(0);
  h.win.visible = true;
  h.revision.n++;
  h.timers[500](h.cx);
  h.timers[500](h.cx);
  expect(h.refreshes.machines).toBe(1);
});

test("two hours in the background ends in one machines refresh and one look, while new spaces are still noticed", () => {
  const h = watching();
  h.view.checkNew(h.cx);
  h.view.shown = new Set(["a"]);
  h.view.wake = Infinity;
  h.win.visible = false;
  h.revision.n++; // the window snapshot
  for (let i = 0; i < 3600 * 4; i++) {
    if (i === 7200) {
      h.snapshot.push({ id: "b", folder: "/b" });
      h.revision.n += 2; // spaces and panes in the same tick
      h.view.shown.add("b");
    }
    h.timers[500](h.cx);
    if (i % 60 === 0) h.timers[15000](h.cx);
  }
  expect(h.refreshes.machines).toBe(0);
  expect(h.due).toHaveLength(3); // the first look, the window going away, the new space
  expect(h.view.ids).toEqual(["a", "b"]);
  h.win.visible = true;
  h.revision.n++;
  for (let i = 0; i < 10; i++) h.timers[500](h.cx);
  expect(h.refreshes.machines).toBe(1);
  expect(h.due).toHaveLength(4);
});

test("a visible but unfocused window keeps refreshing machines; a minimized one does not", () => {
  const h = watching();
  (h.win as any).active = false; // focus is not what the timers look at
  h.timers[15000](h.cx);
  expect(h.refreshes.machines).toBe(1);
  h.win.visible = false;
  h.timers[15000](h.cx);
  expect(h.refreshes.machines).toBe(1);
});

// IDLE-VIS: the look for new spaces moved from every 250 ms to every 500 ms; what it notices did not change.
test("the spaces mod sets up exactly two timers: the 500 ms look and the 15 s machines refresh", () => {
  const h = watching();
  expect(Object.keys(h.timers).map(Number).sort((a, b) => a - b)).toEqual([500, 15000]);
  expect(h.timers[250]).toBeUndefined(); // the old cadence is gone, not kept beside the new one
  expect(h.timers[499]).toBeUndefined();
  expect(h.timers[1000]).toBeUndefined();
});

test("each 500 ms tick is one checkNew with the timer's cx, nothing more", () => {
  const h = watching();
  const check = spyOn(h.view, "checkNew");
  h.timers[500](h.cx);
  expect(check).toHaveBeenCalledTimes(1);
  expect(check.mock.calls[0][0]).toBe(h.cx);
  h.timers[500](h.cx);
  expect(check).toHaveBeenCalledTimes(2);
});

for (const visible of [true, false]) {
  test(`a new space is noticed on the very next 500 ms tick (window ${visible ? "visible" : "in the background"})`, () => {
    const h = watching();
    h.timers[500](h.cx);
    h.view.shown = new Set(["a"]);
    h.view.wake = Infinity;
    h.win.visible = visible;
    h.timers[500](h.cx); // nothing new yet
    const looks = h.due.length;
    h.snapshot.push({ id: "b", folder: "/b" });
    h.revision.n++;
    h.timers[500](h.cx); // one period later
    expect(h.due).toHaveLength(looks + 1);
    expect(h.view.ids).toEqual(["a", "b"]);
  });
}

test("from no spaces at all, the first space is noticed within one tick", () => {
  const h = harness([], new Map(), []);
  const timers: Record<number, (cx: unknown) => void> = {};
  const cx = { timer: { every(ms: number, f: (cx: unknown) => void) { timers[ms] = f; } }, notify() {}, spawn() {} };
  h.view.refreshGit = () => {};
  h.view.init({}, cx);
  timers[500](cx);
  expect(h.view.ids).toEqual([]);
  h.snapshot.push({ id: "a", folder: "/a" });
  h.revision.n++;
  timers[500](cx);
  expect(h.view.ids).toEqual(["a"]);
});

test("ticks with an unchanged revision read no snapshot, however many there are", () => {
  const h = watching();
  h.timers[500](h.cx);
  h.view.shown = new Set(["a"]);
  h.view.wake = Infinity;
  const reads = h.reads.spaces + h.reads.panes;
  for (let i = 0; i < 120; i++) h.timers[500](h.cx); // a minute at the new cadence
  expect(h.reads.spaces + h.reads.panes).toBe(reads);
  expect(h.due).toHaveLength(1);
});

test("git checks run a few spaces at a time, not every space at once", async () => {
  let running = 0, most = 0;
  const execGit = async () => {
    running++;
    most = Math.max(most, running);
    await new Promise((resolve) => setTimeout(resolve, 1));
    running--;
    return { code: 0, stdout: "/repo\n", stderr: "" };
  };
  const h = harness([], new Map(), [], execGit);
  Object.assign(h.view, { git: {}, checked: {}, unsure: new Set(), pending: new Set(), watching: new Set() });
  h.view.watch = () => {};
  const keys = Array.from({ length: 40 }, (_, i) => JSON.stringify([null, `/repo/${i}`]));
  h.view.refreshGit(keys, h.cx);
  await Promise.all(h.tasks);
  expect(Object.values(h.view.git).filter(Boolean)).toHaveLength(40);
  expect(most).toBeLessThanOrEqual(12);
});

// The list scrolls by its direct children, so the active space's place counts machine headers too.
function scrollHarness(spaces: Space[]) {
  const h = harness([], new Map(), spaces);
  Object.assign(h.view, { git: {}, checked: {}, unsure: new Set(), pending: new Set(), watching: new Map(), folded: new Set(), machines: [] });
  h.view.shown = new Set(spaces.map(s => s.id));
  h.view.born = {};
  return h;
}
const withHandle = (h: ReturnType<typeof harness>) => { h.view.scroll = { calls: [] as number[], scroll_to_item(i: number) { this.calls.push(i); } }; return h; };
const spaceList = (ids: string[], active: string, remote: string[] = []): Space[] =>
  ids.map(id => ({ id, folder: "/" + id, active: id === active, ...(remote.includes(id) ? { machine: "cloud" } : {}) }));

test("the active space is brought into view once, and left alone while it stays put", () => {
  const h = withHandle(scrollHarness(spaceList(["a", "b", "c", "d"], "c")));
  h.view.render();
  expect(h.view.scroll.calls).toEqual([2]);
  expect(h.tracked).toEqual([h.view.scroll]); // the list is the area that handle scrolls
  h.view.render();
  h.view.render();
  expect(h.view.scroll.calls).toEqual([2]);
  h.snapshot.forEach(s => { s.active = s.id === "a"; });
  h.view.render();
  expect(h.view.scroll.calls).toEqual([2, 0]);
});

test("the active space's place counts a remote machine's header and rows", () => {
  const ids = ["a", "b", "r1", "r2"];
  const h = withHandle(scrollHarness(spaceList(ids, "r2", ["r1", "r2"])));
  h.view.render();
  // children: this computer's header, a, b, cloud's header, r1, r2
  expect(h.view.scroll.calls).toEqual([5]);
});

test("a folded machine's active space has no row to reveal", () => {
  const h = withHandle(scrollHarness(spaceList(["a", "r1"], "r1", ["r1"])));
  h.view.folded = new Set(["cloud"]);
  h.view.render();
  expect(h.view.scroll.calls).toEqual([]);
});

test("the narrow rail reveals the same child", () => {
  const h = withHandle(scrollHarness(spaceList(["a", "b", "c"], "c")));
  h.env.layout = { id: "sidebar", size: { px: 100 } };
  h.view.render();
  expect(h.view.scroll.calls).toEqual([2]);
});

test("the list reveals the active space again when its own size changes, and only then", () => {
  const h = withHandle(scrollHarness(spaceList(["a", "b", "c", "d"], "d")));
  let notified = 0;
  h.cx.notify = () => { notified++; };
  h.view.render();
  expect(h.view.scroll.calls).toEqual([3]);
  h.handlers.on_size_change({ width: 200, height: 60 }, h.cx);
  expect(h.view.scroll.calls).toEqual([3, 3]);
  expect(notified).toBe(1);
  h.view.render();
  expect(h.view.scroll.calls).toEqual([3, 3]);
});

test("a size change with no active row drawn asks for nothing", () => {
  const h = withHandle(scrollHarness(spaceList(["a", "r1"], "r1", ["r1"])));
  h.view.folded = new Set(["cloud"]);
  h.view.render();
  h.handlers.on_size_change({ width: 200, height: 60 }, h.cx);
  expect(h.view.scroll.calls).toEqual([]);
});

test("the list reveals the active space again when a machine header shifts its place", () => {
  const h = withHandle(scrollHarness(spaceList(["a", "b", "c", "d"], "b")));
  h.view.render();
  expect(h.view.scroll.calls).toEqual([1]);
  // A machine joins: its header and rows come after this computer's, and now this computer gets a header too,
  // so the same active space sits one child further down.
  h.snapshot.push({ id: "r1", folder: "/r1", machine: "cloud", active: false });
  h.view.shown.add("r1");
  h.view.render();
  expect(h.view.scroll.calls).toEqual([1, 2]);
  h.view.render();
  expect(h.view.scroll.calls).toEqual([1, 2]);
});

// Edge cases (UPSTREAM-SYNC-2 A3, A4): git-check limits, revealing the active space,
// long names, the last known repo and unreadable saved values.
function countingGit(fail: (args: string[], cwd: string) => boolean = () => false, ms = 1) {
  const seen = { running: 0, most: 0, calls: 0 };
  const execGit = async (_command: string, args: string[], { cwd }: { cwd: string }) => {
    seen.calls++;
    seen.running++;
    seen.most = Math.max(seen.most, seen.running);
    try {
      if (fail(args, cwd)) throw new Error("connection reset");
      await new Promise((resolve) => setTimeout(resolve, ms));
      return { code: 0, stdout: args.includes("--git-common-dir") ? "/repo/.git\n" : args[0] === "symbolic-ref" ? "refs/heads/main\n" : args[0] === "rev-list" ? "0\t0\n" : `${cwd}\n`, stderr: "" };
    } finally {
      seen.running--;
    }
  };
  return { seen, execGit };
}
function gitHarness(execGit: (...args: any[]) => Promise<unknown>) {
  const h = harness([], new Map(), [], execGit);
  Object.assign(h.view, { git: {}, checked: {}, unsure: new Set(), pending: new Set(), watching: new Set() });
  h.view.watch = () => {};
  return h;
}
const placeKeys = (n: number) => Array.from({ length: n }, (_, i) => JSON.stringify([null, `/repo/${i}`]));

test("no places to check runs no git at all", async () => {
  const { seen, execGit } = countingGit();
  const h = gitHarness(execGit);
  h.view.refreshGit([], h.cx);
  await Promise.all(h.tasks);
  expect(seen.calls).toBe(0);
  expect(h.view.refreshing).toBe(false);
});

for (const n of [1, 6, 7, 13]) test(`${n} places are all checked, never more than six at once`, async () => {
  const { seen, execGit } = countingGit();
  const h = gitHarness(execGit);
  h.view.refreshGit(placeKeys(n), h.cx);
  await Promise.all(h.tasks);
  expect(Object.values(h.view.git).filter(Boolean)).toHaveLength(n);
  // Two git runs per place: six places hold at most twelve.
  expect(seen.most).toBeLessThanOrEqual(Math.min(n, 6) * 2);
  expect(seen.calls).toBe(n * 4);
});

test("a place whose git fails keeps its slot until both runs end, and the rest still get checked", async () => {
  const { seen, execGit } = countingGit((args, cwd) => cwd === "/repo/3" && args.includes("--show-toplevel"), 5);
  const h = gitHarness(execGit);
  const keys = placeKeys(20);
  h.view.git[keys[3]] = { name: "kept", repo: JSON.stringify([null, "/repo/.git"]) };
  h.view.refreshGit(keys, h.cx);
  await Promise.all(h.tasks);
  expect(seen.most).toBeLessThanOrEqual(12);
  expect(Object.values(h.view.git).filter(Boolean)).toHaveLength(20);
  // A failure to ask keeps the last known repo and asks again later.
  expect(h.view.git[keys[3]].name).toBe("kept");
  expect(h.view.unsure.has(keys[3])).toBe(true);
});

test("a place that was never a repo and can't be asked stays unknown, not a repo", async () => {
  const { execGit } = countingGit(() => true);
  const h = gitHarness(execGit);
  const [key] = placeKeys(1);
  await h.view.check(key, h.cx);
  expect(h.view.git[key]).toBeNull();
  expect(h.view.unsure.has(key)).toBe(true);
});

test("the fallback look waits exactly its minute, and not at all while hidden or folded", () => {
  const snapshot = [{ id: "a", folder: "/a" }, { id: "r", folder: "/r", machine: "cloud" }];
  const h = harness([], new Map(), snapshot);
  h.view.init({}, h.cx);
  const local = JSON.stringify([null, "/a"]), remote = JSON.stringify(["cloud", "/r"]);
  const at = 1_000_000;
  for (const key of [local, remote]) { h.view.git[key] = { name: "a", repo: JSON.stringify([null, "/a/.git"]) }; h.view.checked[key] = at; }
  h.view.folded = new Set(["cloud"]);
  h.view.due(at, snapshot, []);
  expect(h.view.due(at + 60000, snapshot, []).has(local)).toBe(false);
  expect(h.view.wake).toBe(at + 60000);
  expect(h.view.due(at + 60001, snapshot, []).has(local)).toBe(true);
  expect(h.view.due(at + 600000, snapshot, []).has(remote)).toBe(false);
  h.win.visible = false;
  expect(h.view.due(at + 600000, snapshot, []).has(local)).toBe(false);
  expect(h.view.wake).toBe(Infinity);
});

test("unreadable saved machine state starts empty instead of throwing", () => {
  for (const bad of ["not json", "{", "null", "[]", "5", "\"text\"", "true", "{\"cloud\":null}", "{\"cloud\":5}"]) {
    const h = harness([], new Map([["machines.reconnecting", bad]]));
    h.view.init({}, h.cx);
    expect(h.view.reconnecting).toEqual({});
  }
  for (const bad of ["not json", "{", "null", "{}", "5", "true", "\"cloud\""]) {
    const h = harness([], new Map([["machines.folded", bad]]));
    h.view.init({}, h.cx);
    expect([...h.view.folded]).toEqual([]);
  }
  const good = harness([], new Map([["machines.folded", "[\"cloud\"]"], ["machines.reconnecting", JSON.stringify({ cloud: { machine, busy: true } })]]));
  good.view.init({}, good.cx);
  expect([...good.view.folded]).toEqual(["cloud"]);
  expect(good.view.reconnecting.cloud.busy).toBe(false);
});

test("repo names come from the git folder, bare or not, on any platform", () => {
  const repoName = new Function("View", `${source}; return repoName;`)(class {});
  expect(repoName("/home/me/proj/.git")).toBe("proj");
  expect(repoName("C:/Users/me/My Project/.git")).toBe("My Project");
  expect(repoName("//server/share/proj/.git")).toBe("proj");
  expect(repoName("/srv/bare.git")).toBe("bare");
  expect(repoName("/srv/bare.git/")).toBe("bare");
  expect(repoName("/srv/proj/.bare")).toBe("proj");
  expect(repoName("D:/work/proj/.bare")).toBe("proj");
  expect(repoName("/home/me/\uc74c\uc545/.git")).toBe("\uc74c\uc545");
  expect(repoName("/srv/plain")).toBe("plain");
});

test("no spaces at all reveal nothing, and a size change asks for nothing", () => {
  const h = withHandle(scrollHarness([]));
  h.view.render();
  expect(h.view.scroll.calls).toEqual([]);
  h.handlers.on_size_change({ width: 200, height: 60 }, h.cx);
  expect(h.view.scroll.calls).toEqual([]);
});

test("one space, and the active space at either end of a long list, are revealed", () => {
  const one = withHandle(scrollHarness(spaceList(["a"], "a")));
  one.view.render();
  expect(one.view.scroll.calls).toEqual([0]);
  const ids = Array.from({ length: 50 }, (_, i) => `s${i}`);
  const h = withHandle(scrollHarness(spaceList(ids, "s49")));
  h.view.render();
  h.snapshot.forEach(s => { s.active = s.id === "s0"; });
  h.view.render();
  expect(h.view.scroll.calls).toEqual([49, 0]);
});

for (const px of [-20, 0, Number.NaN, null]) test(`a sidebar width of ${px} still draws and reveals the active space`, () => {
  const h = withHandle(scrollHarness(spaceList(["a", "b"], "b")));
  h.env.layout = { id: "sidebar", size: { px } };
  expect(() => h.view.render()).not.toThrow();
  expect(h.view.scroll.calls).toEqual([1]);
});

// A recording element tree for one row, to see how the name and the git tail are laid out.
function rowView(sharedValues: Record<string, unknown> = {}) {
  const make = () => {
    const node: any = { calls: {} as Record<string, unknown[]>, children: [] as any[] };
    const el: any = new Proxy(node, { get: (_, method) => {
      if (method === "node") return node;
      return (...args: any[]) => {
        if (method === "child") node.children.push(args[0]?.node ?? args[0]);
        else if (method === "children") node.children.push(...args[0].map((c: any) => c?.node ?? c));
        else if (method === "when") { if (args[0]) args[1](el); }
        else node.calls[String(method)] = args;
        return el;
      };
    } });
    return el;
  };
  const Spaces = new Function("View", "core", "localStorage", "spaces", "panes", "run", "v_flex", "h_flex", "div", "svg", "shared", "machines", "windowVisible", "revision", "exec", "ScrollHandle", "theme", "layout", "setShared", `${source}; return Spaces;`)(
    class {}, async () => ({}), { getItem: () => null, setItem() {} }, () => [], () => [], () => {},
    make, make, make, make, (key: string) => sharedValues[key] ?? null, () => [], () => true, () => 0, async () => ({}),
    { new: () => ({ scroll_to_item() {} }) }, () => ({}), () => null, () => {},
  );
  const view = new Spaces();
  Object.assign(view, { pressed: null, over: null });
  const t = new Proxy({}, { get: () => "#000000" });
  const all = (node: any): any[] => [node, ...(node?.children ?? []).flatMap(all)];
  const holding = (root: any, text: string) => all(root).find((n: any) => n?.children?.includes(text));
  const row = (w: Record<string, unknown>, g: unknown = null, tree: unknown = null) =>
    view.row({ w: { id: "s1", folder: "/x", active: false, ...w }, i: 0, g, tree }, t).node;
  return { view, t, all, holding, row };
}

test("a long space name is truncated with room to give, not a fixed cap", () => {
  const { row, holding } = rowView();
  const long = "quarterly infrastructure cost review and the platform migration plan ".repeat(8).trim();
  const label = holding(row({ name: long }), long);
  expect(label).toBeDefined();
  expect("truncate" in label.calls).toBe(true);
  expect(label.calls.min_w).toEqual([48]);
  expect("max_w" in label.calls).toBe(false);
  expect("flex_shrink" in label.calls).toBe(false);
});

test("a row's avatar letter handles empty, accented and emoji names", () => {
  const { row, holding } = rowView();
  expect(holding(row({ name: "" }), "?")).toBeDefined();
  expect(holding(row({ name: "\u00e9lan" }), "\u00c9")).toBeDefined();
  expect(holding(row({ name: "\ud83d\ude80 launch" }), "\ud83d\ude80")).toBeDefined();
});

test("a long branch is truncated in the git tail, its counts keep their size", () => {
  const { view, t, holding } = rowView();
  const branch = "feature/" + "rate-limiting-".repeat(30);
  const tail = view.gitTail({ name: "repo", branch, ahead: 2, behind: 0 }, null, "api", t).node;
  expect("flex_auto" in tail.calls).toBe(true);
  expect(tail.calls.min_w).toEqual([48]);
  const where = holding(tail, `repo \u00b7 ${branch}`);
  expect("truncate" in where.calls).toBe(true);
  expect(where.calls.min_w).toEqual([0]);
  expect("flex_none" in holding(tail, "\u21912").calls).toBe(true);
  const quiet = view.gitTail({ name: "api", branch: "main", ahead: 0, behind: 0 }, null, "api", t).node;
  expect(quiet.calls.min_w).toEqual([0]);
  expect(quiet.children).toEqual([]);
});

test("only a finite positive session count draws a badge", () => {
  const at = JSON.stringify([null, "/x"]);
  for (const count of [Number.NaN, -2, 0, "3", null, Infinity]) {
    const { row, all } = rowView({ "sessions.count": { [at]: count } });
    expect(all(row({ name: "api" })).some((n: any) => n?.calls?.id?.[0] === "space-sessions-s1")).toBe(false);
  }
  for (const counts of ["garbage", 5, null]) {
    const { row, all } = rowView({ "sessions.count": counts });
    expect(all(row({ name: "api" })).some((n: any) => n?.calls?.id?.[0] === "space-sessions-s1")).toBe(false);
  }
  const { row, holding } = rowView({ "sessions.count": { [at]: 3 } });
  expect(holding(row({ name: "api" }), "3").calls.id).toEqual(["space-sessions-s1"]);
});

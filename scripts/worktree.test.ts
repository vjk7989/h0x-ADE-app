import { test, expect } from "bun:test";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

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

function harness(focusedMachine: string | null = null, overrides: Record<string, unknown> = {}) {
  let spaceList: any[] = [];
  let paneList: any[] = [];
  const reads = { spaces: 0, panes: 0 };
  const writes: unknown[] = [];
  const destinations: (string | null)[] = [];
  const executions: any[] = [];
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
    exec: async (command: string, args: string[], options: any) => {
      executions.push({ command, args, ...options });
      return { code: 0, stdout: "", stderr: "" };
    },
    ...overrides,
  };
  const Worktree = new Function(...Object.keys(globals), `${source}; return Worktree;`)(...Object.values(globals));
  const worktree = new Worktree();
  Object.assign(worktree, { opening: true, text: "", at: 0, repo: { name: "repo" }, userHome: "/home/user" });
  return { worktree, reads, writes, destinations, executions, snapshot: (spaces: any[], panes: any[]) => { spaceList = spaces; paneList = panes; } };
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
      const { worktree, destinations, executions } = harness(focused);
      Object.assign(worktree, {
        on, text: "feature", repo: { name: "repo", main: "/repo", top: "/repo" }, list: [],
      });
      expect(await worktree.create()).toBe("");
      expect(executions).toHaveLength(2);
      expect(executions.every(e => (e.on ?? null) === (on ?? null))).toBe(true);
      expect(executions[1].args).toEqual(["worktree", "add", "/home/user/.h0x/worktrees/repo/feature", "feature"]);
      worktree.open({ path: "/repo/existing" });
      expect(destinations).toEqual([on ?? null, on ?? null]);
    });
  }
}

for (const [branch, folder] of [
  ["음악수행", "음악수행"], ["쪼지학", "쪼지학"],
  ["हिन्दी", "हिन्दी"], ["हिंदी", "हिंदी"],
  ["का", "का"], ["कि", "कि"],
  ["বাংলা", "বাংলা"],
  ["feature/हिन्दी", "feature-हिन्दी"],
  ["feature/음악수행-１２٣", "feature-음악수행-１２٣"],
  ["음악수행".normalize("NFD"), "음악수행"],
  ["feature/ASCII_1.2--fix", "feature-ASCII_1.2--fix"],
]) {
  test(`branch ${branch} uses folder ${folder}`, async () => {
    const { worktree, executions, writes } = harness();
    Object.assign(worktree, { text: branch, repo: { name: "repo", main: "/repo", top: "/repo" } });
    expect(await worktree.create()).toBe("");
    const path = `/home/user/.h0x/worktrees/repo/${folder}`;
    expect(executions[1].args).toEqual(["worktree", "add", path, branch]);
    expect(writes[0][1].folder).toBe(path);
  });
}

test("symbol-only branches get stable distinct non-empty folders", async () => {
  const folders: string[] = [];
  for (const branch of ["🎵", "🎶", "🎵"]) {
    const { worktree, executions } = harness();
    Object.assign(worktree, { text: branch, repo: { name: "repo", main: "/repo", top: "/repo" } });
    expect(await worktree.create()).toBe("");
    const path = executions[1].args[2];
    expect(path).toMatch(/\/repo\/branch-[0-9a-f]{8}$/);
    folders.push(path);
  }
  expect(folders[0]).toBe(folders[2]);
  expect(folders[0]).not.toBe(folders[1]);
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

test("a bare repo hidden beside its worktrees is named after its project", async () => {
  const replies: Record<string, string> = {
    "--show-toplevel": "/home/user/project/topic",
    "--git-common-dir": "/home/user/project/.bare",
  };
  const exec = async (_command: string, args: string[]) => ({ code: 0, stdout: replies[args.at(-1)!] + "\n", stderr: "" });
  const globals = { View: class {}, div: element, v_flex: element, h_flex: element, Input: { new: element }, Scrollbar: { vertical: element }, theme: () => ({}), exec };
  const repoOf = new Function(...Object.keys(globals), `${source}; return repoOf;`)(...Object.values(globals));
  expect(await repoOf("/home/user/project/topic", null)).toEqual({
    top: "/home/user/project/topic", main: "/home/user/project/.bare", name: "project", linked: true,
  });
});

function execute(command: string, args: string[], { cwd }: { cwd: string }) {
  const result = Bun.spawnSync([command, ...args], { cwd, stdout: "pipe", stderr: "pipe" });
  return { code: result.exitCode, stdout: result.stdout.toString(), stderr: result.stderr.toString() };
}

function fixtureGit(cwd: string, ...args: string[]) {
  const result = execute("git", args, { cwd });
  if (result.code !== 0) throw new Error(result.stderr);
  return result.stdout.trim();
}

async function withLinkedCheckout(runTest: (main: string, linked: string) => Promise<void>) {
  const folder = await mkdtemp(join(process.platform === "linux" ? "/var/tmp" : tmpdir(), "h0x-worktree-"));
  try {
    const root = await realpath(folder);
    const main = join(root, "main"), linked = join(root, "linked with spaces");
    fixtureGit(root, "init", "--initial-branch=main", main);
    fixtureGit(main, "-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "--no-gpg-sign", "--allow-empty", "-m", "fixture");
    fixtureGit(main, "worktree", "add", "-b", "linked", linked);
    await runTest(main, linked);
  } finally {
    await rm(folder, { recursive: true, force: true });
  }
}

function removeDialog(folder: string, { branch, platform = "linux", exec = execute }: {
  branch?: string; platform?: string; exec?: typeof execute;
} = {}) {
  const closedSpaces: { id: string; checkoutPresent: boolean }[] = [];
  const closedOverlays: string[] = [];
  const { worktree, snapshot } = harness(null, {
    home: () => folder,
    window: () => ({ platform }),
    props: () => ({ space: "target", action: "remove", overlay: "dialog", branch }),
    InputState: { new: () => ({ on() {}, focus() {} }) },
    run: (command: string, args: { id: string }) => {
      if (command === "space.close") closedSpaces.push({ id: args.id, checkoutPresent: existsSync(folder) });
    },
    closeOverlay: (id: string) => closedOverlays.push(id),
    exec,
  });
  snapshot([{ id: "target", name: "Worktree", folder }], []);
  let pending = Promise.resolve();
  const cx = {
    notify() {},
    focus_handle: () => ({ focus() {} }),
    spawn: (task: (context: { notify(): void }) => Promise<void>) => { pending = task(cx); },
  };
  worktree.init({}, cx);
  return {
    worktree, closedSpaces, closedOverlays, ready: pending,
    enter: async () => { worktree.key({ key: "enter" }, cx); await pending; },
  };
}

test("remove leaves the main checkout and its space alone on Enter and with a branch prop", async () => {
  await withLinkedCheckout(async (main) => {
    for (const branch of [undefined, "main"]) {
      const dialog = removeDialog(main, { branch });
      await dialog.ready;
      if (!branch) await dialog.enter();
      expect(dialog.closedSpaces).toEqual([]);
      expect(dialog.closedOverlays).toEqual([]);
      expect(textOf(dialog.worktree.render({}).node)).toContain("This is the repository's main checkout");
      expect(dialog.worktree.error).toBe("");
      expect(existsSync(main)).toBe(true);
    }
  });
});

test("remove shows a locked checkout's reason and keeps the space open on Windows", async () => {
  await withLinkedCheckout(async (main, linked) => {
    fixtureGit(main, "worktree", "lock", "--reason", "Réservé for another machine\nLeave this checkout in place", linked);
    const dialog = removeDialog(linked, { platform: "windows" });
    await dialog.ready;
    await dialog.enter();
    const message = textOf(dialog.worktree.render({}).node);
    expect(message).toContain("This worktree is locked:");
    expect(message).toContain("Réservé for another machine");
    expect(message).toContain("Leave this checkout in place");
    expect(dialog.closedSpaces).toEqual([]);
    expect(dialog.closedOverlays).toEqual([]);
    expect(existsSync(linked)).toBe(true);
  });
});

test("remove lets Git resolve the checkout when its listed path is spelled differently", async () => {
  await withLinkedCheckout(async (_main, linked) => {
    const top = fixtureGit(linked, "rev-parse", "--show-toplevel");
    const record = `worktree ${top}`;
    const differentlySpelled = record.replace(/linked with spaces$/, "LINKED WITH SPACES");
    expect(differentlySpelled).not.toBe(record);
    const dialog = removeDialog(linked, {
      exec: async (command, args, options) => {
        const result = await execute(command, args, options);
        if (args.includes("worktree") && args.includes("list") && args.includes("--porcelain")) {
          const records = result.stdout.split(/\r?\n/);
          expect(records).toContain(record);
          result.stdout = records.map(line => line === record ? differentlySpelled : line).join("\n");
        }
        return result;
      },
    });
    await dialog.ready;
    await dialog.enter();
    expect(dialog.worktree.error).toBe("");
    expect(dialog.closedSpaces).toEqual([{ id: "target", checkoutPresent: false }]);
    expect(existsSync(linked)).toBe(false);
  });
});

for (const platform of ["linux", "windows"]) {
  const closeBeforeGit = platform === "windows";
  test(`remove reports a late Git refusal on ${platform}`, async () => {
    await withLinkedCheckout(async (main, linked) => {
      const dialog = removeDialog(linked, {
        platform,
        exec: (command, args, options) => {
          if (args[0] === "worktree" && args[1] === "remove") {
            fixtureGit(main, "worktree", "lock", "--reason", "locked just before removal", linked);
          }
          return execute(command, args, options);
        },
      });
      await dialog.ready;
      await dialog.enter();
      expect(dialog.worktree.error).toContain("locked just before removal");
      expect(dialog.closedSpaces).toEqual(closeBeforeGit ? [{ id: "target", checkoutPresent: true }] : []);
      expect(dialog.closedOverlays).toEqual([]);
      expect(existsSync(linked)).toBe(true);
    });
  });

  test(`remove warns about new changes and closes a successfully removed checkout on ${platform}`, async () => {
    await withLinkedCheckout(async (main, linked) => {
      const dialog = removeDialog(linked, { platform });
      await dialog.ready;
      await writeFile(join(linked, "untracked.txt"), "uncommitted");
      await dialog.enter();
      expect(textOf(dialog.worktree.render({}).node)).toContain("It has uncommitted changes");
      expect(dialog.closedSpaces).toEqual([]);
      expect(existsSync(join(linked, "untracked.txt"))).toBe(true);

      await dialog.enter();
      expect(dialog.closedSpaces).toEqual([{ id: "target", checkoutPresent: closeBeforeGit }]);
      expect(dialog.closedOverlays).toEqual(["dialog"]);
      expect(existsSync(linked)).toBe(false);
      expect(fixtureGit(main, "branch", "--list", "linked")).toBe("linked");
    });
  });
}

// Edge cases (UPSTREAM-SYNC-2 A5, A6): Windows and remote paths, a missing folder,
// a dirty checkout, a bare repo and names with marks.
function scriptedRemove({ top, main = "/repos/main", on = null, platform = "linux", status = "", statusError = "", list, removeError = "", dirty = false, spaceThere = true }: {
  top: string; main?: string; on?: string | null; platform?: string; status?: string; statusError?: string;
  list?: string; removeError?: string; dirty?: boolean; spaceThere?: boolean;
}) {
  const events: string[] = [];
  const machines: (string | null)[] = [];
  const ok = (stdout = "") => ({ code: 0, stdout, stderr: "" });
  const exec = async (_command: string, args: string[], options: { on?: string | null }) => {
    machines.push(options.on ?? null);
    if (args.includes("status")) {
      events.push("status");
      return statusError ? { code: 128, stdout: "", stderr: statusError } : ok(status);
    }
    if (args.includes("--absolute-git-dir")) return ok(`${main}/.git/worktrees/topic\n`);
    if (args.includes("--get-regexp")) return { code: 1, stdout: "", stderr: "" };
    if (args.includes("list")) return ok(list ?? `worktree ${main}\nHEAD 1111\nbranch refs/heads/main\n\nworktree ${top}\nHEAD 2222\nbranch refs/heads/topic\n`);
    if (args[0] === "worktree" && args[1] === "remove") {
      events.push(`remove${args.includes("--force") ? " --force" : ""} ${args.at(-1)}`);
      return removeError ? { code: 1, stdout: "", stderr: removeError } : ok();
    }
    return ok();
  };
  const { worktree, snapshot } = harness(null, {
    exec, window: () => ({ platform }),
    props: () => ({ space: "target", action: "remove", overlay: "dialog" }),
    run: (command: string, args: { id: string }) => events.push(`${command} ${args.id}`),
  });
  snapshot(spaceThere ? [{ id: "target", name: "topic", folder: top, machine: on }] : [], []);
  Object.assign(worktree, { on, repo: { top, main, name: "repo", linked: true }, dirty, unpushed: [] });
  return { worktree, events, machines };
}

for (const [where, top, on, platform, first] of [
  ["a remote Windows drive", "C:/Users/me/wt/topic", "winbox", "macos", true],
  ["a remote lowercase drive", "d:/src/wt/topic", "winbox", "linux", true],
  ["a remote UNC share", "//server/share/wt/topic", "winbox", "linux", true],
  ["a remote Linux machine seen from Windows", "/home/me/wt/topic", "linuxbox", "windows", false],
  ["this Windows computer", "C:/Users/J\u00fcrgen \u00c4/wt/\u00fcn\u00ef topic", null, "windows", true],
  ["this Mac", "/Users/me/wt/topic", null, "macos", false],
] as [string, string, string | null, string, boolean][]) {
  test(`remove on ${where} closes the space ${first ? "before" : "after"} git removes the checkout`, async () => {
    const r = scriptedRemove({ top, on, platform });
    expect(await r.worktree.remove()).toBe("");
    const removed = `remove ${top}`;
    expect(r.events).toEqual(first ? ["status", "space.close target", removed] : ["status", removed, "space.close target"]);
    expect(r.machines.every(m => m === on)).toBe(true);
  });
}

for (const platform of ["windows", "linux"]) test(`a refused removal on ${platform} reports git's words and keeps the checkout's space only where it can`, async () => {
  const r = scriptedRemove({ top: platform === "windows" ? "C:/wt/topic" : "/wt/topic", platform, removeError: "fatal: 'C:/wt/topic' contains modified or untracked files" });
  expect(await r.worktree.remove()).toContain("contains modified or untracked files");
  // On Windows the space had to close first (a shell in the folder keeps it busy); elsewhere it stays.
  expect(r.events.filter(e => e.startsWith("space.close"))).toEqual(platform === "windows" ? ["space.close target"] : []);
});

test("a refused removal with no message still says what failed", async () => {
  const r = scriptedRemove({ top: "/wt/topic", removeError: " " });
  expect(await r.worktree.remove()).toBe("git worktree remove failed.");
});

test("a missing checkout folder reports git's error and touches nothing", async () => {
  const r = scriptedRemove({ top: "/wt/gone", statusError: "fatal: cannot change to '/wt/gone': No such file or directory" });
  expect(await r.worktree.remove()).toContain("No such file or directory");
  expect(r.events).toEqual(["status"]);
});

test("a checkout that became dirty asks once more, then removes with --force", async () => {
  const r = scriptedRemove({ top: "/wt/topic", status: "?? new.txt" });
  expect(await r.worktree.remove()).toBe("It changed since this opened. Enter again removes it anyway.");
  expect(r.events).toEqual(["status"]);
  expect(r.worktree.dirty).toBe(true);
  expect(await r.worktree.remove()).toBe("");
  expect(r.events).toEqual(["status", "status", "remove --force /wt/topic", "space.close target"]);
});

test("a checkout already dirty when the dialog opened is removed with --force on the first Enter", async () => {
  const r = scriptedRemove({ top: "C:/wt/topic", platform: "windows", status: " M a.txt", dirty: true });
  expect(await r.worktree.remove()).toBe("");
  expect(r.events).toEqual(["status", "space.close target", "remove --force C:/wt/topic"]);
});

test("a clean checkout is removed without --force", async () => {
  const r = scriptedRemove({ top: "/wt/topic" });
  expect(await r.worktree.remove()).toBe("");
  expect(r.events).toContain("remove /wt/topic");
});

test("a locked checkout without a reason is refused before anything closes", async () => {
  const top = "C:/wt/topic";
  const r = scriptedRemove({ top, platform: "windows", list: `worktree /repos/main\nHEAD 1\nbranch refs/heads/main\n\nworktree ${top}\nHEAD 2\nbranch refs/heads/topic\nlocked\n` });
  expect(await r.worktree.remove()).toBe("This worktree is locked.");
  expect(r.events).toEqual(["status"]);
});

test("a lock on another checkout does not block this one", async () => {
  const top = "/wt/topic";
  const r = scriptedRemove({ top, list: `worktree /repos/main\nHEAD 1\n\nworktree /wt/other\nHEAD 3\nlocked busy\n\nworktree ${top}\nHEAD 2\n` });
  expect(await r.worktree.remove()).toBe("");
  expect(r.events).toContain(`remove ${top}`);
});

test("a checkout whose space is already gone is removed without closing anything", async () => {
  const r = scriptedRemove({ top: "C:/wt/topic", platform: "windows", spaceThere: false });
  expect(await r.worktree.remove()).toBe("");
  expect(r.events).toEqual(["status", "remove C:/wt/topic"]);
});

test("the main checkout is never removed, even when Enter is pressed", async () => {
  const r = scriptedRemove({ top: "/repos/main" });
  r.worktree.repo.linked = false;
  await r.worktree.work({ notify() {} });
  expect(r.events).toEqual([]);
  expect(r.worktree.busy).toBeFalsy();
});

function repoOfWith(replies: Record<string, { code: number; stdout: string; stderr?: string }>, seen: unknown[] = []) {
  const exec = async (_command: string, args: string[], options: unknown) => {
    seen.push(options);
    return { stderr: "", ...replies[args.at(-1)!] };
  };
  const globals = { View: class {}, div: element, v_flex: element, h_flex: element, Input: { new: element }, Scrollbar: { vertical: element }, theme: () => ({}), exec };
  return new Function(...Object.keys(globals), `${source}; return repoOf;`)(...Object.values(globals));
}

test("repoOf reads a Windows checkout, a bare repo's worktree and a bare repo itself", async () => {
  const windows = repoOfWith({ "--show-toplevel": { code: 0, stdout: "C:/Users/me/My Proj\n" }, "--git-common-dir": { code: 0, stdout: "C:/Users/me/My Proj/.git\n" } });
  expect(await windows("C:\\Users\\me\\My Proj", null)).toEqual({ top: "C:/Users/me/My Proj", main: "C:/Users/me/My Proj", name: "My Proj", linked: false });
  const bareTree = repoOfWith({ "--show-toplevel": { code: 0, stdout: "/home/me/wt/topic\n" }, "--git-common-dir": { code: 0, stdout: "/srv/proj.git\n" } });
  expect(await bareTree("/home/me/wt/topic", null)).toMatchObject({ top: "/home/me/wt/topic", main: "/srv/proj.git", linked: true });
  // Inside a bare repo there is no work tree: nothing to make a worktree from.
  const bare = repoOfWith({ "--show-toplevel": { code: 128, stdout: "", stderr: "fatal: this operation must be run in a work tree" }, "--git-common-dir": { code: 0, stdout: "/srv/proj.git\n" } });
  expect(await bare("/srv/proj.git", null)).toBeNull();
  const outside = repoOfWith({ "--show-toplevel": { code: 128, stdout: "", stderr: "fatal: not a git repository" }, "--git-common-dir": { code: 128, stdout: "", stderr: "fatal: not a git repository" } });
  expect(await outside("/tmp", null)).toBeNull();
});

test("repoOf asks the folder's own machine", async () => {
  const seen: any[] = [];
  const remote = repoOfWith({ "--show-toplevel": { code: 0, stdout: "/home/me/\uc74c\uc545\n" }, "--git-common-dir": { code: 0, stdout: "/home/me/\uc74c\uc545/.git\n" } }, seen);
  expect(await remote("/home/me/\uc74c\uc545", "box")).toMatchObject({ name: "\uc74c\uc545", linked: false });
  expect(seen.map(o => o.on)).toEqual(["box", "box"]);
});

for (const [branch, folder] of [
  ["cafe\u0301", "caf\u00e9"],
  ["feat\\win:x*y?", "feat-win-x-y"],
  ["  spaced  name  ", "spaced-name"],
  ["fix/C:\\temp", "fix-C-temp"],
  ["\u00e9\u0301", "\u00e9\u0301"],
]) {
  test(`branch ${JSON.stringify(branch)} uses folder ${folder}`, async () => {
    const { worktree, executions } = harness();
    Object.assign(worktree, { text: branch, repo: { name: "repo", main: "/repo", top: "/repo" } });
    expect(await worktree.create()).toBe("");
    expect(executions[1].args[2]).toBe(`/home/user/.h0x/worktrees/repo/${folder}`);
  });
}

test("an empty or blank branch name is refused before git runs", async () => {
  for (const text of ["", "   ", "\t\n"]) {
    const { worktree, executions, writes } = harness();
    Object.assign(worktree, { text, repo: { name: "repo", main: "/repo", top: "/repo" } });
    expect(await worktree.create()).toBe("Type a branch name.");
    expect(executions).toEqual([]);
    expect(writes).toEqual([]);
  }
});

test("a failed worktree add reports git's words and opens no space", async () => {
  const { worktree, writes } = harness(null, {
    exec: async (_command: string, args: string[]) => args[1] === "add"
      ? { code: 128, stdout: "", stderr: "fatal: 'topic' is already checked out at '/wt/topic'" }
      : { code: 0, stdout: "", stderr: "" },
  });
  Object.assign(worktree, { text: "topic", repo: { name: "repo", main: "/repo", top: "/repo" } });
  expect(await worktree.create()).toContain("already checked out");
  expect(writes).toEqual([]);
});

function remoteDialog(exec: (command: string, args: string[], options: any) => Promise<unknown>) {
  const { worktree, snapshot } = harness(null, {
    exec, home: () => "/local/home", window: () => ({ platform: "macos" }),
    props: () => ({ space: "target", action: "remove", overlay: "dialog" }),
    InputState: { new: () => ({ on() {}, focus() {} }) },
  });
  snapshot([{ id: "target", name: "topic", folder: "C:/Users/me/wt/topic", machine: "winbox" }], []);
  let pending: Promise<void> = Promise.resolve();
  const cx = { notify() {}, focus_handle: () => ({ focus() {} }), spawn: (task: (c: unknown) => Promise<void>) => { pending = task(cx); } };
  worktree.init({}, cx);
  return { worktree, ready: () => pending };
}

test("a remote Windows machine's home is found with PowerShell when sh is missing, and shown as ~", async () => {
  const shells: string[] = [];
  const { worktree, ready } = remoteDialog(async (command, args) => {
    if (command !== "git") {
      shells.push(command);
      if (command === "sh") throw new Error("sh: not found");
      return { code: 0, stdout: "C:\\Users\\me\r\n", stderr: "" };
    }
    if (args.includes("--show-toplevel")) return { code: 0, stdout: "C:/Users/me/wt/topic\n", stderr: "" };
    if (args.includes("--git-common-dir")) return { code: 0, stdout: "C:/Users/me/src/repo/.git\n", stderr: "" };
    if (args.includes("--get-regexp")) return { code: 1, stdout: "", stderr: "" };
    return { code: 0, stdout: "", stderr: "" };
  });
  await ready();
  expect(shells).toEqual(["sh", "powershell.exe"]);
  expect(worktree.userHome).toBe("C:/Users/me");
  expect(worktree.repo).toMatchObject({ top: "C:/Users/me/wt/topic", main: "C:/Users/me/src/repo", linked: true });
  expect(textOf(worktree.render({}).node)).toContain("deletes ~/wt/topic");
});

test("a remote machine whose home can't be found says so and Enter does nothing", async () => {
  const { worktree, ready } = remoteDialog(async (command) => command === "git"
    ? { code: 0, stdout: "", stderr: "" }
    : { code: 1, stdout: "", stderr: "not found" });
  await ready();
  expect(worktree.repo).toBeNull();
  expect(worktree.error).toContain("Could not find the remote home folder.");
  worktree.go({ spawn() { throw new Error("nothing should start"); } });
});

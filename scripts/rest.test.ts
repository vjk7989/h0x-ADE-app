import { test, expect } from "bun:test";
import { readFile } from "node:fs/promises";

// The timers of the default mods rest while the window is unfocused or minimized
// (`window().active` is false): the host does not slow `cx.timer`, so they must do it themselves,
// and catch up at their next tick once the window is back.

async function load(mod: string, name: string, globals: Record<string, unknown>) {
  const source = (await readFile(new URL(`../defaults/mods/${mod}/main.js`, import.meta.url), "utf8"))
    .replace(/^import .*;\n/gm, "").replace(`export default class ${name}`, `class ${name}`).replace(/^export /gm, "");
  // Only the names the test provides are shadowed; anything else is a ReferenceError if used.
  const scope = new Proxy({ ...globals } as Record<string | symbol, unknown>, { has: (t, k) => k in t });
  return new Function("scope", `with (scope) { ${source}; return ${name}; }`)(scope);
}

function clock() {
  const timers: Record<number, (cx: unknown) => void> = {};
  const seen = { notify: 0, spawn: 0 };
  const cx = {
    timer: { every(ms: number, f: (cx: unknown) => void) { timers[ms] = f; } },
    notify() { seen.notify++; },
    spawn() { seen.spawn++; },
  };
  const tick = (ms: number) => timers[ms](cx);
  return { cx, seen, tick, timers };
}

const View = class {};
const tree = (mod: string) => ({ id: "root", type: "row", children: [{ id: "x", content: { mod } }] });

async function agentsMod(win: { active?: boolean } | null, list: { status: string }[]) {
  const Agents = await load("agents", "Agents", {
    View, localStorage: { getItem: () => null, setItem() {}, removeItem() {} }, props: () => ({}),
    window: () => win, agents: () => list,
  });
  const c = clock();
  const view = new Agents();
  view.init({}, c.cx);
  return { ...c, view };
}

test("agents: the redraw timers rest while the window is in the background", async () => {
  const win = { active: false };
  const a = await agentsMod(win, [{ status: "working" }]);
  for (let i = 0; i < 5; i++) { a.tick(1000); a.tick(30000); }
  expect(a.seen.notify).toBe(0);
});

test("agents: a working agent still redraws every second while the window is active", async () => {
  const a = await agentsMod({ active: true }, [{ status: "working" }]);
  a.tick(1000);
  a.tick(1000);
  expect(a.seen.notify).toBe(2);
  a.tick(30000);
  expect(a.seen.notify).toBe(3);
});

test("agents: nothing working and window active: no redraw", async () => {
  const a = await agentsMod({ active: true }, [{ status: "idle" }, { status: "done" }]);
  a.tick(1000);
  expect(a.seen.notify).toBe(0);
});

test("agents: going from inactive to active draws once at the next tick, even if nothing is working", async () => {
  const win = { active: false };
  const a = await agentsMod(win, [{ status: "done" }]);
  a.tick(1000);
  expect(a.seen.notify).toBe(0);
  win.active = true;
  a.tick(1000);
  expect(a.seen.notify).toBe(1);
  a.tick(1000);
  expect(a.seen.notify).toBe(1);
});

test("agents: an agent that finishes while the window is inactive is drawn on return", async () => {
  const win = { active: false };
  const list = [{ status: "working" }];
  const a = await agentsMod(win, list);
  a.tick(1000);
  list[0].status = "done"; // the alert came from the app; the list only has to catch up
  a.tick(1000);
  expect(a.seen.notify).toBe(0);
  win.active = true;
  a.tick(1000);
  expect(a.seen.notify).toBe(1);
});

test("agents: a window state without `active` (or no window) counts as active", async () => {
  for (const win of [{}, null]) {
    const a = await agentsMod(win, [{ status: "working" }]);
    a.tick(1000);
    expect(a.seen.notify).toBe(1);
  }
});

async function busyMod(mod: "pane" | "peek", win: { active: boolean }, status: { v: string }) {
  const name = mod === "pane" ? "Pane" : "Peek";
  const M = await load(mod, name, { View, window: () => win });
  const c = clock();
  const view = new M();
  view.me = () => ({ status: status.v });
  view.status = () => status.v;
  view.init({}, c.cx);
  return { ...c, view };
}

for (const mod of ["pane", "peek"] as const) {
  test(`${mod}: the busy verb rests while the window is in the background and draws once on return`, async () => {
    const win = { active: true }, status = { v: "working" };
    const m = await busyMod(mod, win, status);
    m.tick(1000);
    expect(m.seen.notify).toBe(1);
    win.active = false;
    for (let i = 0; i < 5; i++) m.tick(1000);
    expect(m.seen.notify).toBe(1);
    status.v = "idle";
    win.active = true;
    m.tick(1000);
    expect(m.seen.notify).toBe(2);
    m.tick(1000);
    expect(m.seen.notify).toBe(2);
  });
}

async function listing(mod: "changes" | "files", win: { active: boolean }, layout: unknown) {
  const name = mod === "changes" ? "Changes" : "Files";
  const M = await load(mod, name, { View, window: () => win, layout: () => layout });
  const view = new M();
  const calls = { refresh: 0, flush: 0 };
  view.refresh = () => { calls.refresh++; };
  view.flush = () => { calls.flush++; };
  return { view, calls, cx: {} };
}

for (const mod of ["changes", "files"] as const) {
  test(`${mod}: no refresh while the window is in the background, one at the next tick when it is back`, async () => {
    const win = { active: false };
    const { view, calls, cx } = await listing(mod, win, tree(mod));
    for (let i = 0; i < 25; i++) view.tick(cx);
    expect(calls.refresh).toBe(0);
    win.active = true;
    view.tick(cx);
    expect(calls.refresh).toBe(1);
    view.tick(cx);
    expect(calls.refresh).toBe(1);
  });

  test(`${mod}: still refreshes every 10 ticks while the window is active`, async () => {
    const { view, calls, cx } = await listing(mod, { active: true }, tree(mod));
    for (let i = 0; i < 20; i++) view.tick(cx);
    expect(calls.refresh).toBe(2);
  });
}

test("files: notes to agents still go out every 10 ticks while the window is in the background", async () => {
  const { view, calls, cx } = await listing("files", { active: false }, tree("files"));
  for (let i = 0; i < 30; i++) view.tick(cx);
  expect(calls.flush).toBe(3);
  expect(calls.refresh).toBe(0);
});

test("sessions: no look at the space while the window is in the background, a refresh when it is back", async () => {
  const win = { active: false };
  const space = { id: "s", active: true, folder: "/x" };
  const Sessions = await load("sessions", "Sessions", {
    View, window: () => win, spaces: () => [space], layout: () => tree("sessions"),
  });
  const view = new Sessions();
  const calls = { refresh: 0 };
  view.refresh = () => { calls.refresh++; };
  view.key = JSON.stringify([null, "/x"]);
  view.check({});
  expect(calls.refresh).toBe(0);
  win.active = true;
  view.check({});
  expect(calls.refresh).toBe(1);
  view.check({});
  expect(calls.refresh).toBe(1);
});

for (const mod of ["pane", "peek"] as const) {
  test(`${mod}: started while the window is in the background, it draws once when the window is back`, async () => {
    const win = { active: false }, status = { v: "idle" };
    const m = await busyMod(mod, win, status);
    m.tick(1000);
    m.tick(1000);
    expect(m.seen.notify).toBe(0);
    win.active = true;
    m.tick(1000);
    expect(m.seen.notify).toBe(1);
    m.tick(1000);
    expect(m.seen.notify).toBe(1);
  });
}

test("agents: a working agent rotates its verb every second again after a long rest", async () => {
  const win = { active: false };
  const a = await agentsMod(win, [{ status: "working" }]);
  for (let i = 0; i < 600; i++) a.tick(1000);
  for (let i = 0; i < 20; i++) a.tick(30000);
  expect(a.seen.notify).toBe(0);
  win.active = true;
  a.tick(1000);
  a.tick(1000);
  a.tick(1000);
  expect(a.seen.notify).toBe(3);
  a.tick(30000);
  expect(a.seen.notify).toBe(4);
});

test("an hour in the background ends in one catch-up per mod on the same tick, not one per missed tick", async () => {
  const win = { active: false }, status = { v: "working" };
  const list = [{ status: "working" }];
  const a = await agentsMod(win, list);
  const busy = await Promise.all([busyMod("pane", win, status), busyMod("pane", win, status), busyMod("peek", win, status)]);
  const lists = await Promise.all((["changes", "files"] as const).map((m) => listing(m, win, tree(m))));
  for (let i = 0; i < 3600; i++) {
    a.tick(1000);
    if (i % 30 === 0) a.tick(30000);
    for (const m of busy) m.tick(1000);
    for (const l of lists) l.view.tick(l.cx);
  }
  expect(a.seen.notify).toBe(0);
  for (const m of busy) expect(m.seen.notify).toBe(0);
  for (const l of lists) expect(l.calls.refresh).toBe(0);
  expect(lists[1].calls.flush).toBe(360); // notes to agents kept going out
  // Everything finished while away; the window comes back.
  list[0].status = "done";
  status.v = "idle";
  win.active = true;
  a.tick(1000);
  for (const m of busy) m.tick(1000);
  for (const l of lists) l.view.tick(l.cx);
  expect(a.seen.notify).toBe(1);
  for (const m of busy) expect(m.seen.notify).toBe(1);
  for (const l of lists) expect(l.calls.refresh).toBe(1);
  a.tick(1000);
  for (const m of busy) m.tick(1000);
  for (const l of lists) l.view.tick(l.cx);
  expect(a.seen.notify).toBe(1);
  for (const m of busy) expect(m.seen.notify).toBe(1);
  for (const l of lists) expect(l.calls.refresh).toBe(1);
});

async function sessionsMod(win: { active: boolean } | null, layout: () => unknown) {
  const space = { id: "s", active: true, folder: "/x" };
  const Sessions = await load("sessions", "Sessions", {
    View, window: () => win, spaces: () => [space], layout,
  });
  const view = new Sessions();
  const calls = { refresh: 0 };
  view.refresh = () => { calls.refresh++; };
  return { view, calls };
}

test("sessions: back while its panel is hidden waits, then refreshes once when shown", async () => {
  const win = { active: false };
  const hidden = { id: "root", type: "stack", active: "other", children: [{ id: "other", content: { mod: "files" } }, { id: "x", content: { mod: "sessions" } }] };
  let layout: unknown = hidden;
  const { view, calls } = await sessionsMod(win, () => layout);
  view.key = JSON.stringify([null, "/x"]);
  view.check({});
  win.active = true;
  view.check({});
  view.check({});
  expect(calls.refresh).toBe(0);
  layout = tree("sessions");
  view.check({});
  expect(calls.refresh).toBe(1);
  view.check({});
  expect(calls.refresh).toBe(1);
});

test("sessions: no window snapshot counts as active, so another space is still asked about", async () => {
  const { view, calls } = await sessionsMod(null, () => tree("sessions"));
  view.key = JSON.stringify([null, "/elsewhere"]);
  view.check({});
  expect(calls.refresh).toBe(1);
});

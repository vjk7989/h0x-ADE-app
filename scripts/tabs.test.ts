import { afterAll, beforeEach, expect, setSystemTime, test } from "bun:test";
import { readFile } from "node:fs/promises";

// Exercise the shipped tab bar's wheel handler without a window or a core.
type Handler = (event: unknown) => void;
class Element {
  nodes: Element[] = [];
  calls: Record<string, unknown[]> = {};
  constructor() {
    const proxy: Element = new Proxy(this, {
      get: (target, key: string) => key in target ? Reflect.get(target, key) : (...args: unknown[]) => {
        if (key === "child") target.nodes.push(args[0] as Element);
        else if (key === "children") target.nodes.push(...(args[0] as Element[]));
        else if (key === "when") { if (args[0]) (args[1] as (el: Element) => void)(proxy); }
        else target.calls[key] = args;
        return proxy;
      },
    });
    return proxy;
  }
}
const source = await readFile(new URL("../defaults/mods/tabs/main.js", import.meta.url), "utf8");
const element = () => new Element();
const commands: string[] = [];
const store: Record<string, string> = {};
const values: Record<string, unknown> = {
  View: class {}, div: element, svg: element, image: element, h_flex: element,
  ScrollHandle: { new: () => ({ scroll_to_item() {} }) },
  nextCommand: () => new Promise(() => {}),
  spaces: () => [{ id: "s1", active: true, tabs: [{ id: "t1", name: "a" }, { id: "t2", name: "b", active: true }, { id: "t3", name: "c" }] }],
  panes: () => [], run(command: string) { commands.push(command); }, openOverlay() {}, theme: () => ({}),
  window: () => null, shared: () => null, layout: () => null, settings: () => ({ zoom: 1 }),
  localStorage: { getItem: (key: string) => store[key] ?? null, setItem(key: string, value: string) { store[key] = value; } },
};
const Tabs = new Function(...Object.keys(values), source.replace(/^import .*;\n/gm, "").replace("export default class Tabs", "return class Tabs"))(...Object.values(values));

let now = 1_000_000;
function wheel() {
  const view = new Tabs();
  view.init({}, { spawn() {}, notify() {} });
  // The mod is plain JS: its handler's shape is the documented ScrollWheelEvent listener.
  const handler = (view.render() as Element).calls.on_scroll_wheel[0] as Handler;
  return (y: number, touch_phase = "moved", lines?: number, ms = 8) => {
    now += ms;
    setSystemTime(new Date(now));
    handler({ delta: { x: 0, y }, delta_lines: lines === undefined ? undefined : { x: 0, y: lines }, touch_phase });
  };
}
beforeEach(() => { commands.length = 0; });
// Bun runs every test file in one process: give the later ones the real clock.
afterAll(() => setSystemTime());

test("a short touchpad swipe switches one tab", () => {
  const send = wheel();
  for (let i = 0; i < 12; i++) send(-6);
  expect(commands).toEqual(["tab.next"]);
});

test("a nudge below the threshold switches nothing", () => {
  const send = wheel();
  for (let i = 0; i < 3; i++) send(-5);
  expect(commands).toEqual([]);
});

test("each notch of a smooth-scrolling mouse switches one tab", () => {
  const send = wheel();
  // macOS smooths one notch into about 28 events of 1 px.
  for (let i = 0; i < 28; i++) send(-1, "moved", undefined, 10);
  send(-1, "moved", undefined, 700);
  for (let i = 0; i < 27; i++) send(-1, "moved", undefined, 10);
  expect(commands).toEqual(["tab.next", "tab.next"]);
});

test("a long swipe keeps switching, a tab per 60 px", () => {
  const send = wheel();
  // 20 px for the first tab, then 60 px each: 400 px is seven tabs.
  for (let i = 0; i < 40; i++) send(10, "moved", undefined, 40);
  expect(commands).toEqual(Array(7).fill("tab.prev"));
});

test("a fast flick shows each tab it passes for at least 100 ms", () => {
  const send = wheel();
  // 1200 px within 240 ms: enough for twenty tabs, time for three.
  for (let i = 0; i < 30; i++) send(-40);
  expect(commands).toEqual(["tab.next", "tab.next", "tab.next"]);
});

test("a pause or a turn starts a new gesture", () => {
  const send = wheel();
  for (let i = 0; i < 12; i++) send(-6);
  send(-30, "moved", undefined, 400);
  for (let i = 0; i < 4; i++) send(-5);
  for (let i = 0; i < 12; i++) send(6, "moved", undefined, 20);
  expect(commands).toEqual(["tab.next", "tab.next", "tab.prev"]);
});

test("each wheel notch switches one tab", () => {
  const send = wheel();
  send(-20, "moved", -1);
  send(-20, "moved", -1);
  send(2, "moved", 0.1);
  expect(commands).toEqual(["tab.next", "tab.next", "tab.prev"]);
});

const label = new Function("View", `${source.replace(/^import .*;\n/gm, "").replace("export default class Tabs", "class Tabs")}; return label;`)(class {});

const claude = { program: { name: "Claude Code", key: "claude", agent: true }, title: "my-task" };

test("a tab HyperOx named after its agent follows the agent's session title", () => {
  expect(label({ name: "Claude Code", named: true }, claude)).toBe("my-task");
  expect(label({ name: "", named: false }, claude)).toBe("my-task");
  expect(label({ name: "", named: false }, { ...claude, title: "✦ [web] login" })).toBe("[web] login");
  expect(label({ name: "Claude Code", named: true }, { ...claude, title: "renamed-task" })).toBe("renamed-task");
});

test("an empty OSC title published as the terminal address keeps the program label", () => {
  for (const terminal of ["t2", "remote/t2"]) {
    const lead = { ...claude, terminal, title: terminal };
    expect(label({ name: "2", named: false }, lead)).toBe("Claude Code");
    expect(label({ name: "Claude Code", named: true }, lead)).toBe("Claude Code");
    expect(label({ name: "fix login", named: true }, lead)).toBe("fix login");
    expect(label({ name: "2", named: false }, { ...lead, title: "renamed-task" })).toBe("renamed-task");
  }
});

test("a name the person gave stays", () => {
  expect(label({ name: "fix login", named: true }, claude)).toBe("fix login");
});

test("a title that says nothing new keeps the tab's own name", () => {
  expect(label({ name: "Claude Code", named: true }, { ...claude, title: "Claude Code" })).toBe("Claude Code");
  expect(label({ name: "Claude Code", named: true }, { ...claude, title: "me@build-box" })).toBe("Claude Code");
  expect(label({ name: "", named: false }, { ...claude, title: "" })).toBe("Claude Code");
  expect(label({ name: "", named: false }, { ...claude, title: "~/src/api" })).toBe("Claude Code");
  expect(label({ name: "", named: false }, { ...claude, title: "✦ /tmp" })).toBe("Claude Code");
});

test("a shell's title never replaces its tab's name", () => {
  const shell = { program: { name: "zsh", key: "zsh" }, title: "vim notes.md" };
  expect(label({ name: "", named: false }, shell)).toBe("zsh");
  expect(label({ name: "notes", named: true }, shell)).toBe("notes");
  expect(label({ name: "", named: false }, undefined)).toBe("Terminal");
});

// Edge cases (UPSTREAM-SYNC-2 A2, A11): tab names, the wheel and revealing the active tab.
test("a tab's label survives empty, long and unicode titles", () => {
  const long = "Refactor the session restore path for remote machines ".repeat(40).trim();
  expect(label({ name: "", named: false }, { ...claude, title: null })).toBe("Claude Code");
  expect(label({ name: "", named: false }, { ...claude, title: undefined })).toBe("Claude Code");
  expect(label({ name: "", named: false }, { ...claude, title: "   " })).toBe("Claude Code");
  expect(label({ name: "", named: false }, { ...claude, title: long })).toBe(long);
  expect(label({ name: "", named: false }, { ...claude, title: "\u2726 \u4fee\u590d\u767b\u5f55" })).toBe("\u4fee\u590d\u767b\u5f55");
  expect(label({ name: "", named: false }, { ...claude, title: "e\u0301clair" })).toBe("e\u0301clair");
  expect(label({ name: "", named: false }, { ...claude, title: "[ ! ] Action Required" })).toBe("Action Required");
  expect(label({ name: "", named: false }, { ...claude, title: "C:\\Users\\me\\proj" })).toBe("Claude Code");
});

test("an unnamed tab without a program is a Terminal, one with a program is named after it", () => {
  expect(label({ name: "tb3", named: false }, undefined)).toBe("Terminal");
  expect(label({ name: "tb3", named: false }, { program: null, title: "x" })).toBe("Terminal");
  expect(label({ name: "tb3", named: false }, { program: { name: "htop", key: "htop" }, title: "htop - load" })).toBe("htop");
  // Two tabs of the same agent each follow their own session title.
  expect(label({ name: "", named: false }, { ...claude, title: "task one" })).toBe("task one");
  expect(label({ name: "", named: false }, { ...claude, title: "task two" })).toBe("task two");
});

test("a zero-pixel or sideways wheel event switches nothing", () => {
  const send = wheel();
  for (let i = 0; i < 40; i++) send(0);
  send(0, "moved", 0);
  expect(commands).toEqual([]);
});

test("exactly 20 px switches one tab, 19 px does not", () => {
  let send = wheel();
  send(-19, "moved", undefined, 400);
  expect(commands).toEqual([]);
  send = wheel();
  send(-20, "moved", undefined, 400);
  expect(commands).toEqual(["tab.next"]);
});

test("a 250 ms pause still continues the gesture, 251 ms starts a new one", () => {
  let send = wheel();
  send(-10, "moved", undefined, 400);
  send(-10, "moved", undefined, 250);
  expect(commands).toEqual(["tab.next"]);
  commands.length = 0;
  send = wheel();
  send(-10, "moved", undefined, 400);
  send(-10, "moved", undefined, 251);
  expect(commands).toEqual([]);
});

// The tab strip with a chosen set of tabs and a scroll handle that records requests.
function strip(tabs: { id: string; name: string; active?: boolean }[]) {
  const asked: number[] = [];
  const handles: unknown[] = [];
  const world = { tabs };
  const scoped = { ...values,
    ScrollHandle: { new: () => { const h = { scroll_to_item(i: number) { asked.push(i); } }; handles.push(h); return h; } },
    spaces: () => [{ id: "s1", active: true, tabs: world.tabs }],
  };
  const Strip = new Function(...Object.keys(scoped), source.replace(/^import .*;\n/gm, "").replace("export default class Tabs", "return class Tabs"))(...Object.values(scoped));
  const view = new Strip();
  view.init({}, { spawn() {}, notify() {} });
  const find = (node: Element, id: string): Element | undefined => {
    if (!(node instanceof Element)) return undefined;
    if (node.calls.id?.[0] === id) return node;
    for (const child of node.nodes) { const found = find(child, id); if (found) return found; }
  };
  const render = () => view.render() as Element;
  return { view, asked, handles, world, render, find };
}
const named = (n: number, active: number) => Array.from({ length: n }, (_, i) => ({ id: `t${i}`, name: `tab ${i}`, named: true, active: i === active }));

test("zero tabs: nothing to reveal, and a size change asks for nothing", () => {
  const s = strip([]);
  const root = s.render();
  expect(s.asked).toEqual([]);
  const scroller = s.find(root, "tab-scroll")!;
  expect(scroller.calls.track_scroll[0]).toBe(s.handles[0]);
  (scroller.calls.on_size_change[0] as (size: unknown, cx: unknown) => void)({}, { notify() {} });
  expect(s.asked).toEqual([]);
});

test("no active tab: nothing to reveal", () => {
  const s = strip(named(3, -1));
  s.render();
  expect(s.asked).toEqual([]);
});

test("one tab is revealed once, then left alone", () => {
  const s = strip(named(1, 0));
  s.render();
  s.render();
  expect(s.asked).toEqual([0]);
});

test("the active tab at either end is revealed, and again when it moves", () => {
  const s = strip(named(12, 0));
  s.render();
  expect(s.asked).toEqual([0]);
  s.world.tabs = named(12, 11);
  s.render();
  expect(s.asked).toEqual([0, 11]);
  s.render();
  expect(s.asked).toEqual([0, 11]);
  s.world.tabs = named(12, 0);
  s.render();
  expect(s.asked).toEqual([0, 11, 0]);
});

test("a tab opened before the active one shifts its place and reveals it again", () => {
  const s = strip(named(3, 2));
  s.render();
  s.world.tabs = [{ id: "new", name: "new", active: false }, ...named(3, 2)];
  s.render();
  expect(s.asked).toEqual([2, 3]);
});

test("the strip and the tabs up to the active one re-reveal it when they resize", () => {
  const s = strip(named(4, 2));
  const root = s.render();
  expect(s.asked).toEqual([2]);
  let notified = 0;
  const cx = { notify() { notified++; } };
  (s.find(root, "tab-scroll")!.calls.on_size_change[0] as (size: unknown, cx: unknown) => void)({}, cx);
  expect(s.asked).toEqual([2, 2]);
  for (const id of ["tab-t0", "tab-t1", "tab-t2"]) {
    (s.find(root, id)!.calls.on_size_change[0] as (size: unknown, cx: unknown) => void)({}, cx);
  }
  expect(s.asked).toEqual([2, 2, 2, 2, 2]);
  expect(notified).toBe(4);
  // A tab after the active one can't push it out: it has no size hook.
  expect(s.find(root, "tab-t3")!.calls.on_size_change).toBeUndefined();
});

test("unreadable saved panels start empty instead of throwing", () => {
  for (const bad of ["not json", "null", "[]", "5", "\"text\""]) {
    store.panels = bad;
    const view = new Tabs();
    view.init({}, { spawn() {}, notify() {} });
    expect(view.panels).toEqual({});
  }
  store.panels = JSON.stringify({ s1: "todo" });
  const view = new Tabs();
  view.init({}, { spawn() {}, notify() {} });
  expect(view.panels).toEqual({ s1: "todo" });
  delete store.panels;
});

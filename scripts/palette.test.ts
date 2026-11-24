import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";

// Exercise the shipped palette's handler with the pixel deltas from GPUI.
class Element {
  nodes: Element[] = [];
  calls: Record<string, any[]> = {};
  constructor() {
    const proxy: Element = new Proxy(this, {
      get: (target, key: string) => key in target ? (target as any)[key] : (...args: any[]) => {
        if (key === "child") target.nodes.push(args[0]);
        else if (key === "children") target.nodes.push(...args[0]);
        else if (key === "when") { if (args[0]) args[1](proxy); }
        else target.calls[key] = args;
        return proxy;
      },
    });
    return proxy;
  }
}
const source = await readFile(new URL("../defaults/frame/palette/main.js", import.meta.url), "utf8");
const element = () => new Element();
let platform = "linux";
const icons: string[] = [];
let dark = true;
const shipped = [["graphite", false], ["ink", false], ["moss", false], ["paper", true], ["atom-one-dark", false], ["mine-light", true], ["win98", true]];
// More app commands for a test that needs them; empty otherwise.
const extraCommands: { name: string; args: unknown[]; about: string }[] = [];
const values = {
  View: class {}, div: element, svg: (path: string) => { icons.push(path); return element(); }, v_flex: element, h_flex: element, Input: { new: element }, theme: () => ({}), keys: () => [], win: () => ({ platform }),
  commands: () => [{ name: "app.theme", args: [{ name: "name", required: true }], about: "Use a theme." }, ...extraCommands], layouts: () => ["main", "win98"], shared: () => "main",
  themes: () => shipped.map(([name, light]) => ({ name, light })), settings: () => ({ dark }),
};
const Palette = new Function(...Object.keys(values), source.replace(/^import .*;\n/gm, "").replace("export default class Palette", "return class Palette"))(...Object.values(values));
function find(root: Element, id: string): Element | undefined {
  if (root?.calls?.id?.[0] === id) return root;
  for (const child of root?.nodes ?? []) {
    const found = find(child, id);
    if (found) return found;
  }
}

test("palette follows small pixel movements and converted wheel deltas", () => {
  for (const delta of [-1, -3.9, -4, -48]) {
    const view = new Palette();
    view.query = "command"; view.at = 0; view.scroll = 0; view.input = {}; view.handle = {};
    view.rows = () => Array.from({ length: 50 }, (_, i) => ({ e: { name: "test", title: `Command ${i}`, args: {} } }));
    const list = find(view.render(), "palette-rows");
    expect(list).toBeDefined();
    list!.calls.on_scroll_wheel[0]({ delta: { y: delta } }, { notify() {} });
    expect(view.scroll).toBe(-delta);
  }
});

test("palette input shows the command key only on macos", () => {
  for (const [os, icon] of [["macos", "frame/palette/command.svg"], ["windows", "mods/workspace-chrome/search.svg"], ["linux", "mods/workspace-chrome/search.svg"]]) {
    platform = os; icons.length = 0;
    const view = new Palette();
    view.query = ""; view.at = 0; view.scroll = 0; view.input = {}; view.handle = {};
    view.rows = () => [];
    view.render();
    expect(icons).toEqual([icon]);
  }
});

test("palette scrolls past group titles with small movements", () => {
  for (const delta of [-3, -16, -25]) {
    const view = new Palette();
    view.query = ""; view.at = 0; view.scroll = 0; view.input = {}; view.handle = {};
    view.rows = () => Array.from({ length: 30 }, (_, i) => ({ head: `Group ${Math.floor(i / 6)}`, e: { name: "test", title: `Command ${i}`, args: {} } }));
    for (let step = 0; step < 400 && (step === 0 || view.scroll < view.maxScroll); step++) {
      const list = find(view.render(), "palette-rows");
      const before = view.scroll;
      list!.calls.on_scroll_wheel[0]({ delta: { y: delta } }, { notify() {} });
      view.render();
      expect(view.scroll).toBe(Math.min(before - delta, view.maxScroll));
    }
    expect(view.scroll).toBe(view.maxScroll);
  }
});

test("palette offers only the themes that can show in the current appearance", () => {
  const offered = () => {
    const view = new Palette();
    view.query = ""; view.recent = []; view.agents = []; // as init() starts it
    return view.rows().map((r: any) => r.e).filter((e: any) => e.name === "app.theme").map((e: any) => e.args.name);
  };
  dark = true;
  expect(offered()).toEqual(["graphite", "ink", "moss", "atom-one-dark"]);
  dark = false;
  // win98 is the HyperOx 98 layout's own theme, so it isn't offered here.
  expect(offered()).toEqual(["paper", "mine-light"]);
});

// Edge cases (UPSTREAM-SYNC-2 A7): themes() and settings() at their limits, and the agent rows.
function paletteRows(agents: unknown[] = []) {
  const view = new Palette();
  view.query = ""; view.recent = []; view.agents = agents;
  return view.rows().map((r: any) => r.e);
}
const themeNames = () => paletteRows().filter((e: any) => e.name === "app.theme").map((e: any) => e.args.name);

test("palette offers no theme rows when there are no themes", () => {
  const saved = shipped.splice(0);
  try {
    dark = true;
    expect(themeNames()).toEqual([]);
    dark = false;
    expect(themeNames()).toEqual([]);
  } finally {
    shipped.push(...saved);
    dark = true;
  }
});

test("a theme without a lightness counts as dark, and no dark setting counts as light", () => {
  const saved = shipped.splice(0, shipped.length, ["unmarked", undefined as unknown as boolean], ["paper", true]);
  try {
    dark = true;
    expect(themeNames()).toEqual(["unmarked"]);
    dark = undefined as unknown as boolean;
    expect(themeNames()).toEqual(["paper"]);
  } finally {
    shipped.splice(0, shipped.length, ...saved);
    dark = true;
  }
});

test("theme titles read with spaces for dashes", () => {
  dark = true;
  const theme = paletteRows().find((e: any) => e.name === "app.theme" && e.args.name === "atom-one-dark");
  expect(theme.title).toBe("Theme: atom one dark");
  expect(theme.group).toBe("App");
});

test("the palette keeps a new-session row for each installed or custom agent, by its slot", () => {
  const rows = paletteRows([
    { slot: 1, installed: true, name: "Claude", key: "claude" },
    { slot: 2, installed: false, name: "Missing", key: "missing" },
    { slot: 3, custom: true, name: "Mine", key: "mine" },
    { slot: 0, installed: true, name: "No slot", key: "none" },
    { installed: true, name: "Unslotted", key: "unslotted" },
  ]).filter((e: any) => e.name === "agent.new");
  expect(rows.map((e: any) => [e.args.index, e.title])).toEqual([[1, "New Claude session"], [3, "New Mine session"]]);
  expect(rows.every((e: any) => e.group === "Agent")).toBe(true);
});

test("no agents found leaves no new-session rows", () => {
  expect(paletteRows([]).filter((e: any) => e.name === "agent.new")).toEqual([]);
});

test("a multi-word query needs every word to start a word in the row", () => {
  const view = new Palette();
  view.recent = []; view.agents = [];
  const titles = (q: string) => { view.query = q; return view.rows().map((r: any) => r.e.title); };
  expect(titles("theme atom")).toContain("Theme: atom one dark");
  expect(titles("theme atom").every((t: string) => /theme/i.test(t) && /atom/i.test(t))).toBe(true);
  expect(titles("zzz theme")).toEqual([]);
  expect(titles("thm").length).toBeGreaterThan(0); // one word stays fuzzy
});

// VQA-POLISH-1 D4: several words each start a word of the row (group, title, name or keywords); one word stays fuzzy.
function searching(agents: unknown[] = []) {
  extraCommands.splice(0, extraCommands.length,
    { name: "pane.close", args: [], about: "Close the pane." },
    { name: "pane.go", args: [], about: "Move focus." },
    { name: "terminal.paste", args: [], about: "Paste text." },
    { name: "terminal.paste_image", args: [], about: "Put an image into the terminal." },
    { name: "terminal.copy_screen", args: [], about: "Copy what the terminal shows." },
    { name: "app.whats_new", args: [], about: "What changed." },
  );
  const view = new Palette();
  view.recent = []; view.agents = agents;
  return (q: string) => { view.query = q; return view.rows().map((r: any) => r.e.title as string); };
}
function searchingWith(check: (titles: (q: string) => string[]) => void, agents: unknown[] = []) {
  try { check(searching(agents)); } finally { extraCommands.length = 0; dark = true; }
}

test("close pan finds Close pane, not Paste image terminal or Copy screen terminal", () => searchingWith((titles) => {
  const found = titles("close pan");
  expect(found).toContain("Close pane");
  expect(found).not.toContain("Paste image terminal");
  expect(found).not.toContain("Copy screen terminal");
  expect(found[0]).toBe("Close pane");
}));

test("extra spaces and tabs around and between words change nothing", () => searchingWith((titles) => {
  const want = titles("close pan");
  for (const q of ["  close pan", "close pan  ", "close    pan", "\tclose\t pan\n"]) expect(titles(q)).toEqual(want);
}));

test("multi-word search ignores case", () => searchingWith((titles) => {
  for (const q of ["CLOSE PAN", "Close Pane", "cLoSe pAnE"]) {
    expect(titles(q)).toContain("Close pane");
    expect(titles(q)).not.toContain("Paste image terminal");
  }
}));

test("a repeated word matches the same word of the row", () => searchingWith((titles) => {
  expect(titles("pane pane")).toContain("Close pane");
  expect(titles("close close")).toContain("Close pane");
  expect(titles("close close")).not.toContain("Paste image terminal");
}));

test("a word may start at the group, the command name or a keyword", () => searchingWith((titles) => {
  expect(titles("pane kill")).toContain("Close pane"); // group + keyword
  expect(titles("terminal paste")).toEqual(expect.arrayContaining(["Paste", "Paste image terminal"])); // group + title
  expect(titles("terminal paste")).not.toContain("Copy screen terminal");
}));

test("a word that only matches mid-word does not match a multi-word query", () => searchingWith((titles) => {
  for (const q of ["lose pane", "close ane", "los pan", "ose ane"]) expect(titles(q)).not.toContain("Close pane");
  expect(titles("lose")).toContain("Close pane"); // one word: fuzzy, as before
}));

test("one word keeps fuzzy matching; the same letters split in two do not", () => searchingWith((titles) => {
  expect(titles("clspn")).toContain("Close pane");
  expect(titles("cls pn")).not.toContain("Close pane");
  expect(titles("thm").length).toBeGreaterThan(0);
}));

test("three or more words all have to match", () => searchingWith((titles) => {
  expect(titles("focus pane left")).toEqual(["Focus pane left"]);
  expect(titles("focus pane le")).toEqual(["Focus pane left"]);
  expect(titles("focus pane")).toEqual(expect.arrayContaining(["Focus pane left", "Focus pane right", "Focus pane up", "Focus pane down"]));
  expect(titles("focus pane left zzz")).toEqual([]);
  expect(titles("theme atom one dark")).toEqual(["Theme: atom one dark"]);
}));

test("an empty query and a query of only spaces both show the grouped list", () => searchingWith((titles) => {
  const empty = titles("");
  expect(empty.length).toBeGreaterThan(0);
  expect(empty).toContain("Close pane");
  for (const q of [" ", "   ", "\t", " \t \n "]) expect(titles(q)).toEqual(empty);
}));

test("a query typed with the row's own punctuation still finds it", () => searchingWith((titles) => {
  expect(titles("what's new")).toContain("What's new");
  expect(titles("theme: atom")).toContain("Theme: atom one dark");
  expect(titles("pane.close kill")).toContain("Close pane");
}));

test("unicode titles match by their words, typed in full or in part", () => searchingWith((titles) => {
  expect(titles("café")).toContain("New Café session"); // one word: fuzzy
  expect(titles("new caf")).toContain("New Café session");
  expect(titles("new café")).toContain("New Café session");
  expect(titles("NEW CAFÉ")).toContain("New Café session");
  expect(titles("new 日本")).toContain("New 日本 session");
  expect(titles("new café")).not.toContain("New 日本 session");
}, [
  { slot: 1, installed: true, name: "Café", key: "cafe" },
  { slot: 2, installed: true, name: "日本", key: "nihon" },
]));

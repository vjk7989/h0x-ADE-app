import { test, expect } from "bun:test";
import { readFile } from "node:fs/promises";

// The to-do mod's pure model (defaults/mods/todo/model.js), evaluated without its exports.
const source = (await readFile(new URL("../defaults/mods/todo/model.js", import.meta.url), "utf8")).replace(/^export /gm, "");
const { parseList, serializeList, parseIndex, serializeIndex, slugify, openCount, allDone } = new Function(
  `${source}; return { parseList, serializeList, parseIndex, serializeIndex, slugify, openCount, allDone };`,
)();

const roundTrip = (text: string) => serializeList(parseList(text));
const SLUG = /^[a-z0-9-]{1,60}$/;

test("a list in the written form round-trips byte for byte, notes and blank lines included", () => {
  const text = [
    "# Groceries",
    "",
    "Things for the weekend.",
    "- [ ] Milk",
    "  > from the corner shop",
    "  - [x] Check the fridge",
    "    > it was empty",
    "  - [ ] Ask Sam",
    "",
    "- [x] Bread",
    "",
    "",
  ].join("\n");
  const list = parseList(text);
  expect(list.name).toBe("Groceries");
  expect(list.head).toEqual(["", "Things for the weekend."]);
  expect(list.tasks.map((t: any) => [t.title, t.done])).toEqual([["Milk", false], ["Bread", true]]);
  expect(list.tasks[0].extra).toEqual(["  > from the corner shop"]);
  expect(list.tasks[0].steps).toEqual([
    { title: "Check the fridge", done: true, extra: ["    > it was empty"] },
    { title: "Ask Sam", done: false, extra: [""] },
  ]);
  expect(roundTrip(text)).toBe(text);
});

test("CRLF, a missing final newline and an empty file all read and are written with LF", () => {
  const crlf = parseList("# Tasks\r\n- [ ] A\r\n  - [x] B\r\n");
  expect(crlf.name).toBe("Tasks");
  expect(crlf.tasks[0].title).toBe("A");
  expect(crlf.tasks[0].steps[0]).toEqual({ title: "B", done: true, extra: [] });
  expect(serializeList(crlf)).toBe("# Tasks\n- [ ] A\n  - [x] B\n");
  expect(roundTrip("# Tasks\n- [ ] last")).toBe("# Tasks\n- [ ] last\n");
  const empty = parseList("");
  expect(empty).toEqual({ name: "Untitled", head: [], tasks: [] });
  expect(serializeList(empty)).toBe("# Untitled\n");
});

test("deeper and tab-indented steps become steps; a one-space indent is kept as a plain line", () => {
  const list = parseList("# T\n- [ ] A\n  - [ ] B\n    - [x] C\n\t- [ ] D\n\t\t* [X] E\n - [ ] one space\n");
  expect(list.tasks).toHaveLength(1);
  expect(list.tasks[0].steps.map((s: any) => [s.title, s.done])).toEqual([["B", false], ["C", true], ["D", false], ["E", true]]);
  expect(list.tasks[0].steps[3].extra).toEqual([" - [ ] one space"]);
  expect(serializeList(list)).toBe("# T\n- [ ] A\n  - [ ] B\n  - [x] C\n  - [ ] D\n  - [x] E\n - [ ] one space\n");
});

test("star bullets and capital X are tasks; near misses stay plain lines", () => {
  const list = parseList("# T\n* [X] Star\n- [X] Capital\n-[ ] no space\n- [] empty box\n- [y] other mark\n- [ ]no gap\n");
  expect(list.tasks.map((t: any) => [t.title, t.done])).toEqual([["Star", true], ["Capital", true]]);
  expect(list.tasks[1].extra).toEqual(["-[ ] no space", "- [] empty box", "- [y] other mark", "- [ ]no gap"]);
  expect(serializeList(list)).toBe("# T\n- [x] Star\n- [x] Capital\n-[ ] no space\n- [] empty box\n- [y] other mark\n- [ ]no gap\n");
});

test("box text inside a title, unicode, emoji and very long titles are titles", () => {
  const long = "x".repeat(10_000);
  const text = `# Café ☕\n- [ ] Buy [ ] boxes and [x] tape\n- [x] [ ] starts with a box\n- [ ] 🥛 Milch für Oma — 牛奶\n- [ ] ${long}\n`;
  const list = parseList(text);
  expect(list.name).toBe("Café ☕");
  expect(list.tasks.map((t: any) => [t.title, t.done])).toEqual([
    ["Buy [ ] boxes and [x] tape", false],
    ["[ ] starts with a box", true],
    ["🥛 Milch für Oma — 牛奶", false],
    [long, false],
  ]);
  expect(roundTrip(text)).toBe(text);
});

test("trailing spaces leave a title; a newline in a title or name is written as a space", () => {
  expect(parseList("# T\n- [ ] Milk   \n").tasks[0].title).toBe("Milk");
  expect(parseList("# T\n- [ ]  two  spaces\n").tasks[0].title).toBe(" two  spaces");
  const list = { name: "A\r\nB", head: [], tasks: [{ title: "one\ntwo\rthree", done: false, extra: [], steps: [{ title: "s\nt", done: true, extra: [] }] }] };
  expect(serializeList(list)).toBe("# A B\n- [ ] one two three\n  - [x] s t\n");
});

test("a heading after tasks is kept under the task above it, not taken as the name", () => {
  const text = "# A\n- [ ] one\n# B\n- [ ] two\n";
  const list = parseList(text);
  expect(list.name).toBe("A");
  expect(list.tasks.map((t: any) => t.title)).toEqual(["one", "two"]);
  expect(list.tasks[0].extra).toEqual(["# B"]);
  expect(roundTrip(text)).toBe(text);
});

test("no heading, an empty heading or a non-H1 heading reads as Untitled", () => {
  for (const text of ["- [ ] one\n", "# \n- [ ] one\n", "#Tasks\n- [ ] one\n", "## Sub\n- [ ] one\n"]) {
    const list = parseList(text);
    expect(list.name).toBe("Untitled");
    expect(list.tasks.map((t: any) => t.title)).toEqual(["one"]);
  }
  expect(roundTrip("- [ ] one\n")).toBe("# Untitled\n- [ ] one\n");
  expect(roundTrip("## Sub\n- [ ] one\n")).toBe("# Untitled\n## Sub\n- [ ] one\n");
});

test("lines before the heading are kept (below it) and a step before any task stays a plain line", () => {
  const list = parseList("<!-- mine -->\n# Tasks\n  - [ ] orphan\n- [ ] a\n");
  expect(list.name).toBe("Tasks");
  expect(list.head).toEqual(["<!-- mine -->", "  - [ ] orphan"]);
  expect(list.tasks[0].steps).toEqual([]);
  expect(serializeList(list)).toBe("# Tasks\n<!-- mine -->\n  - [ ] orphan\n- [ ] a\n");
});

test("writing what was read is stable for messy files", () => {
  for (const text of ["", "\n\n", "# T\r\n* [X] a\r\n\t- [ ] b\r\n", "intro\n# T\n- [ ] a\n # not a heading\n", "- [ ] \n- [x]   \n"]) {
    const once = roundTrip(text);
    expect(roundTrip(once)).toBe(once);
  }
});

test("the index keeps valid lines in order and drops duplicates, bad slugs and junk", () => {
  const sixty = "a".repeat(60);
  const text = [
    "# My lists",
    "- [Tasks](tasks.md)",
    "random words",
    "- [Groceries](groceries.md)   ",
    "- [Tasks again](tasks.md)",
    "- [Up](../up.md)",
    "- [Caps](Caps.md)",
    "- [Space](my list.md)",
    "- [Empty](.md)",
    "- [Sub](dir/x.md)",
    "- [Text](a.txt)",
    "* [Star](star.md)",
    "-[Tight](tight.md)",
    `- [Long](${sixty}a.md)`,
    `- [Sixty](${sixty}.md)`,
    "- [a [b] c](abc.md)",
    "",
  ].join("\r\n");
  expect(parseIndex(text)).toEqual([
    { name: "Tasks", slug: "tasks" },
    { name: "Groceries", slug: "groceries" },
    { name: "Sixty", slug: sixty },
    { name: "a [b] c", slug: "abc" },
  ]);
  expect(parseIndex("")).toEqual([]);
  expect(serializeIndex(parseIndex(text))).toBe(`- [Tasks](tasks.md)\n- [Groceries](groceries.md)\n- [Sixty](${sixty}.md)\n- [a [b] c](abc.md)\n`);
  expect(serializeIndex([])).toBe("");
  expect(serializeIndex([{ name: "Two\nlines", slug: "two" }])).toBe("- [Two lines](two.md)\n");
});

test("slugs are lowercase a-z0-9 and dashes; unicode and symbols become one dash; nothing left is list", () => {
  expect(slugify("Groceries", [])).toBe("groceries");
  expect(slugify("My  Big List!", [])).toBe("my-big-list");
  expect(slugify("  --Hello--  ", [])).toBe("hello");
  expect(slugify("Café Ünïcode", [])).toBe("caf-n-code");
  expect(slugify("Ship 0.1.19", [])).toBe("ship-0-1-19");
  for (const name of ["", "   ", "---", "牛奶", "🥛", "Ünï"]) expect(slugify(name, [])).toBe(name === "Ünï" ? "n" : "list");
  expect(slugify("", ["list"])).toBe("list-2");
});

test("lists is reserved and taken slugs get the next free number", () => {
  expect(slugify("lists", [])).toBe("lists-2");
  expect(slugify("Lists", ["lists-2"])).toBe("lists-3");
  expect(slugify("Tasks", ["tasks"])).toBe("tasks-2");
  expect(slugify("Tasks", ["tasks", "tasks-2", "tasks-3"])).toBe("tasks-4");
  expect(slugify("lists-2", [])).toBe("lists-2");
});

test("slugs stay within 60 characters, numbers included, and never end in a dash", () => {
  const a = (n: number) => "a".repeat(n);
  expect(slugify(a(100), [])).toBe(a(60));
  expect(slugify(a(100), [a(60)])).toBe(`${a(58)}-2`);
  const taken = [a(60), ...Array.from({ length: 8 }, (_, k) => `${a(58)}-${k + 2}`)];
  expect(slugify(a(100), taken)).toBe(`${a(57)}-10`);
  // Cut at a dash: the dash goes too.
  expect(slugify(`${a(59)} b`, [])).toBe(a(59));
  expect(slugify(`${a(57)} bc`, [`${a(57)}-bc`])).toBe(`${a(57)}-2`);
  for (const name of [a(200), `${a(59)} b`, "x ".repeat(80), "Ünï ".repeat(30)]) {
    const used: string[] = [];
    for (let k = 0; k < 12; k++) {
      const slug = slugify(name, used);
      expect(slug).toMatch(SLUG);
      expect(slug.endsWith("-")).toBe(false);
      expect(used).not.toContain(slug);
      used.push(slug);
    }
    // Every slug made is one the index accepts back.
    expect(parseIndex(serializeIndex(used.map((slug) => ({ name, slug }))))).toHaveLength(12);
  }
});

test("open count and all done look at tasks only, and an empty list is not done", () => {
  const empty = parseList("# T\n");
  expect([openCount(empty), allDone(empty)]).toEqual([0, false]);
  const doneWithOpenSteps = parseList("# T\n- [x] a\n  - [ ] step\n- [x] b\n");
  expect([openCount(doneWithOpenSteps), allDone(doneWithOpenSteps)]).toEqual([0, true]);
  const openWithDoneSteps = parseList("# T\n- [ ] a\n  - [x] step\n- [x] b\n");
  expect([openCount(openWithDoneSteps), allDone(openWithDoneSteps)]).toEqual([1, false]);
});

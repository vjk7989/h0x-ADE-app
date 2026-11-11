import { test, expect } from "bun:test";
import { readFile } from "node:fs/promises";

// The to-do mod's pure model (defaults/mods/todo/model.js), evaluated without its exports.
const source = (await readFile(new URL("../defaults/mods/todo/model.js", import.meta.url), "utf8")).replace(/^export /gm, "");
const { parseList, serializeList, parseIndex, serializeIndex, slugify, openCount, allDone, setTaskDone, setStepDone } = new Function(
  `${source}; return { parseList, serializeList, parseIndex, serializeIndex, slugify, openCount, allDone, setTaskDone, setStepDone };`,
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

test("ticking a task ticks every step and unticking unticks them; notes and blank lines stay", () => {
  const text = "# T\n- [ ] Milk\n  > a note\n  - [ ] one\n  - [x] two\n\n  - [ ] three\n- [ ] Bread\n";
  const list = parseList(text);
  setTaskDone(list.tasks[0], true);
  expect(serializeList(list)).toBe("# T\n- [x] Milk\n  > a note\n  - [x] one\n  - [x] two\n\n  - [x] three\n- [ ] Bread\n");
  expect([openCount(list), allDone(list)]).toEqual([1, false]);
  setTaskDone(list.tasks[0], false);
  expect(serializeList(list)).toBe("# T\n- [ ] Milk\n  > a note\n  - [ ] one\n  - [ ] two\n\n  - [ ] three\n- [ ] Bread\n");
  // Ticking an already done task again changes nothing.
  setTaskDone(list.tasks[1], true);
  setTaskDone(list.tasks[1], true);
  expect(serializeList(list).endsWith("- [x] Bread\n")).toBe(true);
});

test("a task with no steps ticks and unticks on its own", () => {
  const list = parseList("# T\n- [ ] alone\n");
  setTaskDone(list.tasks[0], true);
  expect(list.tasks[0]).toEqual({ title: "alone", done: true, steps: [], extra: [] });
  expect(allDone(list)).toBe(true);
  setTaskDone(list.tasks[0], false);
  expect(serializeList(list)).toBe("# T\n- [ ] alone\n");
  expect(allDone(list)).toBe(false);
});

test("the last open step ticks the task; any step unticked opens it again", () => {
  const list = parseList("# T\n- [ ] A\n  - [ ] s1\n  - [x] s2\n  - [ ] s3\n");
  const task = list.tasks[0];
  setStepDone(task, 0, true);
  expect(task.done).toBe(false);
  expect(serializeList(list)).toBe("# T\n- [ ] A\n  - [x] s1\n  - [x] s2\n  - [ ] s3\n");
  setStepDone(task, 2, true);
  expect(serializeList(list)).toBe("# T\n- [x] A\n  - [x] s1\n  - [x] s2\n  - [x] s3\n");
  expect(allDone(list)).toBe(true);
  // Unticking one step of a done task opens the task and leaves the other steps done.
  setStepDone(task, 1, false);
  expect(serializeList(list)).toBe("# T\n- [ ] A\n  - [x] s1\n  - [ ] s2\n  - [x] s3\n");
  expect(allDone(list)).toBe(false);
});

test("a single step decides its task both ways, and toggling twice is back where it started", () => {
  const text = "# T\n- [ ] A\n  - [ ] only\n";
  const list = parseList(text);
  setStepDone(list.tasks[0], 0, true);
  expect(serializeList(list)).toBe("# T\n- [x] A\n  - [x] only\n");
  setStepDone(list.tasks[0], 0, false);
  expect(serializeList(list)).toBe(text);
  setTaskDone(list.tasks[0], true);
  setTaskDone(list.tasks[0], false);
  expect(serializeList(list)).toBe(text);
});

test("many steps: the task is done only after the very last one, whatever the order", () => {
  const n = 200;
  const text = "# T\n- [ ] big\n" + Array.from({ length: n }, (_, k) => `  - [ ] step ${k}\n`).join("");
  const list = parseList(text);
  const task = list.tasks[0];
  // Tick from the end backwards: only the final tick (step 0) finishes the task.
  for (let k = n - 1; k > 0; k--) {
    setStepDone(task, k, true);
    expect(task.done).toBe(false);
  }
  setStepDone(task, 0, true);
  expect(task.done).toBe(true);
  expect(task.steps.every((s: any) => s.done)).toBe(true);
  setTaskDone(task, false);
  expect(serializeList(list)).toBe(text);
});

test("unicode and emoji titles are kept when a task and its steps tick together", () => {
  const text = "# Café ☕\n- [ ] 🥛 Milch für Oma — 牛奶\n  - [ ] Ask 👵\n  - [ ] [x] looks ticked\n";
  const list = parseList(text);
  setTaskDone(list.tasks[0], true);
  expect(serializeList(list)).toBe("# Café ☕\n- [x] 🥛 Milch für Oma — 牛奶\n  - [x] Ask 👵\n  - [x] [x] looks ticked\n");
  setStepDone(list.tasks[0], 1, false);
  expect(serializeList(list)).toBe("# Café ☕\n- [ ] 🥛 Milch für Oma — 牛奶\n  - [x] Ask 👵\n  - [ ] [x] looks ticked\n");
});

test("the last step of the last open task is what makes the whole list done", () => {
  const list = parseList("# T\n- [x] a\n- [ ] b\n  - [x] b1\n  - [ ] b2\n- [x] c\n");
  expect(allDone(list)).toBe(false);
  setStepDone(list.tasks[1], 1, true);
  expect([openCount(list), allDone(list)]).toEqual([0, true]);
});

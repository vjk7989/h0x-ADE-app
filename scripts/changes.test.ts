import { test, expect } from "bun:test";
import { readFile } from "node:fs/promises";

const source = await readFile(new URL("../defaults/mods/changes/main.js", import.meta.url), "utf8");
const rowsSource = source.slice(source.indexOf("const untab ="), source.indexOf("\nexport default class"));
const diffRows = new Function(`${rowsSource}; return diffRows;`)();

test("Changes keeps tab indentation, to the next 4-column stop", () => {
  const rows = diffRows(["@@ -1,2 +1,2 @@", "-\tif x {", "+\t\tif x {", " a\tb", "+    spaces"]);
  expect(rows.map((r: { text: string }) => r.text)).toEqual(["@@ -1,2 +1,2 @@", "    if x {", "        if x {", "a   b", "    spaces"]);
});

// A timer turn runs every promise continuation that is ready.
const settled = () => new Promise(resolve => setTimeout(resolve));
const held = () => { let release!: () => void; const wait = new Promise<void>(resolve => { release = resolve; }); return { wait, release }; };
const never = new Promise<void>(() => {});
type Answer<T> = T & { wait?: Promise<void> };

// The panel on a machine with git repos /repo-a and /repo-b; any other folder is not a repo.
// Each git status in a repo, and each read of a file, takes the next answer queued for it, after its
// wait; without one, a repo has one untracked file (a.txt, b.txt) and a file reads "text".
async function panel(statuses: Record<string, Answer<{ files: string[] }>[]> = {}, reads: Record<string, Answer<{ text: string }>[]> = {}) {
  const at = { cwd: "/repo-a" };
  const asked: string[] = [];
  const Changes = new Function("View", "InputState", "spaces", "panes", "focused", "exec", "readTextFile", "watchFolder",
    `${source.replace(/^import .*;\n/gm, "").replace("export default class Changes", "class Changes")}; return Changes;`)(
    class {},
    { new: () => ({ on() {}, value: () => "", set_value() {} }) },
    () => [{ active: true, folder: at.cwd, machine: null }], () => [], () => null,
    async (_git: string, args: string[], { cwd }: { cwd: string }) => {
      const repo = /^\/repo-[ab]/.exec(cwd)?.[0];
      if (!repo) return { code: 128, stdout: "", stderr: "not a git repository" };
      if (args.includes("--show-toplevel")) return { code: 0, stdout: `${repo}\n`, stderr: "" };
      if (args.includes("status")) {
        asked.push(repo);
        const answer = statuses[repo]?.shift() ?? { files: [`${repo.slice(-1)}.txt`] };
        await answer.wait;
        return { code: 0, stdout: answer.files.map(f => `?? ${f}\0`).join(""), stderr: "" };
      }
      if (args.includes("HEAD")) return { code: 128, stdout: "", stderr: "no HEAD" };
      return { code: 0, stdout: "", stderr: "" };
    },
    async (path: string) => {
      const answer = reads[path]?.shift() ?? { text: "text\n" };
      await answer.wait;
      return answer.text;
    },
    () => new Promise(() => {}),
  );
  const view = new Changes();
  const cx = { focus_handle: () => ({}), timer: { every() {} }, notify() {}, spawn(task: (cx: unknown) => Promise<unknown>) { task(cx); } };
  view.init({}, cx);
  await settled();
  const show = async (cwd: string) => { at.cwd = cwd; view.refresh(cx); await settled(); };
  return {
    view, cx, show, asked,
    paths: () => view.files.map((f: { path: string }) => f.path),
    preview: () => view.rows.map((r: { text: string }) => r.text),
  };
}

test.each([
  ["Bengali spacing marks", "বাংলা ".repeat(8) + "END", 51],
  ["tab-indented CJK glyphs", "\t" + "界".repeat(20) + "END", 47],
])("Changes leaves horizontal room for %s", async (_name, text, columns) => {
  const h = await panel({}, { "/repo-a/a.txt": [{ text }] });
  expect(h.view.longest).toBeGreaterThanOrEqual(columns);
});

test("Changes follows a space switch while the last repo's preview never finishes", async () => {
  const h = await panel({}, { "/repo-a/a.txt": [{ text: "", wait: never }] });
  expect(h.paths()).toEqual(["a.txt"]);
  await h.show("/repo-b");
  expect(h.paths()).toEqual(["b.txt"]);
  await h.show("/plain");
  expect(h.paths()).toEqual([]);
  expect(h.view.error).toBe("Not a git repository");
});

test("Changes runs one load at a time in a repo, even when the one it left finishes late", async () => {
  const preview = held(), status = held();
  const h = await panel({ "/repo-b": [{ files: ["b.txt"], wait: status.wait }] }, { "/repo-a/a.txt": [{ text: "text\n", wait: preview.wait }] });
  await h.show("/repo-b");
  preview.release();
  await settled();
  h.view.refresh(h.cx);
  await settled();
  expect(h.asked).toEqual(["/repo-a", "/repo-b"]);
  status.release();
  await settled();
  expect(h.paths()).toEqual(["b.txt"]);
  expect(h.view.loading).toBe(false);
});

test("Changes keeps the newer list when a load from an earlier visit to the repo finishes last", async () => {
  const first = held();
  const h = await panel({ "/repo-a": [{ files: ["old.txt"], wait: first.wait }, { files: ["new.txt"] }] });
  await h.show("/repo-b");
  await h.show("/repo-a");
  expect(h.paths()).toEqual(["new.txt"]);
  first.release();
  await settled();
  expect(h.paths()).toEqual(["new.txt"]);
  expect(h.view.loading).toBe(false);
});

test("Changes keeps the newer preview when one from an earlier visit to the repo finishes last", async () => {
  const first = held();
  const h = await panel({}, { "/repo-a/a.txt": [{ text: "old\n", wait: first.wait }, { text: "new\n" }] });
  await h.show("/repo-b");
  await h.show("/repo-a");
  expect(h.preview()).toEqual(["@@ -0,0 +1,1 @@", "new"]);
  first.release();
  await settled();
  expect(h.preview()).toEqual(["@@ -0,0 +1,1 @@", "new"]);
});

// Edge cases (UPSTREAM-SYNC-2 A7): the diff follows the editor font from settings.
function recorder() {
  const nodes: any[] = [];
  const make = () => {
    const node: any = { calls: {} as Record<string, unknown[]>, children: [] as any[] };
    nodes.push(node);
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
  return { nodes, make };
}
function diffPanel(font: Record<string, unknown>) {
  const { nodes, make } = recorder();
  const lists: any[][] = [];
  const globals = {
    View: class {}, div: make, v_flex: make, h_flex: make, Input: { new: make }, Scrollbar: { vertical: make },
    InputState: { new: () => ({ on() {}, value: () => "", set_value() {} }) },
    v_virtual_list: (...args: any[]) => { lists.push(args); return make(); },
    spaces: () => [{ active: true, folder: "/repo", machine: null }], panes: () => [], focused: () => null,
    exec: async () => ({ code: 128, stdout: "", stderr: "" }), readTextFile: async () => "", watchFolder: () => new Promise(() => {}),
    theme: () => new Proxy({}, { get: () => "#000000" }), settings: () => font, windowVisible: () => true, layout: () => null,
  };
  const Changes = new Function(...Object.keys(globals), `${source.replace(/^import .*;\n/gm, "").replace("export default class Changes", "class Changes")}; return Changes;`)(...Object.values(globals));
  const view = new Changes();
  // Already showing this folder: the render draws, it doesn't start a load.
  Object.assign(view, { files: [], message: "", handle: {}, input: {}, context: { key: JSON.stringify({ cwd: "/repo", on: null }) } });
  view.setLines([]);
  const render = () => { nodes.length = 0; lists.length = 0; view.render({ notify() {} }); return { nodes, list: lists[0] }; };
  return { view, render, font };
}

test("an empty diff keeps the minimum gutter and no width of its own", () => {
  const { view } = diffPanel({ font_size: 11 });
  expect(view.longest).toBe(0);
  expect(view.gutterChars).toBe(4);
  expect(view.gutter(11)).toBe(31);
});

test("the gutter grows past four digits, one digit at a time", () => {
  const { view } = diffPanel({ font_size: 11 });
  view.setLines(["@@ -9997,2 +9998,2 @@", " a", " b"]);
  expect(view.gutterChars).toBe(4); // 9998, 9999
  view.setLines(["@@ -9998,3 +9999,3 @@", " a", " b"]);
  expect(view.gutterChars).toBe(5); // 9999, 10000
  view.setLines(["@@ -1 +123456 @@", "+x"]);
  expect(view.gutterChars).toBe(6);
});

for (const [size, row] of [[11, 20], [8, 15], [22, 40]]) test(`a ${size}px editor font draws ${row}px diff rows`, () => {
  const { view, render } = diffPanel({ font_size: size, font_family: "Fira Code" });
  view.setLines(["@@ -1,1 +1,1 @@", "-a", "+" + "x".repeat(100)]);
  const { nodes, list } = render();
  const area = nodes.find((n: any) => "font_family" in n.calls);
  expect(area.calls.font_family).toEqual(["Fira Code"]);
  expect(area.calls.text_size).toEqual([size]);
  expect(list[2]).toBe(row);
  // The diff is as wide as its longest line in this font, plus the gutter.
  const wide = 100 * size * 0.64 + view.gutter(size) + 20;
  expect(nodes.some((n: any) => typeof n.calls.min_w?.[0] === "number" && Math.abs(n.calls.min_w[0] - Math.max(300, wide)) < 1e-6)).toBe(true);
  const rows = list[4]({ start: 0, end: 3 }).map((r: any) => r.node);
  expect(rows.every((r: any) => r.calls.h?.[0] === row)).toBe(true);
  expect(rows[1].children[0].calls.w).toEqual([view.gutter(size)]);
});

test("the diff follows a font change at the next render, and falls back to JetBrains Mono", () => {
  const { render, font } = diffPanel({ font_size: 11, font_family: null });
  const area = (r: any) => r.nodes.find((n: any) => "font_family" in n.calls);
  expect(area(render()).calls.font_family).toEqual(["JetBrains Mono"]);
  Object.assign(font, { font_size: 16, font_family: "Iosevka" });
  const next = render();
  expect(area(next).calls.font_family).toEqual(["Iosevka"]);
  expect(area(next).calls.text_size).toEqual([16]);
  expect(next.list[2]).toBe(29);
});

import { test, expect } from "bun:test";
import { readdir, readFile } from "node:fs/promises";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

// The mod runtime's cx rule: the cx an event handler (on_click, on_key_down, an input's submit/blur, render …)
// receives is only valid during that call. After a real await it throws StaleContext. The cx of cx.spawn, the
// timers, init and close survives awaits; a Promise executor runs at once. This guard reads every shipped defaults/**/*.js and flags:
//   A. a handler (a function taking cx that is not a cx.spawn/cx.timer callback, init or close) that awaits and
//      then uses its cx, or hands its cx to an async this.method that does (directly or through other methods);
//   B. a function without a cx parameter that uses an outer cx, unless it is a synchronous callback (map, when …).
// A line with `// cx-safe` (or the line above it) opts that finding out.

type Tok = { t: "id" | "num" | "str" | "re" | "p"; v: string; i: number; line: number };
type Fn = {
  kind: "arrow" | "method";
  name: string | null;
  async: boolean;
  params: [number, number];
  body: [number, number];
  cx: boolean;
  callee: string[];
  line: number;
};
export type Finding = { line: number; rule: "A" | "B"; message: string };

const KEYWORDS_BEFORE_REGEX = new Set(["return", "typeof", "case", "do", "else", "in", "of", "new", "delete", "void", "throw", "instanceof", "yield", "await"]);
const NOT_METHODS = new Set(["if", "for", "while", "switch", "catch", "with"]);
const SYNC_CALLBACKS = new Set(["map", "forEach", "filter", "find", "findIndex", "findLast", "findLastIndex", "some", "every", "reduce", "flatMap", "sort", "when", "hover", "active", "Promise"]);
const STATEMENT_START = new Set(["const", "let", "var", "return", "if", "for", "while", "class", "export", "import", "function"]);

function tokenize(src: string): { toks: Tok[]; safe: Set<number> } {
  const toks: Tok[] = [];
  const safe = new Set<number>();
  const stack: ("{" | "${")[] = [];
  let i = 0, line = 1;
  const push = (t: Tok["t"], v: string, at: number, l: number) => toks.push({ t, v, i: toks.length, line: l });
  const template = () => {
    // i is just after a backtick or a template's closing brace.
    while (i < src.length) {
      const c = src[i];
      if (c === "\\") { i += 2; continue; }
      if (c === "\n") line++;
      if (c === "`") { i++; return; }
      if (c === "$" && src[i + 1] === "{") { i += 2; stack.push("${"); return; }
      i++;
    }
  };
  while (i < src.length) {
    const c = src[i], n = src[i + 1];
    if (c === "\n") { line++; i++; continue; }
    if (/\s/.test(c)) { i++; continue; }
    if (c === "/" && n === "/") {
      const end = src.indexOf("\n", i);
      const text = src.slice(i, end < 0 ? src.length : end);
      if (text.includes("cx-safe")) safe.add(line);
      i = end < 0 ? src.length : end;
      continue;
    }
    if (c === "/" && n === "*") {
      const end = src.indexOf("*/", i + 2), stop = end < 0 ? src.length : end + 2;
      const text = src.slice(i, stop);
      if (text.includes("cx-safe")) safe.add(line + (text.match(/\n/g)?.length ?? 0));
      line += text.match(/\n/g)?.length ?? 0;
      i = stop;
      continue;
    }
    if (c === '"' || c === "'") {
      const at = line;
      i++;
      while (i < src.length && src[i] !== c && src[i] !== "\n") i += src[i] === "\\" ? 2 : 1;
      i++;
      push("str", "", 0, at);
      continue;
    }
    if (c === "`") { push("str", "", 0, line); i++; template(); continue; }
    if (c === "/") {
      const prev = toks[toks.length - 1];
      const regex = !prev || (prev.t === "p" && !")]}".includes(prev.v)) || (prev.t === "id" && KEYWORDS_BEFORE_REGEX.has(prev.v));
      if (regex) {
        i++;
        let cls = false;
        while (i < src.length && src[i] !== "\n") {
          const r = src[i];
          if (r === "\\") { i += 2; continue; }
          if (r === "[") cls = true;
          else if (r === "]") cls = false;
          else if (r === "/" && !cls) break;
          i++;
        }
        i++;
        while (/[a-z]/i.test(src[i] ?? "")) i++;
        push("re", "", 0, line);
        continue;
      }
    }
    if (/[A-Za-z_$]/.test(c)) {
      let j = i + 1;
      while (/[\w$]/.test(src[j] ?? "")) j++;
      push("id", src.slice(i, j), 0, line);
      i = j;
      continue;
    }
    if (/[0-9]/.test(c)) {
      let j = i + 1;
      while (/[\w.]/.test(src[j] ?? "")) j++;
      push("num", src.slice(i, j), 0, line);
      i = j;
      continue;
    }
    if (c === "=" && n === ">") { push("p", "=>", 0, line); i += 2; continue; }
    if (c === "=" || c === "!") {
      // == / === / != / !== stay one token so a later ">" is never read as part of "=>".
      let j = i + 1;
      while (src[j] === "=") j++;
      push("p", src.slice(i, j), 0, line);
      i = j;
      continue;
    }
    if (c === "?" && n === "." && !/[0-9]/.test(src[i + 2] ?? "")) { push("p", ".", 0, line); i += 2; continue; }
    if (c === "." && n === "." && src[i + 2] === ".") { push("p", "...", 0, line); i += 3; continue; }
    if (c === "{") stack.push("{");
    if (c === "}") {
      if (stack.pop() === "${") { i++; template(); continue; }
    }
    push("p", c, 0, line);
    i++;
  }
  return { toks, safe };
}

export function scan(src: string): Finding[] {
  const { toks, safe } = tokenize(src);
  const match = new Map<number, number>();
  const opens: number[] = [];
  for (const tk of toks) {
    if (tk.t !== "p") continue;
    if ("([{".includes(tk.v)) opens.push(tk.i);
    else if (")]}".includes(tk.v)) { const o = opens.pop(); if (o !== undefined) { match.set(o, tk.i); match.set(tk.i, o); } }
  }
  const is = (k: number, v: string) => toks[k]?.t === "p" && toks[k].v === v;
  const id = (k: number, v?: string) => toks[k]?.t === "id" && (v === undefined || toks[k].v === v);
  const hasCx = (a: number, b: number) => toks.slice(a, b + 1).some((tk, n) => tk.v === "cx" && tk.t === "id" && !is(a + n - 1, "."));

  // The callee of the call whose argument list directly holds token k, as ["cx", "timer", "every"].
  const calleeOf = (k: number): string[] => {
    const before = toks[k - 1];
    if (!before || before.t !== "p" || (before.v !== "(" && before.v !== ",")) return [];
    let open = k - 1;
    if (before.v === ",") {
      let d = 0;
      for (open = k - 1; open >= 0; open--) {
        const tk = toks[open];
        if (tk.t !== "p") continue;
        if (")]}".includes(tk.v)) d++;
        else if ("([{".includes(tk.v)) { if (!d) break; d--; }
      }
      if (!is(open, "(")) return [];
    }
    const chain: string[] = [];
    for (let p = open - 1; id(p); p -= 2) {
      chain.unshift(toks[p].v);
      if (!is(p - 1, ".")) break;
    }
    return chain;
  };

  const fns: Fn[] = [];
  for (const tk of toks) {
    const k = tk.i;
    if (is(k, "=>")) {
      let start: number, params: [number, number];
      if (is(k - 1, ")")) { start = match.get(k - 1)!; params = [start, k - 1]; }
      else if (id(k - 1)) { start = k - 1; params = [k - 1, k - 1]; }
      else continue;
      const async = id(start - 1, "async");
      let body: [number, number];
      if (is(k + 1, "{")) body = [k + 1, match.get(k + 1) ?? toks.length - 1];
      else {
        let e = k + 1, d = 0;
        for (; e < toks.length; e++) {
          const v = toks[e].t === "p" ? toks[e].v : "";
          if ("([{".includes(v) && v) d++;
          else if (")]}".includes(v) && v) { if (!d) break; d--; }
          else if (!d && (v === "," || v === ";")) break;
          else if (!d && e > k + 1 && toks[e].line > toks[e - 1].line && id(e) && STATEMENT_START.has(toks[e].v)) break;
        }
        body = [k + 1, e - 1];
      }
      const callee = calleeOf(async ? start - 1 : start);
      fns.push({ kind: "arrow", name: null, async, params, body, cx: hasCx(params[0], params[1]), callee, line: tk.line });
    } else if (tk.t === "id" && !NOT_METHODS.has(tk.v) && is(k + 1, "(") && match.has(k + 1) && is(match.get(k + 1)! + 1, "{") && !is(k - 1, ".")) {
      const close = match.get(k + 1)!, open = close + 1;
      const async = id(k - 1, "async") || (id(k - 1, "function") && id(k - 2, "async"));
      const name = tk.v === "function" ? null : tk.v;
      fns.push({ kind: "method", name, async, params: [k + 1, close], body: [open, match.get(open) ?? toks.length - 1], cx: hasCx(k + 1, close), callee: [], line: tk.line });
    }
  }

  const within = (f: Fn, k: number) => (k >= f.params[0] && k <= f.params[1]) || (k >= f.body[0] && k <= f.body[1]);
  // Innermost first.
  const chainAt = (k: number) => fns.filter((f) => within(f, k)).sort((a, b) => (a.body[1] - a.params[0]) - (b.body[1] - b.params[0]));
  const isTask = (f: Fn) => (f.kind === "method" && (f.name === "init" || f.name === "close")) || (f.kind === "arrow" && (f.callee.at(-1) === "spawn" || f.callee.includes("timer")));
  const isSync = (f: Fn) => f.kind === "arrow" && !f.async && SYNC_CALLBACKS.has(f.callee.at(-1) ?? "");

  const findings: Finding[] = [];
  const report = (line: number, rule: Finding["rule"], message: string) => {
    if (safe.has(line) || safe.has(line - 1)) return;
    if (!findings.some((f) => f.line === line && f.rule === rule)) findings.push({ line, rule, message });
  };

  // Every cx use: the function it belongs to and whether only synchronous callbacks lie in between.
  const reach = new Map<Fn, number[]>();
  const awaits = new Map<Fn, number[]>();
  for (const tk of toks) {
    const k = tk.i;
    if (id(k, "await")) { const own = chainAt(k)[0]; if (own) awaits.set(own, [...(awaits.get(own) ?? []), k]); continue; }
    if (!id(k, "cx") || is(k - 1, ".")) continue;
    const chain = chainAt(k);
    const at = chain.findIndex((f) => f.cx);
    if (at < 0 || (k >= chain[at].params[0] && k <= chain[at].params[1])) continue;
    const between = chain.slice(0, at);
    const capturing = between.find((f) => !isSync(f));
    if (capturing) report(tk.line, "B", `a function without a cx parameter (line ${capturing.line}) uses the cx of line ${chain[at].line}; it may run after that call ended`);
    else reach.set(chain[at], [...(reach.get(chain[at]) ?? []), k]);
  }

  const loops = (f: Fn): [number, number][] => {
    const out: [number, number][] = [];
    for (let k = f.body[0]; k <= f.body[1]; k++) {
      if (!id(k, "for") && !id(k, "while") && !id(k, "do")) continue;
      if (chainAt(k)[0] !== f) continue;
      let after = k + 1;
      if (is(after, "(")) after = match.get(after)! + 1;
      let end = after;
      if (is(after, "{")) end = match.get(after)!;
      else for (let d = 0; end <= f.body[1]; end++) {
        const v = toks[end].t === "p" ? toks[end].v : "";
        if (v && "([{".includes(v)) d++;
        else if (v && ")]}".includes(v)) d--;
        else if (v === ";" && !d) break;
      }
      out.push([k, end]);
    }
    return out;
  };
  // The cx use that may run after one of f's own awaits, if any.
  const afterAwait = (f: Fn): number | null => {
    const as = awaits.get(f) ?? [], cs = reach.get(f) ?? [];
    if (!f.async || !as.length) return null;
    const late = cs.find((c) => c > as[0]);
    if (late !== undefined) return late;
    for (const [a, b] of loops(f)) {
      if (as.some((x) => x >= a && x <= b)) { const c = cs.find((x) => x >= a && x <= b); if (c !== undefined) return c; }
    }
    return null;
  };
  // this.NAME(… cx …) calls a function makes with its own cx.
  const calls = (f: Fn): { name: string; at: number }[] => (reach.get(f) ?? []).flatMap((c) => {
    let d = 0, open = c - 1;
    for (; open >= 0; open--) {
      const tk = toks[open];
      if (tk.t !== "p") continue;
      if (")]}".includes(tk.v)) d++;
      else if ("([{".includes(tk.v)) { if (!d) break; d--; }
    }
    return is(open, "(") && id(open - 1) && is(open - 2, ".") && id(open - 3, "this") ? [{ name: toks[open - 1].v, at: open - 3 }] : [];
  });

  const methods = fns.filter((f) => f.kind === "method" && f.cx && f.name);
  const risky = new Map<string, string>();
  for (const m of methods) if (afterAwait(m) !== null) risky.set(m.name!, `${m.name}, which awaits and then uses cx (line ${toks[afterAwait(m)!].line})`);
  for (let grew = true; grew; ) {
    grew = false;
    for (const m of methods) {
      if (risky.has(m.name!)) continue;
      const via = calls(m).find((c) => risky.has(c.name));
      if (via) { risky.set(m.name!, `${m.name} → ${risky.get(via.name)}`); grew = true; }
    }
  }

  for (const f of fns) {
    if (!f.cx || isTask(f) || (f.kind === "method" && f.name !== "render")) continue;
    const late = afterAwait(f);
    if (late !== null) report(toks[late].line, "A", `a handler's cx is used after an await`);
    for (const c of calls(f)) if (risky.has(c.name)) report(toks[c.at].line, "A", `a handler's cx reaches ${risky.get(c.name)}; run it in cx.spawn(async (cx) => …)`);
  }
  return findings.sort((a, b) => a.line - b.line);
}

const root = fileURLToPath(new URL("../defaults/", import.meta.url));
async function shipped(dir: string): Promise<string[]> {
  const out: string[] = [];
  for (const e of await readdir(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) out.push(...(await shipped(p)));
    else if (e.name.endsWith(".js")) out.push(p);
  }
  return out;
}

test("no shipped mod uses a handler's cx after an await or from a later callback", async () => {
  const files = await shipped(root);
  expect(files.length).toBeGreaterThan(10);
  const found: string[] = [];
  for (const file of files) {
    for (const f of scan(await readFile(file, "utf8"))) found.push(`${relative(root, file).replaceAll("\\", "/")}:${f.line} ${f.rule} ${f.message}`);
  }
  expect(found).toEqual([]);
});

const VIEW = (body: string) => `export default class V extends View {\n${body}\n}\n`;

test("the guard flags a click, a key and an input submit that reach an async method using cx after an await", () => {
  const src = VIEW(`
  async remove(cx) { await removeAppFile("x"); cx.notify(); }
  key(e, cx) { if (e.key === "enter") this.remove(cx); }
  render(cx) {
    const input = this.field((v, cx) => this.remove(cx));
    return div().on_click((_e, cx) => this.remove(cx))
      .on_key_down((e, cx) => this.key(e, cx));
  }`);
  expect(scan(src).map((f) => [f.line, f.rule])).toEqual([[6, "A"], [7, "A"], [8, "A"]]);
});

test("the guard flags a sleep loop and an async handler that awaits first", () => {
  const src = VIEW(`
  async drain(cx) { while (this.busy) await cx.sleep(50); }
  render(cx) {
    return div().on_click((_e, cx) => this.drain(cx))
      .on_hover(async (h, cx) => { await later(); cx.notify(); });
  }`);
  expect(scan(src).map((f) => [f.line, f.rule])).toEqual([[5, "A"], [6, "A"]]);
});

test("the guard accepts the same calls inside cx.spawn, timers, init and close", () => {
  const src = VIEW(`
  init(_p, cx) { this.load(cx); cx.timer.every(1000, (cx) => this.load(cx)); }
  async load(cx) { await readTextFile("a"); cx.notify(); }
  async close(cx) { while (this.busy) await cx.sleep(50); }
  render(cx) {
    return div().on_click((_e, cx) => { this.busy = true; cx.notify(); cx.spawn(async (cx) => this.load(cx)); })
      .children([1, 2].map((n) => this.row(n, cx)))
      .when(true, (el) => el.child(this.row(0, cx)));
  }`);
  expect(scan(src)).toEqual([]);
});

test("the guard flags a later callback that uses an outer cx and honours cx-safe", () => {
  const src = VIEW(`
  render(cx) {
    readTextFile("a").then(() => cx.notify());
    cx.timer.after(0, () => cx.notify());
    cx.spawn(async () => { await later(); cx.notify(); }); // cx-safe
    return div();
  }`);
  expect(scan(src).map((f) => [f.line, f.rule])).toEqual([[4, "B"], [5, "B"]]);
});

test("the guard is not fooled by cx in strings, comments, regexes and templates", () => {
  const src = VIEW(`
  async remove(cx) { await x(); cx.notify(); }
  render(cx) {
    const re = /[\\s"'$\`\\\\]/; // this.remove(cx) in a comment
    return div().child("this.remove(cx)").child(\`\${n} this.remove(cx)\`).on_click((_e, cx) => { cx.notify(); });
  }`);
  expect(scan(src)).toEqual([]);
});

test("the guard finds a handler inside a template expression", () => {
  const src = VIEW(`
  async remove(cx) { await x(); cx.notify(); }
  render(cx) { return div().child(\`\${this.row((_e, cx) => this.remove(cx))}\`); }`);
  expect(scan(src).map((f) => [f.line, f.rule])).toEqual([[4, "A"]]);
});

// Workspaces' engine: saves the whole window (every space on every machine: its tabs, splits and each agent with its
// session, and which space and tabs were shown) and restores it.
// Storage is shared("workspaces") = { version: 2, items: [ { id, name, created, saved, active, spaces: [ { name, machine,
// folder, activeTab, tabs } ] } ] }, newest first, at most MAX; an item of version 1 (one space: machine, folder, space,
// tabs) reads as a workspace of one space. `active` and `activeTab` are indexes. A leaf is { cwd, argv?, terminal?, agent?, session? }; a split is { split, children }.
// Nothing here is called while drawing except read(): the rest runs from clicks and tasks.

import { spaces, panes, terminals, trees, machines, core, home, knownAgents, run, shared, setShared, window } from "h0x";

export const MAX = 5;
const MAX_SPACES = 30;
const KEY = "workspaces";

// Stored maps (shared values people's files can hold anything in): only a
// plain object counts, and only its own keys ("constructor" is a name too).
const plain = (v) => (v && typeof v === "object" && !Array.isArray(v) ? v : null);
const has = (o, k) => Boolean(plain(o)) && Object.prototype.hasOwnProperty.call(o, k);
const own = (o, k) => (has(o, k) ? o[k] : undefined);

// One `h0x-core session <verb>`: its JSON, or { error: { kind, message } }.
// An older core without `session` answers kind "usage".
async function ask(on, verb, rest) {
  const args = ["session", verb, "--json", ...(on ? (on.startsWith("-") ? [`--on=${on}`] : ["--on", on]) : []), ...(rest.length ? ["--", ...rest] : [])];
  let r;
  try {
    r = await core(args);
  } catch (e) {
    return { error: { kind: "unavailable", message: String(e?.message ?? e) } };
  }
  if (r.code === 0) {
    try {
      return JSON.parse(r.stdout);
    } catch {
      return { error: { kind: "unreadable", message: "h0x-core answered something unreadable." } };
    }
  }
  try {
    const error = JSON.parse(r.stderr || r.stdout).error;
    if (error?.kind) return { error };
  } catch {}
  const text = (r.stderr || r.stdout || "").trim();
  return { error: { kind: /unrecognized subcommand/i.test(text) ? "usage" : "failed", message: text || "h0x-core failed." } };
}

const first = (node) => (node.children ? first(node.children[0]) : node);
export const leaves = (node) => (node.children ? node.children.flatMap(leaves) : [node]);
const used = (agent, id) => `${agent}\n${id}`;

// A tab's pane tree as a layout keeps it: a pane `leaf(terminal)` describes, or
// { split: "right" | "down", children }. Mod panes are left out.
function shape(node, leaf) {
  if (node?.type === "block") return node.content?.terminal ? leaf(node.content.terminal) : null;
  const children = (node?.children ?? []).map((c) => shape(c, leaf)).filter(Boolean);
  if (children.length < 2) return children[0] ?? null;
  return { split: node.type === "column" ? "down" : "right", children };
}

// A saved node as restore can use it, else null: bad parts are left out.
function sound(node) {
  if (!plain(node)) return null;
  if (Array.isArray(node.children)) {
    const children = node.children.map(sound).filter(Boolean);
    if (children.length < 2) return children[0] ?? null;
    return { split: node.split === "down" ? "down" : "right", children };
  }
  const leaf = { cwd: typeof node.cwd === "string" ? node.cwd : null };
  // Only a program that is not a plain shell is marked: an older save without it restarts as a shell.
  if (node.shell === false) leaf.shell = false;
  if (Array.isArray(node.argv) && node.argv.every((w) => typeof w === "string")) leaf.argv = node.argv.slice();
  if (typeof node.terminal === "string" && node.terminal) leaf.terminal = node.terminal;
  if (typeof node.agent === "string" && node.agent) {
    leaf.agent = node.agent;
    leaf.session = typeof node.session === "string" ? node.session : null;
    leaf.argv ??= [];
  }
  return leaf;
}

const soundTabs = (tabs) =>
  (Array.isArray(tabs) ? tabs : []).flatMap((t) => {
    const root = sound(plain(t)?.root);
    return root ? [{ name: typeof t.name === "string" ? t.name : "", root }] : [];
  });

const text = (v, fallback = "") => (typeof v === "string" ? v : fallback);
const num = (v) => (typeof v === "number" && Number.isFinite(v) ? v : 0);
const base = (folder) => folder.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || folder;

// A stored item as { id, name, created, saved, active, spaces }, or null. Bad spaces are left out (a space with no
// usable tab); `active` follows its space.
function itemOf(it) {
  if (!plain(it) || typeof it.id !== "string" || !it.id || typeof it.name !== "string" || !it.name.trim()) return null;
  const raw = Array.isArray(it.spaces) ? it.spaces : [{ name: it.space, machine: it.machine, folder: it.folder, tabs: it.tabs }];
  const kept = raw.flatMap((sp, index) => {
    if (!plain(sp)) return [];
    const tabs = soundTabs(sp.tabs);
    if (!tabs.length) return [];
    const at = Number.isInteger(sp.activeTab) && sp.activeTab >= 0 && sp.activeTab < tabs.length ? sp.activeTab : 0;
    return [{ index, name: text(sp.name), machine: typeof sp.machine === "string" && sp.machine ? sp.machine : null, folder: text(sp.folder), activeTab: at, tabs }];
  }).slice(0, MAX_SPACES);
  if (!kept.length) return null;
  const active = Math.max(0, kept.findIndex((x) => x.index === it.active));
  return { id: it.id, name: it.name.slice(0, 80), created: num(it.created), saved: num(it.saved), active, spaces: kept.map(({ index, ...sp }) => sp) };
}

/** The stored workspaces, newest first, each valid and fresh (not the stored objects): a value that is not
 * ours reads as none; a bad item is dropped; at most MAX. */
export function read() {
  const items = own(plain(shared(KEY)), "items");
  if (!Array.isArray(items)) return [];
  const seen = new Set();
  const out = [];
  for (const it of items) {
    const item = itemOf(it);
    if (!item || seen.has(item.id)) continue;
    seen.add(item.id);
    out.push(item);
    if (out.length === MAX) break;
  }
  return out;
}

/** Stores the list (newest first). */
export function write(items) {
  setShared(KEY, { version: 2, items });
}

/** The counts of spaces, tabs and panes of a stored workspace, for its row. */
export function counts(item) {
  const tabs = item.spaces.flatMap((sp) => sp.tabs);
  return { spaces: item.spaces.length, tabs: tabs.length, panes: tabs.reduce((n, t) => n + leaves(t.root).length, 0) };
}

/** The whole window as { active, spaces: [{ name, machine, folder, activeTab, tabs: [{ name, root }] }] }, or null
 * (no space has a terminal). Spaces are in the sidebar's order, on every machine; `active` and `activeTab` are indexes
 * of the shown space and of each space's shown tab. Each agent pane has its agent (label h0x.agent), argv and folder;
 * a program pane (not a shell) its argv, folder and `shell: false`; a shell pane its argv and folder; every pane the
 * core address of its terminal. `session` is filled in by saveNew / saveOver. */
export function capture() {
  const all = trees() ?? [];
  const ps = panes();
  const ts = terminals();
  const leaf = (address) => {
    const cwd = ps.find((p) => p.terminal === address)?.cwd ?? null;
    const r = ts.find((x) => x.address === address);
    const agent = r?.labels?.["h0x.agent"];
    const argv = Array.isArray(r?.argv) ? r.argv : [];
    if (agent) return { agent, session: null, argv, cwd, terminal: address };
    // `program.shell` is false for a program started by itself (vim, a server): it starts again from its argv.
    return r?.program && !r.program.shell ? { cwd, argv, terminal: address, shell: false } : { cwd, argv, terminal: address };
  };
  let active = 0;
  const out = [];
  for (const s of spaces()) {
    const tabs = (all.find((t) => t.id === s.id)?.tabs ?? []).map((x) => ({ id: x.id, name: x.name ?? "", root: shape(x.root, leaf) })).filter((x) => x.root);
    if (!tabs.length) continue;
    // The first MAX_SPACES in the sidebar's order are kept; `active` then always names a kept one (0 if the shown one is later).
    if (out.length >= MAX_SPACES) break;
    const shown = s.tabs?.find((x) => x.active)?.id;
    const at = tabs.findIndex((x) => x.id === shown);
    if (s.active) active = out.length;
    out.push({ name: s.name || (s.folder ? base(s.folder) : ""), machine: s.machine ?? null, folder: s.folder ?? "", activeTab: Math.max(0, at), tabs: tabs.map(({ name, root }) => ({ name, root })) });
  }
  return out.length ? { active, spaces: out } : null;
}

// Gives each agent pane the newest session of its agent in its folder (`base`
// when it has none) that no earlier pane took, else null. Known limit: two
// panes of one agent in one folder may get each other's sessions.
async function assign(on, folder, items) {
  const lists = new Map();
  const taken = new Set();
  for (const item of items) {
    const where = item.cwd || folder;
    if (!lists.has(where)) {
      const r = await ask(on, "list", where ? [where] : []);
      lists.set(where, Array.isArray(r.sessions) ? r.sessions : []);
    }
    const x = lists.get(where).find((x) => x.agent === item.agent && !taken.has(used(x.agent, x.id)));
    if (x) taken.add(used(x.agent, x.id));
    item.session = x?.id ?? null;
  }
}

// What an agent pane starts: its session's resume argv, once per session id;
// else (no session, gone, an older core) `fresh`.
async function launch(on, item, fresh, started) {
  if (item.session && !started.has(used(item.agent, item.session))) {
    started.add(used(item.agent, item.session));
    const r = await ask(on, "resume", [item.agent, item.session]);
    if (!r.error && Array.isArray(r.argv) && r.argv.length) return r.argv;
  }
  return fresh?.length ? fresh : undefined;
}

// The app runs a program in the person's shell, so the core's argv is a wrapper (src/programs.rs resolve() reads the
// same two forms): `zsh -ic '"$@"; exec "$0" -i' zsh <cmd>...` (fish: `command $argv[2..]; exec $argv[1] -i`), and on
// Windows `pwsh -NoExit -Command <line>` with the line `powershell_line` builds. The command inside is started again
// (the app wraps it anew); anything that is not exactly one of these forms is returned as it is.
const PS_QUOTES = ["'", "‘", "’", "‚", "‛"];
const PS_START = "& { $PSNativeCommandArgumentPassing = 'Legacy'; $p = ";
const PS_END = "); if ((Get-Command $p -ErrorAction Ignore | Select-Object -First 1).Extension -in '.cmd', '.bat') { $a = $a -replace '\\r?\\n', ' ' }; & $p @a }";
const SHELLS = ["zsh", "bash", "fish", "sh", "dash", "nu", "pwsh", "powershell", "cmd", "elvish", "xonsh", "tcsh", "csh", "ksh", "ion"];
const stem = (p) => p.split(/[\\/]/).pop().replace(/\.exe$/i, "").toLowerCase();

// The quoted word `s` starts with, and what follows it: [word, rest] or null.
function psWord(s) {
  const chars = [...s];
  if (!PS_QUOTES.includes(chars[0])) return null;
  let word = "";
  for (let i = 1; i < chars.length; i++) {
    if (PS_QUOTES.includes(chars[i])) {
      if (PS_QUOTES.includes(chars[i + 1])) { word += chars[i + 1]; i += 1; continue; }
      return [word, chars.slice(i + 1).join("")];
    }
    word += chars[i];
  }
  return null;
}

// One argument back from its Windows command line form (C runtimes' rules, as the app writes them).
function windowsUnquote(arg) {
  let out = "", quoted = false, slashes = 0;
  const chars = [...arg];
  for (let i = 0; i < chars.length; i++) {
    const ch = chars[i];
    if (ch === "\\") { slashes += 1; continue; }
    if (ch === '"') {
      out += "\\".repeat(Math.floor(slashes / 2));
      if (slashes % 2 === 1) out += '"';
      else if (quoted && chars[i + 1] === '"') { out += '"'; i += 1; }
      else quoted = !quoted;
    } else out += "\\".repeat(slashes) + ch;
    slashes = 0;
  }
  return out + "\\".repeat(slashes);
}

// One argument as a Windows command line holds it (the C runtimes' rules, as src/programs.rs windows_arg writes it).
function windowsArg(arg) {
  if (arg !== "" && !/[ \t\n\v"]/.test(arg)) return arg;
  let out = '"';
  let slashes = 0;
  for (const ch of arg) {
    if (ch === "\\") {
      slashes += 1;
      continue;
    }
    out += ch === '"' ? "\\".repeat(2 * slashes) + '""' : "\\".repeat(slashes) + ch;
    slashes = 0;
  }
  return out + "\\".repeat(2 * slashes) + '"';
}

const psQuote = (word) => `'${[...word].map((c) => (PS_QUOTES.includes(c) ? c + c : c)).join("")}'`;

// The line `powershell_line` builds for a command.
function psLine(cmd) {
  return `${PS_START}${psQuote(cmd[0])}; $a = @(${cmd.slice(1).map((a) => psQuote(windowsArg(a))).join(", ")}${PS_END}`;
}

// The words of a `powershell_line`, or null.
function psWords(line) {
  if (!line.startsWith(PS_START) || !line.endsWith(PS_END)) return null;
  const first = psWord(line.slice(PS_START.length, line.length - PS_END.length));
  if (!first || !first[1].startsWith("; $a = @(")) return null;
  const words = [first[0]];
  let rest = first[1].slice("; $a = @(".length);
  while (rest) {
    if (words.length > 1) {
      if (!rest.startsWith(", ")) return null;
      rest = rest.slice(2);
    }
    const next = psWord(rest);
    if (!next) return null;
    words.push(windowsUnquote(next[0]));
    rest = next[1];
  }
  return words;
}

function command(argv) {
  if (argv.length > 4 && SHELLS.includes(stem(argv[0])) && argv[1].startsWith("-") && argv[1].endsWith("c") && argv[3] === argv[0]
    && (argv[2].includes('"$@"') || argv[2].startsWith("command $argv[2..]"))) return argv.slice(4);
  if (argv.length === 4 && SHELLS.includes(stem(argv[0])) && argv[1] === "-NoExit" && argv[2] === "-Command") return psWords(argv[3]) ?? argv;
  return argv;
}

// A snapshot with each agent pane's session found (asks the core: never call while drawing).
async function snapshot() {
  const snap = capture();
  if (!snap) return null;
  for (const sp of snap.spaces) {
    const on = sp.machine;
    await assign(on, sp.folder || (on ? "" : home()), sp.tabs.flatMap((x) => leaves(x.root)).filter((x) => x.agent));
  }
  return snap;
}

const newId = (items) => {
  for (;;) {
    const id = `w${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
    if (!items.some((x) => x.id === id)) return id;
  }
};

/** Saves the whole window as a new workspace named `name` (empty: the shown space's name). Answers the item, "full"
 * (MAX are saved: nothing is written, never one dropped) or "empty" (nothing to save). */
export async function saveNew(name) {
  if (read().length >= MAX) return "full";
  const snap = await snapshot();
  if (!snap) return "empty";
  const items = read();
  if (items.length >= MAX) return "full";
  const now = Date.now();
  const item = {
    id: newId(items),
    name: (name ?? "").trim().slice(0, 80) || snap.spaces[snap.active].name || "Workspace",
    created: now,
    saved: now,
    active: snap.active,
    spaces: snap.spaces,
  };
  write([item, ...items]);
  return item;
}

/** Replaces a workspace's spaces with the window's (name and creation time kept; it moves to the top). Answers the
 * item, "missing" (no such workspace) or "empty". */
export async function saveOver(id) {
  if (!read().some((x) => x.id === id)) return "missing";
  const snap = await snapshot();
  if (!snap) return "empty";
  const items = read();
  const old = items.find((x) => x.id === id);
  if (!old) return "missing";
  const item = { ...old, saved: Date.now(), active: snap.active, spaces: snap.spaces };
  write([item, ...items.filter((x) => x.id !== id)]);
  return item;
}

/** Deletes a workspace. True when it was there. */
export function remove(id) {
  const items = read();
  const rest = items.filter((x) => x.id !== id);
  if (rest.length === items.length) return false;
  write(rest);
  return true;
}

/** Once per install: the layouts Sessions kept (shared "sessions", under each machine and folder, the older home of
 * this) become workspaces, up to MAX in all. The old value is left as it was. Returns { added, skipped }. */
export function importOld() {
  if (shared(`${KEY}.imported`) === true) return { added: 0, skipped: 0, done: true };
  const items = read();
  const old = plain(shared("sessions"));
  let added = 0;
  let skipped = 0;
  for (const key of Object.keys(old ?? {})) {
    let where;
    try {
      where = JSON.parse(key);
    } catch {
      continue;
    }
    if (!Array.isArray(where) || typeof where[1] !== "string") continue;
    const layouts = plain(own(plain(own(old, key)), "layouts"));
    for (const name of Object.keys(layouts ?? {})) {
      const tabs = soundTabs(own(plain(own(layouts, name)), "tabs"));
      if (!tabs.length || !name.trim()) continue;
      if (items.length >= MAX) {
        skipped += 1;
        continue;
      }
      const now = Date.now();
      items.push({ id: newId(items), name: name.slice(0, 80), created: now, saved: now, active: 0, spaces: [{ name: base(where[1]), machine: typeof where[0] === "string" && where[0] ? where[0] : null, folder: where[1], activeTab: 0, tabs }] });
      added += 1;
    }
  }
  if (added) write(items);
  setShared(`${KEY}.imported`, true);
  return { added, skipped };
}

// Commands apply after run() returns: polls (50 ms, at most 10 s) until `find` answers.
async function until(cx, find) {
  for (let i = 0; i < 200; i++) {
    const found = find();
    if (found) return found;
    await cx.sleep(50);
  }
  return null;
}

// Folders as the app compares them: slashes and a trailing one don't count; case doesn't on Windows.
const norm = (p) => {
  const q = String(p ?? "").replaceAll("\\", "/").replace(/\/+$/, "");
  return window()?.platform === "windows" ? q.toLowerCase() : q;
};
const sameArgv = (a, b) => Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((w, i) => w === b[i]);

// Whether the saved pane is still there: its terminal lives, a pane of the space shows it, and it runs the same
// program (a reused address after a reboot must not count).
function open(leaf, space) {
  if (!leaf.terminal) return false;
  const tabs = new Set((space.tabs ?? []).map((t) => t.id));
  if (!panes().some((p) => p.terminal === leaf.terminal && tabs.has(p.tab))) return false;
  const r = terminals().find((x) => x.address === leaf.terminal);
  if (r?.state !== "live") return false;
  return leaf.agent ? r.labels?.["h0x.agent"] === leaf.agent && runsAgent(r, leaf) : sameArgv(r.argv, leaf.argv);
}

// What a program is called: its file's name without the folder or an extension that runs it (opencode.CMD -> opencode).
const program = (word) => String(word ?? "").split(/[\\/]/).pop().replace(/\.(exe|cmd|bat|com)$/i, "").toLowerCase();

// The command a terminal runs, from its argv: the wrapper the app puts around a program is taken off (command()), and
// so is `cmd.exe /c <x>`, which is how Windows runs a .CMD shim. A plain shell (no command) is what it is.
function inside(argv) {
  const inner = command(argv);
  if (inner !== argv) return inner;
  if (argv.length > 2 && stem(argv[0]) === "cmd") {
    const at = argv.findIndex((w, i) => i > 0 && i < 4 && w.toLowerCase() === "/c");
    if (at > 0 && argv[at + 1]) return argv.slice(at + 1);
  }
  return argv;
}

// Whether a terminal runs the agent a saved pane had: the program it runs is that agent's own. The label and the
// program table's idea of it are not asked: the core keeps a lost agent's label and status on the shell that replaces it.
function runsAgent(record, leaf) {
  const want = program((leaf.argv?.length ? inside(leaf.argv) : [leaf.agent])[0]);
  const have = program(inside(Array.isArray(record.argv) ? record.argv : [])[0]);
  return Boolean(have) && have === want;
}

// The saved wrapper (`zsh -ic '"$@"; exec "$0" -i' zsh <cmd>`, or on Windows `pwsh -NoExit -Command <line>`) around a
// new command, so a restarted agent runs in the person's shell as the app starts one; a saved argv that had no wrapper
// gives the command as it is.
function rewrap(saved, cmd) {
  if (!Array.isArray(saved) || command(saved) === saved) return cmd;
  if (saved.length === 4 && saved[1] === "-NoExit") return [saved[0], "-NoExit", "-Command", psLine(cmd)];
  return [...saved.slice(0, 4), ...cmd];
}

// An agent pane whose terminal lives but is only a shell now (the core replaces a lost program by a shell that keeps
// its labels): it needs the agent started again. `folder`: also in the saved folder (a pane found by shape, not address).
function replaced(leaf, address, folder) {
  if (!leaf.agent) return false;
  const r = terminals().find((x) => x.address === address);
  if (r?.state !== "live" || r.labels?.["h0x.agent"] !== leaf.agent || runsAgent(r, leaf)) return false;
  return !folder || leaf.cwd == null || norm(panes().find((p) => p.terminal === address)?.cwd) === norm(leaf.cwd);
}

// `pane` holds the node's first pane: its later children split off one after
// another, then each child grows its own splits.
async function grow(cx, node, pane, start) {
  if (!node.children) return true;
  const ids = [pane];
  for (const child of node.children.slice(1)) {
    const at = await start(first(child));
    const seen = new Set(panes().map((p) => p.pane));
    const tab = panes().find((p) => p.pane === ids.at(-1))?.tab;
    run("pane.focus", { pane: ids.at(-1) });
    run("terminal.open", { ...at, place: node.split });
    const id = await until(cx, () => panes().find((p) => p.tab === tab && !seen.has(p.pane))?.pane);
    if (!id) return false;
    ids.push(id);
  }
  for (const [i, child] of node.children.entries()) if (!(await grow(cx, child, ids[i], start))) return false;
  return true;
}

// A new tab in `space` with the saved tab's panes: its id, or null when it did not come up in time.
async function addTab(cx, space, tab, start) {
  const at = await start(first(tab.root));
  const before = new Set(panes().map((p) => p.tab));
  run("tab.new", { space, ...(tab.name ? { name: tab.name } : {}), ...at });
  const pane = await until(cx, () => panes().find((p) => !before.has(p.tab)));
  return pane && (await grow(cx, tab.root, pane.pane, start)) ? pane.tab : null;
}

// The saved leaf against a live pane: its terminal lives, runs the same program, and is in the same folder.
function alike(leaf, address) {
  const r = terminals().find((x) => x.address === address);
  if (r?.state !== "live") return false;
  if (!(leaf.agent ? r.labels?.["h0x.agent"] === leaf.agent && r.program?.shell !== true : sameArgv(r.argv, leaf.argv))) return false;
  return leaf.cwd == null || norm(panes().find((p) => p.terminal === address)?.cwd) === norm(leaf.cwd);
}

// Whether a saved node and a live one (from shape()) split the same way; `leaf` also compares each pane.
function fits(saved, live, leaf) {
  if (saved.children || live.children) {
    return Boolean(saved.children && live.children) && saved.split === live.split && saved.children.length === live.children.length && saved.children.every((c, i) => fits(c, live.children[i], leaf));
  }
  return !leaf || alike(saved, live.terminal);
}

// The open tab of `space` that shows a saved tab, else null. First by the address the save kept; then, for addresses
// that are stale (terminals started again since), by the same pane shape with a live terminal of the same program
// and folder in each. A tab already given to another saved tab (`taken`) is not given twice.
function findOpen(tab, space, taken) {
  const by = leaves(tab.root).find((l) => open(l, space));
  const id = by && panes().find((p) => p.terminal === by.terminal)?.tab;
  if (id && !taken.has(id)) return id;
  for (const live of trees()?.find((x) => x.id === space.id)?.tabs ?? []) {
    if (taken.has(live.id)) continue;
    const root = shape(live.root, (terminal) => ({ terminal }));
    if (root && fits(tab.root, root, true)) return live.id;
  }
  return null;
}

// The saved panes of a tab against a live tab's shape, each live pane either what was saved or a replaced agent shell:
// the replaced ones as [{ leaf, terminal }], or null when the tab is not that tab.
function dead(saved, live) {
  if (saved.children || live.children) {
    if (!(saved.children && live.children) || saved.split !== live.split || saved.children.length !== live.children.length) return null;
    const parts = saved.children.map((c, i) => dead(c, live.children[i]));
    return parts.some((x) => x === null) ? null : parts.flat();
  }
  if (alike(saved, live.terminal)) return [];
  return replaced(saved, live.terminal, true) ? [{ leaf: saved, terminal: live.terminal }] : null;
}

// After a restore each saved pane that came back is learned under the terminal that now shows it, so the next restore
// finds it by address. Only the addresses change: not the name, the times or the place in the list.
function relearn(item, got) {
  let changed = false;
  const spaces = item.spaces.map((sp, i) => {
    const g = got[i];
    const live = g && trees()?.find((x) => x.id === g.space);
    if (!live) return sp;
    const tabs = sp.tabs.map((tab, j) => {
      const lt = live.tabs.find((x) => x.id === g.tabs[j]);
      const addresses = [];
      const root = lt && shape(lt.root, (terminal) => (addresses.push(terminal), { terminal }));
      if (!root || !fits(tab.root, root, false)) return tab;
      let k = 0;
      const walk = (n) => (n.children ? { ...n, children: n.children.map(walk) } : { ...n, terminal: addresses[k++] });
      const next = walk(tab.root);
      if (leaves(next).some((l, n) => l.terminal !== leaves(tab.root)[n].terminal)) changed = true;
      return { ...tab, root: next };
    });
    return { ...sp, tabs };
  });
  if (!changed) return;
  const items = read();
  const at = items.findIndex((x) => x.id === item.id);
  if (at >= 0) {
    items[at] = { ...items[at], spaces };
    write(items);
  }
}

// The saved space's tabs in an open space: a tab that is open already is kept as it is, the others start. Answers
// { space, tabs: [tab id per saved tab] }, or null on a timeout.
async function fill(cx, target, sp, start, made) {
  const ids = [];
  const todo = [];
  const taken = new Set();
  const revive = [];
  sp.tabs.forEach((t, i) => {
    const there = findOpen(t, target, taken);
    if (there) {
      ids[i] = there;
      taken.add(there);
      // A pane of it that the core brought back as a shell is not the agent it was.
      for (const l of leaves(t.root)) if (l.terminal && panes().find((p) => p.terminal === l.terminal)?.tab === there && replaced(l, l.terminal, false)) revive.push({ leaf: l, terminal: l.terminal });
      return;
    }
    for (const live of trees()?.find((x) => x.id === target.id)?.tabs ?? []) {
      if (taken.has(live.id)) continue;
      const root = shape(live.root, (terminal) => ({ terminal }));
      const lost = root && dead(t.root, root);
      if (lost?.length) {
        ids[i] = live.id;
        taken.add(live.id);
        revive.push(...lost);
        return;
      }
    }
    todo.push(i);
  });
  // Each such agent starts again in the pane's place (a new pane beside it, then the shell's pane closes).
  for (const { leaf, terminal } of revive) {
    const pane = panes().find((p) => p.terminal === terminal);
    const at = pane && (await start(leaf));
    if (!at?.argv) continue;
    const seen = new Set(panes().map((p) => p.pane));
    run("pane.split", { direction: "right", pane: pane.pane, argv: rewrap(leaf.argv, at.argv) });
    if (!(await until(cx, () => panes().find((p) => p.tab === pane.tab && !seen.has(p.pane))))) return null;
    run("pane.close", { pane: pane.pane });
    // Gone before the next step, so what is learned afterwards is the tab as it is.
    if (!(await until(cx, () => !panes().some((p) => p.pane === pane.pane)))) return null;
    made.restarted += 1;
  }
  for (const i of todo) {
    const tab = await addTab(cx, target.id, sp.tabs[i], start);
    if (!tab) return null;
    ids[i] = tab;
    made.tabs += 1;
  }
  made.reused += 1;
  return { space: target.id, tabs: ids };
}

// A new space from the saved one: its first tab makes the space, the others follow.
async function create(cx, sp, start, made) {
  const [head, ...rest] = sp.tabs;
  const at = await start(first(head.root));
  const before = new Set(spaces().map((s) => s.id));
  run("space.new", { name: sp.name || base(sp.folder || "") || "Space", ...(sp.folder ? { folder: sp.folder } : {}), ...(sp.machine ? { machine: sp.machine } : {}), ...(head.name ? { tab: head.name } : {}), ...(at.argv ? { argv: at.argv } : {}) });
  const space = await until(cx, () => spaces().find((s) => !before.has(s.id)));
  if (!space) return null;
  const pane = await until(cx, () => panes().find((p) => p.tab === space.tabs?.[0]?.id)?.pane);
  if (!pane || !(await grow(cx, head.root, pane, start))) return null;
  const ids = [space.tabs[0].id];
  for (const tab of rest) {
    const id = await addTab(cx, space.id, tab, start);
    if (!id) return null;
    ids.push(id);
  }
  made.spaces += 1;
  made.tabs += sp.tabs.length;
  return { space: space.id, tabs: ids };
}

/** Restores a workspace (an item of read()) alongside what is open; nothing is closed. Each saved space, in order:
 * the space with the same machine and folder is filled in, else a new one is made on its machine (a machine that is
 * not connected is skipped). A saved tab with a pane still there (live, shown, same program) is left alone; the
 * others start, each agent resumed in its session when it has one (once), else started from its saved argv, a program
 * from its argv. At the end each space's saved tab and then the saved space are shown. Answers
 * { state: "ok" | "already" | "timeout", spaces, tabs, skipped: { machine: count } }: `already` when nothing had to
 * start. Runs as the caller's task (cx.spawn): it stops with the view that started it. */
export async function restore(cx, stored) {
  // An item of either version, as read() would give it.
  const item = itemOf(stored);
  if (!item) return { state: "already", spaces: 0, tabs: 0, skipped: {} };
  const known = await knownAgents().catch(() => []);
  const started = new Set();
  const made = { spaces: 0, reused: 0, tabs: 0, restarted: 0 };
  const skipped = {};
  const got = [];
  // Each saved space takes its own open space: one nobody took yet (a space this restore made counts as taken).
  const claimed = new Set();
  for (const sp of item.spaces) {
    const on = sp.machine;
    if (on && !machines().some((m) => m.name === on && m.layout === "ready")) {
      skipped[on] = (skipped[on] ?? 0) + 1;
      got.push(null);
      continue;
    }
    const start = async (leaf) => {
      const folder = leaf.cwd || sp.folder;
      const fresh = leaf.argv?.length ? inside(leaf.argv) : known.find((p) => p.key === leaf.agent)?.run;
      // An agent resumes or restarts; a program that is not a shell starts from its argv; a plain shell is the default one.
      const argv = leaf.agent ? await launch(on, leaf, fresh, started) : leaf.shell === false && leaf.argv?.length ? command(leaf.argv) : undefined;
      return { ...(folder ? { folder } : {}), ...(argv ? { argv } : {}) };
    };
    const same = spaces().filter((s) => !claimed.has(s.id) && (s.machine ?? null) === on && norm(s.folder) === norm(sp.folder));
    // Of several, the one that already runs the saved panes.
    const target = same.find((s) => sp.tabs.some((t) => leaves(t.root).some((l) => open(l, s)))) ?? same[0];
    const done = target ? await fill(cx, target, sp, start, made) : await create(cx, sp, start, made);
    if (!done) return { state: "timeout", ...made, skipped };
    claimed.add(done.space);
    got.push(done);
  }
  item.spaces.forEach((sp, i) => {
    const id = got[i]?.tabs[sp.activeTab];
    if (id) run("tab.select", { id });
  });
  const shown = got[item.active] ?? got.find(Boolean);
  if (shown) run("space.select", { id: shown.space });
  relearn(item, got);
  const none = !made.spaces && !made.tabs && !made.restarted && !Object.keys(skipped).length;
  return { state: none ? "already" : "ok", spaces: made.spaces + made.reused, tabs: made.tabs, restarted: made.restarted, skipped };
}

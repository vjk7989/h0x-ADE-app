// Workspaces' engine: saves the shown space (its tabs, splits and each agent with its session) and restores it.
// Storage is shared("workspaces") = { version: 1, items: [ { id, name, created, saved, machine, folder, space, tabs } ] },
// newest first, at most MAX. A leaf is { cwd, argv?, terminal?, agent?, session? }; a split is { split, children }.
// Nothing here is called while drawing except read(): the rest runs from clicks and tasks.

import { spaces, panes, terminals, tree, core, home, knownAgents, run, shared, setShared, window } from "h0x";

export const MAX = 5;
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

/** The stored workspaces, newest first, each valid and fresh (not the stored objects): a value that is not
 * ours reads as none; a bad item is dropped; at most MAX. */
export function read() {
  const items = own(plain(shared(KEY)), "items");
  if (!Array.isArray(items)) return [];
  const seen = new Set();
  const out = [];
  for (const it of items) {
    if (!plain(it) || typeof it.id !== "string" || !it.id || seen.has(it.id) || typeof it.name !== "string" || !it.name.trim()) continue;
    const tabs = soundTabs(it.tabs);
    if (!tabs.length) continue;
    seen.add(it.id);
    out.push({
      id: it.id,
      name: it.name.slice(0, 80),
      created: num(it.created),
      saved: num(it.saved),
      machine: typeof it.machine === "string" && it.machine ? it.machine : null,
      folder: text(it.folder),
      space: text(it.space),
      tabs,
    });
    if (out.length === MAX) break;
  }
  return out;
}

/** Stores the list (newest first). */
export function write(items) {
  setShared(KEY, { version: 1, items });
}

/** The count of tabs and panes of a stored workspace, for its row. */
export function counts(item) {
  return { tabs: item.tabs.length, panes: item.tabs.reduce((n, t) => n + leaves(t.root).length, 0) };
}

/** The shown space as a snapshot { space, machine, folder, tabs: [{ name, root }] }, or null (no shown space, no
 * terminals). Each agent pane has its agent (label h0x.agent), argv and folder; a shell pane its argv and folder;
 * every pane the core address of its terminal. `session` is filled in by saveNew / saveOver. */
export function capture() {
  const s = spaces().find((x) => x.active);
  const t = tree();
  if (!s || t?.id !== s.id) return null;
  const ps = panes();
  const ts = terminals();
  const leaf = (address) => {
    const cwd = ps.find((p) => p.terminal === address)?.cwd ?? null;
    const r = ts.find((x) => x.address === address);
    const agent = r?.labels?.["h0x.agent"];
    const argv = Array.isArray(r?.argv) ? r.argv : [];
    return agent ? { agent, session: null, argv, cwd, terminal: address } : { cwd, argv, terminal: address };
  };
  const tabs = (t.tabs ?? []).map((x) => ({ name: x.name ?? "", root: shape(x.root, leaf) })).filter((x) => x.root);
  if (!tabs.length) return null;
  return { space: s.name || (s.folder ? base(s.folder) : ""), machine: s.machine ?? null, folder: s.folder ?? "", tabs };
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

// A snapshot with each agent pane's session found (asks the core: never call while drawing).
async function snapshot() {
  const snap = capture();
  if (!snap) return null;
  const on = snap.machine;
  await assign(on, snap.folder || (on ? "" : home()), snap.tabs.flatMap((x) => leaves(x.root)).filter((x) => x.agent));
  return snap;
}

const newId = (items) => {
  for (;;) {
    const id = `w${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
    if (!items.some((x) => x.id === id)) return id;
  }
};

/** Saves the shown space as a new workspace named `name` (empty: the space's name). Answers the item, "full"
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
    name: (name ?? "").trim().slice(0, 80) || snap.space || "Workspace",
    created: now,
    saved: now,
    machine: snap.machine,
    folder: snap.folder,
    space: snap.space,
    tabs: snap.tabs,
  };
  write([item, ...items]);
  return item;
}

/** Replaces a workspace's tabs with the shown space's (name and creation time kept; it moves to the top). Answers
 * the item, "missing" (no such workspace) or "empty". */
export async function saveOver(id) {
  if (!read().some((x) => x.id === id)) return "missing";
  const snap = await snapshot();
  if (!snap) return "empty";
  const items = read();
  const old = items.find((x) => x.id === id);
  if (!old) return "missing";
  const item = { ...old, saved: Date.now(), machine: snap.machine, folder: snap.folder, space: snap.space, tabs: snap.tabs };
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
      items.push({ id: newId(items), name: name.slice(0, 80), created: now, saved: now, machine: typeof where[0] === "string" && where[0] ? where[0] : null, folder: where[1], space: base(where[1]), tabs });
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
  return leaf.agent ? r.labels?.["h0x.agent"] === leaf.agent : sameArgv(r.argv, leaf.argv);
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

async function addTab(cx, space, tab, start) {
  const at = await start(first(tab.root));
  const before = new Set(panes().map((p) => p.tab));
  run("tab.new", { space, ...(tab.name ? { name: tab.name } : {}), ...at });
  const pane = await until(cx, () => panes().find((p) => !before.has(p.tab))?.pane);
  return Boolean(pane) && (await grow(cx, tab.root, pane, start));
}

/** Restores a workspace (an item of read()) alongside what is open; nothing is closed. The target is the space
 * with the same machine and folder (selected), else a new one made from the first tab. A saved tab with a pane
 * still there (live, shown, same program) is skipped; the others start, each agent resumed in its session when it
 * has one, else started from its saved argv. Answers { state: "ok" | "already" | "timeout", space }.
 * Runs as the caller's task (cx.spawn): it stops with the view that started it. */
export async function restore(cx, item) {
  const on = item.machine ?? null;
  const known = await knownAgents().catch(() => []);
  const started = new Set();
  const start = async (leaf) => {
    const folder = leaf.cwd || item.folder;
    const fresh = leaf.argv?.length ? leaf.argv : known.find((p) => p.key === leaf.agent)?.run;
    const argv = leaf.agent ? await launch(on, leaf, fresh, started) : undefined;
    return { ...(folder ? { folder } : {}), ...(argv ? { argv } : {}) };
  };
  const target = spaces().find((s) => (s.machine ?? null) === on && norm(s.folder) === norm(item.folder));
  if (target) {
    run("space.select", { id: target.id });
    const todo = item.tabs.filter((t) => !leaves(t.root).some((l) => open(l, target)));
    if (!todo.length) return { state: "already", space: target.id };
    for (const tab of todo) if (!(await addTab(cx, target.id, tab, start))) return { state: "timeout", space: target.id };
    return { state: "ok", space: target.id };
  }
  const [head, ...rest] = item.tabs;
  const at = await start(first(head.root));
  const before = new Set(spaces().map((s) => s.id));
  run("space.new", { name: item.space || item.name, ...(item.folder ? { folder: item.folder } : {}), ...(on ? { machine: on } : {}), ...(head.name ? { tab: head.name } : {}), ...(at.argv ? { argv: at.argv } : {}) });
  const made = await until(cx, () => spaces().find((s) => !before.has(s.id)));
  if (!made) return { state: "timeout", space: null };
  const pane = await until(cx, () => panes().find((p) => p.tab === made.tabs?.[0]?.id)?.pane);
  if (!pane || !(await grow(cx, head.root, pane, start))) return { state: "timeout", space: made.id };
  for (const tab of rest) if (!(await addTab(cx, made.id, tab, start))) return { state: "timeout", space: made.id };
  return { state: "ok", space: made.id };
}

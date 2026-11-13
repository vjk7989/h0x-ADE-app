// Settings > Agents: the agents that start from the + menu and their number keys (agent.new:<slot>).
// The built-in four hold slots 1-4; the person's own (programs.json, entries with `run`) follow. Only those
// can be added, edited or removed here. The file is read, changed and written back only if nobody changed it
// meanwhile, with every other entry and field kept; a file that does not parse is shown and never written.

import { div } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { appHome, readTextFile, writeTextFile, knownAgents, keys, settings } from "h0x";

// The keys of defaults/programs.json: a custom agent may not take one (it would change the built-in).
const BUILT_IN = [
  "claude", "codex", "pi", "pig", "opencode", "cursor", "copilot", "amp", "grok", "dsh-tui", "antigravity", "kimi", "kiro",
  "hermes", "mastracode", "qoder", "muse", "crush", "command-code", "prime-agent", "gemini", "ollama", "agent", "shell",
  "program", "vim", "nvim", "hx", "emacs", "nano", "less", "lazygit", "git", "gh", "glab", "htop",
];
const MAX_WORDS = 32;
const MAX_NAME = 40;

/** A command line as argv, split like a shell would but run by none: spaces separate words, 'single' quotes are
 * literal, "double" quotes keep spaces and take \" and \\. Elsewhere a backslash is just a character (Windows paths).
 * Returns { argv } or { error }. */
export function splitCommand(text) {
  if (/[\r\n]/.test(text)) return { error: "The command is one line." };
  if (/[\u0000-\u0008\u000b-\u001f\u007f]/.test(text)) return { error: "The command has a control character." };
  const argv = [];
  let word = null, quote = null;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quote === "'") { if (c === "'") quote = null; else word += c; continue; }
    if (quote === '"') {
      if (c === '"') quote = null;
      else if (c === "\\" && (text[i + 1] === '"' || text[i + 1] === "\\")) word += text[++i];
      else word += c;
      continue;
    }
    if (c === "'" || c === '"') { quote = c; word ??= ""; continue; }
    if (c === " " || c === "\t") { if (word !== null) argv.push(word); word = null; continue; }
    word = (word ?? "") + c;
  }
  if (quote) return { error: "A quote is not closed." };
  if (word !== null) argv.push(word);
  if (argv.length === 0) return { error: "Enter the command that starts the agent." };
  if (argv.length > MAX_WORDS) return { error: `At most ${MAX_WORDS} words.` };
  return { argv };
}

/** The entry's key for a name: lower case letters and digits, joined by dashes. */
export function slug(name) {
  return name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
}

/** The programs.json text as an object, or { damaged } when it can't be changed safely. */
export function parseTable(raw) {
  if (raw === null || raw === undefined) return { table: {} };
  try {
    const table = JSON.parse(raw.replace(/^\uFEFF/, ""));
    if (table === null || typeof table !== "object" || Array.isArray(table)) throw new Error("not an object");
    if (table.programs !== undefined && (table.programs === null || typeof table.programs !== "object" || Array.isArray(table.programs))) throw new Error("programs is not an object");
    return { table };
  } catch (e) {
    return { damaged: `programs.json can't be read (${e.message}), so nothing was changed. Fix or delete the file, then try again.` };
  }
}

/** The form's name and command as the entry to write, or { error }. `table` is the parsed file. */
export function checkEntry(name, command, table, editing) {
  name = name.trim();
  if (!name) return { error: "Enter a name." };
  if ([...name].length > MAX_NAME) return { error: `The name is at most ${MAX_NAME} characters.` };
  if (/[\u0000-\u001f\u007f]/.test(name)) return { error: "The name has a control character." };
  const run = splitCommand(command);
  if (run.error) return run;
  const key = editing ?? slug(name);
  if (!key) return { error: "The name needs a letter or a digit." };
  if (!editing) {
    if (BUILT_IN.includes(key)) return { error: `"${key}" is a built-in agent's key. Pick another name.` };
    if (table.programs?.[key] !== undefined) return { error: `An agent with the key "${key}" exists. Pick another name.` };
  }
  return { key, name, run: run.argv };
}

const file = () => `${appHome()}/programs.json`;

export function agentsInit(view) {
  const ag = { list: null, raw: null, damaged: null, form: null, error: null, confirm: null };
  ag.name = InputState.new({ placeholder: "Name" });
  ag.command = InputState.new({ placeholder: "Command, like: aider --model \"gpt 5\"" });
  // What the page's links do, by name.
  ag.open = (agent, cx) => openForm(view, agent, cx);
  ag.save = (cx) => saveForm(view, cx);
  ag.remove = (key, cx) => removeAgent(view, key, cx);
  view.ag = ag;
}

/** Reads the table and the agent list; again a moment after a write, when the app has reloaded the file. */
export function loadAgents(view, cx) {
  cx.spawn(async (cx) => {
    const ag = view.ag;
    ag.raw = await readTextFile(file());
    ag.damaged = parseTable(ag.raw).damaged ?? null;
    try { ag.list = await knownAgents({}); } catch { ag.list ??= []; }
    cx.notify();
  });
}

function edit(view, cx, change) {
  const ag = view.ag;
  cx.spawn(async (cx) => {
    for (let attempt = 0; attempt < 3; attempt++) {
      ag.raw = await readTextFile(file());
      const read = parseTable(ag.raw);
      ag.damaged = read.damaged ?? null;
      if (read.damaged) break;
      const next = JSON.parse(JSON.stringify(read.table));
      next.programs ??= {};
      const error = change(next);
      if (error) { ag.error = error; break; }
      try {
        await writeTextFile(file(), JSON.stringify(next, null, 2) + "\n", ag.raw);
        ag.error = null; ag.form = null; ag.confirm = null;
        cx.timer.after(800, (cx) => loadAgents(view, cx));
        break;
      } catch (e) { if (attempt === 2) ag.error = `Could not save programs.json: ${e?.message ?? e}`; }
    }
    cx.notify();
  });
}

// A command as the person would type it: words with spaces or quotes are double-quoted.
function commandText(run) {
  return run.map((w) => (w === "" || /[\s"']/.test(w) ? `"${w.replace(/["\\]/g, (m) => "\\" + m)}"` : w)).join(" ");
}

export function openForm(view, agent, cx) {
  const ag = view.ag;
  ag.form = { key: agent?.key ?? null };
  ag.error = null;
  ag.name.set_value(agent?.name ?? "");
  ag.command.set_value(agent ? commandText(agent.run ?? []) : "");
  cx.notify();
}

export function saveForm(view, cx) {
  const ag = view.ag;
  if (!ag.form || ag.damaged) return;
  edit(view, cx, (next) => {
    const entry = checkEntry(ag.name.value(), ag.command.value(), next, ag.form.key);
    if (entry.error) return entry.error;
    if (ag.form.key) {
      if (next.programs[entry.key] === undefined) return "That agent is gone from programs.json.";
      next.programs[entry.key] = { ...next.programs[entry.key], name: entry.name, run: entry.run };
    } else {
      const orders = Object.values(next.programs).map((e) => e?.order).filter(Number.isFinite);
      next.programs[entry.key] = { name: entry.name, run: entry.run, order: (orders.length ? Math.max(...orders) : 0) + 1 };
    }
  });
}

export function removeAgent(view, key, cx) {
  edit(view, cx, (next) => { delete next.programs[key]; });
}

function keyOf(slot) {
  const all = keys().filter((b) => b.section === "global" && b.action === `agent.new:${slot}`);
  const k = all.find((b) => /shift/.test(b.keys)) ?? all[0];
  return k ? k.keys.replace(/-/g, "+") : "";
}

/** The page: the agent slots as rows with thin rules, then the form. `view` brings the settings page's heading. */
export function agentsPage(view, t, cx) {
  const ag = view.ag;
  const mono = settings().font_family;
  const link = (id, label, color, onClick) => div().id(id).cursor_pointer().text_size(12).text_color(color).hover((s) => s.text_color(t.foreground)).on_click(onClick).child(label);
  const list = (ag.list ?? []).filter((a) => a.slot || a.custom).sort((a, b) => (a.slot ?? 99) - (b.slot ?? 99));
  const cell = (w, text, color) => div().w(w).flex_none().truncate().font_family(mono).text_size(12).text_color(color).child(text);
  const actions = (a) =>
    !a.custom
      ? div().w(110).flex_none().text_size(11).text_color(t.dim).text_right().child("built in")
      : ag.confirm === a.key
        ? h_flex().gap(8).w(110).flex_none().justify_end()
          .child(link(`ag-yes-${a.key}`, "remove", t.blocked, (_e, cx) => ag.remove(a.key, cx)))
          .child(link(`ag-no-${a.key}`, "keep", t.dim, (_e, cx) => { ag.confirm = null; cx.notify(); }))
        : h_flex().gap(8).w(110).flex_none().justify_end()
          .child(link(`ag-edit-${a.key}`, "edit", t.muted, (_e, cx) => ag.open(a, cx)))
          .child(link(`ag-rm-${a.key}`, "remove", t.muted, (_e, cx) => { ag.confirm = a.key; cx.notify(); }));
  const row = (a) =>
    h_flex().id(`ag-${a.key}`).w_full().h(34).gap(12).items_center().border_b(1).border_color(t.border)
      .child(cell(18, a.slot ? String(a.slot) : "-", t.dim))
      .child(cell(110, a.slot ? keyOf(a.slot) : "", t.dim))
      .child(div().w(130).flex_none().truncate().text_size(13).text_color(t.foreground).child(a.name))
      .child(div().flex_1().min_w(0).truncate().font_family(mono).text_size(12).text_color(t.muted).child((a.run ?? []).join(" ")))
      .child(cell(86, a.installed ? "installed" : "not installed", a.installed ? t.done : t.dim))
      .child(actions(a));
  const field = (label, state) =>
    h_flex().gap(12).items_center()
      .child(div().w(66).flex_none().font_family(mono).text_size(12).text_color(t.dim).child(label))
      .child(Input.new(state).flex_1().min_w(0).h(30).px(10).rounded(6).border_1().border_color(t.input).bg(t.background).font_family(mono).text_size(12.5).text_color(t.foreground));
  return v_flex().w_full()
    .child(view.heading(t, "Agents"))
    .child(div().pb(10).text_size(12.5).line_height("18px").text_color(t.dim).child("Agents in the + menu and the palette. A number key starts the agent in that slot in a new tab. Your own agents follow the built-in four, in the order they were added."))
    .when(Boolean(ag.damaged), (el) => el.child(div().pb(10).text_size(12.5).line_height("18px").text_color(t.blocked).child(ag.damaged)))
    .child(v_flex().w_full().border_t(1).border_color(t.border)
      .children(list.map(row))
      .when(ag.list === null, (el) => el.child(div().py(10).font_family(mono).text_size(12).text_color(t.dim).child("loading\u2026"))))
    .child(ag.form
      ? v_flex().w_full().gap(8).pt(14)
        .child(div().font_family(mono).text_size(12).text_color(t.dim).child(ag.form.key ? `edit ${ag.form.key}` : "new agent"))
        .child(field("name", ag.name))
        .child(field("command", ag.command))
        .when(Boolean(ag.error), (el) => el.child(div().text_size(12.5).text_color(t.blocked).child(ag.error)))
        .child(h_flex().gap(14).pt(2)
          .child(link("ag-save", "save", t.accent, (_e, cx) => ag.save(cx)))
          .child(link("ag-cancel", "cancel", t.dim, (_e, cx) => { ag.form = null; ag.error = null; cx.notify(); })))
      : h_flex().pt(12).child(ag.damaged
        ? div().font_family(mono).text_size(12).text_color(t.dim).child("+ add agent")
        : link("ag-add", "+ add agent", t.accent, (_e, cx) => ag.open(null, cx))));
}

// Pure model for the to-do mod: Markdown task lists and the lists index.
// No host imports, no I/O.

/**
 * @typedef {{ title: string, done: boolean, extra: string[] }} Step
 * @typedef {{ title: string, done: boolean, steps: Step[], extra: string[] }} Task
 * @typedef {{ name: string, head: string[], tasks: Task[] }} TodoList
 * @typedef {{ name: string, slug: string }} IndexEntry
 */

const TASK_RE = /^[-*] \[( |x|X)\] (.*)$/;
const STEP_RE = /^(?:  +|\t)[ \t]*[-*] \[( |x|X)\] (.*)$/;
const HEADING_RE = /^# (.*)$/;
const INDEX_RE = /^- \[(.+?)\]\(([^()]*)\.md\)\s*$/;
const SLUG_RE = /^[a-z0-9-]{1,60}$/;

/** A title is one line: newlines become spaces.
 * @param {string} s @returns {string} */
export function escapeTitle(s) {
  return String(s).replace(/\r\n|\r|\n/g, " ");
}

/** Parse a task-list Markdown file. Lines that are not tasks are kept verbatim.
 * @param {string} text @returns {TodoList} */
export function parseList(text) {
  const lines = String(text).split(/\r?\n/);
  if (lines[lines.length - 1] === "") lines.pop();
  /** @type {TodoList} */
  const list = { name: "", head: [], tasks: [] };
  let named = false;
  for (const line of lines) {
    const task = TASK_RE.exec(line);
    if (task) {
      list.tasks.push({ title: task[2].replace(/ +$/, ""), done: task[1] !== " ", steps: [], extra: [] });
      continue;
    }
    const cur = list.tasks[list.tasks.length - 1];
    const step = cur && STEP_RE.exec(line);
    if (step) {
      cur.steps.push({ title: step[2].replace(/ +$/, ""), done: step[1] !== " ", extra: [] });
      continue;
    }
    if (!cur) {
      const h = !named && HEADING_RE.exec(line);
      if (h) {
        named = true;
        list.name = h[1].trim();
      } else {
        list.head.push(line);
      }
    } else if (cur.steps.length) {
      cur.steps[cur.steps.length - 1].extra.push(line);
    } else {
      cur.extra.push(line);
    }
  }
  if (!list.name) list.name = "Untitled";
  return list;
}

/** @param {TodoList} list @returns {string} */
export function serializeList(list) {
  const out = ["# " + escapeTitle(list.name), ...list.head];
  for (const t of list.tasks) {
    out.push(`- [${t.done ? "x" : " "}] ${escapeTitle(t.title)}`, ...t.extra);
    for (const s of t.steps) {
      out.push(`  - [${s.done ? "x" : " "}] ${escapeTitle(s.title)}`, ...s.extra);
    }
  }
  return out.join("\n") + "\n";
}

/** Parse `- [Name](slug.md)` lines; invalid slugs and duplicates are dropped.
 * @param {string} text @returns {IndexEntry[]} */
export function parseIndex(text) {
  /** @type {IndexEntry[]} */
  const lists = [];
  for (const line of String(text).split(/\r?\n/)) {
    const m = INDEX_RE.exec(line);
    if (!m || !SLUG_RE.test(m[2]) || lists.some((l) => l.slug === m[2])) continue;
    lists.push({ name: m[1], slug: m[2] });
  }
  return lists;
}

/** @param {IndexEntry[]} lists @returns {string} */
export function serializeIndex(lists) {
  return lists.map((l) => `- [${escapeTitle(l.name)}](${l.slug}.md)\n`).join("");
}

/** File-safe slug from a list name, unique against `taken`; `lists` is reserved.
 * @param {string} name @param {string[]} taken @returns {string} */
export function slugify(name, taken) {
  const base =
    String(name)
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 60)
      .replace(/-+$/, "") || "list";
  const used = (s) => s === "lists" || taken.includes(s);
  let slug = base;
  for (let n = 2; used(slug); n++) {
    const suffix = "-" + n;
    slug = base.slice(0, 60 - suffix.length).replace(/-+$/, "") + suffix;
  }
  return slug;
}

/** Number of tasks not done. @param {TodoList} list @returns {number} */
export function openCount(list) {
  return list.tasks.filter((t) => !t.done).length;
}

/** At least one task and all done (steps do not count).
 * @param {TodoList} list @returns {boolean} */
export function allDone(list) {
  return list.tasks.length > 0 && list.tasks.every((t) => t.done);
}

// Settings > Profile > "Agent activity · this computer": a 53 x 7 heatmap of the last year, read from
// <app home>/usage/activity.json (src/activity.rs writes it; it never leaves this computer).
//   {"version":1,"days":{"YYYY-MM-DD":{"seconds":n,"sessions":n,"tokens":n}}}  (local dates)

import { div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { appHome, readTextFile } from "h0x";

const WEEKS = 53, CELL = 9, GAP = 2, STEP = CELL + GAP, LABELS = 53;
const DAY_NAMES = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
// Minutes of agent time a day -> level 0..4; the share of the accent mixed over the background.
const LEVELS = [0, 1, 15, 60, 180];
const SHARE = [0, 0.4, 0.62, 0.82, 1];

const pad = (n) => String(n).padStart(2, "0");
const key = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const nth = (from, n) => new Date(from.getFullYear(), from.getMonth(), from.getDate() + n);
// 8,692 (the runtime has no Intl, so no toLocaleString grouping).
const grouped = (n) => String(n).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
const count = (n, one, many) => `${grouped(n)} ${n === 1 ? one : many}`;

export const minutesOf = (day) => (day && day.seconds > 0 ? Math.max(1, Math.round(day.seconds / 60)) : 0);
export const levelOf = (minutes) => (minutes < LEVELS[1] ? 0 : minutes < LEVELS[2] ? 1 : minutes < LEVELS[3] ? 2 : minutes < LEVELS[4] ? 3 : 4);

// "18.2k", "1.4M"; below a thousand as it is.
export function short(n) {
  if (n < 1000) return String(n);
  if (n < 1e6) {
    const k = Math.round(n / 100) / 10;
    if (k < 1000) return `${k}k`;
  }
  return `${Math.round(n / 1e5) / 10}M`;
}

// What the file says, or {} for no file or a damaged one: only well-formed days count.
export function parseActivity(text) {
  if (!text) return {};
  try {
    const file = JSON.parse(text);
    if (!file || file.version !== 1 || typeof file.days !== "object" || file.days === null || Array.isArray(file.days)) return {};
    const days = {};
    for (const [k, v] of Object.entries(file.days)) {
      if (!/^\d{4}-\d{2}-\d{2}$/.test(k) || !v || typeof v !== "object") continue;
      const num = (x) => (Number.isFinite(x) && x > 0 ? Math.floor(x) : 0);
      days[k] = { seconds: num(v.seconds), sessions: num(v.sessions), tokens: num(v.tokens) };
    }
    return days;
  } catch {
    return {};
  }
}

// The grid's days: week by week from the Sunday 52 weeks before this week's, `null` after today.
export function gridDays(today) {
  const first = nth(today, -today.getDay() - (WEEKS - 1) * 7);
  return [...Array(WEEKS).keys()].map((w) => [...Array(7).keys()].map((r) => {
    const d = nth(first, w * 7 + r);
    return d > today ? null : d;
  }));
}

// Total minutes, days with agent time, and the longest run of such days in a row.
export function summarize(weeks, days) {
  let minutes = 0, active = 0, streak = 0, longest = 0;
  for (const d of weeks.flat()) {
    if (!d) continue;
    const m = minutesOf(days[key(d)]);
    minutes += m;
    if (m > 0) { active++; longest = Math.max(longest, ++streak); } else streak = 0;
  }
  return { minutes, active, longest };
}

export function tooltip(d, day) {
  const parts = [`${DAY_NAMES[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}`, `${minutesOf(day)} min`, count(day?.sessions ?? 0, "session", "sessions")];
  if (day?.tokens > 0) parts.push(`${short(day.tokens)} tokens`);
  return parts;
}

// A "#rrggbb" accent mixed over a "#rrggbb" background; the accent itself when either is another form.
export function mix(accent, bg, share) {
  const rgb = (h) => /^#([0-9a-f]{6})/i.exec(h ?? "")?.[1];
  const a = rgb(accent), b = rgb(bg);
  if (share >= 1 || !a || !b) return accent;
  const part = (s, i) => parseInt(s.slice(i, i + 2), 16);
  const out = [0, 2, 4].map((i) => Math.round(part(a, i) * share + part(b, i) * (1 - share)).toString(16).padStart(2, "0"));
  return `#${out.join("")}`;
}

// level 0 is the border; 1-4 climb from the background to the accent.
export const levelColors = (t) => LEVELS.map((_, i) => (i === 0 ? t.border : mix(t.accent, t.popover, SHARE[i])));

export function activityInit(view) {
  view.activity = null; // { days } once read; null before
  view.activityHover = null; // [week, row]
  view.activityModel = null;
}

export async function loadActivity(view) {
  let text = null;
  try { text = await readTextFile(`${appHome()}/usage/activity.json`); } catch {}
  view.activity = { days: parseActivity(text) };
}

const mono = (el) => el.font_family("JetBrains Mono");

// The grid's days, each cell's level and the summary: worked out once per day, record and theme, not per render.
function model(view, t) {
  const today = new Date(), stamp = `${key(today)} ${t.accent} ${t.border} ${t.popover}`;
  const kept = view.activityModel;
  if (kept && kept.stamp === stamp && kept.record === view.activity) return kept;
  const weeks = gridDays(today), days = view.activity?.days ?? {};
  const levels = weeks.map((week) => week.map((d) => (d ? levelOf(minutesOf(days[key(d)])) : 0)));
  return (view.activityModel = { stamp, record: view.activity, weeks, days, levels, colors: levelColors(t), sum: summarize(weeks, days) });
}

export function activityBlock(view, t) {
  const { weeks, days, levels, colors, sum } = model(view, t);
  const { minutes, active, longest } = sum;
  const empty = active === 0;
  const hover = view.activityHover;
  const hovered = !empty && hover ? weeks[hover[0]]?.[hover[1]] : null;
  const num = (text) => mono(div()).text_color(t.foreground).child(text);
  const note = (text) => mono(div()).text_color(t.muted).child(text);
  const grid = h_flex().id("st-prof-activity-grid").relative().gap(GAP).flex_none()
    .children(weeks.map((week, w) => v_flex().gap(GAP).children(week.map((d, r) => {
      if (!d) return div().id(`st-prof-activity-gap-${w}-${r}`).w(CELL).h(CELL).flex_none();
      const on = hover?.[0] === w && hover?.[1] === r && !empty;
      return div().id(`st-prof-activity-cell-${w}-${r}`).w(CELL).h(CELL).flex_none().rounded(2)
        .bg(colors[levels[w][r]])
        .when(on, (el) => el.border_1().border_color(t.foreground))
        .on_hover((over, cx) => {
          if (empty) return;
          const now = view.activityHover;
          if (over) view.activityHover = [w, r];
          else if (now?.[0] === w && now?.[1] === r) view.activityHover = null;
          else return;
          cx.notify();
        });
    }))))
    .when(Boolean(hovered), (el) => {
      const [w, r] = hover;
      const parts = tooltip(hovered, days[key(hovered)]);
      // Left of the cell; the first columns have no room there, so the tip goes right.
      const place = w < 30 ? (tip) => tip.left(w * STEP + STEP + 6) : (tip) => tip.right(WEEKS * STEP - GAP - w * STEP + 6);
      return el.child(place(div().id("st-prof-activity-tip").absolute().top(r * STEP + CELL / 2 - 13)).flex().items_center().px(8).py(5).rounded(5).border_1().border_color(t.input).bg(t.raised)
        .child(mono(h_flex().gap(6).text_size(11).line_height("15px").text_color(t.foreground))
          .children(parts.flatMap((p, i) => (i ? [div().text_color(t.muted).child("·"), div().child(p)] : [div().child(p)])))));
    });
  // "Mon" on row 2, "Wed" on row 4, "Fri" on row 6; each right-aligned in its 53 px column.
  const dayLabels = v_flex().w(LABELS).flex_none().gap(GAP).pr(8)
    .children(DAY_NAMES.map((n, r) => mono(div().h(CELL).flex().items_center().justify_end().text_size(10).line_height("9px").text_color(t.dim)).child(r % 2 === 1 ? n : "")));
  // A month's name sits over the first week whose column holds its 1st.
  const months = weeks.map((week, w) => {
    const first = week.find((d) => d && d.getDate() === 1);
    return first ? [w, MONTHS[first.getMonth()]] : null;
  }).filter(Boolean);
  const monthRow = div().relative().h(14).ml(LABELS).w(WEEKS * STEP - GAP).flex_none()
    .children(months.map(([w, name]) => mono(div().absolute().left(w * STEP).top(0).text_size(10).line_height("14px").text_color(t.dim)).child(name)));
  const legend = h_flex().gap(6).items_center().flex_none()
    .child(mono(div().text_size(10.5).text_color(t.dim)).child("less"))
    .child(h_flex().gap(1).children(colors.map((c, i) => div().id(`st-prof-activity-legend-${i}`).w(CELL).h(CELL).rounded(2).bg(c))))
    .child(mono(div().text_size(10.5).text_color(t.dim)).child("more"));
  return v_flex().id("st-prof-activity").w_full().pb(16).border_b(1).border_color(t.border)
    .child(h_flex().w_full().items_center().justify_between().pt(20).pb(6)
      .child(div().text_size(13).line_height("18px").font_semibold().text_color(t.foreground).child("Agent activity · this computer"))
      .child(mono(div().text_size(12).text_color(t.dim)).child("last 12 months")))
    .child(v_flex().w_full().child(monthRow).child(h_flex().items_start().child(dayLabels).child(grid)))
    .child(h_flex().w_full().mt(10).items_center().justify_between()
      .child(empty
        ? mono(h_flex().id("st-prof-activity-empty").text_size(11)).child(note("No agent activity recorded yet")).child(mono(div().text_color(t.dim)).child(" · starts with your first agent session"))
        : mono(h_flex().id("st-prof-activity-summary").gap(6).text_size(11))
          .child(num(count(minutes, "min", "min")))
          .child(note("·")).child(num(count(active, "active day", "active days")))
          .child(note("·")).child(note("longest streak")).child(num(count(longest, "day", "days"))))
      .when(!empty, (el) => el.child(legend)));
}

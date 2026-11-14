// Settings > Profile > "Agent activity · this computer": a 53 x 7 heatmap of the last year, read from
// <app home>/usage/activity.json (src/activity.rs writes it; it never leaves this computer).
//   {"version":1,"days":{"YYYY-MM-DD":{"seconds":n,"sessions":n,"tokens":n}}}  (local dates)

import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { appHome, readTextFile, theme } from "h0x";

const WEEKS = 53, CELL = 9, GAP = 2, LABELS = 53;
const MIN_CELL = 6, MAX_CELL = 14, DEFAULT_WIDTH = 634; // the content column at the standard window
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

// The grid's one geometry, from the width of the whole row (the weekday labels and the grid), the clamp applied here:
// about 9 px cells and a 2 px gap at 634 px, growing to fill a wider row up to 14 px cells (then left-aligned).
// Every element of the graph is placed at `week * step` and `day * step` from the grid's corner, never by a flex row
// with a gap: the layout rounds each element to the device's pixels on its own, so positions cannot drift apart as
// they add up across 53 columns. Cells, month labels, hover and tooltip use these numbers and nothing else.
export function geometry(avail) {
  const row = Number.isFinite(avail) && avail > 0 ? avail : DEFAULT_WIDTH;
  const grid = Math.max(row - LABELS, WEEKS * (MIN_CELL + GAP) - GAP);
  const cell = Math.min(MAX_CELL, Math.max(MIN_CELL, (grid + GAP) / WEEKS - GAP));
  const step = cell + GAP;
  return { cell, step, left: LABELS, width: WEEKS * step - GAP, height: 7 * step - GAP };
}

// The tooltip's width: an upper bound (11 px mono text, the dots between its parts, padding and border).
export const tipWidth = (parts) => Math.ceil((18 + parts.join("").length * 6.6 + (parts.length - 1) * 18.6) * 1.1);

// Where the tooltip's left edge goes: centred on the column, kept inside the grid (0 .. width - tip).
export const tipLeft = (week, g, tip) => Math.max(0, Math.min(week * g.step + g.step / 2 - tip / 2, g.width - tip));

// The [week, day] under a point of the grid (0, 0 is its top-left corner), or null in a gap or outside.
export function cellAt(x, y, g) {
  if (!(x >= 0 && y >= 0 && x < g.width && y < g.height)) return null;
  const w = Math.floor(x / g.step), r = Math.floor(y / g.step);
  if (w >= WEEKS || r >= 7 || x - w * g.step >= g.cell || y - r * g.step >= g.cell) return null;
  return [w, r];
}

// [week, name] for each month whose 1st is in the grid: its label sits over that week's column.
export function monthColumns(weeks) {
  return weeks.map((week, w) => {
    const first = week.find((d) => d && d.getDate() === 1);
    return first ? [w, MONTHS[first.getMonth()]] : null;
  }).filter(Boolean);
}

export function activityInit(view) {
  view.activity = null; // { days } once read; null before
  view.activityHover = null; // [week, row]
  view.activityModel = null;
  view.activityWidth = DEFAULT_WIDTH; // the row's measured width
}

// The graph is a retained nested view of its own: Settings > Profile draws it once and rebuilds it only when the
// graph itself changes (data, hover, width, theme), not on every Profile render. A render has a 50 ms budget and
// the graph is about half of Profile's work. The page tells it when Profile is shown with `set_props({ shown })`.
export class ActivityGraph extends View {
  init(props, cx) {
    activityInit(this);
    this.cx = cx;
    this.shown = Boolean(props?.shown);
    if (this.shown) this.refresh();
    // The record changes while Profile is open (an agent is working): read it again each minute.
    cx.timer.every(60000, () => { if (this.shown) this.refresh(); });
  }

  // Profile opens or closes: reading the record again when it opens.
  update(props) {
    const was = this.shown;
    this.shown = Boolean(props?.shown);
    if (this.shown && !was) this.refresh();
  }

  refresh() {
    this.cx.spawn(async (cx) => {
      await loadActivity(this);
      cx.notify();
    });
  }

  render() {
    return activityBlock(this, theme());
  }
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
  return (view.activityModel = { stamp, record: view.activity, weeks, days, levels, colors: levelColors(t), sum: summarize(weeks, days), months: monthColumns(weeks), runs: columnRuns(weeks, levels) });
}

// Each week's column as runs of days in a row with the same level: [[days, level], ...] from Sunday down.
// A run is ONE element however many days it has (the row gaps are drawn over it), so a render builds
// a few dozen elements for a quiet year rather than 371 (a render has a 50 ms budget).
export function columnRuns(weeks, levels) {
  return weeks.map((week, w) => {
    const runs = [];
    week.forEach((d, r) => {
      if (!d) return;
      const last = runs[runs.length - 1];
      if (last && last[1] === levels[w][r]) last[0]++;
      else runs.push([1, levels[w][r]]);
    });
    return runs;
  });
}

export function activityBlock(view, t) {
  const m = model(view, t);
  const { weeks, days, colors, months, runs } = m;
  const { minutes, active, longest } = m.sum;
  const empty = active === 0;
  const hover = view.activityHover;
  const hovered = !empty && hover ? weeks[hover[0]]?.[hover[1]] : null;
  const g = geometry(view.activityWidth);
  const num = (text) => mono(div()).text_color(t.foreground).child(text);
  const note = (text) => mono(div()).text_color(t.muted).child(text);
  // A render has a 50 ms budget, so the 371 days are not 371 elements: days in a row with one level are a single run,
  // the gaps between rows are six lines over them, the hovered day is one outline, and ONE handler on the grid maps
  // the pointer (`local_position`, relative to the grid) to its day.
  const at = (e) => {
    const cellUnder = cellAt(e.local_position.x, e.local_position.y, g);
    return cellUnder && weeks[cellUnder[0]][cellUnder[1]] ? cellUnder : null;
  };
  const [hw, hr] = hover ?? [];
  const outline = hovered ? div().absolute().left(hw * g.step).top(hr * g.step).size(g.cell).rounded(2).border_1().border_color(t.foreground) : null;
  const rowGaps = [0, 1, 2, 3, 4, 5].map((r) => div().absolute().left(0).top(r * g.step + g.cell).w(g.width).h(GAP).bg(t.popover));
  const cells = runs.flatMap((col, w) => {
    let day = 0;
    return col.map(([n, level]) => {
      const top = day * g.step;
      day += n;
      return div().absolute().left(w * g.step).top(top).w(g.cell).h(n * g.step - GAP).rounded(2).bg(colors[level]);
    });
  });
  const grid = div().id("st-prof-activity-grid").relative().w(g.width).h(g.height).flex_none()
    .on_mouse_move((e, cx) => {
      if (empty) return;
      const now = view.activityHover, next = at(e);
      if (now?.[0] === next?.[0] && now?.[1] === next?.[1]) return;
      view.activityHover = next;
      cx.notify();
    })
    .on_hover((over, cx) => {
      if (over || !view.activityHover) return;
      view.activityHover = null;
      cx.notify();
    })
    .children(outline ? [...cells, ...rowGaps, outline] : [...cells, ...rowGaps])
    .when(Boolean(hovered), (el) => {
      const [w, r] = hover;
      const parts = tooltip(hovered, days[key(hovered)]);
      // Centred on the column and kept inside the grid; below the top rows, above the others.
      const top = r < 3 ? r * g.step + g.cell + 6 : r * g.step - 33;
      return el.child(div().id("st-prof-activity-tip").absolute().left(tipLeft(w, g, tipWidth(parts))).top(top).flex().items_center().px(8).py(5).rounded(5).border_1().border_color(t.input).bg(t.raised)
        .child(mono(h_flex().gap(6).text_size(11).line_height("15px").text_color(t.foreground))
          .children(parts.flatMap((p, i) => (i ? [div().text_color(t.muted).child("·"), div().child(p)] : [div().child(p)])))));
    });
  // "Mon" on row 2, "Wed" on row 4, "Fri" on row 6; each right-aligned in its 53 px column.
  const dayLabels = div().relative().w(g.left).h(g.height).flex_none()
    .children([1, 3, 5].map((r) => mono(div().absolute().left(0).top(r * g.step).w(g.left - 8).h(g.cell).flex().items_center().justify_end().text_size(10).line_height("9px").text_color(t.dim)).child(DAY_NAMES[r])));
  // A month's name sits over the first week whose column holds its 1st.
  const monthRow = div().relative().h(14).ml(g.left).w(g.width).flex_none()
    .children(months.map(([w, name]) => mono(div().absolute().left(w * g.step).top(0).text_size(10).line_height("14px").text_color(t.dim)).child(name)));
  const legend = h_flex().gap(6).items_center().flex_none()
    .child(mono(div().text_size(10.5).text_color(t.dim)).child("less"))
    .child(h_flex().gap(1).children(colors.map((c, i) => div().id(`st-prof-activity-legend-${i}`).w(CELL).h(CELL).rounded(2).bg(c))))
    .child(mono(div().text_size(10.5).text_color(t.dim)).child("more"));
  return v_flex().id("st-prof-activity-graph").w_full().pb(16).border_b(1).border_color(t.border)
    .child(h_flex().w_full().items_center().justify_between().pt(20).pb(6)
      .child(div().text_size(13).line_height("18px").font_semibold().text_color(t.foreground).child("Agent activity · this computer"))
      .child(mono(div().text_size(12).text_color(t.dim)).child("last 12 months")))
    .child(v_flex().w_full().child(monthRow).child(h_flex().id("st-prof-activity-body").w_full().items_start()
      .on_size_change((size, cx) => {
        if (!Number.isFinite(size?.width) || Math.abs(size.width - view.activityWidth) < 1) return;
        view.activityWidth = size.width;
        cx.notify();
      })
      .child(dayLabels).child(grid)))
    .child(h_flex().w_full().mt(10).items_center().justify_between()
      .child(empty
        ? mono(h_flex().id("st-prof-activity-empty").text_size(11)).child(note("No agent activity recorded yet")).child(mono(div().text_color(t.dim)).child(" · starts with your first agent session"))
        : mono(h_flex().id("st-prof-activity-summary").gap(6).text_size(11))
          .child(num(count(minutes, "min", "min")))
          .child(note("·")).child(num(count(active, "active day", "active days")))
          .child(note("·")).child(note("longest streak")).child(num(count(longest, "day", "days"))))
      .when(!empty, (el) => el.child(legend)));
}

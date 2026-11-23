//! Geometry for panes that aren't drawn: programs starting, and tabs that are hidden.
//! Layout metrics are shared with the renderer; normal resize still handles pixel rounding.

use super::*;
use crate::layout::Header;

pub(super) fn node_border(n: &Node, focused: bool) -> Option<&Line> {
    n.focus.as_ref().filter(|_| focused).and_then(|f| f.border.as_ref()).or(n.border.as_ref())
}

pub(super) fn padding(n: &Node) -> (f32, f32) {
    n.padding.map_or((0.0, 0.0), |p| p.axes())
}

pub(super) fn frame_border(frame: &PaneFrame, focused: bool) -> Option<&Line> {
    if focused { frame.focus.as_ref().or(frame.border.as_ref()) } else { frame.border.as_ref() }
}

pub(super) fn header_height(header: &Header) -> f32 {
    header.height.unwrap_or(HEADER_HEIGHT)
}

pub(super) fn gap(n: &Node, frame: Option<&PaneFrame>) -> f32 {
    n.gap.or(frame.and_then(|f| f.gap)).unwrap_or(0.0)
}

pub(super) fn separator_width(n: &Node, gap: f32) -> f32 {
    gap + n.divider.as_ref().map_or(0.0, |line| line.width)
}

pub(super) fn weight(n: &Node) -> f32 {
    match n.size {
        Some(Span::Px { .. }) => 0.0,
        Some(Span::Weight { weight }) => weight,
        None => 1.0,
    }
}

fn inset(size: Size<Pixels>, x: f32, y: f32) -> Size<Pixels> {
    gpui::size((size.width - px(x)).max(px(0.0)), (size.height - px(y)).max(px(0.0)))
}

fn content_size(n: &Node, size: Size<Pixels>, focused: bool, zoom: f32) -> Size<Pixels> {
    let border = node_border(n, focused).map_or(0.0, |line| line.width);
    let (y, x) = padding(n);
    inset(size, 2.0 * (x + border) * zoom, 2.0 * (y + border) * zoom)
}

/// Flex shares after fixed siblings, gaps and min/max constraints.
fn shares(children: &[&Node], available: f32, zoom: f32) -> Vec<f32> {
    let mut sizes = vec![0.0; children.len()];
    let mut pending = Vec::new();
    let mut left = available;
    for (i, n) in children.iter().enumerate() {
        match n.size {
            Some(Span::Px { px }) => {
                sizes[i] = (px * zoom).max(n.min.unwrap_or(0.0) * zoom).min(n.max.unwrap_or(f32::INFINITY) * zoom);
                left -= sizes[i];
            }
            _ => pending.push(i),
        }
    }
    let total_weight = pending.iter().map(|&i| weight(children[i])).sum::<f32>().max(f32::EPSILON);
    let initial_free = left.max(0.0);
    while !pending.is_empty() {
        let total = pending.iter().map(|&i| weight(children[i])).sum::<f32>().max(f32::EPSILON);
        // Grow factors stay normalized to the original siblings after an item freezes.
        let free = left.max(0.0).min(initial_free * (total / total_weight));
        let mut violations = Vec::new();
        for &i in &pending {
            let n = children[i];
            let share = free * weight(n) / total;
            sizes[i] = share.min(n.max.unwrap_or(f32::INFINITY) * zoom).max(n.min.unwrap_or(0.0) * zoom);
            violations.push((i, sizes[i] - share));
        }
        let total_violation = violations.iter().map(|(_, violation)| violation).sum::<f32>();
        if total_violation == 0.0 { break; }
        for (i, violation) in violations {
            if (total_violation > 0.0 && violation <= 0.0) || (total_violation < 0.0 && violation >= 0.0) { continue; }
            left -= sizes[i];
            pending.retain(|&j| j != i);
        }
    }
    sizes
}

fn block_bounds(n: &Node, bounds: Bounds<Pixels>, target: &str, frame: Option<&PaneFrame>, focused: &HashSet<String>, zoom: f32) -> Option<Bounds<Pixels>> {
    if n.hidden { return None; }
    let border = node_border(n, focused.contains(&n.id)).map_or(0.0, |line| line.width) * zoom;
    let mut origin = bounds.origin + point(px(border), px(border));
    let size = inset(bounds.size, 2.0 * border, 2.0 * border);
    if n.content().is_some() {
        return (n.id == target).then_some(Bounds::new(origin, size));
    }
    let (y, x) = padding(n);
    origin += point(px(x * zoom), px(y * zoom));
    let size = content_size(n, bounds.size, focused.contains(&n.id), zoom);
    match &n.kind {
        Kind::Row { children } | Kind::Column { children } => {
            let row = n.axis() == Some(Axis::Row);
            let children: Vec<_> = children.iter().filter(|n| !n.hidden).collect();
            let separator = separator_width(n, gap(n, frame)) * zoom;
            let available = f32::from(if row { size.width } else { size.height }) - separator * children.len().saturating_sub(1) as f32;
            for (child, span) in children.iter().zip(shares(&children, available, zoom)) {
                let size = if row { gpui::size(px(span), size.height) } else { gpui::size(size.width, px(span)) };
                if let Some(bounds) = block_bounds(child, Bounds::new(origin, size), target, frame, focused, zoom) { return Some(bounds); }
                if row { origin.x += px(span + separator); } else { origin.y += px(span + separator); }
            }
            None
        }
        Kind::Stack { header, .. } => {
            let height = header.as_ref().map_or(0.0, header_height) * zoom;
            if header.as_ref().is_some_and(|h| !h.bottom()) { origin.y += px(height); }
            block_bounds(n.active_child()?, Bounds::new(origin, inset(size, 0.0, height)), target, frame, focused, zoom)
        }
        _ => None,
    }
}

fn pane_size(n: &Node, size: Size<Pixels>, target: &str, frame: Option<&PaneFrame>, focused: &HashSet<String>, zoom: f32) -> Option<Size<Pixels>> {
    let bounds = block_bounds(n, Bounds::new(Point::default(), size), target, frame, focused, zoom)?;
    let n = n.find(target)?;
    let (y, x) = padding(n);
    let mut size = inset(bounds.size, 2.0 * x * zoom, 2.0 * y * zoom);
    if let Some(frame) = frame.filter(|_| n.is_pane()) {
        let border = frame_border(frame, focused.contains(&n.id)).map_or(0.0, |line| line.width) * zoom;
        let header = frame.header.as_ref().filter(|h| !h.hover()).map_or(0.0, header_height) * zoom;
        size = inset(size, 2.0 * border, 2.0 * border + header);
    }
    Some(size)
}

/// The tab area and the cell size.
type Geometry = (Size<Pixels>, Size<Pixels>);

/// Whether hidden tabs fit the window: the geometry seen at the last tick, and
/// whether they were sized for it.
#[derive(Default)]
pub(super) enum HiddenFit {
    #[default]
    Unknown,
    Seen(Geometry),
    Done(Geometry),
}

impl AppWindow {
    pub(super) fn initial_size(&self, root: &Node, pane: &str, window: &Window) -> Option<(u16, u16)> {
        let focused = root.path_to(pane)?.into_iter().collect();
        self.displayed_size(root, root.panes().len(), pane, &focused, window)
    }

    /// The grid `pane` gets when `tab` is the one shown.
    pub(super) fn shown_size(&self, tab: &Tab, pane: &str, window: &Window) -> Option<(u16, u16)> {
        let focused = tab.focused_pane().and_then(|f| tab.root.path_to(&f)).unwrap_or_default().into_iter().collect();
        self.displayed_size(tab.shown(), tab.root.panes().len(), pane, &focused, window)
    }

    /// `shown`: the tree on screen; `panes`: how many the tab has (its frame needs more than one, or `lone`).
    fn displayed_size(&self, shown: &Node, panes: usize, pane: &str, focused: &HashSet<String>, window: &Window) -> Option<(u16, u16)> {
        let (context, area) = self.context_area()?;
        let frame = match context.content() {
            Some(Content::Context { pane: Some(frame), .. }) if frame.lone || panes > 1 => Some(frame),
            _ => None,
        };
        let zoom = settings::zoom();
        // The absolute bounds canvas already excludes the context border (padding box).
        let (y, x) = padding(context);
        let size = inset(area, 2.0 * x * zoom, 2.0 * y * zoom);
        let size = pane_size(shown, size, pane, frame, focused, zoom)?;
        Some(crate::terminal::grid_size(size, crate::terminal::cell_size(window)))
    }

    /// Use the full split layout even if it has never been drawn unzoomed.
    pub(super) fn unzoomed_neighbor(&self, tab: &Tab, from: &str, dir: &str) -> Option<String> {
        let (context, area) = self.context_area()?;
        let frame = match context.content() {
            Some(Content::Context { pane: Some(frame), .. }) if frame.lone || tab.root.panes().len() > 1 => Some(frame),
            _ => None,
        };
        let zoom = settings::zoom();
        let (y, x) = padding(context);
        let origin = self.bounds.borrow().get(&context.id)?.origin + point(px(x * zoom), px(y * zoom));
        let bounds = Bounds::new(origin, inset(area, 2.0 * x * zoom, 2.0 * y * zoom));
        let focused = tab.root.path_to(from)?.into_iter().collect();
        let geometry = |id: &str| block_bounds(&tab.root, bounds, id, frame, &focused, zoom);
        let candidates = tab.root.visible_blocks().into_iter().filter(|n| n.is_pane() && n.id != from)
            .filter_map(|n| Some((n.id.clone(), geometry(&n.id)?)));
        nearest(geometry(from)?, dir, candidates, &self.pane_history)
    }

    /// The block tabs show in, and the area it was last drawn with.
    fn context_area(&self) -> Option<(&Node, Size<Pixels>)> {
        let context = self.chrome.visible_blocks().into_iter().find(|n| matches!(n.content(), Some(Content::Context { .. })))?;
        let size = self.bounds.borrow().get(&context.id)?.size;
        (size.width > px(0.0) && size.height > px(0.0)).then_some((context, size))
    }

    /// Terminals in hidden tabs keep the size they last had on screen, so a tab shown
    /// after the window changed size draws its old screens, then jumps once the programs
    /// redraw. Once the window settles (the same at two ticks), they are sized as their
    /// panes would be, so the programs redraw out of sight.
    pub(super) fn fit_hidden_tabs(&mut self, window: &Window, cx: &App) {
        let Some((_, area)) = self.context_area() else { return };
        let cell = crate::terminal::cell_size(window);
        let now = (area, cell);
        match self.hidden_fit {
            HiddenFit::Done(g) if g == now => return,
            HiddenFit::Seen(g) if g == now => self.hidden_fit = HiddenFit::Done(now),
            _ => {
                self.hidden_fit = HiddenFit::Seen(now);
                return;
            }
        }
        let scale = window.scale_factor();
        let active = self.model.active_tab().map(|t| t.id.clone());
        for tab in self.model.tabs().filter(|t| Some(&t.id) != active.as_ref()) {
            for pane in tab.shown().panes() {
                if let Some(t) = self.terms.get(&pane.id)
                    && let Some((cols, rows)) = self.shown_size(tab, &pane.id, window)
                {
                    t.view.read(cx).fit(cols, rows, cell, scale);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(layout: Value) -> Node {
        serde_json::from_value(layout).unwrap()
    }

    fn block(id: &str, extra: Value) -> Value {
        let mut b = json!({ "id": id, "type": "block", "content": { "terminal": format!("t-{id}") } });
        b.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        b
    }

    /// A root container made of small pieces: one deep `json!` literal exceeds the macro recursion limit.
    fn root(kind: &str, extra: Value, children: Vec<Value>) -> Node {
        let mut r = json!({ "id": "root", "type": kind });
        r.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        r["children"] = Value::Array(children);
        node(r)
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[test]
    fn split_blocks_sit_where_they_are_drawn_after_gaps_and_hidden_siblings() {
        let root = root("row", json!({ "gap": 10 }), vec![block("a", json!({})), block("c", json!({ "hidden": true })), block("b", json!({}))]);
        let at = |id: &str| block_bounds(&root, rect(100., 50., 210., 100.), id, None, &HashSet::new(), 1.0);
        assert_eq!(at("a"), Some(rect(100., 50., 100., 100.)));
        // The hidden sibling takes no room and adds no gap.
        assert_eq!(at("b"), Some(rect(210., 50., 100., 100.)));
        assert_eq!(at("c"), None);
        assert_eq!(at("missing"), None);
        assert_eq!(at(""), None);
    }

    #[test]
    fn padding_gaps_and_zoom_move_column_blocks_down() {
        let root = root("column", json!({ "gap": 10, "padding": [5, 8] }), vec![block("a", json!({})), block("b", json!({}))]);
        let at = |id: &str, zoom: f32| block_bounds(&root, rect(0., 0., 200., 230.), id, None, &HashSet::new(), zoom);
        assert_eq!(at("a", 1.0), Some(rect(8., 5., 184., 105.)));
        assert_eq!(at("b", 1.0), Some(rect(8., 120., 184., 105.)));
        assert_eq!(at("a", 2.0), Some(rect(16., 10., 168., 95.)));
        assert_eq!(at("b", 2.0), Some(rect(16., 125., 168., 95.)));
    }

    #[test]
    fn a_fixed_width_block_pushes_its_neighbour_over() {
        let root = root("row", json!({}), vec![block("a", json!({ "size": { "px": 30 } })), block("b", json!({}))]);
        let at = |id: &str| block_bounds(&root, rect(0., 0., 100., 40.), id, None, &HashSet::new(), 1.0);
        assert_eq!(at("a"), Some(rect(0., 0., 30., 40.)));
        assert_eq!(at("b"), Some(rect(30., 0., 70., 40.)));
    }

    #[test]
    fn a_pane_size_takes_off_borders_padding_and_the_frame() {
        let root = root("row", json!({ "border": { "width": 2 } }), vec![block("a", json!({ "border": { "width": 1 }, "padding": 3 })), block("b", json!({}))]);
        let none = HashSet::new();
        assert_eq!(block_bounds(&root, rect(0., 0., 104., 50.), "a", None, &none, 1.0), Some(rect(3., 3., 48., 44.)));
        assert_eq!(block_bounds(&root, rect(0., 0., 104., 50.), "b", None, &none, 1.0), Some(rect(52., 2., 50., 46.)));
        assert_eq!(pane_size(&root, size(px(104.), px(50.)), "a", None, &none, 1.0), Some(size(px(42.), px(38.))));
        let frame: PaneFrame = serde_json::from_value(json!({ "border": { "width": 2 } })).unwrap();
        assert_eq!(pane_size(&root, size(px(104.), px(50.)), "a", Some(&frame), &none, 1.0), Some(size(px(38.), px(34.))));
        assert_eq!(pane_size(&root, size(px(104.), px(50.)), "missing", Some(&frame), &none, 1.0), None);
    }

    #[test]
    fn a_too_small_area_gives_empty_blocks_never_negative_ones() {
        let root = root("row", json!({ "gap": 10, "padding": 10 }), vec![block("a", json!({ "padding": 4 })), block("b", json!({}))]);
        let none = HashSet::new();
        for area in [rect(0., 0., 10., 10.), rect(0., 0., 0., 0.)] {
            for id in ["a", "b"] {
                let b = block_bounds(&root, area, id, None, &none, 1.0).unwrap();
                assert!(b.size.width >= px(0.) && b.size.height >= px(0.), "{id}: {b:?}");
                let s = pane_size(&root, area.size, id, None, &none, 1.0).unwrap();
                assert_eq!(s, size(px(0.), px(0.)), "{id}");
            }
        }
    }
}

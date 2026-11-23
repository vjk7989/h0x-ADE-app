#[test]
fn dragging_squeezed_layout_blocks_never_crashes() {
    in_own_process("squeezed_layout_drag");
}

#[gpui::test]
fn squeezed_layout_drag(cx: &mut gpui::TestAppContext) {
    use gpui::{point, px, size};
    use serde_json::json;
    if !running_alone("squeezed_layout_drag") { return; }
    let block = |id: &str, height: f32| json!({
        "id": id, "type": "block", "size": {"px": height}, "content": {"mod": "drag-probe"}
    });
    let layout = json!({"id": "root", "type": "row", "children": [
        {"id": "sidebar", "type": "column", "size": {"px": 268}, "resizable": false, "children": [
            block("brand", 46.0), block("button", 52.0),
            {"id": "lists", "type": "column", "divider": {"width": 1, "color": "border"}, "children": [
                block("heading", 34.0),
                {"id": "stack", "type": "stack", "children": [
                    {"id": "list", "type": "block", "content": {"mod": "drag-probe"}}
                ]},
                block("zero1", 0.0), block("zero2", 0.0), block("zero3", 0.0), block("zero4", 0.0), block("zero5", 0.0)
            ]},
            block("pendientes", 200.0), block("vistas", 46.0)
        ]}
    ]});
    let (view, cx) = open_window(cx, "drag-probe", "export default () => null;", layout.clone());
    for height in [800.0, 380.0, 344.0, 300.0] {
        cx.simulate_resize(size(px(900.0), px(height)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.update(cx, |this, _| {
            for (container, count) in [("sidebar", 4), ("lists", 6)] {
                for index in 0..count {
                    for y in [-2000.0, 0.0, 2000.0] {
                        this.chrome = serde_json::from_value(layout.clone()).unwrap();
                        let parent = this.chrome.find(container).unwrap();
                        let a = &parent.children()[index];
                        let b = &parent.children()[index + 1];
                        let bounds = this.bounds.borrow();
                        let total = f32::from(bounds[&a.id].size.height + bounds[&b.id].size.height) / super::settings::zoom();
                        drop(bounds);
                        let before = serde_json::to_value(parent).unwrap();
                        this.drag = Some(super::Drag { container: container.into(), index, axis: super::Axis::Column });
                        this.drag_to(point(px(100.0), px(y)));
                        let after = this.chrome.find(container).unwrap();
                        if container == "sidebar" || total <= 48.0 {
                            assert_eq!(serde_json::to_value(after).unwrap(), before, "{container}[{index}], height {height}");
                        } else {
                            for child in &after.children()[index..=index + 1] {
                                if let Some(super::Span::Px { px }) = child.size {
                                    assert!(px >= 0.0 && px <= total, "{}: {px} out of {total}", child.id);
                                }
                            }
                        }
                    }
                }
            }
            this.drag = None;
        });
    }
}

#[test]
fn dragging_zero_width_and_bounded_row_blocks_never_crashes() {
    in_own_process("squeezed_row_drag");
}

/// Side by side this time (`Axis::Row`): zero-width blocks, a minimum wider than the
/// row, a maximum under the drag minimum, and windows too narrow for any of them.
#[gpui::test]
fn squeezed_row_drag(cx: &mut gpui::TestAppContext) {
    use gpui::{point, px, size};
    use serde_json::json;
    if !running_alone("squeezed_row_drag") { return; }
    let block = |id: &str, width: f32| json!({
        "id": id, "type": "block", "size": {"px": width}, "content": {"mod": "drag-probe"}
    });
    let mut wide_min = block("wide-min", 10.0);
    wide_min["min"] = json!(5000.0);
    let mut tiny_max = block("tiny-max", 30.0);
    tiny_max["max"] = json!(1.0);
    let layout = json!({"id": "root", "type": "row", "children": [
        block("zero1", 0.0), block("zero2", 0.0), wide_min, tiny_max,
        {"id": "flex1", "type": "block", "content": {"mod": "drag-probe"}},
        {"id": "flex2", "type": "block", "size": {"weight": 0.0}, "content": {"mod": "drag-probe"}},
        block("zero3", 0.0)
    ]});
    let (view, cx) = open_window(cx, "drag-probe", "export default () => null;", layout.clone());
    let count = layout["children"].as_array().unwrap().len() - 1;
    for width in [1200.0, 120.0, 60.0, 1.0] {
        cx.simulate_resize(size(px(width), px(400.0)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.update(cx, |this, _| {
            for index in 0..count {
                for x in [-2000.0, 0.0, 24.0, 48.0, width / 2.0, width, 1.0e9] {
                    this.chrome = serde_json::from_value(layout.clone()).unwrap();
                    let parent = this.chrome.find("root").unwrap();
                    let (a, b) = (&parent.children()[index], &parent.children()[index + 1]);
                    let bounds = this.bounds.borrow();
                    let total = match (bounds.get(&a.id), bounds.get(&b.id)) {
                        (Some(ba), Some(bb)) => Some(f32::from(ba.size.width + bb.size.width) / super::settings::zoom()),
                        _ => None,
                    };
                    drop(bounds);
                    let before = serde_json::to_value(parent).unwrap();
                    this.drag = Some(super::Drag { container: "root".into(), index, axis: super::Axis::Row });
                    this.drag_to(point(px(x), px(100.0)));
                    let after = this.chrome.find("root").unwrap();
                    match total {
                        Some(total) if total > 48.0 => {
                            for child in &after.children()[index..=index + 1] {
                                match child.size {
                                    Some(super::Span::Px { px }) => assert!(px.is_finite() && px >= 0.0 && px <= total.ceil(), "{}: {px} out of {total}", child.id),
                                    Some(super::Span::Weight { weight }) => assert!(weight.is_finite() && weight >= 0.0, "{}: weight {weight}", child.id),
                                    None => {}
                                }
                            }
                        }
                        _ => assert_eq!(serde_json::to_value(after).unwrap(), before, "root[{index}], width {width}, x {x}"),
                    }
                }
            }
            this.drag = None;
        });
    }
}

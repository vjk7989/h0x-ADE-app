use super::*;
use gpui::{IntoElement, Render, TestAppContext, VisualTestContext};
use std::ops::Deref;
use gpui_shell::{ShellRoot, ShellRuntime, policy::Policy};
use std::{cell::Cell, rc::Rc, sync::atomic::{AtomicU64, Ordering}, time::Instant};

struct Fixture(PathBuf);
impl Fixture {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = PathBuf::from(format!("/var/tmp/h0x-lifecycle-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.js"), source).unwrap();
        Self(dir)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}
struct Empty;
impl Render for Empty {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        gpui::div()
    }
}

struct Mounted(gpui::Entity<ShellRoot>);
impl Render for Mounted {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        self.0.clone()
    }
}

#[gpui::test]
fn size_change_reports_initial_and_resized_layout_without_pointer_input(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let fixture = Fixture::new(r#"
import { View, div } from "gpui-kit";
import { note } from "fixture";
export default class extends View {
  render() {
    return div().id("root").size_full().border(3).p(7)
      .on_size_change((size, cx) => { note(size.width, size.height); cx.notify(); });
  }
}"#);
    let sizes = Rc::new(RefCell::new(Vec::new()));
    let calls = sizes.clone();
    gpui_shell::policy::set_default(Policy::new().with_host_module(
        HostModule::new("fixture").function("note", move |a| {
            calls.borrow_mut().push((a.number(0)?, a.number(1)?));
            Ok(HostValue::Null)
        })
    ).unwrap());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&fixture.0, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    assert_eq!(sizes.borrow().len(), 1);
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    assert_eq!(sizes.borrow().len(), 1, "unchanged layout must not repeat the callback");
    visual.simulate_resize(gpui::size(gpui::px(720.), gpui::px(480.)));
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    assert_eq!(sizes.borrow().last().copied(), Some((720., 480.)));
    assert_eq!(sizes.borrow().len(), 2);
}


#[gpui::test]
fn plain_mod_scroll_handles_reveal_children_once_without_moving_visible_items(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    for (overflow, horizontal) in [("overflow_x_scroll", true), ("overflow_x_scrollbar", true), ("overflow_y_scroll", false), ("overflow_y_scrollbar", false)] {
        let source = r#"
import { View, div } from "gpui-kit";
import { ScrollHandle } from "gpui-base";
import { note } from "fixture";
export default class extends View {
  init(_, cx) {
    this.scroll = ScrollHandle.new();
    this.scroll.scroll_to_item(4);
    this.focus = cx.focus_handle();
    this.focus.focus();
  }
  render() {
    return div().size_full().track_focus(this.focus)
      .on_key_down((e, cx) => {
        this.scroll.scroll_to_item(Number(e.key));
        cx.notify();
      })
      .child(div().id("area").flex().DIRECTION().w(WIDTH).h(HEIGHT).OVERFLOW().track_scroll(this.scroll).on_scroll_wheel(() => {})
        .children([0, 1, 2, 3, 4].map(i => div().id("item-" + i).w(40).h(40).flex_none()
          .on_click(() => note(i)).child(String(i)))));
  }
}"#.replace("OVERFLOW", overflow)
            .replace("DIRECTION", if horizontal { "flex_row" } else { "flex_col" })
            .replace("WIDTH", if horizontal { "120" } else { "40" })
            .replace("HEIGHT", if horizontal { "40" } else { "120" });
        let fixture = Fixture::new(&source);
        let notes = Rc::new(RefCell::new(Vec::new()));
        let calls = notes.clone();
        gpui_shell::policy::set_default(Policy::new().with_host_module(
            HostModule::new("fixture").function("note", move |a| {
                calls.borrow_mut().push(a.number(0)? as usize);
                Ok(HostValue::Null)
            })
        ).unwrap());
        let runtime = ShellRuntime::new_isolated().unwrap();
        let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&fixture.0, window, cx).unwrap()));
        let mut visual = VisualTestContext::from_window(*window.deref(), cx);
        for _ in 0..3 {
            visual.update(|window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            });
            visual.run_until_parked();
        }
        let position = |along| if horizontal { gpui::point(gpui::px(along), gpui::px(20.)) } else { gpui::point(gpui::px(20.), gpui::px(along)) };
        visual.simulate_click(position(100.), Default::default());
        assert_eq!(*notes.borrow(), vec![4], "{overflow}: the last child must be visible");
        visual.simulate_keystrokes("3");
        for _ in 0..3 {
            visual.update(|window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            });
            visual.run_until_parked();
        }
        visual.simulate_click(position(10.), Default::default());
        assert_eq!(*notes.borrow(), vec![4, 2], "{overflow}: revealing a visible child must keep the offset");
        visual.simulate_keystrokes("0");
        for _ in 0..3 {
            visual.update(|window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            });
            visual.run_until_parked();
        }
        visual.simulate_click(position(10.), Default::default());
        assert_eq!(*notes.borrow(), vec![4, 2, 0], "{overflow}: the first child must be visible again");
        let delta = if horizontal { gpui::point(gpui::px(-80.), gpui::px(0.)) } else { gpui::point(gpui::px(0.), gpui::px(-80.)) };
        visual.simulate_mouse_move(position(20.), None, Default::default());
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: position(20.),
            delta: gpui::ScrollDelta::Pixels(delta),
            ..Default::default()
        });
        for _ in 0..3 {
            visual.update(|window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            });
            visual.run_until_parked();
        }
        visual.simulate_click(position(100.), Default::default());
        assert_eq!(*notes.borrow(), vec![4, 2, 0, 4], "{overflow}: manual scrolling must work after a consumed request");
        visual.update(|window, _| window.remove_window());
        visual.run_until_parked();
    }
}

/// `scroll_to_bottom` means the end of the list as it is drawn next: asked
/// before the first frame, or in the handler that adds rows, it still lands on
/// the last row.
#[gpui::test]
fn a_virtual_list_scrolls_to_the_bottom_of_the_rows_it_draws_next(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let fixture = Fixture::new(r#"
import { View, div } from "gpui-kit";
import { VirtualListScrollHandle, v_virtual_list } from "gpui-base";
import { note } from "fixture";
export default class extends View {
  init(_, cx) {
    this.count = 50;
    this.rows = VirtualListScrollHandle.new();
    this.rows.scroll_to_bottom();
    this.focus = cx.focus_handle();
    this.focus.focus();
  }
  render() {
    return div().size_full().track_focus(this.focus)
      .on_key_down((e, cx) => {
        this.count += 10;
        this.rows.scroll_to_bottom();
        cx.notify();
      })
      .child(div().w(40).h(100).child(v_virtual_list("rows", this.count, 20, (i) => String(i),
        (range) => {
          note(range.end);
          return Array.from({ length: range.end - range.start }, (_, i) => div().h(20).child(String(range.start + i)));
        })
        .track_scroll(this.rows)));
  }
}"#);
    let drawn = Rc::new(Cell::new(0));
    let last = drawn.clone();
    gpui_shell::policy::set_default(Policy::new().with_host_module(
        HostModule::new("fixture").function("note", move |a| {
            last.set(a.number(0)? as usize);
            Ok(HostValue::Null)
        })
    ).unwrap());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&fixture.0, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    let settle = |visual: &mut VisualTestContext| {
        for _ in 0..3 {
            visual.update(|window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            });
            visual.run_until_parked();
        }
        drawn.get()
    };
    assert_eq!(settle(&mut visual), 50, "asked before the first frame: the last row is drawn");
    visual.simulate_keystrokes("a");
    assert_eq!(settle(&mut visual), 60, "asked in the handler that added rows: the new last row is drawn");
    visual.update(|window, _| window.remove_window());
    visual.run_until_parked();
}

#[gpui::test]
fn external_file_drops_reach_mod_elements(cx: &mut TestAppContext) {
    use gpui::{ExternalPaths, FileDropEvent, InputEvent};
    cx.update(gpui_shell::init);
    let fixture = Fixture::new(r#"
import { View, div } from "gpui-kit";
import { note } from "fixture";
export default class extends View {
  render() {
    return div().size_full().on_drop((event, cx) => { note(event.paths); cx.stop_propagation(); });
  }
}"#);
    let paths = Rc::new(RefCell::new(Vec::new()));
    let calls = paths.clone();
    gpui_shell::policy::set_default(Policy::new().with_host_module(
        HostModule::new("fixture").function("note", move |a| {
            calls.borrow_mut().push(from_host(a.value(0)?));
            Ok(HostValue::Null)
        })
    ).unwrap());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&fixture.0, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    visual.update(|window, cx| {
        let position = gpui::point(gpui::px(20.), gpui::px(20.));
        window.dispatch_event(FileDropEvent::Entered {
            position,
            paths: ExternalPaths([PathBuf::from("/var/tmp/screenshot.png"), PathBuf::from("/var/tmp/a file.jpg")].into_iter().collect()),
        }.to_platform_input(), cx);
        window.draw(cx).clear(cx);
        window.dispatch_event(FileDropEvent::Submit { position }.to_platform_input(), cx);
    });
    visual.run_until_parked();
    assert_eq!(*paths.borrow(), vec![json!(["/var/tmp/screenshot.png", "/var/tmp/a file.jpg"])]);
}

#[gpui::test]
fn mod_run_rejects_unknown_arguments_before_queueing_a_command(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let fixture = Fixture::new(r#"
import { View, div } from "gpui-kit";
import { run } from "h0x";
import { note } from "fixture";
export default class extends View {
  init() {
    for (const [name, args] of [["space.close", {space: "X", confirmed: true}], ["terminal.kill", {id: "X"}]]) {
      try { run(name, args); note("accepted invalid command"); }
      catch (error) { note(String(error)); }
    }
  }
  render() { return div(); }
}"#);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let calls = seen.clone();
    gpui_shell::policy::set_default(Policy::new()
        .with_host_module(module(Instance { name: "test".into(), props: Value::Null }, 0)).unwrap()
        .with_host_module(HostModule::new("fixture").function("note", move |a| {
            calls.borrow_mut().push(a.string(0)?.to_owned());
            Ok(HostValue::Null)
        })).unwrap());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let _root = window.update(cx, |_, window, cx| runtime.try_load(&fixture.0, window, cx).unwrap()).unwrap();
    let seen = seen.borrow();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].contains("space.close: unknown argument(s): space; valid arguments: id, confirmed"), "{}", seen[0]);
    assert!(seen[1].contains("terminal.kill: unknown argument(s): id; valid arguments: terminal"), "{}", seen[1]);
}

#[gpui::test]
fn focus_command_lets_a_mod_take_arrow_keys(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let fixture = Fixture::new(r#"
import { View, div } from "gpui-kit";
import { nextCommand } from "h0x";
import { note } from "fixture";
export default class extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    cx.spawn(async (cx) => {
      for (;;) {
        const command = await nextCommand();
        if (command === "focus") { this.handle.focus(); note(command); }
        cx.notify();
      }
    });
  }
  render() {
    return div().size_full().track_focus(this.handle)
      .on_key_down((e, cx) => { note(e.key); cx.stop_propagation(); });
  }
}"#);
    let notes = Rc::new(RefCell::new(Vec::new()));
    let calls = notes.clone();
    gpui_shell::policy::set_default(Policy::new()
        .with_host_module(module(Instance { name: "focus-test".into(), props: Value::Null }, 42)).unwrap()
        .with_host_module(HostModule::new("fixture").function("note", move |a| {
            calls.borrow_mut().push(a.string(0)?.to_owned());
            Ok(HostValue::Null)
        })).unwrap());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&fixture.0, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    set_shown_copies([42].into());
    assert!(send_command_to(42, "focus"));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_keystrokes("down up");
    visual.run_until_parked();
    assert_eq!(*notes.borrow(), vec!["focus", "down", "up"]);
    set_shown_copies(Default::default());
}

#[gpui::test]
fn close_is_once_and_awaits_its_promise(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let fixture = Fixture::new(r#"
import { View, div } from "gpui-kit";
import { note } from "fixture";
export default class extends View {
  close(cx) {
    note("start");
    return Promise.resolve().then(() => { note("done"); cx.notify(); });
  }
  render() { return div(); }
}"#);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let calls = seen.clone();
    gpui_shell::policy::set_default(Policy::new().with_host_module(
        HostModule::new("fixture").function("note", move |a| {
            calls.borrow_mut().push(a.string(0)?.to_owned());
            Ok(HostValue::Null)
        })
    ).unwrap());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let root = window.update(cx, |_, window, cx| runtime.try_load(&fixture.0, window, cx).unwrap()).unwrap();
    window.update(cx, |_, window, cx| {
        let deadline = Instant::now() + Duration::from_millis(250);
        runtime.close(&root, deadline, window, cx).detach();
        runtime.close(&root, deadline, window, cx).detach();
    }).unwrap();
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), ["start", "done"]);
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
}

#[gpui::test]
fn mod_socket_writes_reads_and_reports_eof(cx: &mut TestAppContext) {
    use std::{io::{Read, Write}, net::{Shutdown, TcpListener}};

    cx.executor().allow_parking();
    cx.update(gpui_shell::init);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let source = r#"
import { View, div } from "gpui-kit";
import { connect } from "net";
import { note } from "fixture";
export default class extends View {
  init(_, cx) {
    cx.spawn(async () => {
      const socket = await connect("127.0.0.1", __PORT__);
      await socket.write("pingé");
      const bytes = await socket.read(4);
      const eof = await socket.read(4);
      note(bytes instanceof Uint8Array, bytes?.length, bytes?.[0], eof === null);
      socket.close();
    });
  }
  render() { return div(); }
}"#.replace("__PORT__", &listener.local_addr().unwrap().port().to_string());
    let fixture = Fixture::new(&source);
    let seen = Rc::new(RefCell::new(None));
    let calls = seen.clone();
    gpui_shell::policy::set_default(Policy::new().with_capabilities(capabilities())
        .with_host_module(HostModule::new("fixture").function("note", move |a| {
            calls.replace(Some((a.boolean(0)?, a.number(1)?, a.number(2)?, a.boolean(3)?)));
            Ok(HostValue::Null)
        })).unwrap());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&fixture.0, window, cx).unwrap()));
    cx.run_until_parked();

    let (mut peer, _) = listener.accept().unwrap();
    peer.set_nonblocking(false).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    peer.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
    let server = std::thread::spawn(move || {
        let mut request = [0; 6];
        peer.read_exact(&mut request).unwrap();
        assert_eq!(&request, "pingé".as_bytes());
        peer.write_all(&[0xfe]).unwrap();
        peer.shutdown(Shutdown::Write).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while seen.borrow().is_none() && Instant::now() < deadline {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(1));
    }
    server.join().unwrap();
    assert_eq!(*seen.borrow(), Some((true, 1.0, 254.0, true)));
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
}

#[gpui::test]
fn reload_releases_a_pending_tcp_read_without_a_close_hook(cx: &mut TestAppContext) {
    use std::io::Read as _;
    use std::net::TcpListener;

    cx.update(gpui_shell::init);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let source = r#"
import { View, div } from "gpui-kit";
import { nextCommand } from "h0x";
import { connect } from "net";
import { noteRead } from "fixture";
export default class extends View {
  init(_, cx) {
    cx.spawn(async () => {
      const socket = await connect("127.0.0.1", __PORT__);
      await nextCommand();
      noteRead();
      await socket.read(1);
    });
  }
  render() { return div(); }
}"#.replace("__PORT__", &listener.local_addr().unwrap().port().to_string());
    let fixture = Fixture::new(&source);
    let reading = Rc::new(Cell::new(false));
    let started = reading.clone();
    gpui_shell::policy::set_default(Policy::new().with_capabilities(capabilities())
        .with_host_module(module(Instance { name: "read-test".into(), props: Value::Null }, 44)).unwrap()
        .with_host_module(HostModule::new("fixture").function("noteRead", move |_| {
            started.set(true);
            Ok(HostValue::Null)
        })).unwrap());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let root = window.update(cx, |_, window, cx| runtime.try_load(&fixture.0, window, cx).unwrap()).unwrap();
    cx.run_until_parked();

    let (mut peer, _) = listener.accept().unwrap();
    peer.set_nonblocking(false).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let peer = std::thread::spawn(move || peer.read(&mut [0]));
    set_shown_copies([44].into());
    assert!(send_command_to(44, "read"));
    cx.run_until_parked();
    assert!(reading.get());

    std::fs::write(fixture.0.join("main.js"), r#"
import { View, div } from "gpui-kit";
export default class extends View { render() { return div(); } }
"#).unwrap();
    window.update(cx, |_, window, cx| runtime.reload_now(&root, window, cx).unwrap()).unwrap();
    cx.run_until_parked();
    assert_eq!(peer.join().unwrap().expect("the old socket is still open"), 0);
    set_shown_copies(Default::default());
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
}

/// `scroll_to_item` takes a whole, non-negative index; a released handle says so;
/// an index past the last child is harmless and leaves the area where it is.
#[gpui::test]
fn plain_scroll_handle_rejects_bad_indices_and_ignores_missing_children(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let fixture = Fixture::new(r#"
import { View, div } from "gpui-kit";
import { ScrollHandle } from "gpui-base";
import { note, refused } from "fixture";
export default class extends View {
  init() {
    this.scroll = ScrollHandle.new();
    const bad = [-1, 1.5, NaN, Infinity, -Infinity, "2", null, undefined, 2 ** 53];
    const rejected = bad.filter((index) => {
      try { this.scroll.scroll_to_item(index); return false; } catch (e) { return e instanceof TypeError; }
    }).length;
    const gone = ScrollHandle.new();
    gone.release();
    let released = false;
    try { gone.scroll_to_item(0); } catch (e) { released = e instanceof TypeError && /released/.test(e.message); }
    refused(rejected, released);
    this.scroll.scroll_to_item(99);
  }
  render() {
    return div().size_full()
      .child(div().id("area").flex().flex_col().w(40).h(120).overflow_y_scroll().track_scroll(this.scroll)
        .children([0, 1, 2, 3, 4].map(i => div().id("item-" + i).w(40).h(40).flex_none()
          .on_click(() => note(i)).child(String(i)))));
  }
}"#);
    let notes = Rc::new(RefCell::new(Vec::new()));
    let calls = notes.clone();
    let checked = Rc::new(Cell::new(None));
    let refusals = checked.clone();
    gpui_shell::policy::set_default(Policy::new().with_host_module(
        HostModule::new("fixture")
            .function("note", move |a| {
                calls.borrow_mut().push(a.number(0)? as usize);
                Ok(HostValue::Null)
            })
            .function("refused", move |a| {
                refusals.set(Some((a.number(0)? as usize, a.boolean(1)?)));
                Ok(HostValue::Null)
            })
    ).unwrap());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&fixture.0, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    for _ in 0..3 {
        visual.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        visual.run_until_parked();
    }
    assert_eq!(checked.get(), Some((9, true)), "every bad index is a TypeError, and so is a released handle");
    visual.simulate_click(gpui::point(gpui::px(20.), gpui::px(10.)), Default::default());
    assert_eq!(*notes.borrow(), vec![0], "a request past the last child must not move the area");
    visual.update(|window, _| window.remove_window());
    visual.run_until_parked();
}

/// `scroll_to_bottom` on an empty list: nothing to land on, no panic, and the
/// request is spent on that frame rather than replayed when rows arrive later.
#[gpui::test]
fn a_virtual_list_scroll_to_bottom_on_an_empty_list_is_harmless(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let fixture = Fixture::new(r#"
import { View, div } from "gpui-kit";
import { VirtualListScrollHandle, v_virtual_list } from "gpui-base";
import { note } from "fixture";
export default class extends View {
  init(_, cx) {
    this.count = 0;
    this.rows = VirtualListScrollHandle.new();
    this.rows.scroll_to_bottom();
    this.focus = cx.focus_handle();
    this.focus.focus();
  }
  render() {
    return div().size_full().track_focus(this.focus)
      .on_key_down((e, cx) => {
        if (e.key === "a") this.count += 50;
        if (e.key === "b") this.rows.scroll_to_bottom();
        if (e.key === "c") { this.count = 0; this.rows.scroll_to_bottom(); }
        cx.notify();
      })
      .child(div().w(40).h(100).child(v_virtual_list("rows", this.count, 20, (i) => String(i),
        (range) => {
          note(range.end);
          return Array.from({ length: range.end - range.start }, (_, i) => div().h(20).child(String(range.start + i)));
        })
        .track_scroll(this.rows)));
  }
}"#);
    let drawn = Rc::new(Cell::new(0));
    let last = drawn.clone();
    gpui_shell::policy::set_default(Policy::new().with_host_module(
        HostModule::new("fixture").function("note", move |a| {
            last.set(a.number(0)? as usize);
            Ok(HostValue::Null)
        })
    ).unwrap());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&fixture.0, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    let settle = |visual: &mut VisualTestContext| {
        for _ in 0..3 {
            visual.update(|window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            });
            visual.run_until_parked();
        }
        drawn.get()
    };
    assert_eq!(settle(&mut visual), 0, "an empty list draws no rows");
    visual.simulate_keystrokes("a");
    assert!(settle(&mut visual) < 50, "rows added later start at the top: the empty frame spent the request");
    visual.simulate_keystrokes("b");
    assert_eq!(settle(&mut visual), 50, "a new request lands on the last row");
    visual.simulate_keystrokes("c");
    settle(&mut visual);
    visual.update(|window, _| window.remove_window());
    visual.run_until_parked();
}

/// A reload that leaves script work waiting on something that will never
/// settle names it in the log; a reload with nothing pending stays quiet.
#[gpui::test]
fn reload_reports_the_tasks_it_strands(cx: &mut TestAppContext) {
    #[derive(Clone, Default)]
    struct Logs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Logs {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let logs = Logs::default();
    let writer = logs.clone();
    let _log = tracing::subscriber::set_default(
        tracing_subscriber::fmt().with_writer(move || writer.clone()).with_max_level(tracing::Level::WARN).finish(),
    );
    // tracing caches each callsite's interest process-wide. While one dispatcher is
    // registered, a callsite first hit on another test thread (a parallel reload that
    // strands its own task) asks that thread's default, gets `never`, and this
    // thread's warning is then dropped. A second registered dispatcher makes every
    // registration consult this one as well; events still go to the thread default.
    let _shared_interest = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::new());

    cx.update(gpui_shell::init);
    for (pending, summary) in [(true, Some("2 cx.sleep(ms)")), (false, None)] {
        let fixture = Fixture::new(&r#"
import { View, div } from "gpui-kit";
import { nextCommand } from "h0x";
export default class extends View {
  init(_, cx) {
    // Started by a command, after init: the sleeps belong to the running application.
    if (PENDING) {
      cx.spawn(async (cx) => {
        await nextCommand();
        await Promise.all([cx.sleep(600000), cx.sleep(600000)]);
      });
    }
  }
  render() { return div(); }
}"#.replace("PENDING", if pending { "true" } else { "false" }));
        gpui_shell::policy::set_default(Policy::new().with_capabilities(capabilities())
            .with_host_module(module(Instance { name: "strand-test".into(), props: Value::Null }, 45)).unwrap());
        let runtime = ShellRuntime::new_isolated().unwrap();
        let window = cx.add_window(|_, _| Empty);
        let root = window.update(cx, |_, window, cx| runtime.try_load(&fixture.0, window, cx).unwrap()).unwrap();
        cx.run_until_parked();
        if pending {
            set_shown_copies([45].into());
            assert!(send_command_to(45, "go"));
            cx.run_until_parked();
        }
        logs.0.lock().unwrap().clear();

        std::fs::write(fixture.0.join("main.js"), r#"
import { View, div } from "gpui-kit";
export default class extends View { render() { return div(); } }
"#).unwrap();
        window.update(cx, |_, window, cx| runtime.reload_now(&root, window, cx).unwrap()).unwrap();
        cx.run_until_parked();
        let log = String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();
        match summary {
            Some(summary) => assert!(log.contains("stranded") && log.contains(summary), "the reload must name what it stranded: {log:?}"),
            None => assert!(!log.contains("stranded"), "nothing was pending: {log:?}"),
        }
        set_shown_copies(Default::default());
        window.update(cx, |_, window, _| window.remove_window()).unwrap();
        cx.run_until_parked();
    }
}

#[gpui::test]
fn close_starts_native_work_before_releasing_the_copy(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let fixture = Fixture::new(r#"
import { View, div } from "gpui-kit";
import { start } from "fixture";
export default class extends View {
  close() { start(); }
  render() { return div(); }
}"#);
    let started = Rc::new(Cell::new(false));
    let call = started.clone();
    gpui_shell::policy::set_default(Policy::new().with_host_module(
        HostModule::new("fixture").async_function("start", move |_| {
            call.set(true);
            // Starting native work must not depend on its promise settling.
            Ok(std::future::pending::<gpui_shell::HostResult>())
        })
    ).unwrap());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let root = window.update(cx, |_, window, cx| runtime.try_load(&fixture.0, window, cx).unwrap()).unwrap();
    window.update(cx, |_, window, cx| runtime.close(&root, Instant::now() + Duration::from_millis(250), window, cx).detach()).unwrap();
    drop(root);
    cx.run_until_parked();
    assert!(started.get());
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
}

#[gpui::test]
fn final_close_pumps_native_awaits_before_the_window_goes(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let fixture = Fixture::new(r#"
import { View, div } from "gpui-kit";
import { wait, note } from "fixture";
export default class extends View {
  async close(cx) { await cx.sleep(1); await wait(); note(); await wait(); note(); }
  render() { return div(); }
}"#);
    let completed = Rc::new(Cell::new(0));
    let calls = completed.clone();
    gpui_shell::policy::set_default(Policy::new().with_host_module(
        HostModule::new("fixture")
            .async_function("wait", |_| Ok(async {
                std::thread::sleep(Duration::from_millis(10));
                Ok(HostValue::Null)
            }))
            .function("note", move |_| { calls.set(calls.get() + 1); Ok(HostValue::Null) })
    ).unwrap());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let root = window.update(cx, |_, window, cx| runtime.try_load(&fixture.0, window, cx).unwrap()).unwrap();
    window.update(cx, |_, window, cx| {
        // Each native await runs on its own thread; on a loaded runner that took up to 205ms for the
        // whole close. The deadline only bounds a hang, and the pump returns as soon as both finish.
        runtime.close_now(&root, Instant::now() + Duration::from_secs(5), window, cx);
        assert_eq!(completed.get(), 2, "both native awaits must finish before window destruction");
        window.remove_window();
    }).unwrap();
}

#[gpui::test]
fn final_close_stops_waiting_at_its_deadline(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let fixture = Fixture::new(r#"
import { View, div } from "gpui-kit";
import { wait } from "fixture";
export default class extends View {
  async close() { await wait(); }
  render() { return div(); }
}"#);
    gpui_shell::policy::set_default(Policy::new().with_host_module(
        HostModule::new("fixture").async_function("wait", |_| Ok(std::future::pending::<gpui_shell::HostResult>()))
    ).unwrap());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let root = window.update(cx, |_, window, cx| runtime.try_load(&fixture.0, window, cx).unwrap()).unwrap();
    let start = Instant::now();
    window.update(cx, |_, window, cx| {
        runtime.close_now(&root, start + Duration::from_millis(15), window, cx);
        window.remove_window();
    }).unwrap();
    assert!(start.elapsed() < Duration::from_millis(200));
}

#[gpui::test]
fn close_interrupts_a_synchronous_loop_at_the_shared_deadline(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let fixture = Fixture::new(r#"
import { View, div } from "gpui-kit";
export default class extends View {
  close() { while (true) {} }
  render() { return div(); }
}"#);
    gpui_shell::policy::set_default(Policy::new());
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let root = window.update(cx, |_, window, cx| runtime.try_load(&fixture.0, window, cx).unwrap()).unwrap();
    let start = Instant::now();
    window.update(cx, |_, window, cx| runtime.close(&root, start + Duration::from_millis(15), window, cx).detach()).unwrap();
    assert!(start.elapsed() < Duration::from_millis(200));
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
}

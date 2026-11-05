//! Shipped JS descriptions and native rendering regressions.
use super::*;
use gpui::TestAppContext;
use gpui_shell::{ShellRuntime, plugin::DescriptionSession, policy::Policy};
use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

struct Staged(PathBuf);
impl Staged {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = PathBuf::from(format!(
            "/var/tmp/h0x-render-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let staged = Self(path);
        for (folder, files) in [("mods", DEFAULT_MODS), ("frame", FRAME_MODS)] {
            for (name, bytes) in files {
                let path = staged.0.join(folder).join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            }
        }
        staged
    }
    fn entries(&self) -> Vec<String> {
        let mut entries = Vec::new();
        for folder in ["mods", "frame"] {
            for entry in std::fs::read_dir(self.0.join(folder)).unwrap() {
                let entry = entry.unwrap();
                assert!(entry.path().join("main.js").is_file(), "missing main.js: {:?}", entry.path());
                entries.push(format!("{folder}/{}", entry.file_name().to_str().unwrap()));
            }
        }
        entries.sort();
        entries
    }
    fn wrapper(&self, entry: &str, setup: &str) -> PathBuf {
        let dir = self.0.clone();
        std::fs::write(dir.join("main.js"), format!(
            "import Real from './{entry}/main.js'; export default class Scenario extends Real {{ init(props, cx) {{ super.init(props, cx); {setup} }} }}"
        )).unwrap();
        dir
    }
}
impl Drop for Staged {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[derive(Default)]
struct Calls {
    unexpected: Vec<String>,
    effects: Vec<(String, Vec<Value>)>,
    shared: BTreeMap<String, Value>,
}

fn updates(app: &str, core: &str) -> Value {
    json!({
        "app": {"version":"0.1.0", "updates":app != "source", "ready": (app == "ready").then_some("0.1.1")},
        "core": {"version":"0.1.5", "ready":(core == "ready").then_some("0.1.6"), "managed":(core == "managed").then_some("homebrew"), "error":(core == "error").then_some("offline"), "updating":core == "updating"},
        "checked":1700000000, "checks":app != "source", "checking":false
    })
}

fn fixture(populated: bool, props: Value, update: Value) -> BTreeMap<String, Value> {
    let program =
        json!({"key":"pi", "name":"Pi", "icon":null, "tint":true, "tile":"#334455", "color":"#aabbcc", "shell":false, "agent":true});
    let mut values = BTreeMap::from([
        ("props".into(), props),
        ("machines".into(), json!([{ "name": null, "layout": "ready" }])),
        ("spaces".into(), json!([])), ("agents".into(), json!([])), ("terminals".into(), json!([])), ("panes".into(), json!([])), ("focused".into(), Value::Null),
        ("layout".into(), serde_json::from_str(include_str!("../../defaults/layouts/main.json")).unwrap()),
        ("layouts".into(), json!(crate::window::shipped_layouts())),
        ("theme".into(), serde_json::to_value(&*crate::settings::theme()).unwrap()),
        ("settings".into(), serde_json::to_value(crate::settings::Settings::default()).unwrap()),
        ("themes".into(), json!([{"name":"graphite", "colors": &*crate::settings::theme()}])),  ("fonts".into(), json!(["Monospace", "Sans"])),
        ("updates".into(), update),
        ("window".into(), json!({"platform":"linux","titlebar":"none","buttons":true,"maximized":false,"fullscreen":false,"active":true})),
        ("placement".into(), Value::Null),
        ("home".into(), json!("/fixture")), ("appHome".into(), json!("/fixture/app")),
        ("keys".into(), Value::Array(serde_json::from_str::<BTreeMap<String, BTreeMap<String, String>>>(include_str!("../../defaults/keys.json")).unwrap().into_iter().flat_map(|(section, bindings)| bindings.into_iter().map(move |(keys, action)| json!({"section":section,"keys":keys,"action":action}))).collect())),
        ("commands".into(), Value::Array(crate::commands::COMMANDS.iter().map(|c| json!({"name":c.name,"about":c.about,"args":c.args.iter().map(|a| json!({"name":a.name,"required":a.required})).collect::<Vec<_>>()})).collect())),
    ]);
    if populated {
        values.insert("spaces".into(), json!([
            {"id":"s1","index":1,"name":"Local","folder":"/fixture/project","machine":null,"branch":"main","active":true,"status":"working","tabs":[{"id":"tab1","index":1,"name":"Build","active":true,"status":"working","panes":1,"zoomed":false,"settled":false,"settled_at":null,"changed_at":1700000000000i64,"working_since":1700000000000i64}]},
            {"id":"s2","index":2,"name":"Remote","folder":"/remote/project","machine":"remote","branch":null,"active":false,"status":"done","tabs":[{"id":"tab2","index":1,"name":"Done","active":true,"status":"done","panes":1,"zoomed":false,"settled":false,"settled_at":null,"changed_at":null,"working_since":null}]}
        ]));
        values.insert("panes".into(), json!([{"pane":"p1","terminal":"t1","tab":"tab1","title":"Build","cwd":"/fixture/project","branch":"main","status":"working","app":"pi","program":program,"focused":true,"zoomed":false}, {"pane":"p2","terminal":"t2","tab":"tab2","title":"Remote task","cwd":"/remote/project","branch":null,"status":"done","app":"pi","program":program,"focused":false,"zoomed":false}]));
        values.insert("terminals".into(), json!([{"address":"t1","name":"Build","title":"Build","argv":["pi"],"parent":null,"labels":{},"state":"running","status":"working","report":{"app":"pi"},"program":program,"cwd":"/fixture/project"}, {"address":"t2","name":"Remote task","title":"Remote task","argv":["pi"],"parent":null,"labels":{},"state":"running","status":"done","report":{"app":"pi"},"program":program,"cwd":"/remote/project"}]));
        values.insert("agents".into(), json!([{"address":"t1","pane":"p1","space":"s1","space_name":"Local","tab":"tab1","tab_index":1,"tab_name":"Build","status":"working","app":"pi","program":program,"title":"Build","seq":1,"focused":true}, {"address":"t2","pane":"p2","space":"s2","space_name":"Remote","tab":"tab2","tab_index":1,"tab_name":"Done","status":"done","app":"pi","program":program,"title":"Remote task","seq":2,"focused":false}]));
        values.insert(
            "focused".into(),
            json!({"space":"s1","tab":"tab1","pane":"p1","terminal":"t1","agent":"t1","shown":null,"cwd":"/fixture/project"}),
        );
    }
    values
}

fn module(values: BTreeMap<String, Value>, calls: Rc<RefCell<Calls>>) -> HostModule {
    let mut module = HostModule::new("h0x");
    for (name, value) in values {
        module = module.function(name, move |_| Ok(to_host(&value)));
    }
    module = module.function("feedbackPreview", |a| {
        Ok(to_host(
            &json!({"available":true,"body":{"message":a.string(0)?,"email":a.string(1)?}}),
        ))
    });
    let shared = calls.clone();
    module = module.function("shared", move |a| {
        Ok(shared.borrow().shared.get(a.string(0)?).map(to_host).unwrap_or(HostValue::Null))
    });
    let shared = calls.clone();
    module = module.function("setShared", move |a| {
        shared.borrow_mut().shared.insert(a.string(0)?.into(), from_host(a.value(1)?));
        Ok(HostValue::Null)
    });
    for name in ["run", "playSound", "setAgentOrder", "openOverlay", "closeOverlay", "notify", "presentUpdateNotes"] {
        let calls = calls.clone();
        module = module.function(name, move |a| {
            if name == "run" {
                let command = a.string(0)?;
                let command = crate::commands::find(command).unwrap_or_else(|| panic!("unknown shipped mod command {command}"));
                let args = a.get(1).map(from_host).unwrap_or(Value::Null);
                crate::commands::validate_args(command, &args).unwrap();
            }
            calls
                .borrow_mut()
                .effects
                .push((name.into(), (0..2).filter_map(|i| a.get(i).map(from_host)).collect()));
            Ok(if name == "openOverlay" || name == "notify" {
                "fixture-overlay".into()
            } else {
                HostValue::Null
            })
        });
    }
    for name in ["feedbackPasteImage", "feedbackClearImages", "feedbackAttachImages"] {
        module = module.function(name, |_| Ok(HostValue::Null));
    }
    module = module.async_function("feedbackPickImages", |_| Ok(async { Ok(HostValue::Array(vec![])) }));
    module = module.async_function("knownAgents", |_| Ok(async { Ok(HostValue::Array(vec![])) }));
    module = module.async_function("pickFolder", |_| Ok(async { Ok(HostValue::Null) }));
    for name in ["nextCommand", "watchFolder"] {
        module = module.async_function(name, |_| Ok(std::future::pending::<gpui_shell::HostResult>()));
    }
    for name in ["exec", "core", "readTextFile", "writeTextFile", "sendFeedback"] {
        let calls = calls.clone();
        module = module.async_function(name, move |a| {
            let first = a.get(0).map(from_host).unwrap_or(Value::Null);
            let reply = match name {
                "readTextFile" if first == "/fixture/app/settings.json" || first == "/fixture/.ssh/config" => Some(Value::Null),
                "readTextFile" => first
                    .as_str()
                    .and_then(|p| p.strip_prefix("/fixture/app/layouts/"))
                    .and_then(|p| p.strip_suffix(".json"))
                    .filter(|n| crate::window::shipped_layouts().contains(n))
                    .map(|_| json!("{}")),
                "core" if first == json!(["machine", "ls", "--json"]) => {
                    Some(json!({"code":0,"stdout":"{\"machines\":[{\"name\":\"remote\",\"state\":\"online\"}]}","stderr":""}))
                }
                "core" if first.as_array().and_then(|a| a.first()).and_then(Value::as_str) == Some("run") => {
                    Some(json!({"code":0,"stdout":"{\"address\":\"t-setup\"}","stderr":""}))
                }
                // The sessions panel lists the shown space's folder.
                "core" if first.as_array().and_then(|a| a.first()).and_then(Value::as_str) == Some("session") => {
                    Some(json!({"code":0,"stdout":"{\"sessions\":[]}","stderr":""}))
                }
                // The files panel lists a folder with sh: kind and name pairs.
                "exec" if first == "sh" && a.get(1).map(from_host).and_then(|v| v.get(2).cloned()) == Some(json!("files")) => {
                    Some(json!({"code":0,"stdout":"d\u{0}src\u{0}f\u{0}README.md\u{0}","stderr":""}))
                }
                "exec"
                    if first == "git"
                        && [
                            json!(["rev-parse", "--show-toplevel"]),
                            json!(["rev-parse", "--path-format=absolute", "--git-common-dir"]),
                            json!(["status", "--porcelain=v2", "--branch"]),
                            json!(["--no-optional-locks", "-c", "core.fsmonitor=false", "status", "--porcelain=v2", "--branch"]),
                            json!(["--no-optional-locks", "-c", "core.fsmonitor=false", "status", "--porcelain"]),
                            json!(["rev-parse", "--abbrev-ref", "HEAD"]),
                        ]
                        .contains(&a.get(1).map(from_host).unwrap_or(Value::Null)) =>
                {
                    Some(json!({"code":128,"stdout":"","stderr":"fixture: not a git repository"}))
                }
                _ => None,
            };
            let result = if let Some(reply) = reply {
                Ok(to_host(&reply))
            } else {
                let message = format!("unexpected {name}: {first}, {:?}", a.get(1));
                calls.borrow_mut().unexpected.push(message.clone());
                Err(HostError::new(message))
            };
            Ok(std::future::ready(result))
        });
    }
    module.declarations(DECLARATIONS)
}

struct Empty;
impl gpui::Render for Empty {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        gpui::div()
    }
}

fn describe(
    cx: &mut TestAppContext,
    staged: &Staged,
    entry: &str,
    populated: bool,
    props: Value,
    update: Value,
    setup: Option<&str>,
) -> anyhow::Result<String> {
    describe_with_values(cx, staged, entry, fixture(populated, props, update), setup)
}

fn describe_with_values(
    cx: &mut TestAppContext,
    staged: &Staged,
    entry: &str,
    values: BTreeMap<String, Value>,
    setup: Option<&str>,
) -> anyhow::Result<String> {
    let calls = Rc::new(RefCell::new(Calls::default()));
    static NEXT_STORE: AtomicU64 = AtomicU64::new(0);
    let policy = Policy::new()
        .with_storage_path(staged.0.join(format!("store-{}.json", NEXT_STORE.fetch_add(1, Ordering::Relaxed))))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))?;
    let runtime = ShellRuntime::new_isolated()?;
    let root = setup.map(|s| staged.wrapper(entry, s)).unwrap_or_else(|| staged.0.join(entry));
    let window = cx.add_window(|_, _| Empty);
    let result = window.update(cx, |_, window, cx| {
        let mut session = DescriptionSession::load(runtime, root, policy, window, cx)?;
        let first = session.describe(window, cx)?;
        let second = session.describe(window, cx)?;
        assert!(!first.is_empty() && !second.is_empty(), "empty description: {entry}");
        Ok(second)
    })?;
    window.update(cx, |_, window, _| window.remove_window())?;
    assert!(calls.borrow().unexpected.is_empty(), "{entry}: {:?}", calls.borrow().unexpected);
    result
}

fn props() -> Value {
    json!({"overlay":"o1","pane":"p1","terminal":"t1","space":"s1","tab":"pane","target":{"pane":"p1","terminal":"t1","id":"s1","space":"s1"},"title":"Fixture","value":"Name","message":"A prefilled message","command":{"name":"space.rename","args":{"id":"s1"}},"notification":{"title":"Finished","body":"Build done","action":{"name":"app.help"}},"action":"new"})
}

#[gpui::test]
fn shipped_initial_descriptions(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let start = Instant::now();
    let staged = Staged::new();
    let entries = staged.entries();
    let mut count = 0;
    for entry in &entries {
        for populated in [false, true] {
            describe(cx, &staged, entry, populated, props(), updates("current", "current"), None)
                .unwrap_or_else(|e| panic!("{entry}/{}: {e:#}", if populated { "populated" } else { "empty" }));
            count += 1;
        }
    }
    eprintln!(
        "{count} initial-description cases ({} entries, each twice): {:?}",
        entries.len(),
        start.elapsed()
    );
}

#[gpui::test]
fn brand_leaves_room_for_macos_traffic_lights_only_outside_full_screen(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (fullscreen, inset) in [(false, true), (true, false)] {
        let mut values = fixture(true, props(), updates("current", "current"));
        values.insert("window".into(), json!({"platform":"macos","titlebar":"none","fullscreen":fullscreen,"buttons":false}));
        let tree = describe_with_values(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
        let root = tree.lines().next().unwrap();
        assert_eq!(root.contains(".pl[Number(76.0)]"), inset, "fullscreen={fullscreen}: {tree}");
    }
}

#[gpui::test]
fn tabs_leave_room_for_macos_traffic_lights(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (platform, titlebar, fullscreen, width, lights, zoom, expected) in [
        ("macos", "none", false, 44, true, 1.0, 32),
        ("macos", "none", false, 44, true, 0.5, 108),
        ("macos", "none", false, 44, true, 2.0, 10),
        ("macos", "none", false, 60, true, 1.0, 16),
        ("macos", "none", false, 76, true, 1.0, 10),
        ("macos", "none", false, 268, true, 1.0, 10),
        ("macos", "none", true, 44, true, 1.0, 10),
        ("macos", "system", false, 44, true, 1.0, 10),
        ("macos", "none", false, 44, false, 1.0, 10),
        ("linux", "none", false, 44, true, 1.0, 10),
        ("windows", "none", false, 44, true, 1.0, 10),
    ] {
        let mut values = fixture(true, props(), updates("current", "current"));
        values.insert("window".into(), json!({"platform":platform,"titlebar":titlebar,"fullscreen":fullscreen,"buttons":false}));
        values.get_mut("settings").unwrap()["zoom"] = json!(zoom);
        let layout = values.get_mut("layout").unwrap();
        layout["children"][0]["size"]["px"] = json!(width);
        if !lights {
            layout.as_object_mut().unwrap().remove("traffic_lights");
        }
        let tree = describe_with_values(cx, &staged, "mods/tabs", values, None).unwrap();
        let root = tree.lines().next().unwrap();
        assert!(root.contains(&format!(".pl[Number({expected}.0)]")), "{platform}/{titlebar}/{fullscreen}/{width}/{lights}: {tree}");
        assert!(tree.contains("tab-tab1"), "first tab must remain visible: {tree}");
    }
}

#[gpui::test]
fn windows_tab_buttons_use_matching_vector_icons(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for platform in ["windows", "linux"] {
        for maximized in [false, true] {
            let mut values = fixture(true, props(), updates("current", "current"));
            values.insert("window".into(), json!({"platform":platform,"buttons":true,"maximized":maximized}));
            let tree = describe_with_values(cx, &staged, "mods/tabs", values, None).unwrap();
            if platform == "windows" {
                for icon in ["win-min.svg", if maximized { "win-restore.svg" } else { "win-max.svg" }, "win-close.svg"] {
                    assert!(tree.contains(icon), "missing {icon}: {tree}");
                }
            } else {
                assert!(!tree.contains("mods/tabs/win-"), "{tree}");
                assert!(tree.contains("mods/pane/close.svg"), "{tree}");
            }
        }
    }
}

#[gpui::test]
fn font_pickers_contain_scroll_on_the_wrapper(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for which in ["font", "ui_font"] {
        let mut p = props();
        p["section"] = json!("look");
        let tree = describe(
            cx, &staged, "frame/settings", true, p, updates("current", "current"),
            Some(&format!("this.picking = '{which}';")),
        ).unwrap();
        let scroll = tree.lines().find(|line| line.contains(&format!("st-fl-{which}"))).unwrap();
        assert!(scroll.contains("overflow_y_scrollbar"), "{tree}");
        assert!(!scroll.contains("on_scroll_wheel"), "the native scroller must receive the event: {tree}");
        assert_eq!(tree.matches("on_scroll_wheel").count(), 1, "the wrapper must stop scroll chaining: {tree}");
    }
}

#[gpui::test]
fn long_theme_names_stay_inside_their_card(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let long = "catppuccin-macchiato";
    let mut p = props();
    p["section"] = json!("look");
    let mut values = fixture(true, p, updates("current", "current"));
    let theme = values["theme"].clone();
    values.insert("themes".into(), json!([{ "name": long, "light": false, "colors": theme }]));
    let mut settings = values["settings"].clone();
    settings["theme_dark"] = json!(long);
    values.insert("settings".into(), settings);
    let tree = describe_with_values(cx, &staged, "frame/settings", values, None).unwrap();
    let lines: Vec<&str> = tree.lines().collect();
    let card = lines.iter().position(|l| l.contains(&format!("st-t-{long}"))).unwrap();
    let row = lines[card..].iter().find(|l| l.trim_start().starts_with("h_flex") && l.contains("min_w")).unwrap();
    assert!(row.contains(".w_full"), "the name row must fill the card: {row}");
    assert!(row.contains(".min_w[Number(0.0)]"), "the name row must be allowed to shrink: {row}");
    let name = lines[card..].iter().find(|l| l.contains("truncate")).unwrap();
    assert!(name.contains(":tooltip[Str(\"catppuccin macchiato\")]"), "a cut name needs its full text on hover: {name}");
    let check = lines[card..].iter().find(|l| l.contains("check.svg")).unwrap();
    assert!(check.contains(".flex_none"), "the selected check must not shrink: {check}");
}

#[gpui::test]
fn check_materializes_retained_elements_in_a_rendered_view(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for body in [
        "v_flex().h(400).child(v_virtual_list('rows', 0, 28, (i) => 'a', () => { throw new Error('deferred rows ran'); }))",
        "v_flex().h(400).child(v_virtual_list('rows', 1, 28, (i) => 'a', () => { throw new Error('deferred rows ran'); }))",
        "div().w(100).h(50).left(10).transition('left', 200)",
        "div().w(100).h(50).transition('width', 200)",
        "div().w(100).h(50).left(10).spring('left')",
        "div().opacity(0.5).transition('opacity', 200)",
    ] {
        std::fs::write(staged.0.join("main.js"), format!(
            "import {{ View, div }} from 'gpui-kit'; import {{ v_flex, v_virtual_list }} from 'gpui-base'; export default class Demo extends View {{ render() {{ return {body}; }} }}"
        )).unwrap();
        let runtime = ShellRuntime::new_isolated().unwrap();
        let window = cx.add_window(|_, _| Empty);
        let description = window.update(cx, |_, window, cx| runtime.check(&staged.0, window, cx)).unwrap().unwrap();
        assert!(!description.is_empty(), "{body}");
        gpui::AnyWindowHandle::from(window).update(cx, |_, window, _| window.remove_window()).unwrap();
    }
}

#[gpui::test]
fn rebuilt_machine_offers_connect_only_for_identity_mismatch(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (state, why, offered) in [
        ("unreachable", "a different core answers there (\"new-id\")", true),
        ("unreachable", "connection refused", false),
        ("unreachable", "a different core answers there", false),
        ("connected", "a different core answers there (\"new-id\")", false),
    ] {
        let setup = format!(
            "this.machines = [{{name:'cloud', state:{}, why:{}, target:'host'}}]; this.folded.add('cloud');",
            json!(state), json!(why)
        );
        let tree = describe(cx, &staged, "mods/spaces", false, props(), updates("current", "current"), Some(&setup)).unwrap();
        assert_eq!(tree.contains("machine-connect-cloud"), offered, "{state}/{why}: {tree}");
        assert_eq!(tree.contains("rebuilt ·"), offered, "{tree}");
        assert_eq!(tree.contains("This machine was rebuilt or replaced. Click to connect to the new one."), offered, "{tree}");
        assert!(!tree.contains("machine-reconnect-"), "reconnect belongs in the machine row: {tree}");
    }
}

#[gpui::test]
fn machine_picker_keeps_long_lists_scrollable(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe(
        cx, &staged, "mods/machine-add", false, props(), updates("current", "current"),
        Some("this.hosts = Array.from({length: 30}, (_, i) => ({alias: `host-${i}`, aliases: []}));"),
    ).unwrap();
    let scroll = tree.lines().find(|line| line.contains("ma-hosts")).unwrap();
    assert!(scroll.contains("overflow_y_scroll") && !scroll.contains("overflow_y_scrollbar"), "{tree}");
    assert!(scroll.contains(".min_h[Number(0.0)]"), "{tree}");
    assert!(tree.contains("ma-row-host-29"), "{tree}");
    assert_eq!(tree.matches(".flex_none").count(), 60, "rows must not shrink into the viewport: {tree}");
}

#[gpui::test]
fn machine_setup_prompt_stacks_above_the_picker(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new().with_host_module(module(fixture(false, props(), updates("current", "current")), calls.clone())).unwrap();
    let root = staged.wrapper("mods/machine-add", "cx.spawn(async () => { await this.inTerminal('host'); });");
    let window = cx.add_window(|_, _| Empty);
    let mut session = window.update(cx, |_, window, cx| {
        DescriptionSession::load(ShellRuntime::new_isolated().unwrap(), root, policy, window, cx).unwrap()
    }).unwrap();
    cx.run_until_parked();
    window.update(cx, |_, window, cx| {
        session.describe(window, cx).unwrap();
        window.remove_window();
    }).unwrap();
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    let spec = &calls.effects.iter().find(|(name, _)| name == "openOverlay").unwrap().1[0];
    assert_eq!(spec["kind"], "dialog");
    assert_eq!(spec["layout"]["children"][1]["content"]["terminal"], "t-setup");
    assert!(!calls.effects.iter().any(|(name, _)| name == "closeOverlay"), "the picker must keep monitoring setup");
}

#[gpui::test]
fn option_key_settings_are_only_shown_on_macos(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for platform in ["linux", "windows", "macos"] {
        let mut p = props();
        p["section"] = json!("keyboard");
        let mut values = fixture(true, p, updates("current", "current"));
        values.insert("window".into(), json!({"platform":platform}));
        let tree = describe_with_values(cx, &staged, "frame/settings", values, None).unwrap();
        for value in ["auto", "on", "off", "left", "right"] {
            assert_eq!(tree.contains(&format!("st-option-{value}")), platform == "macos", "{platform}: {tree}");
        }
        assert_eq!(tree.contains("st-s-keyboard"), platform == "macos", "{platform}: {tree}");
    }
}

#[gpui::test]
fn settings_sections_and_updates(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let start = Instant::now();
    let staged = Staged::new();
    for section in ["look", "alerts", "mods", "updates"] {
        let mut p = props();
        p["section"] = json!(section);
        let tree = describe(cx, &staged, "frame/settings", true, p, updates("current", "current"), None).unwrap();
        if section == "mods" {
            assert!(tree.contains("No mod has asked yet"), "an empty mods list says so: {tree}");
        }
    }
    for app in ["source", "current", "ready"] {
        for core in ["current", "ready", "error", "managed", "updating"] {
            let mut p = props();
            p["section"] = json!("updates");
            let tree = describe(cx, &staged, "frame/settings", true, p, updates(app, core), None)
                .unwrap_or_else(|e| panic!("updates/{app}/{core}: {e:#}"));
            if app == "current" && core == "current" {
                assert!(
                    tree.contains("check-app") && tree.contains("check-core"),
                    "both update buttons must exist: {tree}"
                );
            }
        }
    }
    for (checking, checked, asked) in [
        (false, None, false), (false, None, true),
        (true, None, false), (true, None, true),
        (true, Some(1700000000u64), false), (true, Some(1700000000), true),
        (false, Some(1700000000), false), (false, Some(1700000000), true),
    ] {
        let mut p = props();
        p["section"] = json!("updates");
        let mut u = updates("current", "current");
        u["checked"] = json!(checked);
        u["checking"] = json!(checking);
        let setup = asked.then_some("this.asked = Date.now();");
        let tree = describe(cx, &staged, "frame/settings", true, p, u, setup).unwrap();
        let succeeded = !checking && checked.is_some();
        assert_eq!(tree.contains("Up to date"), succeeded, "success requires a completed, successful check: {tree}");
        if !checking && checked.is_none() {
            assert!(tree.contains("check-app") && tree.contains("check-core"), "failed checks must remain retryable: {tree}");
        }
        assert!(tree.contains(if checking { "Checking" } else if succeeded { "Up to date" } else { "Check your connection" }), "check status missing: {tree}");
    }
    eprintln!("27 settings cases, each twice: {:?}", start.elapsed());
}

/// The consent dialog (ADR-048) names the mod, its folder and its powers, and
/// offers both answers; a failed save shows why.
#[gpui::test]
fn consent_dialog_names_the_mod_and_both_answers(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let p = json!({"overlay":"o1","id":"my-mod","folder":"/fixture/app/mods/my-mod","hash":"abc"});
    let tree = describe(cx, &staged, "frame/consent", true, p.clone(), updates("current", "current"), None).unwrap();
    for want in ["my-mod", "/fixture/app/mods/my-mod", "run programs, read and write files in your home folder and your spaces, and talk to agents", "consent-yes", "consent-no", "Don't allow"] {
        assert!(tree.contains(want), "consent dialog lacks {want:?}: {tree}");
    }
    let tree = describe(cx, &staged, "frame/consent", true, p, updates("current", "current"), Some("this.error = 'Could not save: denied';")).unwrap();
    assert!(tree.contains("Could not save: denied"), "a failed save shows: {tree}");
}

#[gpui::test]
fn animated_images_advance_with_explicit_and_implicit_ids(cx: &mut TestAppContext) {
    use gpui::{IntoElement as _, http_client::{AsyncBody, FakeHttpClient, Response}};
    use image::{Delay, Frame, Rgba, RgbaImage, codecs::gif::GifEncoder};
    use std::{sync::Arc, time::Duration};

    cx.update(gpui_shell::init);
    let mut bytes = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut bytes);
        for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
            encoder.encode_frame(Frame::from_parts(RgbaImage::from_pixel(2, 2, Rgba(color)), 0, 0, Delay::from_numer_denom_ms(20, 1))).unwrap();
        }
    }
    let client = FakeHttpClient::create(move |_| {
        let bytes = bytes.clone();
        async move { Ok(Response::builder().status(200).body(AsyncBody::from(bytes)).unwrap()) }
    });
    cx.update(|cx| cx.set_http_client(client));
    let staged = Staged::new();
    for explicit in [false, true] {
        let runtime = ShellRuntime::new_isolated().unwrap();
        let url = format!("https://images.example/animation-{explicit}.gif");
        let source = format!("import {{ View, image }} from 'gpui-kit'; export default class Demo extends View {{ render() {{ return image('{url}'){}.size(20); }} }}", if explicit { ".id('clip')" } else { "" });
        std::fs::write(staged.0.join("main.js"), source).unwrap();
        let context = cx.add_empty_window();
        let view = context.update(|window, cx| {
            window.activate_window();
            let root = runtime.try_load(&staged.0, window, cx).unwrap();
            root.read(cx).content().clone()
        });
        let draw = |context: &mut gpui::VisualTestContext| {
            let view = view.clone();
            context.draw(gpui::Point::default(), gpui::size(gpui::px(40.), gpui::px(40.)), move |_, _| view.into_any_element());
        };
        draw(context);
        context.run_until_parked();
        draw(context);
        assert!(context.update(|window, _| window.is_window_active()));
        let resource = gpui::Resource::Uri(url.into());
        let data: Arc<gpui::RenderImage> = context.update(|_, cx| cx.fetch_asset::<gpui::ImgResourceLoader>(&resource).unwrap().unwrap());
        assert_eq!(data.frame_count(), 2);
        assert!(!context.update(|window, _| window.has_image_atlas_entry(&data)));
        // GPUI's image clock is wall time, not the test executor's clock.
        std::thread::sleep(Duration::from_millis(25));
        draw(context);
        assert!(context.update(|window, _| window.has_image_atlas_entry(&data)), "both animation frames must have painted (explicit id: {explicit})");
    }
}

#[gpui::test]
fn settings_keep_notes_in_the_whats_new_window(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let notes = json!([
        {"version":"1.8.0","date":"2026-10-26","items":[{"headline":"Newest headline","detail":"Newest details","fixed":false,"media":[]},{"headline":"","detail":"Newest fix","fixed":true,"media":[]}],"spotlight":true},
        {"version":"1.7.0","date":"2026-10-25","items":[{"headline":"Older headline","detail":"Older details","fixed":false,"media":[]},{"headline":"Another older headline","detail":"Hidden detail","fixed":false,"media":[]}],"spotlight":true}
    ]);
    let mut update = updates("ready", "current");
    update["notes"] = notes.clone();
    let mut p = props();
    p["section"] = json!("updates");
    let tree = describe(cx, &staged, "frame/settings", true, p, update.clone(), None).unwrap();
    assert!(tree.contains("st-whats-new") && tree.contains("st-app-update") && tree.contains("st-check-core"));
    assert!(!tree.contains("Newest details") && !tree.contains("Older headline"));
    let mut p = props();
    p["notes"] = notes;
    let tree = describe(cx, &staged, "frame/whatsnew", true, p, update, None).unwrap();
    assert!(tree.contains("Newest details") && tree.contains("Older details"));
    assert!(tree.contains("Also fixed") && tree.contains("Newest fix"));
    assert!(tree.contains("Oct 26") && !tree.contains("2026-10-26"));
}

#[gpui::test]
fn local_state_and_overlay_initial_descriptions(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let start = Instant::now();
    let staged = Staged::new();
    for page in 1..=4 {
        let tree = describe(
            cx,
            &staged,
            "frame/welcome",
            true,
            props(),
            updates("current", "current"),
            Some(&format!("this.page = {page};")),
        )
        .unwrap();
        let expected = ["HyperOx runs your terminals in the background", "Add your other machines", "Choose your panels", "Make it yours"][page - 1];
        assert!(tree.contains(expected), "welcome page {page}: {tree}");
    }
    for setup in ["", "this.sent = true;", "this.error = 'Fixture error';"] {
        let tree = describe(
            cx,
            &staged,
            "frame/feedback",
            true,
            props(),
            updates("current", "current"),
            Some(setup),
        )
        .unwrap();
        assert!(tree.contains("feedback"), "{tree}");
        if setup.is_empty() {
            assert!(
                tree.contains("Input #") && tree.contains("Textarea #") && tree.contains("A prefilled message"),
                "{tree}"
            );
        } else {
            assert!(
                tree.contains(if setup.contains("sent") { "Your browser opened a new GitHub issue" } else { "Fixture error" }),
                "{tree}"
            );
        }
    }
    for query in ["settings", "no-such-command"] {
        let tree = describe(
            cx,
            &staged,
            "frame/palette",
            true,
            props(),
            updates("current", "current"),
            Some(&format!("this.query = {query:?};")),
        )
        .unwrap();
        // The query sits in a native text field; what the list shows tells it apart.
        let expected = if query == "settings" { "Settings" } else { "Nothing matches" };
        assert!(tree.contains(expected), "palette query {query}: {tree}");
    }
    // Reviewed overlay contract table, not extracted from arbitrary JS call sites.
    let scenarios = [
        ("mods/menu", json!({"tab":"pane","target":{"pane":"p1","terminal":"t1"}})),
        ("mods/menu", json!({"tab":"tab","target":{"id":"tab1","space":"s1"}})),
        ("mods/menu", json!({"tab":"space","target":{"id":"s1"}})),
        ("mods/prompt", json!({"machines":true})),
        (
            "mods/confirm",
            json!({"title":"Close space?","command":{"name":"space.close","args":{"id":"s1"}}}),
        ),
        (
            "mods/toast",
            json!({"title":"Done","body":"Build complete","level":"success","action":{"name":"app.help"}}),
        ),
        ("mods/peek", json!({"terminal":"t1"})),
        ("mods/peek", json!({"terminal":"missing"})),
        ("mods/worktree", json!({"space":"s1","action":"new"})),
        ("mods/worktree", json!({"space":"s1","action":"remove"})),
    ];
    let entries = staged.entries();
    for (entry, patch) in scenarios {
        assert!(entries.iter().any(|e| e == entry), "orphan scenario: {entry}");
        let mut p = props();
        p.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
        describe(cx, &staged, entry, true, p, updates("current", "current"), None).unwrap();
    }
    eprintln!("18 local-state/overlay cases, each twice: {:?}", start.elapsed());
}

#[gpui::test]
fn duplicate_element_negative_control(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let dir = staged.0.join("negative");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("main.js"), "import { View, div } from 'gpui-kit'; export default class Bad extends View { render() { const child = div().child('reused'); return div().child(div().child(child)).child(div().child(child)); } }").unwrap();
    let error = describe(cx, &staged, "negative", false, props(), updates("current", "current"), None).unwrap_err();
    assert!(format!("{error:#}").contains("already added to a parent"), "{error:#}");
}

#[test]
fn shipped_layouts_and_keys() {
    let staged = Staged::new();
    let mods: BTreeSet<_> = staged
        .entries()
        .into_iter()
        .filter_map(|p| p.strip_prefix("mods/").map(str::to_owned))
        .collect();
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("defaults/layouts");
    let mut layouts = BTreeSet::new();
    fn references(v: &Value, mods: &BTreeSet<String>) {
        match v {
            Value::Object(o) => {
                if let Some(Value::String(name)) = o.get("mod") {
                    assert!(mods.contains(name), "unknown layout mod {name}");
                }
                for v in o.values() {
                    references(v, mods);
                }
            }
            Value::Array(a) => {
                for v in a {
                    references(v, mods);
                }
            }
            _ => {}
        }
    }
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        serde_json::from_str::<Node>(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        references(&serde_json::from_str::<Value>(&text).unwrap(), &mods);
        layouts.insert(path.file_stem().unwrap().to_str().unwrap().to_owned());
    }
    assert_eq!(layouts, crate::window::shipped_layouts().into_iter().map(str::to_owned).collect());
    let mod_commands = ["ribbon.next"];
    for text in [
        include_str!("../../defaults/keys.json"),
        include_str!("../../defaults/keys.macos.json"),
    ] {
        crate::keys::parse(text).unwrap();
        let sections: BTreeMap<String, BTreeMap<String, String>> = serde_json::from_str(text).unwrap();
        for bindings in sections.values() {
            for action in bindings.values() {
                let name = action.split(':').next().unwrap();
                if let Some(command) = crate::commands::find(name) {
                    let args = match action.split_once(':') {
                        Some((_, value)) => {
                            assert!(!command.args.is_empty(), "unexpected shipped key argument {action}");
                            crate::commands::shorthand(command, value)
                        }
                        None => Value::Null,
                    };
                    crate::commands::validate_args(command, &args).unwrap();
                    assert!(crate::commands::missing(command, &args).is_none(), "missing shipped key argument {action}");
                }
                assert!(
                    action == "terminal"
                        || action.starts_with("send:")
                        || crate::commands::COMMANDS.iter().any(|c| c.name == name)
                        || (mod_commands.contains(&name) && mods.contains(name.split('.').next().unwrap())),
                    "unknown shipped key action {action}"
                );
            }
        }
    }
}

#[gpui::test]
fn picker_scrolls_the_selection_into_view(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    let tabs: Vec<Value> = (1..=12)
        .map(|i| json!({"id":format!("tab{i}"),"index":i,"name":format!("Task {i}"),"active":i==1,"status":"done","panes":1,"zoomed":false,"settled":false,"settled_at":null,"changed_at":null,"working_since":null}))
        .collect();
    values.insert("spaces".into(), json!([
        {"id":"s1","index":1,"name":"Local","folder":"/fixture/project","machine":null,"branch":"main","active":true,"status":"working","tabs":tabs}
    ]));
    // Selecting the last row (with the arrows) must scroll it into the visible 320px.
    let tree = describe_with_values(cx, &staged, "mods/picker", values, Some("this.at = 999; cx.notify();")).unwrap();
    let top = tree.lines()
        .find(|l| l.contains("pk-space-here"))
        .unwrap_or_else(|| panic!("selected row missing: {tree}"));
    let px: f64 = top.split(".top[Number(").nth(1).unwrap_or_else(|| panic!("selected row must be placed inside the scrolled list: {tree}")).split(")").next().unwrap().parse().unwrap();
    assert!(px <= 320.0 - 32.0, "selected row at {px}px is below the visible list: {tree}");
}

#[gpui::test]
fn status_bar_shows_the_real_palette_key(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut bound = fixture(true, props(), updates("current", "current"));
    bound.insert("keys".into(), json!([{"section":"global","keys":"ctrl-p","action":"app.palette"}]));
    let tree = describe_with_values(cx, &staged, "mods/workspace-status", bound, None).unwrap();
    assert!(tree.contains("Ctrl+P"), "hint must show the real binding: {tree}");
    assert!(!tree.contains("Shift P"), "stale hardcoded hint: {tree}");

    let mut unbound = fixture(true, props(), updates("current", "current"));
    unbound.insert("keys".into(), json!([{"section":"global","keys":"ctrl-k","action":"app.help"}]));
    let tree = describe_with_values(cx, &staged, "mods/workspace-status", unbound, None).unwrap();
    assert!(tree.contains("commands"), "unbound hint still offers commands: {tree}");
    assert!(!tree.contains("Shift P"), "unbound hint must not invent a key: {tree}");
}

// The files panel's editing: the real mod under a wrapper that takes commands from the test,
// with the disk, the writes and the core calls faked and recorded.
#[derive(Default)]
struct FilesDisk {
    files: BTreeMap<String, String>,
    fail: Option<String>,
    send_code: i64,
    reads: Vec<Value>,
    writes: Vec<(String, String, Value)>,
    sends: Vec<Vec<String>>,
    commands: std::collections::VecDeque<String>,
    notes: Vec<String>,
}

const FILES_SCENARIO: &str = r#"
import Real from './mods/files/main.js';
import { command, note } from 'fixture';
export default class Scenario extends Real {
  init(props, cx) {
    super.init(props, cx);
    cx.timer.every(50, cx => {
      for (let c = command(); c !== null; c = command()) {
        const at = c.indexOf(' '), verb = at < 0 ? c : c.slice(0, at), arg = c.slice(at + 1);
        if (verb === 'open') this.show(arg, cx);
        else if (verb === 'back') this.show(null, cx);
        else if (verb === 'keep') this.save(cx, true);
        else if (verb === 'reload') this.reload(cx);
        else if (verb === 'refresh') this.refresh(cx);
        else if (verb === 'state') {
          const d = this.doc;
          note(JSON.stringify({ file: this.file ?? null, value: this.code ? this.code.value() : null, editable: Boolean(d?.editable),
            dirty: Boolean(d?.dirty), conflict: Boolean(d?.conflict), error: d?.error ?? null, hint: d?.hint ?? null }));
        }
        cx.notify();
      }
    });
  }
}
"#;

fn files_disk(files: &[(&str, &str)]) -> Rc<RefCell<FilesDisk>> {
    Rc::new(RefCell::new(FilesDisk {
        files: files.iter().map(|(p, t)| (p.to_string(), t.to_string())).collect(),
        ..Default::default()
    }))
}

fn files_values(agents: Value) -> BTreeMap<String, Value> {
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("agents".into(), agents);
    values
}

fn files_module(values: BTreeMap<String, Value>, disk: Rc<RefCell<FilesDisk>>) -> HostModule {
    let mut module = HostModule::new("h0x");
    for (name, value) in values {
        module = module.function(name, move |_| Ok(to_host(&value)));
    }
    module = module.async_function("watchFolder", |_| Ok(std::future::pending::<gpui_shell::HostResult>()));
    module = module.async_function("exec", |_| Ok(std::future::ready(Ok(to_host(&json!({"code":0,"stdout":"","stderr":""}))))));
    let read = disk.clone();
    module = module.async_function("readTextFile", move |a| {
        let path = a.string(0)?.to_owned();
        let mut disk = read.borrow_mut();
        disk.reads.push(json!([path, a.get(1).map(from_host).unwrap_or(Value::Null)]));
        let text = disk.files.get(&path).cloned();
        Ok(std::future::ready(Ok(to_host(&json!(text)))))
    });
    let write = disk.clone();
    module = module.async_function("writeTextFile", move |a| {
        let path = a.string(0)?.to_owned();
        let text = a.string(1)?.to_owned();
        let expected = a.get(2).map(from_host).unwrap_or(Value::Null);
        let mut disk = write.borrow_mut();
        disk.writes.push((path.clone(), text.clone(), expected.clone()));
        let current = disk.files.get(&path).map_or(Value::Null, |t| json!(t));
        let result = if current != expected {
            Err(HostError::new("stale snapshot: the file changed on disk"))
        } else if let Some(message) = disk.fail.clone() {
            Err(HostError::new(message))
        } else {
            disk.files.insert(path, text);
            Ok(HostValue::Null)
        };
        Ok(std::future::ready(result))
    });
    module.async_function("core", move |a| {
        let args: Vec<String> = serde_json::from_value(a.get(0).map(from_host).unwrap_or(Value::Null)).unwrap();
        let mut disk = disk.borrow_mut();
        let code = disk.send_code;
        disk.sends.push(args);
        Ok(std::future::ready(Ok(to_host(&json!({"code":code,"stdout":"","stderr":""})))))
    })
}

struct FilesMounted(gpui::Entity<gpui_shell::ShellRoot>);
impl gpui::Render for FilesMounted {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        self.0.clone()
    }
}

fn mount_files(
    cx: &mut TestAppContext,
    staged: &Staged,
    values: BTreeMap<String, Value>,
    disk: &Rc<RefCell<FilesDisk>>,
) -> (gpui::VisualTestContext, Rc<ShellRuntime>, gpui::Entity<gpui_shell::ShellRoot>) {
    std::fs::write(staged.0.join("main.js"), FILES_SCENARIO).unwrap();
    let (commands, notes) = (disk.clone(), disk.clone());
    gpui_shell::policy::set_default(
        Policy::new()
            .with_capabilities(Capabilities::new().read_roots([staged.0.clone()]))
            .with_host_module(files_module(values, disk.clone()))
            .unwrap()
            .with_host_module(
                HostModule::new("fixture")
                    .function("command", move |_| {
                        Ok(commands.borrow_mut().commands.pop_front().map_or(HostValue::Null, |c| to_host(&json!(c))))
                    })
                    .function("note", move |a| {
                        notes.borrow_mut().notes.push(a.string(0)?.to_owned());
                        Ok(HostValue::Null)
                    }),
            )
            .unwrap(),
    );
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| FilesMounted(runtime.try_load(&staged.0, window, cx).unwrap()));
    let root = window.update(cx, |mounted, _, _| mounted.0.clone()).unwrap();
    let mut visual = gpui::VisualTestContext::from_window(*window, cx);
    visual.run_until_parked();
    (visual, runtime, root)
}

fn files_wait(visual: &mut gpui::VisualTestContext, ms: u64) {
    visual.executor().advance_clock(std::time::Duration::from_millis(ms));
    visual.run_until_parked();
}

// Hands the scenario one command; its poll timer runs it within the 50 ms advanced here.
fn files_step(visual: &mut gpui::VisualTestContext, disk: &Rc<RefCell<FilesDisk>>, command: &str) {
    disk.borrow_mut().commands.push_back(command.into());
    files_wait(visual, 50);
}

fn files_state(visual: &mut gpui::VisualTestContext, disk: &Rc<RefCell<FilesDisk>>) -> Value {
    files_step(visual, disk, "state");
    serde_json::from_str(disk.borrow().notes.last().expect("the scenario reports its state")).unwrap()
}

// Focuses the open editor with a click into its body (below the header), then types.
fn files_type(visual: &mut gpui::VisualTestContext, text: &str) {
    visual.update(|window, cx| {
        window.activate_window();
        window.draw(cx).clear(cx)
    });
    visual.simulate_click(gpui::point(gpui::px(300.), gpui::px(300.)), gpui::Modifiers::none());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_input(text);
}

fn files_keys(visual: &mut gpui::VisualTestContext, keys: &str) {
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_keystrokes(keys);
}

fn files_writes(disk: &Rc<RefCell<FilesDisk>>) -> Vec<(String, String, Value)> {
    disk.borrow().writes.clone()
}

fn files_sends(disk: &Rc<RefCell<FilesDisk>>) -> Vec<Vec<String>> {
    disk.borrow().sends.iter().filter(|a| a.first().map(String::as_str) == Some("send")).cloned().collect()
}

fn files_note(list: &str) -> String {
    format!("HyperOx: the user edited {list} in the editor. Re-read before changing it.")
}

fn idle_agent() -> Value {
    json!([{"address":"idle1","space":"s1","status":"idle"}])
}

fn describe_files(
    cx: &mut TestAppContext,
    staged: &Staged,
    values: BTreeMap<String, Value>,
    disk: &Rc<RefCell<FilesDisk>>,
    setup: &str,
) -> String {
    let policy = Policy::new()
        .with_capabilities(Capabilities::new().read_roots([staged.0.clone()]))
        .with_host_module(files_module(values, disk.clone()))
        .unwrap();
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("mods/files", setup);
    let window = cx.add_window(|_, _| Empty);
    let mut session = window
        .update(cx, |_, window, cx| DescriptionSession::load(runtime, root, policy, window, cx))
        .unwrap()
        .unwrap();
    cx.run_until_parked();
    let tree = window.update(cx, |_, window, cx| session.describe(window, cx)).unwrap().unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    tree
}

#[gpui::test]
fn files_panel_marks_remote_and_oversized_files_read_only_and_shows_unsaved_state(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let notes = "/fixture/project/notes.txt";

    let mut remote = files_values(idle_agent());
    remote.get_mut("spaces").unwrap()[0]["machine"] = json!("remote");
    let disk = files_disk(&[(notes, "hello\n")]);
    let tree = describe_files(cx, &staged, remote, &disk, "this.show('notes.txt', cx);");
    assert!(tree.contains("read-only: remote machine"), "a remote file is a read-only preview: {tree}");
    assert!(disk.borrow().reads.contains(&json!([notes, {"on":"remote"}])), "{:?}", disk.borrow().reads);

    // 400,000 characters is the largest file still edited in place.
    for (size, readonly) in [(400_000, false), (400_001, true)] {
        let disk = files_disk(&[("/fixture/project/big.txt", &"a".repeat(size))]);
        let tree = describe_files(cx, &staged, files_values(idle_agent()), &disk, "this.show('big.txt', cx);");
        assert_eq!(tree.contains("read-only: too large"), readonly, "{size} characters");
        assert!(!tree.contains("This file is no longer available") && !tree.contains("Cannot preview"), "{size} characters");
    }

    // An empty file is still a file: editable, not "no longer available".
    let disk = files_disk(&[(notes, "")]);
    let tree = describe_files(cx, &staged, files_values(idle_agent()), &disk, "this.show('notes.txt', cx);");
    assert!(
        !tree.contains("read-only") && !tree.contains("This file is no longer available") && !tree.contains("Loading file"),
        "{tree}"
    );

    let disk = files_disk(&[(notes, "hello\n")]);
    let tree = describe_files(cx, &staged, files_values(idle_agent()), &disk, "this.show('notes.txt', cx);");
    assert!(
        tree.contains("notes.txt") && !tree.contains("● ") && !tree.contains("Changed on disk") && !tree.contains("read-only"),
        "{tree}"
    );

    // Edits held back by a conflict come back with the file: marked unsaved, with the bar and the error.
    let disk = files_disk(&[(notes, "theirs\n")]);
    let tree = describe_files(
        cx,
        &staged,
        files_values(idle_agent()),
        &disk,
        "this.drafts.set('/fixture/project/notes.txt', { saved: 'old\\n', draft: 'mine\\n', error: 'Not saved: disk full' }); this.show('notes.txt', cx);",
    );
    for expected in ["● notes.txt", "Changed on disk", "files-reload", "Reload", "files-keep", "Keep mine", "Not saved: disk full"] {
        assert!(tree.contains(expected), "missing {expected}: {tree}");
    }
    assert!(disk.borrow().writes.is_empty(), "showing a file writes nothing");
}

#[gpui::test]
fn files_panel_auto_saves_once_a_second_after_the_last_change(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let notes = "/fixture/project/notes.txt";
    let disk = files_disk(&[(notes, "one\ntwo\n")]);
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, files_values(json!([])), &disk);
    files_step(&mut visual, &disk, "open notes.txt");
    let state = files_state(&mut visual, &disk);
    assert_eq!((state["editable"].clone(), state["dirty"].clone(), state["hint"].clone()), (json!(true), json!(false), Value::Null), "{state}");

    files_type(&mut visual, "q");
    let state = files_state(&mut visual, &disk);
    assert_eq!(state["dirty"], json!(true), "an edit marks the file unsaved: {state}");
    files_wait(&mut visual, 550);
    files_type(&mut visual, "z");
    // 1,550 ms after the first change, 950 ms after the second.
    files_wait(&mut visual, 950);
    assert!(files_writes(&disk).is_empty(), "a change within the second restarts the wait: {:?}", files_writes(&disk));
    files_wait(&mut visual, 100);
    let writes = files_writes(&disk);
    assert_eq!(writes.len(), 1, "two changes, one write: {writes:?}");
    assert_eq!(writes[0].0, notes);
    assert_eq!(writes[0].1.replace(['q', 'z'], ""), "one\ntwo\n", "{writes:?}");
    assert_eq!(writes[0].2, json!("one\ntwo\n"), "written over what was read");
    assert_eq!(files_state(&mut visual, &disk)["dirty"], json!(false));
    files_wait(&mut visual, 2000);
    assert_eq!(files_writes(&disk).len(), 1, "a saved buffer is not written again");
}

#[gpui::test]
fn files_panel_saves_at_once_on_ctrl_s_switch_back_blur_and_close(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (a, b) = ("/fixture/project/a.txt", "/fixture/project/b.txt");
    let disk = files_disk(&[(a, "alpha\n"), (b, "beta\n")]);
    let (mut visual, runtime, root) = mount_files(cx, &staged, files_values(json!([])), &disk);
    files_step(&mut visual, &disk, "open a.txt");

    files_type(&mut visual, "q");
    files_keys(&mut visual, "ctrl-shift-s");
    assert!(files_writes(&disk).is_empty(), "ctrl-shift-s is not save");
    files_keys(&mut visual, "ctrl-s");
    assert_eq!(files_writes(&disk).len(), 1, "ctrl-s saves before the second is up");

    files_type(&mut visual, "z");
    files_step(&mut visual, &disk, "open b.txt");
    let writes = files_writes(&disk);
    assert_eq!(writes.len(), 2, "switching files saves the one left: {writes:?}");
    assert_eq!(writes[1].0, a);
    assert_eq!(writes[1].1.replace(['q', 'z'], ""), "alpha\n");

    files_type(&mut visual, "q");
    files_step(&mut visual, &disk, "back");
    let writes = files_writes(&disk);
    assert_eq!(writes.len(), 3, "back saves: {writes:?}");
    assert_eq!(writes[2].0, b);

    files_step(&mut visual, &disk, "open b.txt");
    files_type(&mut visual, "z");
    visual.update(|window, cx| {
        window.blur(cx);
        window.draw(cx).clear(cx)
    });
    visual.run_until_parked();
    let writes = files_writes(&disk);
    assert_eq!(writes.len(), 4, "leaving the editor saves: {writes:?}");
    assert_eq!(writes[3].1.replace(['q', 'z'], ""), "beta\n");

    files_type(&mut visual, "k");
    visual.update(|window, cx| runtime.close(&root, Instant::now() + std::time::Duration::from_secs(5), window, cx).detach());
    visual.run_until_parked();
    files_wait(&mut visual, 20);
    let writes = files_writes(&disk);
    assert_eq!(writes.len(), 5, "closing saves what is pending: {writes:?}");
    assert!(writes[4].1.contains('k') && writes[4].1.replace(['q', 'z', 'k'], "") == "beta\n", "{writes:?}");
}

#[gpui::test]
fn files_panel_writes_back_the_line_endings_it_read(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let cases = [("crlf.txt", "one\r\ntwo\r\n"), ("lf.txt", "one\ntwo\n"), ("mixed.txt", "one\r\ntwo\nthree"), ("empty.txt", "")];
    let files: Vec<(String, &str)> = cases.iter().map(|(n, t)| (format!("/fixture/project/{n}"), *t)).collect();
    let disk = files_disk(&files.iter().map(|(p, t)| (p.as_str(), *t)).collect::<Vec<_>>());
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, files_values(json!([])), &disk);
    for (i, (name, original)) in cases.iter().enumerate() {
        files_step(&mut visual, &disk, &format!("open {name}"));
        files_type(&mut visual, "q");
        files_keys(&mut visual, "ctrl-s");
        let writes = files_writes(&disk);
        assert_eq!(writes.len(), i + 1, "{name}: {writes:?}");
        let (path, text, expected) = &writes[i];
        assert_eq!(path, &format!("/fixture/project/{name}"));
        assert_eq!(text.replace('q', ""), *original, "{name}: the line endings and the final newline are kept");
        // An existing empty file is written over "", never created over null.
        assert_eq!(expected, &json!(original), "{name}");
    }
    let crlf = &files_writes(&disk)[0].1;
    assert_eq!(crlf.matches("\r\n").count(), crlf.matches('\n').count(), "no bare LF in a CRLF file: {crlf:?}");
}

#[gpui::test]
fn files_panel_follows_the_disk_until_edited_then_asks(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let notes = "/fixture/project/notes.txt";
    let disk = files_disk(&[(notes, "v1\n")]);
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, files_values(idle_agent()), &disk);
    files_step(&mut visual, &disk, "open notes.txt");

    disk.borrow_mut().files.insert(notes.into(), "v2\n".into());
    files_step(&mut visual, &disk, "refresh");
    let state = files_state(&mut visual, &disk);
    assert_eq!(
        (state["value"].clone(), state["dirty"].clone(), state["conflict"].clone()),
        (json!("v2\n"), json!(false), json!(false)),
        "an unedited file reloads silently: {state}"
    );

    files_type(&mut visual, "q");
    disk.borrow_mut().files.insert(notes.into(), "v3\n".into());
    files_step(&mut visual, &disk, "refresh");
    let state = files_state(&mut visual, &disk);
    assert_eq!((state["dirty"].clone(), state["conflict"].clone()), (json!(true), json!(true)), "{state}");
    assert!(state["value"].as_str().unwrap().contains('q'), "the edits stay: {state}");
    files_wait(&mut visual, 2000);
    assert!(files_writes(&disk).is_empty(), "a conflict pauses auto-save: {:?}", files_writes(&disk));

    files_step(&mut visual, &disk, "reload");
    files_wait(&mut visual, 50);
    let state = files_state(&mut visual, &disk);
    assert_eq!(
        (state["value"].clone(), state["dirty"].clone(), state["conflict"].clone()),
        (json!("v3\n"), json!(false), json!(false)),
        "reload takes the disk text: {state}"
    );
    assert!(files_writes(&disk).is_empty() && files_sends(&disk).is_empty());
}

#[gpui::test]
fn files_panel_stale_write_asks_and_keep_mine_overwrites(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let notes = "/fixture/project/notes.txt";
    let disk = files_disk(&[(notes, "v1\n")]);
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, files_values(idle_agent()), &disk);
    files_step(&mut visual, &disk, "open notes.txt");

    files_type(&mut visual, "q");
    disk.borrow_mut().files.insert(notes.into(), "theirs\n".into());
    files_keys(&mut visual, "ctrl-s");
    let state = files_state(&mut visual, &disk);
    assert_eq!(
        (state["dirty"].clone(), state["conflict"].clone(), state["error"].clone()),
        (json!(true), json!(true), Value::Null),
        "{state}"
    );
    assert_eq!(disk.borrow().files[notes], "theirs\n", "the other writer's text is not overwritten");
    assert!(files_sends(&disk).is_empty(), "a conflicted save tells no agent");
    files_type(&mut visual, "z");
    files_wait(&mut visual, 2000);
    assert_eq!(files_writes(&disk).len(), 1, "no auto-save while the conflict is open: {:?}", files_writes(&disk));

    files_step(&mut visual, &disk, "keep");
    let writes = files_writes(&disk);
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert_eq!(writes[1].2, json!("theirs\n"), "keep mine writes over the changed disk text");
    assert_eq!(disk.borrow().files[notes].replace(['q', 'z'], ""), "v1\n");
    let state = files_state(&mut visual, &disk);
    assert_eq!((state["dirty"].clone(), state["conflict"].clone()), (json!(false), json!(false)), "{state}");
    assert_eq!(files_sends(&disk).len(), 1, "the kept save is announced: {:?}", files_sends(&disk));
}

#[gpui::test]
fn files_panel_keeps_edits_when_a_write_fails(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let notes = "/fixture/project/notes.txt";
    let disk = files_disk(&[(notes, "v1\n")]);
    disk.borrow_mut().fail = Some("disk full".into());
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, files_values(idle_agent()), &disk);
    files_step(&mut visual, &disk, "open notes.txt");

    files_type(&mut visual, "q");
    files_keys(&mut visual, "ctrl-s");
    let state = files_state(&mut visual, &disk);
    let error = state["error"].as_str().unwrap_or_default();
    assert!(error.starts_with("Not saved: ") && error.contains("disk full"), "{state}");
    assert_eq!((state["dirty"].clone(), state["conflict"].clone()), (json!(true), json!(false)), "{state}");
    assert!(state["value"].as_str().unwrap().contains('q'), "the edits stay in the buffer: {state}");
    assert_eq!(disk.borrow().files[notes], "v1\n");
    assert!(files_sends(&disk).is_empty(), "a failed save tells no agent: {:?}", files_sends(&disk));

    disk.borrow_mut().fail = None;
    files_keys(&mut visual, "ctrl-s");
    let state = files_state(&mut visual, &disk);
    assert_eq!((state["dirty"].clone(), state["error"].clone()), (json!(false), Value::Null), "{state}");
    assert_eq!(disk.borrow().files[notes].replace('q', ""), "v1\n");
    assert_eq!(files_sends(&disk).len(), 1);
}

#[gpui::test]
fn files_panel_leaves_remote_files_unwritten(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let notes = "/fixture/project/notes.txt";
    let disk = files_disk(&[(notes, "remote text\n")]);
    let mut values = files_values(idle_agent());
    values.get_mut("spaces").unwrap()[0]["machine"] = json!("remote");
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, values, &disk);
    files_step(&mut visual, &disk, "open notes.txt");
    files_type(&mut visual, "q");
    files_keys(&mut visual, "ctrl-s");
    let state = files_state(&mut visual, &disk);
    assert_eq!(
        (state["editable"].clone(), state["hint"].clone(), state["value"].clone()),
        (json!(false), json!("read-only: remote machine"), json!("remote text\n")),
        "{state}"
    );
    files_step(&mut visual, &disk, "back");
    files_wait(&mut visual, 2000);
    assert!(files_writes(&disk).is_empty() && files_sends(&disk).is_empty(), "{:?}", files_writes(&disk));
    assert!(disk.borrow().reads.iter().all(|r| r[1] == json!({"on":"remote"})), "{:?}", disk.borrow().reads);
}

#[gpui::test]
fn files_panel_tells_idle_agents_in_the_space_once_per_burst(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let disk = files_disk(&[("/fixture/project/notes.txt", "v1\n")]);
    let agents = json!([
        {"address":"idle1","space":"s1","status":"idle"},
        {"address":"done1","space":"s1","status":"done"},
        {"address":"busy","space":"s1","status":"working"},
        {"address":"asking","space":"s1","status":"blocked"},
        {"address":"elsewhere","space":"s2","status":"idle"}
    ]);
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, files_values(agents), &disk);
    files_step(&mut visual, &disk, "open notes.txt");
    for key in ["q", "z", "k"] {
        files_type(&mut visual, key);
        files_keys(&mut visual, "ctrl-s");
    }
    assert_eq!(files_writes(&disk).len(), 3);
    let mut sends = files_sends(&disk);
    sends.sort();
    let note = files_note("notes.txt");
    assert_eq!(
        sends,
        vec![
            vec!["send".to_string(), "done1".into(), note.clone(), "--if".into(), "idle".into()],
            vec!["send".to_string(), "idle1".into(), note, "--if".into(), "idle".into()],
        ],
        "three saves within the burst: one note per idle agent of the space"
    );
}

#[gpui::test]
fn files_panel_lists_five_files_then_the_rest_and_retries_until_heard(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let files: Vec<String> = (1..=7).map(|i| format!("/fixture/project/f{i}.txt")).collect();
    let disk = files_disk(&files.iter().map(|p| (p.as_str(), "data\n")).collect::<Vec<_>>());
    disk.borrow_mut().send_code = 1;
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, files_values(idle_agent()), &disk);
    for i in 1..=7 {
        files_step(&mut visual, &disk, &format!("open f{i}.txt"));
        files_type(&mut visual, "q");
    }
    assert_eq!(files_writes(&disk).len(), 6, "each switch saved the file left");
    let failed = files_sends(&disk);
    assert_eq!(failed.len(), 6, "an unheard note is offered again with the next save: {failed:?}");
    assert_eq!(failed[1][2], files_note("f1.txt, f2.txt"));

    disk.borrow_mut().send_code = 0;
    files_step(&mut visual, &disk, "back");
    let sends = files_sends(&disk);
    assert_eq!(files_writes(&disk).len(), 7);
    assert_eq!(sends.len(), 7, "{sends:?}");
    assert_eq!(sends[6][2], files_note("f1.txt, f2.txt, f3.txt, f4.txt, f5.txt and 2 more"));

    files_step(&mut visual, &disk, "refresh");
    assert_eq!(files_sends(&disk).len(), 7, "a heard note is not repeated");
}

#[gpui::test]
fn files_panel_keeps_windows_paths_with_spaces_unchanged(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let folder = r"C:\Users\Me\My Project";
    let full = format!("{folder}/notes file.txt");
    let disk = files_disk(&[(full.as_str(), "v1\r\n")]);
    let mut values = files_values(idle_agent());
    values.insert(
        "window".into(),
        json!({"platform":"windows","titlebar":"none","buttons":true,"maximized":false,"fullscreen":false,"active":true}),
    );
    values.get_mut("spaces").unwrap()[0]["folder"] = json!(folder);
    values.get_mut("panes").unwrap()[0]["cwd"] = json!(folder);
    values.get_mut("focused").unwrap()["cwd"] = json!(folder);
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, values, &disk);
    files_step(&mut visual, &disk, "open notes file.txt");
    files_type(&mut visual, "q");
    files_keys(&mut visual, "ctrl-s");
    assert_eq!(disk.borrow().reads[0], json!([&full, {"on":null}]));
    let writes = files_writes(&disk);
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].0, full);
    assert_eq!(writes[0].1.replace('q', ""), "v1\r\n");
    assert_eq!(
        files_sends(&disk),
        vec![vec!["send".to_string(), "idle1".into(), files_note("notes file.txt"), "--if".into(), "idle".into()]],
        "the path travels inside one argv element"
    );
}

#[gpui::test]
fn files_panel_note_drops_control_characters_and_caps_each_name(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // A newline in a file name would submit a fake prompt line to the agent.
    let name = format!("a\r\nb\u{1b}c\u{7f}\u{85}\t{}.txt", "x".repeat(250));
    let full = format!("/fixture/project/{name}");
    let disk = files_disk(&[(full.as_str(), "data\n")]);
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, files_values(idle_agent()), &disk);
    files_step(&mut visual, &disk, &format!("open {name}"));
    files_type(&mut visual, "q");
    files_keys(&mut visual, "ctrl-s");
    assert_eq!(files_writes(&disk).len(), 1, "the file itself is saved under its real name");
    assert_eq!(files_writes(&disk)[0].0, full);
    let clean = format!("abc{}", "x".repeat(197));
    assert_eq!(
        files_sends(&disk),
        vec![vec!["send".to_string(), "idle1".into(), files_note(&clean), "--if".into(), "idle".into()]],
        "control characters are dropped and the name is capped at 200 characters"
    );
}

#[gpui::test]
fn files_panel_note_caps_names_at_200_code_points_not_utf16_units(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // Each emoji is one code point but two UTF-16 units.
    let exact = format!("{}.txt", "\u{1f600}".repeat(196));
    let over = format!("{}.txt", "\u{1f600}".repeat(197));
    assert_eq!((exact.chars().count(), over.chars().count()), (200, 201));
    let paths = [format!("/fixture/project/{exact}"), format!("/fixture/project/{over}")];
    let disk = files_disk(&paths.iter().map(|p| (p.as_str(), "data\n")).collect::<Vec<_>>());
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, files_values(idle_agent()), &disk);
    files_step(&mut visual, &disk, &format!("open {exact}"));
    files_type(&mut visual, "q");
    files_step(&mut visual, &disk, &format!("open {over}"));
    files_type(&mut visual, "q");
    files_keys(&mut visual, "ctrl-s");
    assert_eq!(files_writes(&disk).len(), 2);
    let cut = format!("{}.tx", "\u{1f600}".repeat(197));
    let notes: Vec<String> = files_sends(&disk).into_iter().map(|send| send[2].clone()).collect();
    assert_eq!(notes, [files_note(&exact), files_note(&cut)], "200 code points stay whole, 201 lose the last one");
}

#[gpui::test]
fn files_panel_note_for_a_name_made_only_of_control_characters_carries_none(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let name = "\u{1}\r\n\u{1b}\u{7f}\u{9f}";
    let full = format!("/fixture/project/{name}");
    let disk = files_disk(&[(full.as_str(), "data\n")]);
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, files_values(idle_agent()), &disk);
    files_step(&mut visual, &disk, &format!("open {name}"));
    files_type(&mut visual, "q");
    files_keys(&mut visual, "ctrl-s");
    assert_eq!(files_writes(&disk).len(), 1);
    assert_eq!(files_writes(&disk)[0].0, full);
    let sends = files_sends(&disk);
    assert_eq!(sends.len(), 1, "{sends:?}");
    assert!(!sends[0][2].chars().any(char::is_control), "{:?}", sends[0][2]);
    assert_eq!(sends[0][2], files_note("a file"), "a name that sanitises to nothing is not listed");
}

#[gpui::test]
fn files_panel_note_drops_invisible_and_bidi_format_characters(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // U+202E would show "a.txt.exe" reversed; U+2028 is a line break to some readers.
    let name = "a\u{202E}gpj\u{200B}\u{200F}\u{2028}\u{2029}\u{202A}\u{2060}\u{2064}\u{2066}\u{2069}\u{FEFF}.exe.txt";
    let full = format!("/fixture/project/{name}");
    let disk = files_disk(&[(full.as_str(), "data\n")]);
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, files_values(idle_agent()), &disk);
    files_step(&mut visual, &disk, &format!("open {name}"));
    files_type(&mut visual, "q");
    files_keys(&mut visual, "ctrl-s");
    assert_eq!(files_writes(&disk).len(), 1);
    assert_eq!(files_writes(&disk)[0].0, full, "the file itself is saved under its real name");
    assert_eq!(
        files_sends(&disk),
        vec![vec!["send".to_string(), "idle1".into(), files_note("agpj.exe.txt"), "--if".into(), "idle".into()]],
    );
}

#[gpui::test]
fn files_panel_note_leaves_out_a_name_that_sanitises_to_nothing(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let empty = "\u{202E}\u{200B}\u{FEFF}";
    let paths = [format!("/fixture/project/{empty}"), "/fixture/project/b.txt".to_owned()];
    let disk = files_disk(&paths.iter().map(|p| (p.as_str(), "data\n")).collect::<Vec<_>>());
    disk.borrow_mut().send_code = 1;
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, files_values(idle_agent()), &disk);
    files_step(&mut visual, &disk, &format!("open {empty}"));
    files_type(&mut visual, "q");
    files_step(&mut visual, &disk, "open b.txt");
    files_type(&mut visual, "q");
    let failed = files_sends(&disk);
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0][2], files_note("a file"));
    disk.borrow_mut().send_code = 0;
    files_step(&mut visual, &disk, "back");
    let sends = files_sends(&disk);
    assert_eq!(files_writes(&disk).len(), 2);
    assert_eq!(sends.len(), 2, "{sends:?}");
    assert_eq!(sends[1][2], files_note("b.txt"), "the unheard edit with no name is not listed");
}

#[derive(Default)]
struct SessionsHost {
    // `h0x-core session <verb>` answers, by verb; anything else fails with code 1.
    replies: BTreeMap<String, Value>,
    cores: Vec<Vec<String>>,
    execs: Vec<Value>,
    effects: Vec<(String, Vec<Value>)>,
    shared: BTreeMap<String, Value>,
}

fn sessions_host(replies: &[(&str, Value)], shared: &[(&str, Value)]) -> Rc<RefCell<SessionsHost>> {
    Rc::new(RefCell::new(SessionsHost {
        replies: replies.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
        shared: shared.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
        ..Default::default()
    }))
}

fn core_ok(stdout: Value) -> Value {
    json!({"code":0, "stdout":stdout.to_string(), "stderr":""})
}

fn core_error(kind: &str) -> Value {
    json!({"code":1, "stdout":"", "stderr":json!({"error":{"kind":kind, "message":format!("fixture {kind}"), "hint":""}}).to_string()})
}

// One active space on `machine` (null: this computer) in `folder`.
fn sessions_values(machine: Value, folder: &str, platform: &str) -> BTreeMap<String, Value> {
    let mut values = fixture(false, props(), updates("current", "current"));
    values.insert("spaces".into(), json!([{"id":"s1","index":1,"name":"Here","named":true,"folder":folder,"machine":machine,"branch":null,"active":true,"status":null,"tabs":[]}]));
    values.insert("window".into(), json!({"platform":platform,"titlebar":"none","buttons":true,"maximized":false,"fullscreen":false,"active":true}));
    values
}

fn sessions_module(values: BTreeMap<String, Value>, host: Rc<RefCell<SessionsHost>>) -> HostModule {
    let mut module = HostModule::new("h0x");
    for (name, value) in values {
        module = module.function(name, move |_| Ok(to_host(&value)));
    }
    let h = host.clone();
    module = module.function("shared", move |a| Ok(h.borrow().shared.get(a.string(0)?).map(to_host).unwrap_or(HostValue::Null)));
    let h = host.clone();
    module = module.function("setShared", move |a| {
        h.borrow_mut().shared.insert(a.string(0)?.into(), from_host(a.value(1)?));
        Ok(HostValue::Null)
    });
    for name in ["run", "openOverlay", "closeOverlay", "notify"] {
        let h = host.clone();
        module = module.function(name, move |a| {
            let args: Vec<Value> = (0..2).filter_map(|i| a.get(i).map(from_host)).collect();
            if name == "run" {
                let command = crate::commands::find(a.string(0)?).unwrap_or_else(|| panic!("unknown command {args:?}"));
                crate::commands::validate_args(command, args.get(1).unwrap_or(&Value::Null)).unwrap();
            }
            h.borrow_mut().effects.push((name.into(), args));
            Ok(if name == "openOverlay" || name == "notify" { "fixture-overlay".into() } else { HostValue::Null })
        });
    }
    module = module.async_function("knownAgents", |_| Ok(async { Ok(HostValue::Array(vec![])) }));
    for name in ["nextCommand", "watchFolder"] {
        module = module.async_function(name, |_| Ok(std::future::pending::<gpui_shell::HostResult>()));
    }
    let h = host.clone();
    module = module.async_function("exec", move |a| {
        h.borrow_mut().execs.push(json!([a.get(0).map(from_host), a.get(1).map(from_host)]));
        Ok(std::future::ready(Ok(to_host(&json!({"code":1,"stdout":"","stderr":""})))))
    });
    module.async_function("core", move |a| {
        let args: Vec<String> = serde_json::from_value(a.get(0).map(from_host).unwrap_or(Value::Null)).unwrap();
        let mut h = host.borrow_mut();
        let verb = if args.first().map(String::as_str) == Some("session") { args.get(1).cloned() } else { None };
        let reply = verb.and_then(|v| h.replies.get(&v).cloned()).unwrap_or_else(|| json!({"code":1,"stdout":"","stderr":"fixture: no reply"}));
        h.cores.push(args);
        Ok(std::future::ready(Ok(to_host(&reply))))
    })
}

// Loads `entry` (with `setup` run after its init), lets its tasks finish, then describes it.
fn describe_sessions(
    cx: &mut TestAppContext,
    staged: &Staged,
    entry: &str,
    values: BTreeMap<String, Value>,
    host: &Rc<RefCell<SessionsHost>>,
    setup: &str,
) -> String {
    static NEXT_STORE: AtomicU64 = AtomicU64::new(0);
    let policy = Policy::new()
        .with_storage_path(staged.0.join(format!("sessions-store-{}.json", NEXT_STORE.fetch_add(1, Ordering::Relaxed))))
        .with_capabilities(Capabilities::new().storage(true).clipboard_write(true).read_roots([staged.0.clone()]))
        .with_host_module(sessions_module(values, host.clone()))
        .unwrap();
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper(entry, setup);
    let window = cx.add_window(|_, _| Empty);
    let mut session = window
        .update(cx, |_, window, cx| DescriptionSession::load(runtime, root, policy, window, cx))
        .unwrap()
        .unwrap();
    cx.run_until_parked();
    let tree = window.update(cx, |_, window, cx| session.describe(window, cx)).unwrap().unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    tree
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

// A session as the panel holds it, for scenarios that act on one directly.
fn session_target(on: Option<&str>, folder: &str, agent: &str, id: &str, path: &str) -> String {
    let mark = json!([on, folder, agent, id]).to_string();
    format!(
        "this.target = {{ agent: {}, id: {}, path: {}, title: 'Fixture', mark: {}, mine: {{}} }}; this.space = {{ id: 's1', on: {}, folder: {} }};",
        json!(agent), json!(id), json!(path), json!(mark), json!(on), json!(folder)
    )
}

const MINUTE: u64 = 60_000;

#[gpui::test]
fn sessions_panel_puts_pinned_first_hides_hidden_and_counts_what_it_shows(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let now = now_ms();
    let long = "Refactor the whole renderer ".repeat(20);
    let list = json!({"sessions":[
        {"agent":"claude","id":"a1","title":"Fix the build","last_used":now - 2 * 60 * MINUTE - MINUTE,"path":"/fixture/.claude/projects/p/a1.jsonl"},
        {"agent":"claude","id":"a2","title":"","last_used":now - 25 * 60 * MINUTE,"path":"/fixture/.claude/projects/p/a2.jsonl"},
        {"agent":"codex","id":"c1","title":null,"last_used":now - 3 * 24 * 60 * MINUTE - MINUTE,"path":"/fixture/.codex/sessions/c1.jsonl"},
        {"agent":"opencode","id":"ses_1","title":long,"last_used":now - 4 * 24 * 60 * MINUTE,"path":"/fixture/opencode.db"}
    ]});
    let mark = |agent: &str, id: &str| json!([null, "/fixture/project", agent, id]).to_string();
    let marks = json!({ mark("codex", "c1"): {"pinned":true, "name":"My codex"}, mark("opencode", "ses_1"): {"hidden":true} });
    let host = sessions_host(&[("list", core_ok(list))], &[("sessions", marks.clone())]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "");
    let h = host.borrow();
    assert_eq!(h.cores, vec![vec!["session", "list", "--json", "--", "/fixture/project"]]);
    let (pinned, first, untitled) = (tree.find("My codex"), tree.find("Fix the build"), tree.find("claude session"));
    assert!(pinned.is_some() && first.is_some() && untitled.is_some(), "{tree}");
    assert!(pinned < first && first < untitled, "pinned first, then newest first: {tree}");
    assert!(!tree.contains("Refactor the whole renderer"), "hidden sessions are not shown: {tree}");
    assert!(tree.contains("2h") && tree.contains("1d") && tree.contains("3d"), "{tree}");
    assert!(tree.contains("session-more-2") && !tree.contains("session-more-3"), "{tree}");
    assert_eq!(h.shared["sessions.count"], json!({ json!([null, "/fixture/project"]).to_string(): 3 }));
    assert_eq!(h.shared["sessions"], marks, "showing never writes the marks");
}

#[gpui::test]
fn sessions_panel_with_an_older_core_asks_once_and_says_to_update(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let plain = json!({"code":2, "stdout":"", "stderr":"error: unrecognized subcommand 'session'\n"});
    for reply in [core_error("usage"), plain] {
        let host = sessions_host(&[("list", reply)], &[]);
        let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "");
        assert!(tree.contains("Update h0x-core to see sessions"), "{tree}");
        assert!(!tree.contains("fixture usage") && !tree.contains("unrecognized"), "no raw error: {tree}");
        let h = host.borrow();
        assert_eq!(h.cores.len(), 1, "asked once, not in a loop: {:?}", h.cores);
        assert!(!h.shared.contains_key("sessions.count"), "no count without a list");
        assert!(h.effects.is_empty(), "no toasts: {:?}", h.effects);
    }
}

#[gpui::test]
fn sessions_panel_shows_an_empty_folder_and_a_failed_list(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = sessions_host(&[("list", core_ok(json!({"sessions":[]})))], &[]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/empty", "linux"), &host, "");
    assert!(tree.contains("No agent sessions in this folder yet"), "{tree}");
    assert_eq!(host.borrow().shared["sessions.count"], json!({ json!([null, "/fixture/empty"]).to_string(): 0 }));

    let host = sessions_host(&[("list", core_error("unreachable"))], &[]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(json!("cloud"), "/srv/app", "linux"), &host, "");
    assert!(tree.contains("fixture unreachable") && !tree.contains("Update h0x-core"), "{tree}");
    assert_eq!(host.borrow().cores.len(), 1);

    // Output that isn't JSON is an error, not a crash or an empty list.
    let host = sessions_host(&[("list", json!({"code":0,"stdout":"not json","stderr":""}))], &[]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "");
    assert!(tree.contains("unreadable") && !tree.contains("No agent sessions"), "{tree}");
}

#[gpui::test]
fn sessions_panel_asks_the_spaces_machine_and_folder(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let empty = core_ok(json!({"sessions":[]}));
    for (machine, folder, expected) in [
        (json!("cloud"), "", vec!["session", "list", "--json", "--on", "cloud"]),
        (json!("cloud"), "/srv/app", vec!["session", "list", "--json", "--on", "cloud", "--", "/srv/app"]),
        (Value::Null, "", vec!["session", "list", "--json", "--", "/fixture"]),
        (Value::Null, "C:\\Users\\me\\my project", vec!["session", "list", "--json", "--", "C:\\Users\\me\\my project"]),
        (Value::Null, "-odd", vec!["session", "list", "--json", "--", "-odd"]),
    ] {
        let host = sessions_host(&[("list", empty.clone())], &[]);
        describe_sessions(cx, &staged, "mods/sessions", sessions_values(machine.clone(), folder, "windows"), &host, "");
        let h = host.borrow();
        assert_eq!(h.cores, vec![expected], "{machine}/{folder}");
        assert_eq!(h.shared["sessions.count"], json!({ json!([machine, folder]).to_string(): 0 }), "counted under the space's own folder");
    }
}

#[gpui::test]
fn sessions_panel_resumes_beside_the_pane_or_in_a_new_tab(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let argv = json!(["claude", "--resume", "a1"]);
    let replies = [("list", core_ok(json!({"sessions":[]}))), ("resume", core_ok(json!({"argv": argv})))];
    let host = sessions_host(&replies, &[]);
    let setup = format!("{} this.command('resume', cx); this.command('resume-tab', cx);", session_target(None, "/fixture/project", "claude", "a1", "/p/a1.jsonl"));
    describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, &setup);
    let h = host.borrow();
    let resumes: Vec<_> = h.cores.iter().filter(|a| a[1] == "resume").cloned().collect();
    assert_eq!(resumes, vec![vec!["session", "resume", "--json", "--", "claude", "a1"]; 2]);
    let runs: Vec<_> = h.effects.iter().filter(|(n, _)| n == "run").map(|(_, a)| a.clone()).collect();
    assert!(runs.contains(&vec![json!("terminal.open"), json!({"argv": argv, "folder":"/fixture/project", "place":"right"})]), "{runs:?}");
    assert!(runs.contains(&vec![json!("tab.new"), json!({"space":"s1", "folder":"/fixture/project", "argv": argv})]), "{runs:?}");

    // A remote space's home: --on, and no folder (its terminals start at home).
    let host = sessions_host(&replies, &[]);
    let setup = format!("{} this.command('resume-tab', cx);", session_target(Some("cloud"), "", "codex", "c1", "/p/c1.jsonl"));
    describe_sessions(cx, &staged, "mods/sessions", sessions_values(json!("cloud"), "", "linux"), &host, &setup);
    let h = host.borrow();
    assert!(h.cores.contains(&vec!["session".into(), "resume".into(), "--json".into(), "--on".into(), "cloud".into(), "--".into(), "codex".into(), "c1".into()]), "{:?}", h.cores);
    assert!(h.effects.contains(&("run".into(), vec![json!("tab.new"), json!({"space":"s1", "argv": argv})])), "{:?}", h.effects);

    // No argv: a toast, never a terminal.
    let host = sessions_host(&[("list", core_ok(json!({"sessions":[]}))), ("resume", core_error("not_found"))], &[]);
    let setup = format!("{} this.command('resume', cx);", session_target(None, "/fixture/project", "claude", "gone", "/p/gone.jsonl"));
    describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, &setup);
    let h = host.borrow();
    assert!(!h.effects.iter().any(|(n, _)| n == "run"), "{:?}", h.effects);
    assert!(h.effects.iter().any(|(n, a)| n == "notify" && a[0]["level"] == "error" && a[0]["body"] == "fixture not_found"), "{:?}", h.effects);
}

#[gpui::test]
fn sessions_panel_copies_the_resume_command_quoted(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let replies = [("list", core_ok(json!({"sessions":[]}))), ("resume", core_ok(json!({"argv":["opencode", "--session", "ses 1"]})))];
    let host = sessions_host(&replies, &[]);
    let setup = format!("{} this.command('copy', cx);", session_target(None, "/fixture/project", "opencode", "ses 1", "/p/db"));
    describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, &setup);
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()).as_deref(), Some("opencode --session \"ses 1\""));
    assert!(host.borrow().effects.iter().any(|(n, a)| n == "notify" && a[0]["body"] == "opencode --session \"ses 1\""));
}

#[gpui::test]
fn sessions_panel_pins_hides_and_forgets_marks_per_machine_and_folder(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let list = core_ok(json!({"sessions":[]}));
    let local = json!([null, "/fixture/project", "claude", "a1"]).to_string();
    let other = json!(["cloud", "/fixture/project", "claude", "a1"]).to_string();
    let kept = json!({ other.clone(): {"pinned":true} });
    for (commands, expected) in [
        ("this.command('pin', cx);", json!({ other.clone(): {"pinned":true}, local.clone(): {"pinned":true} })),
        ("this.command('pin', cx); this.command('unpin', cx);", kept.clone()),
        ("this.command('hide', cx);", json!({ other.clone(): {"pinned":true}, local.clone(): {"hidden":true} })),
    ] {
        let host = sessions_host(&[("list", list.clone())], &[("sessions", kept.clone())]);
        let setup = format!("{} {commands}", session_target(None, "/fixture/project", "claude", "a1", "/p/a1.jsonl"));
        describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, &setup);
        assert_eq!(host.borrow().shared["sessions"], expected, "{commands}");
    }
}

#[gpui::test]
fn sessions_panel_delete_asks_three_ways_and_trash_handles_each_answer(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let list = core_ok(json!({"sessions":[]}));
    for (platform, machine, bin) in [("linux", None, "the Trash"), ("windows", None, "the Recycle Bin"), ("linux", Some("cloud"), "the Recycle Bin or Trash on cloud")] {
        let host = sessions_host(&[("list", list.clone())], &[]);
        let setup = format!("{} this.command('delete', cx);", session_target(machine, "/fixture/project", "claude", "a1", "/p/a1.jsonl"));
        describe_sessions(cx, &staged, "mods/sessions", sessions_values(json!(machine), "/fixture/project", platform), &host, &setup);
        let h = host.borrow();
        let (_, open) = h.effects.iter().find(|(n, _)| n == "openOverlay").expect("the delete dialog");
        assert_eq!(open[0]["mod"], "confirm");
        assert!(open[0]["props"]["detail"].as_str().unwrap().contains(&format!("moves its files to {bin}.")), "{open:?}");
        assert_eq!(open[0]["props"]["buttons"], json!([
            {"label":"Remove from HyperOx", "command":{"name":"sessions.hide"}, "primary":true},
            {"label":"Delete completely", "command":{"name":"sessions.trash"}}
        ]));
    }

    let local = json!([null, "/fixture/project", "opencode", "ses_1"]).to_string();
    let marks = json!({ local.clone(): {"pinned":true} });
    for (answer, forgotten, toast) in [
        (core_ok(json!({"trashed":["/p/a.jsonl"]})), true, None),
        (core_error("not_found"), true, None),
        (core_error("unsupported"), false, Some("This session can only be removed from HyperOx")),
        (core_error("invalid_params"), false, Some("Couldn't delete the session")),
    ] {
        let host = sessions_host(&[("list", list.clone()), ("trash", answer.clone())], &[("sessions", marks.clone())]);
        let setup = format!("{} this.command('trash', cx);", session_target(None, "/fixture/project", "opencode", "ses_1", "/p/db"));
        describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, &setup);
        let h = host.borrow();
        assert!(h.cores.contains(&vec!["session".into(), "trash".into(), "--json".into(), "--".into(), "opencode".into(), "ses_1".into()]), "{:?}", h.cores);
        assert_eq!(h.cores.iter().filter(|a| a[1] == "list").count(), 2, "listed again after: {:?}", h.cores);
        assert_eq!(h.shared["sessions"], if forgotten { json!({}) } else { marks.clone() }, "{answer}");
        let toasts: Vec<_> = h.effects.iter().filter(|(n, _)| n == "notify").map(|(_, a)| a[0]["title"].clone()).collect();
        assert_eq!(toasts, toast.map(|t| vec![json!(t)]).unwrap_or_default(), "{answer}");
    }

    // A remote session goes to that machine's trash.
    let host = sessions_host(&[("list", list.clone()), ("trash", core_ok(json!({"trashed":[]})))], &[]);
    let setup = format!("{} this.command('trash', cx);", session_target(Some("cloud"), "/srv/app", "claude", "a1", "/p/a1.jsonl"));
    describe_sessions(cx, &staged, "mods/sessions", sessions_values(json!("cloud"), "/srv/app", "linux"), &host, &setup);
    assert!(host.borrow().cores.contains(&vec!["session".into(), "trash".into(), "--json".into(), "--on".into(), "cloud".into(), "--".into(), "claude".into(), "a1".into()]));
}

#[gpui::test]
fn sessions_panel_shows_the_file_in_this_computers_file_manager(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let list = core_ok(json!({"sessions":[]}));
    for (platform, path, expected) in [
        ("windows", "C:/Users/me/.claude/projects/p/a1.jsonl", json!(["explorer", ["/select,C:\\Users\\me\\.claude\\projects\\p\\a1.jsonl"]])),
        ("windows", "C:\\Users\\me\\.codex\\c1.jsonl", json!(["explorer", ["/select,C:\\Users\\me\\.codex\\c1.jsonl"]])),
        ("macos", "/Users/me/.claude/projects/p/a1.jsonl", json!(["open", ["-R", "/Users/me/.claude/projects/p/a1.jsonl"]])),
        ("linux", "/home/me/.claude/projects/p/a1.jsonl", json!(["xdg-open", ["/home/me/.claude/projects/p"]])),
    ] {
        let host = sessions_host(&[("list", list.clone())], &[]);
        let setup = format!("{} this.command('reveal', cx);", session_target(None, "/fixture/project", "claude", "a1", path));
        describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", platform), &host, &setup);
        let h = host.borrow();
        assert_eq!(h.execs, vec![expected], "{platform}");
        assert!(h.effects.is_empty(), "explorer's exit code 1 is not an error: {:?}", h.effects);
    }
}

#[gpui::test]
fn sessions_menu_dialog_and_space_count_render(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (target, shown, hidden) in [
        (json!({"pinned":false, "local":true}), ["Resume", "Resume in new tab", "Pin", "Rename", "Copy resume command", "Show file", "Delete"], "Unpin"),
        (json!({"pinned":true, "local":false}), ["Resume", "Resume in new tab", "Unpin", "Rename", "Copy resume command", "Delete", "Delete"], "Show file"),
    ] {
        let mut p = props();
        p.as_object_mut().unwrap().extend(json!({"tab":"session", "target":target}).as_object().unwrap().clone());
        let tree = describe(cx, &staged, "mods/menu", true, p, updates("current", "current"), None).unwrap();
        for label in shown {
            assert!(tree.contains(label), "{label}: {tree}");
        }
        assert!(!tree.contains(hidden), "{hidden}: {tree}");
    }

    let mut p = props();
    p.as_object_mut().unwrap().extend(json!({"title":"Delete session?", "detail":"Fixture detail", "buttons":[
        {"label":"Remove from HyperOx", "command":{"name":"sessions.hide"}, "primary":true},
        {"label":"Delete completely", "command":{"name":"sessions.trash"}}
    ]}).as_object().unwrap().clone());
    let tree = describe(cx, &staged, "mods/confirm", true, p, updates("current", "current"), None).unwrap();
    for label in ["Cancel", "Remove from HyperOx", "Delete completely", "choice-0", "choice-1"] {
        assert!(tree.contains(label), "{label}: {tree}");
    }
    assert!(!tree.contains("Confirm"), "{tree}");
    let tree = describe(cx, &staged, "mods/confirm", true, props(), updates("current", "current"), None).unwrap();
    assert!(tree.contains("Confirm") && tree.contains("Cancel") && !tree.contains("choice-0"), "Yes/No stays the default: {tree}");

    let counts = json!({ json!([null, "/fixture/project"]).to_string(): 4, json!(["remote", "/remote/project"]).to_string(): 0 });
    let host = sessions_host(&[], &[("sessions.count", counts)]);
    let tree = describe_sessions(cx, &staged, "mods/spaces", fixture(true, props(), updates("current", "current")), &host, "");
    assert!(tree.contains("space-sessions-s1") && !tree.contains("space-sessions-s2"), "{tree}");
}

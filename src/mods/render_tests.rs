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
    /// Scripted `core` answers by the start of the joined arguments; with none, nothing is scripted.
    replies: BTreeMap<String, Value>,
    cores: Vec<Vec<String>>,
    writes: Vec<(String, String)>,
    /// Paths passed to `removeAppFile`, and the files a script of writes and removes leaves behind.
    removes: Vec<String>,
    files: BTreeMap<String, String>,
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
        ("spaces".into(), json!([])), ("agents".into(), json!([])), ("terminals".into(), json!([])), ("panes".into(), json!([])), ("tree".into(), Value::Null), ("focused".into(), Value::Null),
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
    module = module.function("busyVerb", busy_verb_call);
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
    for name in ["exec", "core", "readTextFile", "writeTextFile", "removeAppFile", "sendFeedback"] {
        let calls = calls.clone();
        module = module.async_function(name, move |a| {
            let first = a.get(0).map(from_host).unwrap_or(Value::Null);
            let scripted = scripted(&calls, name, &first, a.get(1).map(from_host));
            let reply = match name {
                _ if scripted.is_some() => scripted,
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
fn brand_shows_beta_pill_only_when_expanded(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (platform, width, shown) in [("linux", 268, true), ("macos", 268, true), ("windows", 160, true), ("linux", 159, false), ("linux", 44, false), ("macos", 76, false)] {
        let mut values = fixture(true, props(), updates("current", "current"));
        values.insert("window".into(), json!({"platform":platform,"titlebar":"none","fullscreen":false,"buttons":false}));
        values.get_mut("layout").unwrap()["children"][0]["size"]["px"] = json!(width);
        let tree = describe_with_values(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
        assert_eq!(tree.contains("brand-beta"), shown, "{platform}/{width}: {tree}");
        assert_eq!(tree.contains("Beta"), shown, "{platform}/{width}: {tree}");
        assert_eq!(tree.contains("HyperOx"), shown, "the pill goes with the brand: {platform}/{width}: {tree}");
        if shown {
            let (brand, pill, search) = (tree.find("HyperOx").unwrap(), tree.find("brand-beta").unwrap(), tree.find("search").unwrap());
            assert!(brand < pill && pill < search, "the pill sits right after the name, before the search button: {tree}");
        }
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
fn confirm_dialog_background_follows_the_frame_corners(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["title"] = json!("Close space?");
    p["command"] = json!({"name": "space.close", "args": {"id": "s1"}});
    let tree = describe(cx, &staged, "mods/confirm", true, p, updates("current", "current"), None).unwrap();
    let root = tree.lines().find(|l| l.contains(":id[Str(\"confirm\")]")).unwrap();
    assert!(root.contains(".rounded[Number(8.0)]"), "the dialog background must follow the frame's corners: {root}");
}

#[gpui::test]
fn confirm_dialog_with_its_own_buttons_or_none_keeps_the_rounded_corners(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let two = json!([
        {"label":"Remove from HyperOx", "command":{"name":"sessions.hide"}, "primary":true},
        {"label":"Delete completely", "command":{"name":"sessions.trash"}}
    ]);
    for (buttons, choices) in [(two, 2), (json!([]), 0)] {
        let mut p = props();
        p["title"] = json!("Delete session?");
        p["buttons"] = buttons;
        let tree = describe(cx, &staged, "mods/confirm", true, p, updates("current", "current"), None).unwrap();
        let root = tree.lines().find(|l| l.contains(":id[Str(\"confirm\")]")).unwrap();
        assert!(root.contains(".rounded[Number(8.0)]"), "{choices} choices: {root}");
        assert!(tree.contains(":id[Str(\"no\")]"), "Cancel is always there: {tree}");
        let shown = (0..3).filter(|i| tree.contains(&format!(":id[Str(\"choice-{i}\")]"))).count();
        assert_eq!(shown, choices, "the buttons given, no Confirm added: {tree}");
        assert!(!tree.contains(":id[Str(\"yes\")]"), "{tree}");
    }
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

// The scripted answer to a `core` or `writeTextFile` call, recording it; None when nothing is scripted.
fn scripted(calls: &Rc<RefCell<Calls>>, name: &str, first: &Value, text: Option<Value>) -> Option<Value> {
    let mut c = calls.borrow_mut();
    if c.replies.is_empty() {
        return None;
    }
    match name {
        "writeTextFile" => {
            let (path, text): (String, String) = (first.as_str()?.into(), text?.as_str()?.into());
            c.files.insert(path.clone(), text.clone());
            c.writes.push((path, text));
            Some(Value::Null)
        }
        "removeAppFile" => {
            c.removes.push(first.as_str()?.into());
            // A `removeAppFile refuses` reply makes every removal reject.
            if c.replies.contains_key("removeAppFile refuses") {
                return None;
            }
            c.files.remove(first.as_str()?);
            Some(Value::Null)
        }
        "core" => {
            let args: Vec<String> = serde_json::from_value(first.clone()).ok()?;
            let key = args.join(" ");
            let reply = c.replies.iter().filter(|(k, _)| key.starts_with(k.as_str())).max_by_key(|(k, _)| k.len()).map(|(_, v)| v.clone());
            c.cores.push(args);
            reply
        }
        _ => None,
    }
}

// The Remote page as it looks once its tasks have finished: `replies` answer core, `setup` runs after init.
fn describe_remote(cx: &mut TestAppContext, staged: &Staged, replies: &[(&str, Value)], setup: &str) -> (String, Rc<RefCell<Calls>>) {
    describe_remote_in(cx, staged, replies, setup, "/fixture/app")
}

// The Remote page with the app folder at `app_home`.
fn describe_remote_in(
    cx: &mut TestAppContext,
    staged: &Staged,
    replies: &[(&str, Value)],
    setup: &str,
    app_home: &str,
) -> (String, Rc<RefCell<Calls>>) {
    describe_remote_with(cx, staged, replies, setup, app_home, &[])
}

// The Remote page with `files` already on disk.
fn describe_remote_with(
    cx: &mut TestAppContext,
    staged: &Staged,
    replies: &[(&str, Value)],
    setup: &str,
    app_home: &str,
    files: &[(&str, &str)],
) -> (String, Rc<RefCell<Calls>>) {
    let calls = Rc::new(RefCell::new(Calls::default()));
    calls.borrow_mut().files = files.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    calls.borrow_mut().replies = replies.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    let mut p = props();
    p["section"] = json!("remote");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("appHome".into(), json!(app_home));
    let policy = Policy::new()
        .with_capabilities(Capabilities::new().read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("frame/settings", setup);
    let window = cx.add_window(|_, _| Empty);
    let mut session = window
        .update(cx, |_, window, cx| DescriptionSession::load(runtime, root, policy, window, cx))
        .unwrap()
        .unwrap();
    cx.run_until_parked();
    let tree = window.update(cx, |_, window, cx| session.describe(window, cx)).unwrap().unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    (tree, calls)
}

fn bridge_ls(identities: Value, locked: bool, token: &str) -> Value {
    core_ok(json!({"identities":identities, "locked":locked, "telegram_token":token}))
}

fn remote_replies(ls: Value, terminals: Value) -> Vec<(&'static str, Value)> {
    vec![
        ("bridge ls", ls),
        ("bridge log", core_ok(json!({"entries":[
            {"time_ms":1700000000000i64, "identity":"i1", "command":"list", "result":"ok"},
            {"time_ms":1700000060000i64, "identity":"i1", "command":"send", "what":"t1", "result":"ok"}
        ]}))),
        ("ls --json", core_ok(json!({"terminals":terminals}))),
    ]
}

#[gpui::test]
fn settings_remote_page_shows_unpaired_paired_and_locked(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let paired = json!([{"id":"i1", "transport":"telegram", "user_id":"42", "label":"My phone", "paired_at_ms":1700000000000i64}]);
    let (tree, _) = describe_remote(cx, &staged, &remote_replies(bridge_ls(json!([]), false, "unset"), json!([])), "");
    for want in ["Remote control: On", "No paired devices yet.", "Not set", "@BotFather", "Stopped", "Same Wi-Fi", "ACTIVITY", "send"] {
        assert!(tree.contains(want), "unpaired page lacks {want:?}: {tree}");
    }
    let (tree, _) = describe_remote(cx, &staged, &remote_replies(bridge_ls(paired.clone(), false, "set"), json!([])), "");
    assert!(tree.contains("My phone") && tree.contains("st-r-unpair-i1") && tree.contains("Set."), "{tree}");
    assert!(tree.contains("st-r-lock") && !tree.contains("No paired devices yet."), "{tree}");
    let (tree, _) = describe_remote(cx, &staged, &remote_replies(bridge_ls(paired, true, "set"), json!([])), "");
    assert!(tree.contains("Remote control: Locked") && tree.contains("Unlock"), "{tree}");
}

#[gpui::test]
fn settings_remote_page_shows_the_bot_running_or_stopped(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let ls = || bridge_ls(json!([]), false, "set");
    let (tree, _) = describe_remote(cx, &staged, &remote_replies(ls(), json!([])), "");
    assert!(tree.contains("st-r-bot-start") && !tree.contains("st-r-bot-stop"), "{tree}");
    let running = json!([{"name":"h0x-bridge-telegram", "state":"live", "labels":{"h0x-bridge":"telegram"}}]);
    let (tree, _) = describe_remote(cx, &staged, &remote_replies(ls(), running), "");
    assert!(tree.contains("st-r-bot-stop") && !tree.contains("st-r-bot-start"), "{tree}");
    let finished = json!([{"name":"h0x-bridge-telegram", "state":"finished"}]);
    let (tree, _) = describe_remote(cx, &staged, &remote_replies(ls(), finished), "");
    assert!(tree.contains("st-r-bot-start"), "a finished bot is stopped: {tree}");
}

#[gpui::test]
fn settings_remote_page_shows_the_pair_code_and_the_web_qr(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let web = json!([{"name":"h0x-bridge-web", "state":"live"}]);
    let mut replies = remote_replies(bridge_ls(json!([]), false, "set"), web);
    let output = json!({"url":"http://192.168.1.5:7000", "pair":"http://192.168.1.5:7000/#t=abc"});
    replies.push(("read h0x-bridge-web", json!({"code":0, "stdout":format!("starting\n{output}\n"), "stderr":""})));
    replies.push(("bridge qr http://192.168.1.5:7000/#t=abc",core_ok(json!({"size":3, "rows":["101", "010", "111"]}))));
    let far = now_ms() + 600_000;
    let setup = format!("this.pairing = {{ code: 'AB12CD', expires_ms: {far} }}; this.reach = 'tunnel';");
    let (tree, calls) = describe_remote(cx, &staged, &replies, &setup);
    assert!(tree.contains("AB12CD") && tree.contains("Send /pair AB12CD to your bot"), "{tree}");
    assert!(tree.contains("Scan with your phone") && tree.contains("st-r-web-stop"), "{tree}");
    assert!(tree.contains("http://192.168.1.5:7000/#t=abc"), "the link is shown: {tree}");
    assert!(tree.contains("Traffic passes through Cloudflare"), "{tree}");
    // 3 rows of runs: 1 0 1 / 0 1 0 / 111, inside a quiet zone, 5 px per module.
    assert!(tree.matches("#000000").count() >= 6 && tree.contains("#ffffff"), "the QR grid is drawn: {tree}");
    assert!(calls.borrow().cores.iter().any(|c| c.get(..2) == Some(&["bridge".to_string(), "qr".to_string()][..])), "{:?}", calls.borrow().cores);
}

#[gpui::test]
fn settings_remote_page_with_an_older_core_says_to_update(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let old = json!({"code":2, "stdout":"", "stderr":"error: unrecognized subcommand 'bridge'\n"});
    let (tree, _) = describe_remote(cx, &staged, &[("bridge", old)], "");
    assert!(tree.contains("Update h0x-core to use remote control."), "{tree}");
    assert!(!tree.contains("unrecognized") && !tree.contains("Remote control: On"), "{tree}");
}

#[gpui::test]
fn settings_remote_token_goes_through_a_file_that_is_deleted_and_is_never_shown(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    replies.push(("bridge telegram token --file", core_ok(json!({"telegram_token":"set"}))));
    let setup = "this.tokenInput.set_value('123456:SECRET-TOKEN'); this.saveToken(cx);";
    let (tree, calls) = describe_remote(cx, &staged, &replies, setup);
    let c = calls.borrow();
    let path = "/fixture/app/telegram-token.tmp";
    assert_eq!(c.writes, vec![(path.to_string(), "123456:SECRET-TOKEN".to_string())], "written once");
    assert_eq!(c.removes, vec![path.to_string(), path.to_string()], "a leftover is removed first, the file last");
    assert!(c.files.is_empty(), "no file holds the token: {:?}", c.files);
    assert!(c.cores.iter().any(|a| a.join(" ") == format!("bridge telegram token --file {path}")), "--file is called: {:?}", c.cores);
    assert!(c.cores.iter().all(|a| !a.join(" ").contains("SECRET")), "the token is never an argument: {:?}", c.cores);
    assert!(tree.contains("Bot token saved."), "{tree}");
    assert!(!tree.contains("SECRET"), "the token is never shown: {tree}");
}

#[gpui::test]
fn settings_remote_token_file_is_deleted_when_core_refuses(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut replies = remote_replies(bridge_ls(json!([]), false, "unset"), json!([]));
    replies.push(("bridge telegram token --file", core_error("invalid")));
    let (tree, calls) = describe_remote(cx, &staged, &replies, "this.tokenInput.set_value('9:BAD-TOKEN'); this.saveToken(cx);");
    let c = calls.borrow();
    assert_eq!(c.writes.len(), 1, "{:?}", c.writes);
    assert_eq!(c.removes.last().map(String::as_str), Some("/fixture/app/telegram-token.tmp"), "deleted even on failure: {:?}", c.removes);
    assert!(c.files.is_empty(), "no file holds the token: {:?}", c.files);
    assert!(tree.contains("fixture invalid") && !tree.contains("BAD-TOKEN"), "{tree}");
}

#[gpui::test]
fn settings_remote_token_leftover_file_is_removed_before_and_after(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let path = "/fixture/app/telegram-token.tmp";
    let setup = "this.tokenInput.set_value('8:NEW-TOKEN'); this.saveToken(cx);";
    for (label, reply) in [("saved", core_ok(json!({"telegram_token":"set"}))), ("refused", core_error("invalid"))] {
        let mut replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
        replies.push(("bridge telegram token --file", reply));
        let (_, calls) = describe_remote_with(cx, &staged, &replies, setup, "/fixture/app", &[(path, "1:OLD-TOKEN")]);
        let c = calls.borrow();
        assert_eq!(c.removes.first().map(String::as_str), Some(path), "{label}: the leftover goes first: {:?}", c.removes);
        assert!(c.files.is_empty(), "{label}: no file holds a token: {:?}", c.files);
    }
}

#[gpui::test]
fn settings_remote_token_file_is_deleted_when_the_core_call_throws(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // No reply for the token command: the core call itself rejects.
    let replies = remote_replies(bridge_ls(json!([]), false, "unset"), json!([]));
    let setup = "this.tokenInput.set_value('5:THROW-TOKEN'); this.saveToken(cx);";
    let (tree, calls) = describe_remote(cx, &staged, &replies, setup);
    let c = calls.borrow();
    let path = "/fixture/app/telegram-token.tmp".to_string();
    assert_eq!(c.writes, vec![(path.clone(), "5:THROW-TOKEN".to_string())], "{:?}", c.writes);
    assert!(c.cores.iter().any(|a| a.join(" ") == format!("bridge telegram token --file {path}")), "{:?}", c.cores);
    assert_eq!(c.removes, vec![path.clone(), path], "removed before and after the throw");
    assert!(c.files.is_empty(), "no file holds the token: {:?}", c.files);
    assert!(c.unexpected.iter().all(|m| !m.contains("THROW-TOKEN")), "{:?}", c.unexpected);
    assert!(tree.contains("unexpected core") && !tree.contains("Bot token saved.") && !tree.contains("THROW-TOKEN"), "{tree}");
}

#[gpui::test]
fn settings_remote_token_is_never_written_when_the_leftover_cannot_be_removed(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let path = "/fixture/app/telegram-token.tmp";
    let mut replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    replies.push(("bridge telegram token --file", core_ok(json!({"telegram_token":"set"}))));
    replies.push(("removeAppFile refuses", Value::Null));
    let setup = "this.tokenInput.set_value('6:NEW-TOKEN'); this.saveToken(cx);";
    let (tree, calls) = describe_remote_with(cx, &staged, &replies, setup, "/fixture/app", &[(path, "1:OLD-TOKEN")]);
    let c = calls.borrow();
    assert!(c.writes.is_empty(), "nothing is written over a leftover: {:?}", c.writes);
    assert!(c.cores.iter().all(|a| a.get(..3) != Some(&["bridge".to_string(), "telegram".to_string(), "token".to_string()][..])), "core is never asked: {:?}", c.cores);
    assert_eq!(c.removes, vec![path.to_string(), path.to_string()], "tried first, and again in the finally");
    assert_eq!(c.files.get(path).map(String::as_str), Some("1:OLD-TOKEN"), "the leftover is untouched, not overwritten");
    assert!(tree.contains("unexpected removeAppFile") && !tree.contains("Bot token saved."), "{tree}");
    assert!(!tree.contains("NEW-TOKEN") && !tree.contains("OLD-TOKEN"), "{tree}");
}

// The Remote page's replies with the web panel running and `core read h0x-bridge-web` printing `stdout`.
fn web_replies(stdout: &str) -> Vec<(&'static str, Value)> {
    let mut replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([{"name":"h0x-bridge-web", "state":"live"}]));
    replies.push(("read h0x-bridge-web", json!({"code":0, "stdout":stdout, "stderr":""})));
    replies
}

// The `core` calls that start a terminal.
fn starts(calls: &Rc<RefCell<Calls>>) -> Vec<Vec<String>> {
    calls.borrow().cores.iter().filter(|a| a.first().map(String::as_str) == Some("run")).cloned().collect()
}

#[gpui::test]
fn settings_remote_pair_code_shows_only_until_it_expires(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    for (label, expires) in [("exactly now", "Date.now()"), ("a minute ago", "Date.now() - 60000"), ("at zero", "0"), ("missing", "undefined")] {
        let setup = format!("this.pairing = {{ code: 'GONE42', expires_ms: {expires} }};");
        let (tree, _) = describe_remote(cx, &staged, &replies, &setup);
        assert!(!tree.contains("GONE42"), "a code expiring {label} is hidden: {tree}");
        assert!(tree.contains("Get a one-time code, then send it to your bot."), "{label}: {tree}");
    }
    let (tree, _) = describe_remote(cx, &staged, &replies, "this.pairing = { code: 'DAY123', expires_ms: Date.now() + 86400000 };");
    assert!(tree.contains("Send /pair DAY123 to your bot."), "{tree}");
    assert!(tree.contains("works for 86400 s.") || tree.contains("works for 86399 s."), "a day ahead counts whole seconds: {tree}");
    let far = "this.pairing = { code: 'FAR999', expires_ms: Date.now() + 10 * 365 * 86400000 };";
    let (tree, _) = describe_remote(cx, &staged, &replies, far);
    assert!(tree.contains("Send /pair FAR999") && !tree.contains("NaN") && !tree.contains("Infinity"), "{tree}");
}

#[gpui::test]
fn settings_remote_blank_token_makes_no_call(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let replies = remote_replies(bridge_ls(json!([]), false, "unset"), json!([]));
    for value in ["", "   ", "\\t \\t"] {
        let setup = format!("this.tokenInput.set_value('{value}'); this.saveToken(cx);");
        let (tree, calls) = describe_remote(cx, &staged, &replies, &setup);
        let c = calls.borrow();
        assert!(c.writes.is_empty(), "nothing is written for {value:?}: {:?}", c.writes);
        assert!(c.removes.is_empty(), "nothing is removed for {value:?}: {:?}", c.removes);
        assert!(c.cores.iter().all(|a| !a.join(" ").starts_with("bridge telegram")), "no call for {value:?}: {:?}", c.cores);
        assert!(!tree.contains("Bot token saved.") && tree.contains("Not set"), "{value:?}: {tree}");
    }
}

#[gpui::test]
fn settings_remote_long_padded_token_is_trimmed_and_never_an_argument(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    replies.push(("bridge telegram token --file", core_ok(json!({"telegram_token":"set"}))));
    let setup = "this.tokenInput.set_value('  ' + 'Z'.repeat(4096) + '  \\n'); this.saveToken(cx);";
    let (tree, calls) = describe_remote(cx, &staged, &replies, setup);
    let c = calls.borrow();
    let path = "/fixture/app/telegram-token.tmp".to_string();
    assert_eq!(c.writes, vec![(path.clone(), "Z".repeat(4096))], "trimmed, written whole");
    assert_eq!(c.removes.last(), Some(&path), "then deleted");
    assert!(c.files.is_empty(), "no file holds the token: {:?}", c.files);
    assert!(c.cores.iter().all(|a| !a.join(" ").contains("ZZZZ")), "the token is never an argument: {:?}", c.cores);
    assert!(tree.contains("Bot token saved.") && !tree.contains("ZZZZ"), "{tree}");
}

#[gpui::test]
fn settings_remote_token_file_sits_in_a_windows_app_home(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    replies.push(("bridge telegram token --file", core_ok(json!({"telegram_token":"set"}))));
    let home = r"C:\Users\Ana López\AppData\Roaming\h0x-ade";
    let setup = "this.tokenInput.set_value('7:WIN-TOKEN'); this.saveToken(cx);";
    let (tree, calls) = describe_remote_in(cx, &staged, &replies, setup, home);
    let c = calls.borrow();
    let path = format!("{home}/telegram-token.tmp");
    assert_eq!(c.writes, vec![(path.clone(), "7:WIN-TOKEN".to_string())], "{:?}", c.writes);
    assert_eq!(c.removes.last(), Some(&path), "{:?}", c.removes);
    assert!(c.files.is_empty(), "no file holds the token: {:?}", c.files);
    assert!(
        c.cores.iter().any(|a| *a == ["bridge", "telegram", "token", "--file", path.as_str()]),
        "the path with a space is one argument: {:?}",
        c.cores
    );
    assert!(tree.contains("Bot token saved.") && !tree.contains("WIN-TOKEN"), "{tree}");
}

#[gpui::test]
fn settings_remote_web_link_is_the_first_json_line_with_a_url_or_pair(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let qr = |calls: &Rc<RefCell<Calls>>| calls.borrow().cores.iter().filter(|a| a.get(..2) == Some(&["bridge".to_string(), "qr".to_string()][..])).cloned().collect::<Vec<_>>();
    let stdout = "Starting…\r\n42\r\nnull\r\n{\"event\":\"ready\"}\r\n{\"url\":\"http://10.0.0.2:7000\",\"pair\":\"http://10.0.0.2:7000/#t=first\"}\r\n{\"url\":\"http://10.0.0.3:7000\",\"pair\":\"http://10.0.0.3:7000/#t=second\"}\r\n";
    let mut replies = web_replies(stdout);
    replies.push(("bridge qr", core_ok(json!({"size":1, "rows":["1"]}))));
    let (tree, calls) = describe_remote(cx, &staged, &replies, "");
    assert!(tree.contains("http://10.0.0.2:7000/#t=first") && !tree.contains("#t=second"), "the first link wins: {tree}");
    assert_eq!(qr(&calls), vec![vec!["bridge", "qr", "http://10.0.0.2:7000/#t=first", "--json"]], "{:?}", calls.borrow().cores);

    let (tree, calls) = describe_remote(cx, &staged, &web_replies("Starting the panel\n"), "");
    assert!(tree.contains("st-r-web-stop") && tree.contains("Running"), "{tree}");
    assert!(!tree.contains("Scan with your phone") && qr(&calls).is_empty(), "no link yet: {tree}");

    let (tree, calls) = describe_remote(cx, &staged, &web_replies("{\"url\":\"http://10.0.0.4:7000\"}\n"), "");
    assert!(tree.contains("http://10.0.0.4:7000") && tree.contains("Scan with your phone"), "a url alone is the link: {tree}");
    assert!(qr(&calls).is_empty(), "no pair link, no QR: {:?}", calls.borrow().cores);
}

#[gpui::test]
fn settings_remote_odd_qr_replies_keep_the_page_and_the_link(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let blacks = |tree: &str| tree.matches("#000000").count();
    // The same page without a QR: a url-only link with the same text.
    let (base, _) = describe_remote(cx, &staged, &web_replies("{\"url\":\"http://10.0.0.2:7000/#t=qr\"}\n"), "");
    for (label, reply, extra) in [
        ("no modules", core_ok(json!({"size":0, "rows":[]})), 0),
        ("uneven rows", core_ok(json!({"size":4, "rows":["1", "0110", "", "2x"]})), 2),
        ("not json", json!({"code":0, "stdout":"QR unavailable", "stderr":""}), 0),
        ("no rows", core_ok(json!({"size":21})), 0),
        ("rows that are not text", core_ok(json!({"size":2, "rows":[1, 0]})), 0),
    ] {
        let mut replies = web_replies("{\"pair\":\"http://10.0.0.2:7000/#t=qr\"}\n");
        replies.push(("bridge qr", reply));
        let (tree, calls) = describe_remote(cx, &staged, &replies, "");
        assert!(tree.contains("Scan with your phone") && tree.contains("#t=qr") && tree.contains("st-r-web-stop"), "{label}: {tree}");
        assert_eq!(blacks(&tree), blacks(&base) + extra, "{label}: dark modules drawn: {tree}");
        assert!(calls.borrow().cores.iter().any(|a| a.join(" ").starts_with("bridge qr")), "{label}: {:?}", calls.borrow().cores);
    }
    let mut replies = web_replies("{\"pair\":\"http://10.0.0.2:7000/#t=qr\"}\n");
    replies.push(("bridge qr", core_error("qr")));
    let (tree, _) = describe_remote(cx, &staged, &replies, "");
    assert!(tree.contains("fixture qr") && tree.contains("Scan with your phone") && tree.contains("#t=qr"), "a refused QR says why: {tree}");
}

#[gpui::test]
fn settings_remote_vps_start_needs_a_machine_and_a_public_host(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    for (label, setup) in [
        ("nothing picked", "this.reach = 'vps'; this.startWeb(cx);"),
        ("no host", "this.reach = 'vps'; this.via = 'remote'; this.startWeb(cx);"),
        ("a blank host", "this.reach = 'vps'; this.via = 'remote'; this.publicHost.set_value('   '); this.startWeb(cx);"),
        ("no machine", "this.reach = 'vps'; this.publicHost.set_value('vps.example.com'); this.startWeb(cx);"),
    ] {
        let (tree, calls) = describe_remote(cx, &staged, &replies, setup);
        assert!(tree.contains("Pick a machine and type its public host."), "{label}: {tree}");
        assert!(starts(&calls).is_empty(), "{label}: nothing starts: {:?}", calls.borrow().cores);
    }
    let setup = "this.reach = 'vps'; this.via = 'remote'; this.publicHost.set_value(' vps.example.com '); this.startWeb(cx);";
    let (tree, calls) = describe_remote(cx, &staged, &replies, setup);
    let runs = starts(&calls);
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert!(runs[0].ends_with(&["bridge", "web", "--json", "--via", "remote", "--public-host", "vps.example.com"].map(String::from)), "{runs:?}");
    assert!(tree.contains("st-r-via-remote") && !tree.contains("Pick a machine"), "{tree}");

    let mut replies = replies.clone();
    replies.push(("machine ls", core_ok(json!({"machines":[]}))));
    let (tree, _) = describe_remote(cx, &staged, &replies, "this.reach = 'vps';");
    assert!(tree.contains("Add a machine first (Machines page).") && !tree.contains("st-r-via-"), "{tree}");
}

#[gpui::test]
fn settings_remote_web_starts_once_with_the_chosen_reach(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    for (reach, tail) in [("lan", vec!["--lan"]), ("tunnel", vec!["--tunnel", "--yes"])] {
        let setup = format!("this.reach = '{reach}'; this.startWeb(cx); this.startWeb(cx);");
        let (_, calls) = describe_remote(cx, &staged, &replies, &setup);
        let runs = starts(&calls);
        assert_eq!(runs.len(), 1, "{reach}: a second Start while busy is ignored: {runs:?}");
        let mut want = vec!["run", "--bg", "--keep", "--no-parent", "--outlive-parent", "--name", "h0x-bridge-web", "--label", "h0x-bridge=web"];
        want.extend(["--", "h0x-core", "bridge", "web", "--json"]);
        want.extend(tail);
        assert_eq!(runs[0], want, "{reach}");
    }
    let (tree, _) = describe_remote(cx, &staged, &web_replies(""), "");
    assert!(tree.contains("st-r-web-stop") && !tree.contains("st-r-web-start"), "a running panel offers only Stop: {tree}");
}

#[gpui::test]
fn settings_remote_stopping_a_bridge_that_is_gone_shows_why(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut replies = web_replies("");
    replies.push(("kill", core_error("not_found")));
    let setup = "this.remoteAction('st-r-web-stop', () => this.coreJson(['kill', 'h0x-bridge-web']), cx);";
    let (tree, calls) = describe_remote(cx, &staged, &replies, setup);
    assert!(calls.borrow().cores.iter().any(|a| *a == ["kill", "h0x-bridge-web"]), "{:?}", calls.borrow().cores);
    assert!(tree.contains("fixture not_found") && tree.contains("Remote control: On") && tree.contains("st-r-web-stop"), "{tree}");
}

#[gpui::test]
fn settings_remote_activity_keeps_the_newest_twenty_newest_first(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let log = |n: i64| {
        let entries: Vec<Value> =
            (0..n).map(|i| json!({"time_ms":1700000000000i64 + i * 1000, "identity":"i1", "command":format!("cmd-{i:02}"), "result":"ok"})).collect();
        core_ok(json!({ "entries": entries }))
    };
    let with_log = |reply: Value| {
        let mut replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
        replies.iter_mut().find(|(k, _)| *k == "bridge log").unwrap().1 = reply;
        replies
    };
    for (label, reply) in [("no entries", log(0)), ("a failed log", core_error("log"))] {
        let (tree, _) = describe_remote(cx, &staged, &with_log(reply), "");
        assert!(tree.contains("Nothing yet.") && tree.contains("Remote control: On"), "{label}: {tree}");
        assert!(!tree.contains("cmd-") && !tree.contains("fixture log"), "{label}: {tree}");
    }
    let (tree, calls) = describe_remote(cx, &staged, &with_log(log(20)), "");
    assert!(calls.borrow().cores.iter().any(|a| *a == ["bridge", "log", "-n", "20", "--json"]), "{:?}", calls.borrow().cores);
    assert!((0..20).all(|i| tree.contains(&format!("cmd-{i:02}"))) && !tree.contains("Nothing yet."), "all 20 shown: {tree}");
    assert!(tree.find("cmd-19") < tree.find("cmd-00"), "newest first: {tree}");
    let (tree, _) = describe_remote(cx, &staged, &with_log(log(21)), "");
    assert!(!tree.contains("cmd-00") && (1..21).all(|i| tree.contains(&format!("cmd-{i:02}"))), "the oldest of 21 is dropped: {tree}");
    assert!(tree.find("cmd-20") < tree.find("cmd-01"), "newest first: {tree}");
}

#[gpui::test]
fn settings_remote_unicode_device_labels_show_in_devices_and_activity(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let ids = json!([
        {"id":"i1", "transport":"telegram", "user_id":"42", "label":"Ана · 手机 📱", "paired_at_ms":1700000000000i64},
        {"id":"i2", "transport":"web", "user_id":"user-77"},
        {"id":"i3", "transport":"web"}
    ]);
    let (tree, _) = describe_remote(cx, &staged, &remote_replies(bridge_ls(ids, false, "set"), json!([])), "");
    // The device row and both activity entries (identity i1) carry the label.
    assert!(tree.matches("Ана · 手机 📱").count() >= 3, "{tree}");
    assert!(tree.contains("user-77") && tree.contains("web · paired"), "{tree}");
    for id in ["i1", "i2", "i3"] {
        assert!(tree.contains(&format!("st-r-unpair-{id}")), "{id} can be removed: {tree}");
    }
}

#[gpui::test]
fn settings_remote_locked_with_the_bot_and_the_panel_both_running(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let both = json!([{"name":"h0x-bridge-telegram", "state":"live"}, {"name":"h0x-bridge-web", "state":"live"}]);
    let mut replies = remote_replies(bridge_ls(json!([]), true, "set"), both);
    replies.push(("read h0x-bridge-web", json!({"code":0, "stdout":"{\"url\":\"http://10.0.0.9:7000\"}\n", "stderr":""})));
    let (tree, _) = describe_remote(cx, &staged, &replies, "");
    assert!(tree.contains("Remote control: Locked") && tree.contains("Unlock"), "{tree}");
    assert!(tree.contains("st-r-bot-stop") && tree.contains("st-r-web-stop"), "{tree}");
    assert!(!tree.contains("st-r-bot-start") && !tree.contains("st-r-web-start"), "{tree}");
    assert!(tree.matches("Running").count() >= 2 && tree.contains("http://10.0.0.9:7000"), "{tree}");
    // Only the exact terminal names count as running.
    let lookalikes = json!([{"name":"h0x-bridge-web-2", "state":"live"}, {"name":"old-h0x-bridge-telegram", "state":"live"}]);
    let (tree, _) = describe_remote(cx, &staged, &remote_replies(bridge_ls(json!([]), false, "set"), lookalikes), "");
    assert!(tree.contains("st-r-bot-start") && tree.contains("st-r-web-start"), "{tree}");
}

#[gpui::test]
fn settings_remote_unreadable_bridge_answers_show_a_message(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (label, reply, want) in [
        ("not json", json!({"code":0, "stdout":"garbage", "stderr":""}), "Couldn't read remote control."),
        ("no identities", core_ok(json!({"locked":false})), "Couldn't read remote control."),
        ("a silent failure", json!({"code":1, "stdout":"", "stderr":""}), "It didn't work."),
        ("an unknown command", json!({"code":2, "stdout":"", "stderr":"error: unknown command \"bridge\"\n"}), "Update h0x-core to use remote control."),
    ] {
        let (tree, _) = describe_remote(cx, &staged, &[("bridge ls", reply)], "");
        assert!(tree.contains(want), "{label}: {tree}");
        assert!(!tree.contains("Reading remote control") && !tree.contains("Remote control: On"), "{label}: {tree}");
    }
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

#[test]
fn busy_verbs_are_24_and_never_herd() {
    assert_eq!(BUSY_VERBS.len(), 24);
    assert_eq!(BUSY_VERBS.iter().collect::<BTreeSet<_>>().len(), 24, "no verb twice");
    for verb in BUSY_VERBS {
        assert!(!verb.to_lowercase().contains("herd"), "{verb}");
        assert!(!verb.is_empty() && verb.ends_with("ing"), "{verb}");
    }
    assert_eq!(BUSY_VERBS[0], "Oxygenating");
    assert_eq!(BUSY_VERBS[23], "Pondering");
}

#[test]
fn busy_verb_rotates_every_four_seconds() {
    let since = 1_700_000_000_000.0;
    let at = |ms: f64| busy_verb("t1", Some(since), since + ms);
    // "t1" hashes to 21: it starts on the 22nd verb.
    assert_eq!(at(0.0), "Assembling\u{2026}");
    assert_eq!(at(3999.0), "Assembling\u{2026}", "same verb inside one 4 s step");
    assert_eq!(at(4000.0), "Noodling\u{2026}");
    assert_eq!(at(8000.0), "Pondering\u{2026}");
    assert_eq!(at(12000.0), "Oxygenating\u{2026}", "wraps after the last verb");
    assert_eq!(at(24.0 * 4000.0), at(0.0), "a full turn is 24 steps");
    for step in 0..24u32 {
        let verb = at(f64::from(step) * 4000.0);
        assert!(BUSY_VERBS.iter().any(|v| verb == format!("{v}\u{2026}")), "{verb}");
        assert_ne!(verb, at(f64::from(step + 1) * 4000.0), "step {step} moves on");
    }
    // A clock behind working_since shows the first step, not a panic.
    assert_eq!(at(-5000.0), at(0.0));
}

#[test]
fn busy_verb_differs_between_panes_deterministically() {
    let since = Some(1_700_000_000_000.0);
    let now = 1_700_000_123_456.0;
    assert_ne!(busy_verb("t1", since, now), busy_verb("t2", since, now));
    assert_eq!(busy_verb("t1", since, now), busy_verb("t1", since, now));
    assert_eq!(busy_verb("t2", since, since.unwrap()), "Noodling\u{2026}");
    // Remote addresses and Windows-looking ids hash too.
    assert_ne!(busy_verb("vps/t7", since, now), busy_verb("vps/t8", since, now));
    assert!(busy_verb("C:\\work\\t1", since, now).ends_with('\u{2026}'));
}

#[test]
fn busy_verb_without_working_since_uses_the_id_alone() {
    for now in [0.0, 1_700_000_000_000.0, 1_700_000_004_000.0, f64::NAN] {
        assert_eq!(busy_verb("t1", None, now), "Assembling\u{2026}");
    }
    assert_eq!(busy_verb("", None, 0.0), "Oxygenating\u{2026}", "an empty id is the first verb");
}

#[gpui::test]
fn busy_verb_shows_only_while_working(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let verbs = |tree: &str| BUSY_VERBS.iter().filter(|v| tree.contains(&format!("{v}\u{2026}"))).count();
    for status in ["working", "blocked", "done", "idle"] {
        let mut values = fixture(true, props(), updates("current", "current"));
        for list in ["panes", "agents", "terminals"] {
            values.get_mut(list).unwrap()[0]["status"] = json!(status);
        }
        for entry in ["mods/peek", "mods/pane", "mods/agents"] {
            let tree = describe_with_values(cx, &staged, entry, values.clone(), None).unwrap();
            let shown = verbs(&tree);
            if status == "working" {
                assert_eq!(shown, 1, "{entry}: one verb for the one working agent: {tree}");
            } else {
                assert_eq!(shown, 0, "{entry}: no verb while {status}: {tree}");
            }
            assert!(!tree.to_lowercase().contains("herding"), "{entry}: {tree}");
        }
    }
}

#[gpui::test]
fn a_kept_working_sub_agent_is_listed_once_under_its_parent_with_one_busy_verb(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let verbs = |tree: &str| BUSY_VERBS.iter().map(|v| tree.matches(&format!("{v}\u{2026}")).count()).sum::<usize>();
    // t3 was started by t1 (working, in the fixture) and then given a pane of its own.
    for pinned in [false, true] {
        let mut values = fixture(true, props(), updates("current", "current"));
        let program = values["agents"][0]["program"].clone();
        values.get_mut("agents").unwrap().as_array_mut().unwrap().push(json!({"address":"t3","pane":"p3","space":"s1","space_name":"Local","tab":"tab1","tab_index":1,"tab_name":"Build","status":"working","app":"pi","program":program,"title":"Helper","seq":3,"focused":false,"pinned":pinned}));
        values.get_mut("panes").unwrap().as_array_mut().unwrap().push(json!({"pane":"p3","terminal":"t3","tab":"tab1","title":"Helper","cwd":"/fixture/project","branch":"main","status":"working","app":"pi","program":program,"focused":false,"zoomed":false}));
        values.get_mut("terminals").unwrap().as_array_mut().unwrap().push(json!({"address":"t3","name":"Helper","title":"Helper","argv":["pi"],"parent":"t1","labels":{},"state":"live","status":"working","report":{"app":"pi"},"program":program,"cwd":"/fixture/project"}));
        let tree = describe_with_values(cx, &staged, "mods/agents", values, None).unwrap();
        assert!(tree.contains(":id[Str(\"ag-t1\")]") && tree.contains(":id[Str(\"sub-t3\")]"), "pinned {pinned}: {tree}");
        if pinned {
            // A pinned one keeps its own row too, and that row says it is busy.
            assert!(tree.contains(":id[Str(\"ag-t3\")]"), "{tree}");
            assert_eq!(verbs(&tree), 2, "one verb per working row: {tree}");
        } else {
            assert!(!tree.contains(":id[Str(\"ag-t3\")]"), "no second row for the sub-agent: {tree}");
            assert_eq!(verbs(&tree), 2, "the parent's row and the sub-agent's sub-row carry one verb each: {tree}");
        }
        assert!(!tree.to_lowercase().contains("herding"), "{tree}");
    }
}

#[gpui::test]
fn a_working_kept_sub_agent_under_an_idle_parent_shows_one_busy_verb_on_its_sub_row(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let verbs = |tree: &str| BUSY_VERBS.iter().map(|v| tree.matches(&format!("{v}\u{2026}")).count()).sum::<usize>();
    let mut values = fixture(true, props(), updates("current", "current"));
    let program = values["agents"][0]["program"].clone();
    values.get_mut("agents").unwrap().as_array_mut().unwrap()[0]["status"] = json!("idle");
    values.get_mut("agents").unwrap().as_array_mut().unwrap().push(json!({"address":"t3","pane":"p3","space":"s1","space_name":"Local","tab":"tab1","tab_index":1,"tab_name":"Build","status":"working","app":"pi","program":program,"title":"Helper","seq":3,"focused":false,"pinned":false}));
    values.get_mut("panes").unwrap().as_array_mut().unwrap().push(json!({"pane":"p3","terminal":"t3","tab":"tab1","title":"Helper","cwd":"/fixture/project","branch":"main","status":"working","app":"pi","program":program,"focused":false,"zoomed":false}));
    values.get_mut("terminals").unwrap().as_array_mut().unwrap().push(json!({"address":"t3","name":"Helper","title":"Helper","argv":["pi"],"parent":"t1","labels":{},"state":"live","status":"working","report":{"app":"pi"},"program":program,"cwd":"/fixture/project"}));
    let tree = describe_with_values(cx, &staged, "mods/agents", values, None).unwrap();
    assert!(tree.contains(":id[Str(\"sub-t3\")]") && !tree.contains(":id[Str(\"ag-t3\")]"), "{tree}");
    assert_eq!(verbs(&tree), 1, "only the working sub-agent has a verb: {tree}");
}

// The files panel's editing: the real mod under a wrapper that takes commands from the test,
// with the disk, the writes and the core calls faked and recorded.
#[derive(Default)]
struct FilesDisk {
    files: BTreeMap<String, String>,
    fail: Option<String>,
    read_fail: Option<String>,
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
        let result = match disk.read_fail.clone() {
            Some(message) => Err(HostError::new(message)),
            None => Ok(to_host(&json!(text))),
        };
        Ok(std::future::ready(result))
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

// A read that fails shows its reason, cut to 120 characters (not bytes).
#[gpui::test]
fn files_panel_shows_why_a_file_cannot_be_previewed_capped_at_120_characters(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // The reason the panel shows when reading the file fails with `message`.
    let shown = |cx: &mut TestAppContext, message: &str| -> String {
        let disk = files_disk(&[("/fixture/project/notes.txt", "hello\n")]);
        disk.borrow_mut().read_fail = Some(message.to_owned());
        let tree = describe_files(cx, &staged, files_values(idle_agent()), &disk, "this.show('notes.txt', cx);");
        let line = tree
            .lines()
            .find_map(|l| l.split_once("Cannot preview this file: "))
            .unwrap_or_else(|| panic!("no reason shown for {message:?}: {tree}"))
            .1;
        line[..line.rfind('"').unwrap_or_else(|| panic!("{line}"))].to_owned()
    };

    let message = "/etc/notes.txt is outside your home folder and your spaces";
    let short = shown(cx, message);
    assert!(short.ends_with(message), "a short reason shows whole: {short}");
    // What the host puts before the message (the call it rejected), if anything.
    let lead = short.chars().count() - message.chars().count();
    assert!(lead < 120, "{short}");
    let prefix: String = short.chars().take(lead).collect();

    // Exactly 120 characters show whole; the 121st is cut, and so is the rest of a long one.
    let fill = 120 - lead;
    let exact = "a".repeat(fill);
    assert_eq!(shown(cx, &exact), format!("{prefix}{exact}"));
    assert_eq!(shown(cx, &format!("{exact}b")), format!("{prefix}{exact}"));
    assert_eq!(shown(cx, &"x".repeat(500)).chars().count(), 120);
    // Characters, not bytes: a two-byte letter on the boundary stays whole.
    let accents = format!("{}é", "a".repeat(fill - 1));
    assert_eq!(shown(cx, &accents), format!("{prefix}{accents}"));
    assert_eq!(shown(cx, &"é".repeat(fill + 1)), format!("{prefix}{}", "é".repeat(fill)));
    // A character outside the BMP straddling the cap: never shown half, the panel still renders.
    let straddle = shown(cx, &format!("{}😀tail", "a".repeat(fill - 1)));
    assert!(straddle.starts_with(&format!("{prefix}{}", "a".repeat(fill - 1))), "{straddle}");
    assert!(!straddle.contains('😀') && !straddle.contains("tail"), "{straddle}");
    // An empty message still shows the line, with only what the host adds.
    assert_eq!(shown(cx, ""), prefix);
}

// Something in the preview that throws no message at all (null, undefined, an
// empty error or string) reads "unknown error", never "null" or "undefined".
#[gpui::test]
fn files_panel_says_unknown_error_when_a_preview_fails_without_a_message(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for thrown in ["null", "undefined", "new Error('')", "''"] {
        let disk = files_disk(&[("/fixture/project/notes.txt", "hello\n")]);
        // The drafts lookup runs inside the preview's try, after the read.
        let setup = format!(
            "this.drafts = {{ get() {{ throw {thrown}; }}, delete() {{}}, set() {{}} }}; this.show('notes.txt', cx);"
        );
        let tree = describe_files(cx, &staged, files_values(idle_agent()), &disk, &setup);
        assert!(tree.contains("Cannot preview this file: unknown error"), "{thrown}: {tree}");
        assert!(!tree.contains("Cannot preview this file: null") && !tree.contains("Cannot preview this file: undefined"), "{thrown}: {tree}");
    }
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
    // panes, terminals and tree; tab.new and terminal.open add a pane ("rp<n>") as the app would.
    live: BTreeMap<String, Value>,
    made: usize,
    focus: Option<String>,
    // The app never shows the panes it is asked for (a restore that times out).
    stuck: bool,
}

impl SessionsHost {
    fn apply(&mut self, command: &str, args: &Value) {
        let tab = match command {
            "pane.focus" => {
                self.focus = args["pane"].as_str().map(str::to_owned);
                return;
            }
            "tab.new" => None,
            "terminal.open" if args["place"] != "tab" => {
                let panes = self.live.get("panes").and_then(Value::as_array).cloned().unwrap_or_default();
                panes.iter().find(|p| p["pane"].as_str() == self.focus.as_deref()).map(|p| p["tab"].clone())
            }
            _ => return,
        };
        if self.stuck {
            return;
        }
        self.made += 1;
        let n = self.made;
        let pane = json!({"pane":format!("rp{n}"), "terminal":format!("ra{n}"), "tab":tab.unwrap_or_else(|| json!(format!("rt{n}"))), "cwd":args["folder"]});
        self.live.entry("panes".into()).or_insert_with(|| json!([])).as_array_mut().unwrap().push(pane);
        self.focus = Some(format!("rp{n}"));
    }
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
    values.insert("layout".into(), panel_layout(false, "sessions"));
    values
}

// The shipped layout with its side panel shown or hidden, on its `active` tab.
fn panel_layout(hidden: bool, active: &str) -> Value {
    fn walk(n: &mut Value, hidden: bool, active: &str) {
        if n["id"] == "panel" {
            n["hidden"] = json!(hidden);
        }
        if n["id"] == "panel-body" {
            n["active"] = json!(active);
        }
        for c in n.get_mut("children").and_then(Value::as_array_mut).into_iter().flatten() {
            walk(c, hidden, active);
        }
    }
    let mut layout: Value = serde_json::from_str(include_str!("../../defaults/layouts/main.json")).unwrap();
    walk(&mut layout, hidden, active);
    layout
}

fn sessions_module(mut values: BTreeMap<String, Value>, host: Rc<RefCell<SessionsHost>>) -> HostModule {
    let mut module = HostModule::new("h0x");
    for name in ["panes", "terminals", "tree", "layout"] {
        let value = values.remove(name).unwrap_or(Value::Null);
        host.borrow_mut().live.entry(name.into()).or_insert(value);
        let h = host.clone();
        module = module.function(name, move |_| Ok(to_host(&h.borrow().live[name])));
    }
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
    for name in ["run", "openOverlay", "closeOverlay", "notify", "setAgentOrder"] {
        let h = host.clone();
        module = module.function(name, move |a| {
            let args: Vec<Value> = (0..2).filter_map(|i| a.get(i).map(from_host)).collect();
            if name == "run" {
                let command = crate::commands::find(a.string(0)?).unwrap_or_else(|| panic!("unknown command {args:?}"));
                crate::commands::validate_args(command, args.get(1).unwrap_or(&Value::Null)).unwrap();
                h.borrow_mut().apply(args[0].as_str().unwrap_or_default(),args.get(1).unwrap_or(&Value::Null));
            }
            h.borrow_mut().effects.push((name.into(), args));
            Ok(if name == "openOverlay" || name == "notify" { "fixture-overlay".into() } else { HostValue::Null })
        });
    }
    module = module.function("busyVerb", busy_verb_call);
    module = module.async_function("knownAgents", |_| Ok(async { Ok(HostValue::Array(vec![])) }));
    for name in ["nextCommand", "watchFolder", "readTextFile", "writeTextFile"] {
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
        // Other commands answer by their first word (`machine`).
        let verb = if args.first().map(String::as_str) == Some("session") { args.get(1).cloned() } else { args.first().cloned() };
        let reply = verb.and_then(|v| h.replies.get(&v).cloned()).unwrap_or_else(|| json!({"code":1,"stdout":"","stderr":"fixture: no reply"}));
        h.cores.push(args);
        Ok(std::future::ready(Ok(to_host(&reply))))
    })
}

// Loads `entry` (with `setup` run after its init) and lets its tasks finish; it keeps running.
fn load_sessions(
    cx: &mut TestAppContext,
    staged: &Staged,
    entry: &str,
    values: BTreeMap<String, Value>,
    host: &Rc<RefCell<SessionsHost>>,
    setup: &str,
) -> (gpui::WindowHandle<Empty>, DescriptionSession) {
    static NEXT_STORE: AtomicU64 = AtomicU64::new(0);
    let policy = Policy::new()
        .with_storage_path(staged.0.join(format!("sessions-store-{}.json", NEXT_STORE.fetch_add(1, Ordering::Relaxed))))
        .with_capabilities(Capabilities::new().storage(true).clipboard_write(true).read_roots([staged.0.clone()]))
        .with_host_module(sessions_module(values, host.clone()))
        .unwrap();
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper(entry, setup);
    let window = cx.add_window(|_, _| Empty);
    let session = window
        .update(cx, |_, window, cx| DescriptionSession::load(runtime, root, policy, window, cx))
        .unwrap()
        .unwrap();
    cx.run_until_parked();
    (window, session)
}

// Moves the clock `ms` forward in 50 ms steps, letting every task finish at each.
fn advance(cx: &mut TestAppContext, ms: u64) {
    for _ in 0..ms / 50 {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
    }
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
    let (window, mut session) = load_sessions(cx, staged, entry, values, host, setup);
    // Long enough for a restore that waits for panes to give up (200 × 50 ms).
    advance(cx, 210 * 50);
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
fn sessions_panel_asks_core_only_while_it_is_shown(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let elsewhere = json!({"id":"root", "type":"block", "content":{"terminal":"t1"}});
    for (layout, asks) in [
        (panel_layout(true, "sessions"), false),
        (panel_layout(false, "files"), false),
        (panel_layout(false, "sessions"), true),
        // Not in the layout (an overlay or a pane): shown while it runs.
        (elsewhere, true),
    ] {
        let mut values = sessions_values(Value::Null, "/fixture/project", "linux");
        values.insert("layout".into(), layout.clone());
        let host = sessions_host(&[("list", core_ok(json!({"sessions":[]})))], &[]);
        describe_sessions(cx, &staged, "mods/sessions", values, &host, "");
        let h = host.borrow();
        let expected: Vec<Vec<String>> = if asks { vec![vec!["session".into(), "list".into(), "--json".into(), "--".into(), "/fixture/project".into()]] } else { vec![] };
        assert_eq!(h.cores, expected, "{}", layout["id"]);
        assert_eq!(h.shared.contains_key("sessions.count"), asks, "the count waits for a list");
    }
}

// The cx a view is given, with each notify through it (or through the cx its
// tasks and timers get) reported to `seen`.
const COUNTED: &str = r#"
const counted = (c, seen) => ({
  ...c,
  notify: (...a) => { seen(); return c.notify(...a); },
  spawn: (f, o) => c.spawn((t) => f(counted(t, seen)), o),
  timer: {
    ...c.timer,
    every: (ms, f, o) => c.timer.every(ms, (t) => f(counted(t, seen)), o),
    after: (ms, f, o) => c.timer.after(ms, (t) => f(counted(t, seen)), o),
  },
});
"#;

// Entry "probe": `body` (a `class Probe extends Real`, Real being the shipped
// `entry`) reports what it saw with setShared("probe", ...).
fn write_probe(staged: &Staged, entry: &str, body: &str) {
    std::fs::create_dir_all(staged.0.join("probe")).unwrap();
    std::fs::write(
        staged.0.join("probe").join("main.js"),
        format!("import Real from '../{entry}/main.js';\nimport {{ setShared }} from 'h0x';\n{COUNTED}\n{body}"),
    )
    .unwrap();
}

#[gpui::test]
fn spaces_draw_again_only_when_the_machine_list_changed(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    write_probe(&staged, "mods/spaces", r#"
export default class Probe extends Real {
  init(props, cx) {
    super.init(props, cx);
    cx.timer.after(1000, (cx) => this.refreshMachines(cx));
    cx.timer.after(2000, (cx) => this.refreshMachines(cx));
  }
  refreshMachines(cx) {
    super.refreshMachines(counted(cx, () => setShared("probe", (this.drawn = (this.drawn ?? 0) + 1))));
  }
}
"#);
    let ls = vec!["machine".to_string(), "ls".into(), "--json".into()];
    for (reply, drawn) in [
        // The first answer is new; the same answer twice more draws nothing.
        (core_ok(json!({"machines":[{"name":"remote","state":"online"}]})), Some(json!(1))),
        // No answer: the empty list it had stays, nothing to draw.
        (core_error("unreachable"), None),
    ] {
        let host = sessions_host(&[("machine", reply)], &[]);
        describe_sessions(cx, &staged, "probe", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "");
        let h = host.borrow();
        assert_eq!(h.cores.iter().filter(|a| **a == ls).count(), 3, "{:?}", h.cores);
        assert_eq!(h.shared.get("probe"), drawn.as_ref(), "{:?}", h.cores);
    }
}

#[gpui::test]
fn files_and_changes_ask_nothing_while_hidden_and_refresh_when_shown(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (name, other) in [("files", "changes"), ("changes", "files")] {
        let host = sessions_host(&[], &[]);
        let mut values = sessions_values(Value::Null, "/fixture/project", "linux");
        values.insert("layout".into(), panel_layout(true, name));
        let (window, _session) = load_sessions(cx, &staged, &format!("mods/{name}"), values, &host, "");
        let started = host.borrow().execs.len();
        // Hidden (the panel closed, then another of its tabs), past two 10 s refreshes each.
        for layout in [panel_layout(true, name), panel_layout(false, other)] {
            host.borrow_mut().live.insert("layout".into(), layout);
            advance(cx, 21_000);
            let h = host.borrow();
            assert_eq!(h.execs.len(), started, "{name}: nothing runs while hidden: {:?}", h.execs);
        }
        // Shown again: a refresh at once, not at the next 10 s.
        host.borrow_mut().live.insert("layout".into(), panel_layout(false, name));
        advance(cx, 1000);
        let execs = host.borrow().execs[started..].to_vec();
        assert!(!execs.is_empty(), "{name}: shown again, it refreshes");
        if name == "changes" {
            assert_eq!(execs[0], json!(["git", ["rev-parse", "--show-toplevel"]]));
        }
        window.update(cx, |_, window, _| window.remove_window()).unwrap();
    }
}

#[gpui::test]
fn agents_list_redraws_its_times_only_with_agents(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    write_probe(&staged, "mods/agents", r#"
export default class Probe extends Real {
  init(props, cx) {
    super.init(props, counted(cx, () => setShared("probe", (this.drawn = (this.drawn ?? 0) + 1))));
  }
}
"#);
    let idle = json!([{"address":"a1","pane":"p1","space":"s1","space_name":"Here","tab":"tab1","tab_index":1,"tab_name":"One","status":"idle","app":"pi","program":null,"title":"One","seq":1,"focused":false}]);
    // Over 30 s: no agent, nothing to draw; one (not working), its time once.
    for (list, drawn) in [(json!([]), None), (idle, Some(json!(1)))] {
        let host = sessions_host(&[], &[]);
        let mut values = sessions_values(Value::Null, "/fixture/project", "linux");
        values.insert("agents".into(), list);
        let (window, _session) = load_sessions(cx, &staged, "probe", values, &host, "");
        advance(cx, 31_000);
        assert_eq!(host.borrow().shared.get("probe"), drawn.as_ref());
        window.update(cx, |_, window, _| window.remove_window()).unwrap();
    }
}

#[gpui::test]
fn agents_list_ticks_every_second_while_a_kept_sub_agent_works(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    write_probe(&staged, "mods/agents", r#"
export default class Probe extends Real {
  init(props, cx) {
    super.init(props, counted(cx, () => setShared("probe", (this.drawn = (this.drawn ?? 0) + 1))));
  }
}
"#);
    // The parent is idle; only its kept sub-agent (no row of its own, under the parent) works.
    let agent = |address: &str, status: &str, seq: i64| json!({"address":address,"pane":format!("p-{address}"),"space":"s1","space_name":"Here","tab":"tab1","tab_index":1,"tab_name":"One","status":status,"app":"pi","program":null,"title":address,"seq":seq,"focused":false});
    let live = |address: &str, parent: Value, status: &str| json!({"address":address,"name":address,"title":address,"argv":["pi"],"parent":parent,"labels":{},"state":"live","status":status,"report":{"app":"pi"},"program":null,"cwd":"/fixture/project"});
    let host = sessions_host(&[], &[]);
    let mut values = sessions_values(Value::Null, "/fixture/project", "linux");
    values.insert("agents".into(), json!([agent("a1", "idle", 1), agent("a2", "working", 2)]));
    values.insert("terminals".into(), json!([live("a1", Value::Null, "idle"), live("a2", json!("a1"), "working")]));
    let (window, _session) = load_sessions(cx, &staged, "probe", values, &host, "");
    advance(cx, 31_000);
    let drawn = host.borrow().shared.get("probe").and_then(Value::as_f64).unwrap_or(0.0);
    assert!((30.0..=33.0).contains(&drawn),"a working agent redraws its verb once a second: {drawn}");
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
}

#[gpui::test]
fn sessions_rows_are_made_again_only_when_the_list_or_the_marks_change(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    write_probe(&staged, "mods/sessions", r#"
export default class Probe extends Real {
  init(props, cx) {
    super.init(props, cx);
    cx.timer.after(2000, () => {
      const a = this.rows(), b = this.rows();
      this.setMark(a[1], { pinned: true });
      const c = this.rows(), d = this.rows();
      this.list = [...this.list];
      const e = this.rows();
      setShared("probe", { reused: a === b, pinned: c !== b, first: c[0].id, again: c === d, relisted: e !== d });
    });
  }
}
"#);
    let now = now_ms();
    let list = json!({"sessions":[
        {"agent":"claude","id":"a1","title":"One","last_used":now - MINUTE,"path":"/fixture/.claude/projects/p/a1.jsonl"},
        {"agent":"claude","id":"a2","title":"Two","last_used":now - 2 * MINUTE,"path":"/fixture/.claude/projects/p/a2.jsonl"}
    ]});
    let host = sessions_host(&[("list", core_ok(list))], &[]);
    describe_sessions(cx, &staged, "probe", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "");
    assert_eq!(
        host.borrow().shared["probe"],
        json!({"reused":true, "pinned":true, "first":"a2", "again":true, "relisted":true})
    );
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

fn block(pane: &str, terminal: &str) -> Value {
    json!({"id":pane, "type":"block", "content":{"terminal":terminal}})
}

// Space s1 as tree() gives it: "Agents" is t1 beside (t2 over t3); the second tab is t4 beside a mod pane.
fn layout_values(machine: Value, folder: &str, platform: &str, cwd: &str) -> BTreeMap<String, Value> {
    let mut values = sessions_values(machine, folder, platform);
    values.insert("tree".into(), json!({"id":"s1", "tabs":[
        {"id":"tb1", "name":"Agents", "root":{"id":"c1", "type":"row", "children":[block("p1", "t1"), {"id":"c2", "type":"column", "children":[block("p2", "t2"), block("p3", "t3")]}]}},
        {"id":"tb2", "name":"", "root":{"id":"c3", "type":"row", "children":[block("p4", "t4"), {"id":"p5", "type":"block", "content":{"mod":"files", "props":{}}}]}}
    ]}));
    values.insert("panes".into(), json!([
        {"pane":"p1", "terminal":"t1", "tab":"tb1", "cwd":cwd}, {"pane":"p2", "terminal":"t2", "tab":"tb1", "cwd":cwd},
        {"pane":"p3", "terminal":"t3", "tab":"tb1", "cwd":"/fixture/project/web"}, {"pane":"p4", "terminal":"t4", "tab":"tb2", "cwd":null},
        {"pane":"p5", "terminal":null, "mod":"files", "tab":"tb2"}
    ]));
    values.insert("terminals".into(), json!([
        {"address":"t1", "argv":["claude"], "labels":{"h0x.agent":"claude"}, "state":"live"},
        {"address":"t2", "argv":["claude", "--model", "opus"], "labels":{"h0x.agent":"claude"}, "state":"live"},
        {"address":"t3", "argv":["bash"], "labels":{}, "state":"live"},
        {"address":"t4", "argv":["bash"], "labels":{}, "state":"live"}
    ]));
    values
}

fn saved_layout(cwd: &str) -> Value {
    json!({"tabs":[
        {"name":"Agents", "root":{"split":"right", "children":[
            {"agent":"claude", "session":"a2", "argv":["claude"], "cwd":cwd},
            {"split":"down", "children":[{"agent":"claude", "session":"a1", "argv":["claude", "--model", "opus"], "cwd":cwd}, {"cwd":"/fixture/project/web"}]}
        ]}},
        {"name":"", "root":{"cwd":null}}
    ]})
}

fn session_list() -> Value {
    let now = now_ms();
    core_ok(json!({"sessions":[
        {"agent":"claude", "id":"a2", "title":"New", "last_used":now - MINUTE, "path":"/p/a2.jsonl"},
        {"agent":"codex", "id":"c1", "title":"Codex", "last_used":now - 2 * MINUTE, "path":"/p/c1.jsonl"},
        {"agent":"claude", "id":"a1", "title":"Old", "last_used":now - 3 * MINUTE, "path":"/p/a1.jsonl"}
    ]}))
}

fn runs(host: &Rc<RefCell<SessionsHost>>) -> Vec<Value> {
    host.borrow().effects.iter().filter(|(n, _)| n == "run").map(|(_, a)| json!(a)).collect()
}

fn toasts(host: &Rc<RefCell<SessionsHost>>) -> Vec<Value> {
    host.borrow().effects.iter().filter(|(n, _)| n == "notify").map(|(_, a)| a[0]["title"].clone()).collect()
}

#[gpui::test]
fn sessions_save_layout_keeps_tabs_splits_and_each_agents_newest_unused_session(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mark = json!({ json!([null, "/fixture/project", "claude", "a1"]).to_string(): {"pinned":true} });
    for (machine, folder, platform, cwd, list) in [
        (Value::Null, "/fixture/project", "linux", "/fixture/project", vec!["session", "list", "--json", "--", "/fixture/project"]),
        (json!("cloud"), "", "windows", "C:\\Users\\me\\my project", vec!["session", "list", "--json", "--on", "cloud", "--", "C:\\Users\\me\\my project"]),
    ] {
        let host = sessions_host(&[("list", session_list())], &[("sessions", mark.clone())]);
        describe_sessions(cx, &staged, "mods/sessions", layout_values(machine.clone(), folder, platform, cwd), &host, "this.named('Work', cx);");
        let h = host.borrow();
        let key = json!([machine, folder]).to_string();
        let mut expected = mark.clone();
        expected[&key] = json!({"layouts":{"Work":saved_layout(cwd)}});
        assert_eq!(h.shared["sessions"], expected, "{machine}");
        assert_eq!(h.cores.iter().filter(|a| **a == list).count(), 1 + usize::from(machine.is_null()), "one list per folder: {:?}", h.cores);
        assert!(h.cores.iter().all(|a| a[1] == "list"), "saving resumes nothing: {:?}", h.cores);
        drop(h);
        assert_eq!(toasts(&host), vec![json!("Saved layout “Work”")]);
    }

    // An older core lists nothing: the layout is kept, its agents without sessions.
    let host = sessions_host(&[("list", core_error("usage"))], &[]);
    describe_sessions(cx, &staged, "mods/sessions", layout_values(Value::Null, "/fixture/project", "linux", "/fixture/project"), &host, "this.named('Work', cx);");
    let mut expected = saved_layout("/fixture/project");
    expected["tabs"][0]["root"]["children"][0]["session"] = Value::Null;
    expected["tabs"][0]["root"]["children"][1]["children"][0]["session"] = Value::Null;
    assert_eq!(host.borrow().shared["sessions"][json!([null, "/fixture/project"]).to_string()]["layouts"]["Work"], expected);
}

#[gpui::test]
fn sessions_save_layout_of_an_empty_space_saves_nothing(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for tree in [
        json!({"id":"s1", "tabs":[]}),
        json!({"id":"s1", "tabs":[{"id":"tb1", "name":"", "root":{"id":"p5", "type":"block", "content":{"mod":"files"}}}]}),
        json!({"id":"s1", "tabs":[{"id":"tb1", "name":"", "root":{"id":"c1", "type":"row", "children":[]}}]}),
        Value::Null,
    ] {
        let mut values = sessions_values(Value::Null, "/fixture/project", "linux");
        values.insert("tree".into(), tree.clone());
        let host = sessions_host(&[("list", session_list())], &[]);
        describe_sessions(cx, &staged, "mods/sessions", values, &host, "this.named('Work', cx); this.named('', cx);");
        assert!(!host.borrow().shared.contains_key("sessions"), "{tree}");
        let expected = if tree.is_null() { vec![] } else { vec![json!("Nothing to save")] };
        assert_eq!(toasts(&host), expected, "{tree}");
    }
}

#[gpui::test]
fn sessions_save_layout_under_a_taken_name_asks_before_replacing(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let key = json!([null, "/fixture/project"]).to_string();
    let old = json!({ key.clone(): {"layouts":{"Work":{"tabs":[]}, "Other":{"tabs":[]}}} });
    let host = sessions_host(&[("list", session_list())], &[("sessions", old.clone())]);
    describe_sessions(cx, &staged, "mods/sessions", layout_values(Value::Null, "/fixture/project", "linux", "/fixture/project"), &host, "this.named('Work', cx);");
    let h = host.borrow();
    assert_eq!(h.shared["sessions"], old, "nothing replaced before the answer");
    let (_, open) = h.effects.iter().find(|(n, _)| n == "openOverlay").expect("the confirm dialog");
    assert_eq!(open[0]["mod"], "confirm");
    assert_eq!(open[0]["props"]["command"], json!({"name":"sessions.overwrite"}));
    drop(h);

    let host = sessions_host(&[("list", session_list())], &[("sessions", old.clone())]);
    describe_sessions(cx, &staged, "mods/sessions", layout_values(Value::Null, "/fixture/project", "linux", "/fixture/project"), &host, "this.named('Work', cx); this.command('overwrite', cx); this.command('overwrite', cx);");
    let expected = json!({ key: {"layouts":{"Work":saved_layout("/fixture/project"), "Other":{"tabs":[]}}} });
    assert_eq!(host.borrow().shared["sessions"], expected);
    assert_eq!(toasts(&host).len(), 1, "a second overwrite without a name does nothing: {:?}", toasts(&host));
}

#[gpui::test]
fn sessions_confirm_dialogs_keep_their_own_size_now_that_the_default_one_fits_its_content(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // A dialog with a height is not content-fitted; ours size themselves for their longer text.
    let host = sessions_host(&[("list", core_ok(json!({"sessions":[]})))], &[]);
    let setup = format!("{} this.command('delete', cx);", session_target(None, "C:\\Users\\me\\project", "claude", "a1", "C:\\Users\\me\\.claude\\a1.jsonl"));
    describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "C:\\Users\\me\\project", "windows"), &host, &setup);
    let h = host.borrow();
    let (_, open) = h.effects.iter().find(|(n, _)| n == "openOverlay").expect("the delete dialog");
    assert_eq!((open[0]["kind"].clone(), open[0]["width"].clone(), open[0]["height"].clone()), (json!("dialog"), json!(460), json!(210)), "{open:?}");
    assert_eq!(open[0]["props"]["buttons"].as_array().map(Vec::len), Some(2), "{open:?}");
    drop(h);

    let key = json!([null, "/fixture/project"]).to_string();
    let host = sessions_host(&[("list", session_list())], &[("sessions", json!({ key: {"layouts":{"Work":{"tabs":[]}}} }))]);
    describe_sessions(cx, &staged, "mods/sessions", layout_values(Value::Null, "/fixture/project", "linux", "/fixture/project"), &host, "this.named('Work', cx);");
    let h = host.borrow();
    let (_, open) = h.effects.iter().find(|(n, _)| n == "openOverlay").expect("the replace dialog");
    assert_eq!((open[0]["width"].clone(), open[0]["height"].clone()), (json!(420), json!(180)), "{open:?}");
    assert!(open[0]["props"].get("buttons").is_none(), "Replace keeps the default Confirm: {open:?}");
}

#[gpui::test]
fn sessions_layout_restore_rebuilds_tabs_and_splits_resuming_each_session_once(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let key = json!([null, "/fixture/project"]).to_string();
    let mut layout = saved_layout("/fixture/project");
    // Both agents saved with one session: the second starts fresh.
    layout["tabs"][0]["root"]["children"][1]["children"][0]["session"] = json!("a2");
    let argv = json!(["claude", "--resume", "a2"]);
    let host = sessions_host(&[("list", session_list()), ("resume", core_ok(json!({"argv":argv})))], &[("sessions", json!({ key: {"layouts":{"Work":layout}} }))]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "this.restore('Work', cx);");
    assert_eq!(runs(&host), vec![
        json!(["tab.new", {"space":"s1", "name":"Agents", "folder":"/fixture/project", "argv":argv}]),
        json!(["pane.focus", {"pane":"rp1"}]),
        json!(["terminal.open", {"folder":"/fixture/project", "argv":["claude", "--model", "opus"], "place":"right"}]),
        json!(["pane.focus", {"pane":"rp2"}]),
        json!(["terminal.open", {"folder":"/fixture/project/web", "place":"down"}]),
        json!(["tab.new", {"space":"s1", "folder":"/fixture/project"}]),
    ]);
    let h = host.borrow();
    assert_eq!(h.cores.iter().filter(|a| a[1] == "resume").cloned().collect::<Vec<_>>(), vec![vec!["session", "resume", "--json", "--", "claude", "a2"]]);
    assert!(toasts(&host).is_empty(), "{:?}", h.effects);
    assert!(tree.contains("Save layout…") && tree.contains("layout-0") && tree.contains("Work"), "{tree}");
}

#[gpui::test]
fn sessions_layout_restore_starts_a_missing_session_fresh_and_never_stops(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let key = json!([null, "/fixture/project"]).to_string();
    let layout = json!({"tabs":[{"name":"", "root":{"split":"right", "children":[
        {"agent":"claude", "session":"gone", "argv":["claude"], "cwd":"/fixture/project"}, {"cwd":"/fixture/project"}
    ]}}]});
    let plain = json!({"code":2, "stdout":"", "stderr":"error: unrecognized subcommand 'session'\n"});
    for answer in [core_error("not_found"), core_error("usage"), plain] {
        let host = sessions_host(&[("list", session_list()), ("resume", answer.clone())], &[("sessions", json!({ key.clone(): {"layouts":{"Work":layout.clone()}} }))]);
        describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "this.restore('Work', cx);");
        assert_eq!(runs(&host), vec![
            json!(["tab.new", {"space":"s1", "folder":"/fixture/project", "argv":["claude"]}]),
            json!(["pane.focus", {"pane":"rp1"}]),
            json!(["terminal.open", {"folder":"/fixture/project", "place":"right"}]),
        ], "{answer}");
        assert!(toasts(&host).is_empty(), "{answer}");
    }
}

#[gpui::test]
fn sessions_layout_restore_on_another_machine_asks_it_and_starts_at_its_home(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let key = json!(["cloud", ""]).to_string();
    let layout = json!({"tabs":[{"name":"", "root":{"agent":"codex", "session":"c1", "argv":["codex"], "cwd":null}}]});
    let argv = json!(["codex", "resume", "c1"]);
    let host = sessions_host(&[("list", session_list()), ("resume", core_ok(json!({"argv":argv})))], &[("sessions", json!({ key: {"layouts":{"Work":layout}} }))]);
    describe_sessions(cx, &staged, "mods/sessions", sessions_values(json!("cloud"), "", "linux"), &host, "this.restore('Work', cx);");
    assert_eq!(runs(&host), vec![json!(["tab.new", {"space":"s1", "argv":argv}])]);
    assert!(host.borrow().cores.contains(&vec!["session".into(), "resume".into(), "--json".into(), "--on".into(), "cloud".into(), "--".into(), "codex".into(), "c1".into()]));
}

#[gpui::test]
fn sessions_layout_restore_that_never_sees_its_pane_stops_with_a_toast(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let key = json!([null, "/fixture/project"]).to_string();
    let host = sessions_host(&[("list", session_list())], &[("sessions", json!({ key: {"layouts":{"Work":saved_layout("/fixture/project")}} }))]);
    host.borrow_mut().stuck = true;
    describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "this.restore('Work', cx); this.restore('Work', cx);");
    let runs = runs(&host);
    assert_eq!(runs.len(), 1, "one restore at a time, and none past a missing pane: {runs:?}");
    assert_eq!(toasts(&host), vec![json!("Couldn't restore the layout")]);
}

#[gpui::test]
fn sessions_layout_delete_forgets_only_that_layout(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let key = json!([null, "/fixture/project"]).to_string();
    let other = json!(["cloud", "/fixture/project"]).to_string();
    let mark = json!([null, "/fixture/project", "claude", "a1"]).to_string();
    let all = json!({ key.clone(): {"layouts":{"Work":{"tabs":[]}, "Old":{"tabs":[]}}}, other.clone(): {"layouts":{"Old":{"tabs":[]}}}, mark.clone(): {"pinned":true} });
    for (forget, expected) in [
        ("this.forget('Old');", json!({ key.clone(): {"layouts":{"Work":{"tabs":[]}}}, other.clone(): {"layouts":{"Old":{"tabs":[]}}}, mark.clone(): {"pinned":true} })),
        ("this.forget('Old'); this.forget('Work');", json!({ other.clone(): {"layouts":{"Old":{"tabs":[]}}}, mark.clone(): {"pinned":true} })),
        ("this.forget('Missing');", all.clone()),
    ] {
        let host = sessions_host(&[("list", session_list())], &[("sessions", all.clone())]);
        let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, forget);
        assert_eq!(host.borrow().shared["sessions"], expected, "{forget}");
        assert!(tree.contains("layout-delete-0") || forget.contains("Work"), "{tree}");
    }
}

#[gpui::test]
fn sessions_resume_all_agents_replaces_each_stale_agent_pane_once_per_session(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let shell = json!({"key":"shell", "agent":false, "shell":true});
    let agent = json!({"key":"claude", "agent":true, "shell":false});
    let mut values = sessions_values(Value::Null, "C:\\Users\\me\\proj", "windows");
    values.insert("spaces".into(), json!([{"id":"s1","index":1,"name":"Here","named":true,"folder":"C:\\Users\\me\\proj","machine":null,"branch":null,"active":true,"status":null,"tabs":[{"id":"tb1"}]}]));
    values.insert("panes".into(), json!([
        {"pane":"p1", "terminal":"t1", "tab":"tb1", "cwd":"C:\\Users\\me\\proj"},
        {"pane":"p2", "terminal":"t2", "tab":"tb1", "cwd":"C:\\Users\\me\\proj"},
        {"pane":"p3", "terminal":"t3", "tab":"tb1", "cwd":null},
        {"pane":"p4", "terminal":"t4", "tab":"tb1", "cwd":null},
        {"pane":"p9", "terminal":"t9", "tab":"elsewhere", "cwd":"C:\\Users\\me\\proj"}
    ]));
    values.insert("terminals".into(), json!([
        {"address":"t1", "argv":["claude"], "labels":{"h0x.agent":"claude"}, "state":"live", "program":shell},
        {"address":"t2", "argv":["claude"], "labels":{"h0x.agent":"claude"}, "state":"live", "program":agent},
        {"address":"t3", "argv":["claude"], "labels":{"h0x.agent":"claude"}, "state":"exited", "program":agent},
        {"address":"t4", "argv":["pwsh"], "labels":{}, "state":"live", "program":shell},
        {"address":"t9", "argv":["claude"], "labels":{"h0x.agent":"claude"}, "state":"live", "program":shell}
    ]));
    let argv = json!(["claude", "--resume", "x"]);
    let host = sessions_host(&[("list", session_list()), ("resume", core_ok(json!({"argv":argv})))], &[]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", values.clone(), &host, "this.resumeAll(cx);");
    assert!(tree.contains("Resume all agents"), "{tree}");
    assert_eq!(runs(&host), vec![
        json!(["pane.split", {"direction":"right", "pane":"p1", "argv":argv}]),
        json!(["pane.close", {"pane":"p1"}]),
        json!(["pane.split", {"direction":"right", "pane":"p3", "argv":argv}]),
        json!(["pane.close", {"pane":"p3"}]),
    ]);
    let h = host.borrow();
    let resumes: Vec<_> = h.cores.iter().filter(|a| a[1] == "resume").map(|a| a[a.len() - 1].clone()).collect();
    assert_eq!(resumes, vec!["a2", "a1"], "newest unused first, each once");
    assert!(h.cores.iter().filter(|a| a[1] == "list").all(|a| a.last().map(String::as_str) == Some("C:\\Users\\me\\proj")), "{:?}", h.cores);
    drop(h);

    // No stale pane: no button, nothing run.
    values.insert("terminals".into(), json!([{"address":"t2", "argv":["claude"], "labels":{"h0x.agent":"claude"}, "state":"live", "program":agent}]));
    let host = sessions_host(&[("list", session_list())], &[]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", values, &host, "this.resumeAll(cx);");
    assert!(!tree.contains("Resume all agents"), "{tree}");
    assert!(runs(&host).is_empty());
}

// One local claude session "a1" titled "Fix the build" in /fixture/project.
fn one_session() -> Value {
    core_ok(json!({"sessions":[{"agent":"claude", "id":"a1", "title":"Fix the build", "last_used":now_ms() - MINUTE, "path":"/p/a1.jsonl"}]}))
}

#[gpui::test]
fn sessions_panel_shows_relative_time_on_each_side_of_every_boundary(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let now = now_ms();
    let day = 24 * 60 * MINUTE;
    let row = |id: &str, last_used: u64| json!({"agent":"claude", "id":id, "title":format!("Session {id}"), "last_used":last_used, "path":format!("/p/{id}.jsonl")});
    let list = json!({"sessions":[
        row("future", now + 5 * MINUTE), row("59m", now - 59 * MINUTE), row("1h", now - 60 * MINUTE),
        row("23h", now - day + MINUTE), row("1d", now - day), row("29d", now - 29 * day),
        row("1mo", now - 30 * day), row("12mo", now - 364 * day), row("1y", now - 365 * day)
    ]});
    let host = sessions_host(&[("list", core_ok(list))], &[]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "");
    for label in ["now", "59m", "1h", "23h", "1d", "29d", "1mo", "12mo", "1y"] {
        assert!(tree.contains(&format!("text {label:?}")), "{label}: {tree}");
    }
    for wrong in ["60m", "24h", "30d", "0mo", "13mo", "0y", "-5m"] {
        assert!(!tree.contains(&format!("text {wrong:?}")), "{wrong}: {tree}");
    }
}

#[gpui::test]
fn sessions_panel_keeps_a_pin_across_a_refresh(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let now = now_ms();
    let list = core_ok(json!({"sessions":[
        {"agent":"claude", "id":"a2", "title":"Newer", "last_used":now - MINUTE, "path":"/p/a2.jsonl"},
        {"agent":"claude", "id":"a1", "title":"Older", "last_used":now - 2 * MINUTE, "path":"/p/a1.jsonl"}
    ]}));
    let host = sessions_host(&[("list", list)], &[]);
    let setup = format!("{} this.command('pin', cx); this.refresh(cx);", session_target(None, "/fixture/project", "claude", "a1", "/p/a1.jsonl"));
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, &setup);
    let h = host.borrow();
    assert_eq!(h.cores.iter().filter(|a| a[1] == "list").count(), 2, "listed again: {:?}", h.cores);
    assert_eq!(h.shared["sessions"], json!({ json!([null, "/fixture/project", "claude", "a1"]).to_string(): {"pinned":true} }));
    let (older, newer) = (tree.find("Older"), tree.find("Newer"));
    assert!(older.is_some() && newer.is_some() && older < newer, "the pinned older one stays first: {tree}");
}

#[gpui::test]
fn sessions_panel_with_duplicate_ids_marks_per_agent_and_counts_each_shown_row(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let now = now_ms();
    let list = core_ok(json!({"sessions":[
        {"agent":"claude", "id":"a1", "title":"Claude one", "last_used":now - MINUTE, "path":"/p/a1.jsonl"},
        {"agent":"claude", "id":"a1", "title":"Claude two", "last_used":now - 2 * MINUTE, "path":"/p/a1.jsonl"},
        {"agent":"codex", "id":"a1", "title":"Codex one", "last_used":now - 3 * MINUTE, "path":"/p/c.jsonl"}
    ]}));
    let key = json!([null, "/fixture/project"]).to_string();
    let host = sessions_host(&[("list", list.clone())], &[]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "");
    assert!(tree.contains("session-more-2") && tree.contains("Claude two") && tree.contains("Codex one"), "{tree}");
    assert_eq!(host.borrow().shared["sessions.count"], json!({ key.clone(): 3 }));

    // Hiding claude's a1 hides both its rows and never codex's a1.
    let mark = |agent: &str| json!([null, "/fixture/project", agent, "a1"]).to_string();
    let host = sessions_host(&[("list", list)], &[("sessions", json!({ mark("claude"): {"hidden":true} }))]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "");
    assert!(!tree.contains("Claude one") && !tree.contains("Claude two") && tree.contains("Codex one"), "{tree}");
    assert_eq!(host.borrow().shared["sessions.count"], json!({ key: 1 }));
}

#[gpui::test]
fn sessions_panel_renders_five_hundred_sessions(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let now = now_ms();
    let sessions: Vec<Value> = (0..500u64)
        .map(|i| json!({"agent":"claude", "id":format!("s{i}"), "title":format!("Session number {i}"), "last_used":now - i * MINUTE, "path":format!("/p/s{i}.jsonl")}))
        .collect();
    let host = sessions_host(&[("list", core_ok(json!({ "sessions": sessions })))], &[]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "");
    // ADR-056 addendum: a frame draws the newest 50 rows, then "Show N more".
    assert!(tree.contains("session-more-49") && tree.contains("Session number 49") && !tree.contains("session-more-50") && !tree.contains("Session number 50"), "the newest 50");
    assert!(tree.contains("Show 450 more") && tree.contains("text \"500\""), "the rest stays reachable, and the count is all of them");
    let h = host.borrow();
    assert_eq!(h.cores.len(), 1, "one list for all of them: {:?}", h.cores);
    assert_eq!(h.shared["sessions.count"], json!({ json!([null, "/fixture/project"]).to_string(): 500 }));
}

#[gpui::test]
fn sessions_panel_survives_corrupt_shared_marks_layouts_and_counts(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mark = json!([null, "/fixture/project", "claude", "a1"]).to_string();
    let key = json!([null, "/fixture/project"]).to_string();
    for corrupt in [
        Value::Null,
        json!([]),
        json!(["x"]),
        json!("text"),
        json!(42),
        json!({ mark.clone(): null }),
        json!({ mark.clone(): "x" }),
        json!({ mark.clone(): [1] }),
        json!({ key.clone(): null }),
        json!({ key.clone(): "x" }),
        json!({ key.clone(): {"layouts":null} }),
        json!({ key.clone(): {"layouts":[]} }),
    ] {
        let host = sessions_host(&[("list", one_session())], &[("sessions", corrupt.clone()), ("sessions.count", corrupt.clone())]);
        let setup = format!("{} this.command('pin', cx);", session_target(None, "/fixture/project", "claude", "a1", "/p/a1.jsonl"));
        let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, &setup);
        assert!(tree.contains("Fix the build") && tree.contains("Save layout…"), "{corrupt}: {tree}");
        let h = host.borrow();
        assert_eq!(h.shared["sessions"][&mark]["pinned"], json!(true), "{corrupt}");
        assert_eq!(h.shared["sessions.count"][&key], json!(1), "{corrupt}");
    }

    // The spaces list shows no badge for a count it can't read.
    for corrupt in [Value::Null, json!([]), json!("abc"), json!(7)] {
        let host = sessions_host(&[], &[("sessions.count", corrupt.clone())]);
        let tree = describe_sessions(cx, &staged, "mods/spaces", fixture(true, props(), updates("current", "current")), &host, "");
        assert!(tree.contains("space-s1") && !tree.contains("space-sessions-"), "{corrupt}: {tree}");
    }
}

#[gpui::test]
fn sessions_panel_blank_rename_falls_back_to_the_title_and_keeps_the_pin(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mark = json!([null, "/fixture/project", "claude", "a1"]).to_string();
    for (before, after) in [
        (json!({ mark.clone(): {"pinned":true, "name":"Mine"} }), json!({ mark.clone(): {"pinned":true} })),
        (json!({ mark.clone(): {"name":"Mine"} }), json!({})),
    ] {
        let host = sessions_host(&[("list", one_session())], &[("sessions", before.clone())]);
        // What the rename box's submit stores for "   ".
        let setup = format!("{} this.setMark(this.target, {{ name: '   '.trim() }});", session_target(None, "/fixture/project", "claude", "a1", "/p/a1.jsonl"));
        let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, &setup);
        assert_eq!(host.borrow().shared["sessions"], after, "{before}");
        assert!(tree.contains("Fix the build") && !tree.contains("Mine"), "{tree}");
    }
}

#[gpui::test]
fn sessions_panel_delete_then_cancel_and_commands_without_a_session_change_nothing(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let marks = json!({ json!([null, "/fixture/project", "claude", "a1"]).to_string(): {"pinned":true} });
    let replies = [("list", one_session()), ("trash", core_ok(json!({"trashed":[]}))), ("resume", core_ok(json!({"argv":["claude"]})))];

    // The dialog only opens; Cancel closes it in the confirm mod and sends nothing back.
    let host = sessions_host(&replies, &[("sessions", marks.clone())]);
    let setup = format!("{} this.command('delete', cx);", session_target(None, "/fixture/project", "claude", "a1", "/p/a1.jsonl"));
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, &setup);
    let h = host.borrow();
    assert_eq!(h.shared["sessions"], marks);
    assert!(h.cores.iter().all(|a| a[1] == "list"), "{:?}", h.cores);
    assert_eq!(h.effects.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["openOverlay"]);
    assert!(tree.contains("Fix the build"), "{tree}");
    drop(h);

    // No session picked yet (no ⋯ opened): every command is ignored.
    let host = sessions_host(&replies, &[("sessions", marks.clone())]);
    let setup = "for (const c of ['resume', 'resume-tab', 'pin', 'unpin', 'rename', 'copy', 'reveal', 'delete', 'hide', 'trash', 'overwrite', 'bogus', '']) this.command(c, cx);";
    describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, setup);
    let h = host.borrow();
    assert_eq!(h.shared["sessions"], marks);
    assert!(h.cores.iter().all(|a| a[1] == "list"), "{:?}", h.cores);
    assert!(h.effects.is_empty() && h.execs.is_empty(), "{:?} {:?}", h.effects, h.execs);
}

#[gpui::test]
fn sessions_menu_without_a_target_and_a_dialog_without_buttons_render(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p.as_object_mut().unwrap().extend(json!({"tab":"session"}).as_object().unwrap().clone());
    let tree = describe(cx, &staged, "mods/menu", true, p, updates("current", "current"), None).unwrap();
    for label in ["Resume", "Resume in new tab", "Pin", "Rename", "Copy resume command", "Delete"] {
        assert!(tree.contains(label), "{label}: {tree}");
    }
    assert!(!tree.contains("Unpin") && !tree.contains("Show file"), "an unknown session is neither pinned nor local: {tree}");

    let mut p = props();
    p.as_object_mut().unwrap().extend(json!({"title":"Delete session?", "detail":"Fixture detail", "buttons":[]}).as_object().unwrap().clone());
    let tree = describe(cx, &staged, "mods/confirm", true, p, updates("current", "current"), None).unwrap();
    assert!(tree.contains("Cancel") && !tree.contains("Confirm") && !tree.contains("choice-0"), "{tree}");
}

#[gpui::test]
fn terminal_menu_offers_pane_items_only_for_a_pane_in_a_tab_and_never_on_a_session_row(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let pane_items = ["Split right", "Split down", "Zoom", "Close pane"];
    let items = |tree: &str| (0..8).filter(|i| tree.contains(&format!(":id[Str(\"mi-{i}\")]"))).count();
    let menu = |cx: &mut TestAppContext, populated: bool, target: Value, kept: bool| {
        let mut p = props();
        p["tab"] = json!("pane");
        p["target"] = target;
        let mut values = fixture(populated, p, updates("current", "current"));
        if kept {
            // A kept sub-agent of t1, shown in a window: no pane of its own in any tab.
            values.get_mut("terminals").unwrap().as_array_mut().unwrap().push(json!({"address":"t3","name":"Helper","title":"Helper","argv":["pi"],"parent":"t1","labels":{},"state":"live","status":"working","report":{"app":"pi"},"program":null,"cwd":"/fixture/project"}));
        }
        describe_with_values(cx, &staged, "mods/menu", values, None).unwrap()
    };

    // A pane in a tab, local or on the remote machine: every pane item.
    for (pane, terminal) in [("p1", "t1"), ("p2", "t2")] {
        let tree = menu(cx, true, json!({"pane":pane, "terminal":terminal}), false);
        for label in pane_items.iter().chain(["Send right-clicks to pane"].iter()) {
            assert!(tree.contains(label), "{pane}: {label}: {tree}");
        }
        assert_eq!(items(&tree), 5, "{pane}: {tree}");
    }

    // A terminal block in a layout or a window: its pane is the block's id, in no tab.
    let tree = menu(cx, true, json!({"pane":"term-block", "terminal":"t1"}), false);
    for label in pane_items {
        assert!(!tree.contains(label), "{label}: {tree}");
    }
    assert!(tree.contains("Send right-clicks to pane"), "the terminal's own item stays: {tree}");
    assert_eq!(items(&tree), 1, "{tree}");

    // A kept sub-agent in a window still goes back under its parent.
    let tree = menu(cx, true, json!({"pane":"w1:term", "terminal":"t3"}), true);
    assert!(tree.contains("Send back to Build") && !tree.contains("Close pane") && !tree.contains("Zoom"), "{tree}");
    assert_eq!(items(&tree), 2, "{tree}");

    // No panes at all, no terminal, no pane id: nothing to offer, and it still renders.
    for target in [json!({"pane":"p1", "terminal":"missing"}), json!({})] {
        let tree = menu(cx, false, target.clone(), false);
        assert_eq!(items(&tree), 0, "{target}: {tree}");
        assert!(tree.contains(":id[Str(\"menu\")]"), "{tree}");
    }

    // The sessions row menu has no pane, and none of the pane items.
    let mut p = props();
    p["tab"] = json!("session");
    p["target"] = json!({"pinned":false, "local":true});
    let tree = describe(cx, &staged, "mods/menu", true, p, updates("current", "current"), None).unwrap();
    for label in pane_items {
        assert!(!tree.contains(label), "{label}: {tree}");
    }
    for label in ["Resume", "Resume in new tab", "Pin", "Rename", "Copy resume command", "Show file", "Delete"] {
        assert!(tree.contains(label), "{label}: {tree}");
    }
}

#[gpui::test]
fn sessions_save_layout_keeps_a_unicode_name_with_quotes_and_restores_it(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let name = "Wörk \"α\" 'β' ✓ 日本";
    let key = json!([null, "/fixture/project"]).to_string();
    let host = sessions_host(&[("list", session_list())], &[]);
    let setup = format!("this.named({}, cx);", json!(name));
    let tree = describe_sessions(cx, &staged, "mods/sessions", layout_values(Value::Null, "/fixture/project", "linux", "/fixture/project"), &host, &setup);
    assert_eq!(host.borrow().shared["sessions"][&key]["layouts"][name], saved_layout("/fixture/project"));
    assert_eq!(toasts(&host), vec![json!(format!("Saved layout “{name}”"))]);
    assert!(tree.contains("日本") && tree.contains("layout-0"), "{tree}");

    let layout = json!({"tabs":[{"name":name, "root":{"cwd":"/fixture/project"}}]});
    let host = sessions_host(&[("list", session_list())], &[("sessions", json!({ key: {"layouts":{ name: layout }} }))]);
    let setup = format!("this.restore({}, cx);", json!(name));
    describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, &setup);
    assert_eq!(runs(&host), vec![json!(["tab.new", {"space":"s1", "name":name, "folder":"/fixture/project"}])]);
}

#[gpui::test]
fn sessions_save_layout_under_a_new_name_that_objects_already_have_saves_without_asking(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let key = json!([null, "/fixture/project"]).to_string();
    for name in ["constructor", "toString", "__proto__"] {
        let host = sessions_host(&[("list", session_list())], &[]);
        let setup = format!("this.named({}, cx);", json!(name));
        describe_sessions(cx, &staged, "mods/sessions", layout_values(Value::Null, "/fixture/project", "linux", "/fixture/project"), &host, &setup);
        let h = host.borrow();
        assert!(!h.effects.iter().any(|(n, _)| n == "openOverlay"), "{name} is not saved here yet: {:?}", h.effects);
        assert_eq!(h.shared.get("sessions").map(|s| s[&key]["layouts"][name].clone()), Some(saved_layout("/fixture/project")), "{name}");
    }
}

#[gpui::test]
fn sessions_layout_restore_of_an_empty_layout_runs_nothing_and_frees_the_next_restore(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let key = json!([null, "/fixture/project"]).to_string();
    let layouts = json!({"Empty":{"tabs":[]}, "Bare":{}, "Work":{"tabs":[{"name":"", "root":{"cwd":"/fixture/project"}}]}});
    let host = sessions_host(&[("list", session_list())], &[("sessions", json!({ key: {"layouts":layouts} }))]);
    let setup = "this.restore('Empty', cx); this.restore('Missing', cx); cx.spawn(async (cx) => { await cx.sleep(100); this.restore('Bare', cx); await cx.sleep(100); this.restore('Work', cx); });";
    describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, setup);
    assert_eq!(runs(&host), vec![json!(["tab.new", {"space":"s1", "folder":"/fixture/project"}])]);
    assert!(toasts(&host).is_empty(), "{:?}", host.borrow().effects);
    assert!(host.borrow().cores.iter().all(|a| a[1] == "list"), "{:?}", host.borrow().cores);
}

#[gpui::test]
fn sessions_layout_restore_passes_windows_spaced_and_dash_paths_and_ids_as_is(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let argv = json!(["claude", "--resume", "-a1"]);
    for (machine, folder, platform) in [(Value::Null, "C:\\Users\\me\\my project", "windows"), (json!("cloud"), "/srv/my app", "linux")] {
        let key = json!([machine, folder]).to_string();
        let layout = json!({"tabs":[{"name":"", "root":{"split":"right", "children":[
            {"agent":"claude", "session":"-a1", "argv":["claude"], "cwd":folder}, {"cwd":"-odd dir"}
        ]}}]});
        let host = sessions_host(&[("list", session_list()), ("resume", core_ok(json!({"argv":argv})))], &[("sessions", json!({ key: {"layouts":{"Work":layout}} }))]);
        describe_sessions(cx, &staged, "mods/sessions", sessions_values(machine.clone(), folder, platform), &host, "this.restore('Work', cx);");
        assert_eq!(runs(&host), vec![
            json!(["tab.new", {"space":"s1", "folder":folder, "argv":argv}]),
            json!(["pane.focus", {"pane":"rp1"}]),
            json!(["terminal.open", {"folder":"-odd dir", "place":"right"}]),
        ], "{machine}");
        let on: Vec<String> = machine.as_str().map(|m| vec!["--on".into(), m.into()]).unwrap_or_default();
        let resume = [vec!["session".to_string(), "resume".into(), "--json".into()], on, vec!["--".into(), "claude".into(), "-a1".into()]].concat();
        assert_eq!(host.borrow().cores.iter().filter(|a| a[1] == "resume").cloned().collect::<Vec<_>>(), vec![resume], "{machine}");
    }
}

#[gpui::test]
fn sessions_resume_all_agents_ignores_panes_without_an_agent_label(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let shell = json!({"key":"shell", "agent":false, "shell":true});
    let mut values = sessions_values(Value::Null, "/fixture/project", "linux");
    values.insert("spaces".into(), json!([{"id":"s1","index":1,"name":"Here","named":true,"folder":"/fixture/project","machine":null,"branch":null,"active":true,"status":null,"tabs":[{"id":"tb1"}]}]));
    values.insert("panes".into(), json!([
        {"pane":"p1", "terminal":"t1", "tab":"tb1", "cwd":"/fixture/project"},
        {"pane":"p2", "terminal":"t2", "tab":"tb1", "cwd":"/fixture/project"},
        {"pane":"p3", "terminal":"t3", "tab":"tb1", "cwd":"/fixture/project"},
        {"pane":"p4", "terminal":null, "mod":"files", "tab":"tb1"},
        {"pane":"p5", "terminal":"gone", "tab":"tb1", "cwd":"/fixture/project"}
    ]));
    values.insert("terminals".into(), json!([
        {"address":"t1", "argv":["claude"], "state":"exited", "program":shell},
        {"address":"t2", "argv":["claude"], "labels":null, "state":"exited", "program":null},
        {"address":"t3", "argv":["claude"], "labels":{"h0x.agent":""}, "state":"live", "program":shell}
    ]));
    let host = sessions_host(&[("list", session_list()), ("resume", core_ok(json!({"argv":["claude"]})))], &[]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", values, &host, "this.resumeAll(cx);");
    assert!(!tree.contains("Resume all agents"), "{tree}");
    assert!(runs(&host).is_empty());
    assert!(host.borrow().cores.iter().all(|a| a[1] == "list"), "{:?}", host.borrow().cores);
}

#[gpui::test]
fn sessions_panel_shows_no_time_for_a_missing_or_unreadable_last_used(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let list = json!({"sessions":[
        {"agent":"claude", "id":"a1", "title":"No time", "path":"/p/a1.jsonl"},
        {"agent":"claude", "id":"a2", "title":"Null time", "last_used":null, "path":"/p/a2.jsonl"},
        {"agent":"claude", "id":"a3", "title":"Text time", "last_used":"yesterday", "path":"/p/a3.jsonl"}
    ]});
    let host = sessions_host(&[("list", core_ok(list))], &[]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "");
    for title in ["No time", "Null time", "Text time"] {
        assert!(tree.contains(title), "{title}: {tree}");
    }
    assert!(!tree.contains("NaN") && !tree.contains("text \"now\""), "{tree}");
    assert_eq!(host.borrow().shared["sessions.count"], json!({ json!([null, "/fixture/project"]).to_string(): 3 }));
}

#[gpui::test]
fn sessions_panel_beyond_its_row_limit_offers_the_rest_and_keeps_pins(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let now = now_ms();
    // ADR-056 addendum: pinned rows plus the newest 50.
    let sessions: Vec<Value> = (0..103u64)
        .map(|i| json!({"agent":"claude", "id":format!("s{i}"), "title":format!("Row {i} end"), "last_used":now - i * MINUTE, "path":"/p/s.jsonl"}))
        .collect();
    let pinned = json!({ json!([null, "/fixture/project", "claude", "s102"]).to_string(): {"pinned":true} });
    let host = sessions_host(&[("list", core_ok(json!({ "sessions": sessions })))], &[("sessions", pinned)]);
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, "");
    assert!(tree.contains("Row 102 end") && tree.contains("Row 49 end") && !tree.contains("Row 50 end") && !tree.contains("Row 101 end"), "the pin, then the newest 50");
    assert!(tree.contains("Show 52 more"), "the rest stays reachable");
    assert_eq!(host.borrow().shared["sessions.count"], json!({ json!([null, "/fixture/project"]).to_string(): 103 }), "the count is every shown session");
}

#[gpui::test]
fn sessions_layout_restore_skips_malformed_parts_and_frees_the_next_restore(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let key = json!([null, "/fixture/project"]).to_string();
    let layouts = json!({
        "Null":{"tabs":[{"name":"", "root":null}]},
        "Text":{"tabs":"x"},
        "Junk":"x",
        "Mixed":{"tabs":[null, 5, {"name":7, "root":{"split":"down", "children":[null, {"cwd":"/a"}, "x"]}}, {"root":{"agent":5, "cwd":7, "argv":"claude"}}]}
    });
    let host = sessions_host(&[("list", session_list())], &[("sessions", json!({ key: {"layouts":layouts} }))]);
    let setup = "this.restore('Null', cx); cx.spawn(async (cx) => { await cx.sleep(100); this.restore('Text', cx); this.restore('Junk', cx); await cx.sleep(100); this.restore('Mixed', cx); });";
    describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, setup);
    assert_eq!(runs(&host), vec![
        json!(["tab.new", {"space":"s1", "folder":"/a"}]),
        json!(["tab.new", {"space":"s1", "folder":"/fixture/project"}]),
    ]);
    assert!(toasts(&host).is_empty(), "{:?}", host.borrow().effects);
    assert!(host.borrow().cores.iter().all(|a| a[1] == "list"), "{:?}", host.borrow().cores);
}

#[gpui::test]
fn sessions_save_layout_over_non_object_stored_values_keeps_no_junk(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let key = json!([null, "/fixture/project"]).to_string();
    let other = json!(["cloud", "/srv"]).to_string();
    for (stored, kept) in [
        (json!("text"), json!({})),
        (json!([1, 2]), json!({})),
        (json!(42), json!({})),
        (json!({ key.clone(): "x", other.clone(): {"layouts":{"Old":{"tabs":[]}}} }), json!({ other.clone(): {"layouts":{"Old":{"tabs":[]}}} })),
        (json!({ key.clone(): {"layouts":"x"} }), json!({})),
        (json!({ key.clone(): {"layouts":[{"tabs":[]}]} }), json!({})),
    ] {
        let host = sessions_host(&[("list", session_list())], &[("sessions", stored.clone())]);
        describe_sessions(cx, &staged, "mods/sessions", layout_values(Value::Null, "/fixture/project", "linux", "/fixture/project"), &host, "this.named('Work', cx);");
        let mut expected = kept;
        expected[&key] = json!({"layouts":{"Work":saved_layout("/fixture/project")}});
        assert_eq!(host.borrow().shared["sessions"], expected, "{stored}");
    }
}

// ADR-056: a stored map whose keys are named like Object.prototype members reaches a mod as own keys.
#[gpui::test]
fn sessions_layouts_named_like_prototype_members_round_trip(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let key = json!([null, "/fixture/project"]).to_string();
    let layout = json!({"tabs":[{"name":"", "root":{"cwd":"/fixture/project"}}]});
    let stored = json!({ key.clone(): {"layouts":{"constructor":layout, "toString":layout, "__proto__":layout}} });
    let host = sessions_host(&[("list", session_list())], &[("sessions", stored.clone())]);
    let setup = "this.forget('toString'); this.restore('__proto__', cx);";
    let tree = describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", "linux"), &host, setup);
    assert!(tree.contains("constructor") && tree.contains("__proto__") && !tree.contains("toString") && tree.contains("layout-1"), "{tree}");
    let mut expected = stored;
    expected[&key]["layouts"].as_object_mut().unwrap().remove("toString");
    assert_eq!(host.borrow().shared["sessions"], expected);
    assert_eq!(runs(&host), vec![json!(["tab.new", {"space":"s1", "folder":"/fixture/project"}])]);
}

#[gpui::test]
fn sessions_count_badge_needs_a_finite_number(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let at = json!([null, "/fixture/project"]).to_string();
    for (value, badge) in [(json!("3"), false), (Value::Null, false), (json!(true), false), (json!([3]), false), (json!({"n":3}), false), (json!(0), false), (json!(3), true)] {
        let host = sessions_host(&[], &[("sessions.count", json!({ at.clone(): value.clone() }))]);
        let tree = describe_sessions(cx, &staged, "mods/spaces", fixture(true, props(), updates("current", "current")), &host, "");
        assert_eq!(tree.contains("space-sessions-s1"), badge, "{value}: {tree}");
    }
}

#[gpui::test]
fn sessions_panel_passes_a_dash_machine_name_as_one_word(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let argv = json!(["claude", "--resume", "a1"]);
    let replies = [("list", core_ok(json!({"sessions":[]}))), ("resume", core_ok(json!({"argv":argv})))];
    let host = sessions_host(&replies, &[]);
    let setup = format!("{} this.command('resume-tab', cx);", session_target(Some("-x"), "/srv", "claude", "a1", "/p/a1.jsonl"));
    describe_sessions(cx, &staged, "mods/sessions", sessions_values(json!("-x"), "/srv", "linux"), &host, &setup);
    let h = host.borrow();
    assert!(h.cores.contains(&vec!["session".into(), "list".into(), "--json".into(), "--on=-x".into(), "--".into(), "/srv".into()]), "{:?}", h.cores);
    assert!(h.cores.contains(&vec!["session".into(), "resume".into(), "--json".into(), "--on=-x".into(), "--".into(), "claude".into(), "a1".into()]), "{:?}", h.cores);
    assert!(!h.cores.iter().flatten().any(|w| w == "--on"), "{:?}", h.cores);
}

#[gpui::test]
fn tab_bar_adds_sessions_to_a_stored_panel_list_once(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // The tab bar shown once over `stored`: its description and the shared values after.
    fn tabs(cx: &mut TestAppContext, staged: &Staged, stored: &[(&str, Value)]) -> (String, BTreeMap<String, Value>) {
        let host = sessions_host(&[], stored);
        let tree = describe_sessions(cx, staged, "mods/tabs", fixture(true, props(), updates("current", "current")), &host, "");
        let shared = host.borrow().shared.clone();
        (tree, shared)
    }
    let added = json!(["sessions"]);
    for (before, after) in [
        (json!(["files", "changes"]), json!(["files", "changes", "sessions"])),
        // The welcome's "nothing picked yet".
        (json!([]), json!(["sessions"])),
        (json!(["sessions", "files"]), json!(["sessions", "files"])),
    ] {
        let (tree, shared) = tabs(cx, &staged, &[("workspace.panels", before.clone())]);
        assert_eq!(shared["workspace.panels"], after, "{before}");
        assert_eq!(shared["workspace.panels.added"], added, "{before}");
        assert!(tree.contains("panel-sessions"), "{before}: {tree}");
        // Shown again with what it stored: nothing changes.
        let (_, again) = tabs(cx, &staged, &[("workspace.panels", after.clone()), ("workspace.panels.added", added.clone())]);
        assert_eq!(again, shared, "{before}");
    }

    // Turned off after it was added: stays off.
    for off in [json!(["files", "changes"]), json!([])] {
        let (tree, shared) = tabs(cx, &staged, &[("workspace.panels", off.clone()), ("workspace.panels.added", added.clone())]);
        assert_eq!(shared["workspace.panels"], off);
        assert_eq!(shared["workspace.panels.added"], added);
        assert!(!tree.contains("panel-sessions"), "{off}: {tree}");
    }

    // A fresh install stores nothing and shows all three.
    let (tree, shared) = tabs(cx, &staged, &[]);
    assert!(shared.is_empty(), "{shared:?}");
    assert!(tree.contains("panel-files") && tree.contains("panel-changes") && tree.contains("panel-sessions"), "{tree}");

    // The welcome's own list already offers Sessions: the tab bar adds nothing to it.
    let host = sessions_host(&[], &[]);
    let tree = describe_sessions(cx, &staged, "frame/welcome", fixture(true, props(), updates("current", "current")), &host, "this.page = 3;");
    assert!(tree.contains("Choose your panels") && tree.contains("Sessions"), "{tree}");
    let welcome = host.borrow().shared.clone();
    assert_eq!(welcome.get("workspace.panels"), Some(&json!([])));
    assert_eq!(welcome.get("workspace.panels.added"), Some(&added));
    let picked: Vec<(&str, Value)> = vec![("workspace.panels", json!(["files"])), ("workspace.panels.added", added.clone())];
    let (tree, shared) = tabs(cx, &staged, &picked);
    assert_eq!(shared["workspace.panels"], json!(["files"]));
    assert!(!tree.contains("panel-sessions"), "{tree}");
}

//! Shipped JS descriptions and native rendering regressions.
use super::*;
use gpui::TestAppContext;
use gpui_shell::{ShellRuntime, plugin::DescriptionSession, policy::Policy};
use std::{
    cell::Cell,
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
    /// `exec` calls (program, then its arguments) while replies are scripted; they answer like `core`.
    execs: Vec<Vec<String>>,
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
        ("revision".into(), json!(0)),
        ("windowVisible".into(), json!(true)),
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
    for name in ["run", "playSound", "setAgentOrder", "openOverlay", "closeOverlay", "notify", "presentUpdateNotes", "openLink", "copyText"] {
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
            // Never opens anything: answers as the real one would.
            Ok(if name == "openLink" && !a.string(0).is_ok_and(allowed_link) {
                "that link is not allowed".into()
            } else if name == "openOverlay" || name == "notify" {
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
                // The to-do panel's first run: no lists yet, so it creates the index and its first list.
                "readTextFile" if first == "/fixture/app/todo/lists.md" || first == "/fixture/app/todo/tasks.md" => Some(Value::Null),
                "writeTextFile" if first == "/fixture/app/todo/lists.md" || first == "/fixture/app/todo/tasks.md" => Some(Value::Null),
                "readTextFile" => first
                    .as_str()
                    .and_then(|p| p.strip_prefix("/fixture/app/layouts/"))
                    .and_then(|p| p.strip_suffix(".json"))
                    .filter(|n| crate::window::shipped_layouts().contains(n))
                    .map(|_| json!("{}")),
                "core" if first == json!(["machine", "ls", "--json"]) => {
                    Some(json!({"code":0,"stdout":"{\"machines\":[{\"name\":\"remote\",\"state\":\"online\"}]}","stderr":""}))
                }
                "core" if first == json!(["machine", "add", "host", "--json"]) => {
                    Some(json!({"code":1,"stdout":"{\"error\":{\"kind\":\"host_key\",\"message\":\"unknown host key\"}}","stderr":""}))
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

#[gpui::test]
fn mod_copies_share_code_without_sharing_props_state_or_lifetime(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("cache");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.js"), r#"
import { View, div } from 'gpui-kit';
import { props, mark, next } from 'copy';
const evaluation = mark();
const row = globalThis.__template(value => div().child(value));
export default class Panel extends View {
  init(_, cx) {
    if (props() === 'bad') throw Error('bad props');
    this.count = 0;
    this.count++;
    this.async = 'waiting';
    cx.spawn(async cx => { await next(); this.async = props(); cx.notify(); });
  }
  render() { return row(props() + ':' + this.count + ':' + evaluation + ':' + this.async); }
}
"#).unwrap();
    let marks = Rc::new(Cell::new(0));
    let policy = |name: &'static str| {
        let marks = marks.clone();
        Policy::new().with_host_module(HostModule::new("copy")
            .function("props", move |_| Ok(name.into()))
            .function("mark", move |_| { marks.set(marks.get() + 1); Ok((marks.get() as f64).into()) })
            .async_function("next", |_| Ok(std::future::ready(Ok(HostValue::Null))))
        ).unwrap()
    };
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let (first, mut second) = window.update(cx, |_, window, cx| {
        let mut first = DescriptionSession::load(runtime.clone(), &root, policy("A"), window, cx).unwrap();
        let mut second = DescriptionSession::load(runtime.clone(), &root, policy("B"), window, cx).unwrap();
        assert!(first.describe(window, cx).unwrap().contains("A:1:1"));
        assert!(second.describe(window, cx).unwrap().contains("B:1:1"));
        assert!(DescriptionSession::load(runtime.clone(), &root, policy("bad"), window, cx).is_err());
        (first, second)
    }).unwrap();
    cx.run_until_parked();
    drop(first);
    cx.run_until_parked();
    window.update(cx, |_, window, cx| {
        let tree = second.describe(window, cx).unwrap();
        assert!(tree.contains("B:1:1:B"), "{tree}");
    }).unwrap();
    drop(second);
    cx.run_until_parked();
    window.update(cx, |_, window, cx| {
        let mut reopened = DescriptionSession::load(runtime.clone(), &root, policy("C"), window, cx).unwrap();
        assert!(reopened.describe(window, cx).unwrap().contains("C:1:1"));
    }).unwrap();
    assert_eq!(marks.get(), 1, "reopening and failed init must reuse evaluated code");
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

#[gpui::test]
fn mod_code_revisions_share_successes_and_failures_without_poisoning_old_copies(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("cache");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.js"), r#"
import { View, div } from 'gpui-kit';
import { version } from './part.js';
export default class Panel extends View { render() { return div().child('version:' + version); } }
"#).unwrap();
    std::fs::write(root.join("part.js"), "import { mark } from 'copy'; mark(); export const version = 1;").unwrap();
    let marks = Rc::new(Cell::new(0));
    let policy = || {
        let marks = marks.clone();
        Policy::new().with_host_module(HostModule::new("copy").function("mark", move |_| {
            marks.set(marks.get() + 1); Ok(HostValue::Null)
        })).unwrap()
    };
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        let mut old = DescriptionSession::load(runtime.clone(), &root, policy(), window, cx).unwrap();
        for version in [2, 3] {
            std::fs::write(root.join("part.js"), format!("import {{ mark }} from 'copy'; mark(); export const version = {version};{}", " ".repeat(version))).unwrap();
            for _ in 0..20 {
                let mut new = DescriptionSession::load(runtime.clone(), &root, policy(), window, cx).unwrap();
                assert!(new.describe(window, cx).unwrap().contains(&format!("version:{version}")));
            }
        }
        assert_eq!(marks.get(), 3);
        std::fs::write(root.join("part.js"), "import { mark } from 'copy'; mark(); throw Error('broken save'); export const version = 4;").unwrap();
        for _ in 0..20 {
            assert!(DescriptionSession::load(runtime.clone(), &root, policy(), window, cx).is_err());
        }
        assert_eq!(marks.get(), 4, "a failed revision evaluates once, not on every open");
        assert!(old.describe(window, cx).unwrap().contains("version:1"));
        std::fs::write(root.join("part.js"), "import { mark } from 'copy'; mark(); export const version = 5;").unwrap();
        let mut fixed = DescriptionSession::load(runtime.clone(), &root, policy(), window, cx).unwrap();
        assert!(fixed.describe(window, cx).unwrap().contains("version:5"));
        assert_eq!(marks.get(), 5);
        assert!(DescriptionSession::load(runtime.clone(), &root, Policy::new(), window, cx).is_err(),
            "an ungranted host module must not reuse another policy's linked class");
    }).unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

#[gpui::test]
fn mod_host_imports_keep_sync_async_and_missing_export_surfaces_distinct(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("cache");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.js"), r#"
import { View, div } from 'gpui-kit';
import { value } from 'copy';
export default class Panel extends View {
  init() { this.kind = typeof value(); }
  render() { return div().child('kind:' + this.kind); }
}
"#).unwrap();
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        let sync = HostModule::new("copy").function("value", |_| Ok(HostValue::Number(1.0)));
        let mut a = DescriptionSession::load(runtime.clone(), &root, Policy::new().with_host_module(sync).unwrap(), window, cx).unwrap();
        assert!(a.describe(window, cx).unwrap().contains("kind:number"));
        let asynchronous = HostModule::new("copy").async_function("value", |_| Ok(std::future::ready(Ok(HostValue::Null))));
        let mut b = DescriptionSession::load(runtime.clone(), &root, Policy::new().with_host_module(asynchronous).unwrap(), window, cx).unwrap();
        assert!(b.describe(window, cx).unwrap().contains("kind:object"));
        let missing = HostModule::new("copy").function("other", |_| Ok(HostValue::Null));
        assert!(DescriptionSession::load(runtime.clone(), &root, Policy::new().with_host_module(missing).unwrap(), window, cx).is_err());
        assert!(a.describe(window, cx).unwrap().contains("kind:number"));
    }).unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx: the shared code of one mod is never another mod's. Each root keeps its own module
// state, and a file imported by one mod can't be reached by the other's cache entry.
#[gpui::test]
fn different_mods_never_share_module_state_and_a_copy_of_each_shares_its_own(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let source = r#"
import { View, div } from 'gpui-kit';
import { mark } from 'copy';
import { tag } from './part.js';
const evaluation = mark();
export default class Panel extends View {
  render() { return div().child(tag + ':' + evaluation); }
}
"#;
    let roots = ["one", "two"].map(|name| {
        let root = staged.0.join(name);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("main.js"), source).unwrap();
        std::fs::write(root.join("part.js"), format!("export const tag = '{name}';")).unwrap();
        root
    });
    let marks = Rc::new(Cell::new(0));
    let policy = || {
        let marks = marks.clone();
        Policy::new().with_host_module(HostModule::new("copy").function("mark", move |_| {
            marks.set(marks.get() + 1);
            Ok((marks.get() as f64).into())
        })).unwrap()
    };
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        let mut sessions = Vec::new();
        for _ in 0..3 {
            for root in &roots {
                sessions.push(DescriptionSession::load(runtime.clone(), root, policy(), window, cx).unwrap());
            }
        }
        // Six copies of two mods evaluate the code twice: once per mod.
        assert_eq!(marks.get(), 2, "copies of one mod share its evaluated code; two mods do not");
        for (index, session) in sessions.iter_mut().enumerate() {
            let tree = session.describe(window, cx).unwrap();
            let (tag, evaluation) = if index % 2 == 0 { ("one", 1) } else { ("two", 2) };
            assert!(tree.contains(&format!("{tag}:{evaluation}")), "{index}: {tree}");
        }
    }).unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx: the editor helpers at a mod's root are never code (the consent hash leaves the generated
// ones out), and sharing evaluated code between copies must not open that door again.
#[gpui::test]
fn a_mod_still_cannot_import_the_editor_helpers_after_code_sharing(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("helpers");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("part.js"), "export const value = 1;").unwrap();
    std::fs::write(root.join("gpui-kit.d.ts"), "export const value = 2;").unwrap();
    let entry = |imported: &str| format!("import {{ View, div }} from 'gpui-kit'; import {{ value }} from '{imported}'; export default class Panel extends View {{ render() {{ return div().child('v' + value); }} }}");
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        std::fs::write(root.join("main.js"), entry("./part.js")).unwrap();
        for _ in 0..2 {
            let mut ok = DescriptionSession::load(runtime.clone(), &root, Policy::new(), window, cx).unwrap();
            assert!(ok.describe(window, cx).unwrap().contains("v1"));
        }
        for imported in ["./gpui-kit.d.ts", "./GPUI-KIT.D.TS"] {
            std::fs::write(root.join("main.js"), entry(imported)).unwrap();
            for _ in 0..2 {
                assert!(DescriptionSession::load(runtime.clone(), &root, Policy::new(), window, cx).is_err(), "{imported}");
            }
        }
    }).unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx (FX-A2): the stubs that name the replacement for a timer or `require` still stand on
// the new engine, at module scope and in init: the stdlib now carries LLRT's timers, and a real
// setTimeout installed first would silently replace them.
#[gpui::test]
fn timers_and_require_still_refuse_a_mod_and_name_the_replacement(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("timers");
    std::fs::create_dir_all(&root).unwrap();
    let body = "import { View, div } from 'gpui-kit'; export default class Panel extends View { init() { let hint = 'none'; try { setTimeout(() => {}, 0); } catch (error) { hint = error.message; } this.hint = hint; } render() { return div().child('hint:' + this.hint); } }";
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        for (call, hint) in [
            ("setTimeout(() => {}, 0)", "cx.timer.after"),
            ("setInterval(() => {}, 0)", "cx.timer.every"),
            ("clearTimeout(1)", "cx.timer.after"),
            ("clearInterval(1)", "cx.timer.every"),
            ("require('fs')", "this runtime uses ES modules"),
        ] {
            std::fs::write(root.join("main.js"), format!("{call};\n{body}")).unwrap();
            let error = DescriptionSession::load(runtime.clone(), &root, Policy::new(), window, cx)
                .err()
                .map(|error| format!("{error:#}"))
                .unwrap_or_else(|| panic!("{call} at module scope must stop the mod"));
            assert!(error.contains("is not available in the shell") && error.contains(hint), "{call}: {error}");
        }
        std::fs::write(root.join("main.js"), body).unwrap();
        let mut session = DescriptionSession::load(runtime.clone(), &root, Policy::new(), window, cx).unwrap();
        let tree = session.describe(window, cx).unwrap();
        assert!(tree.contains("hint:`setTimeout` is not available in the shell: use cx.timer.after"), "{tree}");
    }).unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx (FX-A2): the quickjs-jit bump moved the stdlib onto vendored LLRT crates (timers, events,
// streams, abort, ...). A mod still imports only the runtime's own modules: none of those crates,
// nor the engine's std/os/bjson modules, nor a node: alias, became importable. A new reserved
// module is a new host capability and has to change this list on purpose.
#[gpui::test]
fn a_mod_imports_only_the_runtime_s_own_modules_after_the_engine_bump(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    assert_eq!(
        gpui_shell::RESERVED_SPECIFIERS,
        &[
            "gpui-kit", "gpui", "gpui-base", "gpui-component", "gpui-shell", "gpui-fps", "buffer", "console", "crypto",
            "fs/promises", "net", "os", "path", "process", "url", "websocket", "zlib",
        ][..]
    );
    let staged = Staged::new();
    let root = staged.0.join("imports");
    std::fs::create_dir_all(&root).unwrap();
    let entry = |name: &str| format!("import * as imported from '{name}'; import {{ View, div }} from 'gpui-kit'; export default class Panel extends View {{ render() {{ return div().child('exports:' + Object.keys(imported).length); }} }}");
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        for name in ["buffer", "console", "crypto", "fs/promises", "net", "os", "path", "process", "url", "websocket", "zlib"] {
            std::fs::write(root.join("main.js"), entry(name)).unwrap();
            let mut session = DescriptionSession::load(runtime.clone(), &root, Policy::new(), window, cx)
                .unwrap_or_else(|error| panic!("{name}: {error:#}"));
            assert!(session.describe(window, cx).unwrap().contains("exports:"), "{name}");
        }
        for name in [
            "qjs:std", "qjs:os", "qjs:bjson", "std", "bjson", "timers", "node:timers", "events", "node:events", "stream/web",
            "stream", "abort", "async_hooks", "exceptions", "child_process", "node:child_process", "fs", "node:fs",
            "node:crypto", "http", "https", "dns", "tls", "dgram", "llrt:timers", "llrt:fs", "test",
        ] {
            std::fs::write(root.join("main.js"), entry(name)).unwrap();
            assert!(DescriptionSession::load(runtime.clone(), &root, Policy::new(), window, cx).is_err(), "{name} must not resolve");
        }
    }).unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx (FX-A2): every kind of byte view a mod can hand to the host, built fresh on each call.
// `ran` turns true if the host ever ran script while reading one (a getter or a proxy trap).
const BYTE_VIEWS: &str = r#"
function views() {
  const ran = { value: false };
  const detached = new Uint8Array([1, 2, 3]);
  detached.buffer.transfer();
  const shrinking = new ArrayBuffer(4, { maxByteLength: 8 });
  const outOfBounds = new Uint8Array(shrinking, 2, 2);
  shrinking.resize(1);
  const growing = new ArrayBuffer(2, { maxByteLength: 8 });
  const grown = new Uint8Array(growing);
  grown.set([1, 2]);
  growing.resize(5);
  grown[4] = 5;
  const halving = new ArrayBuffer(4, { maxByteLength: 8 });
  const shrunk = new Uint8Array(halving);
  shrunk.set([1, 2, 3, 4]);
  halving.resize(2);
  const shared = new Uint8Array(new SharedArrayBuffer(3));
  shared.set([4, 5, 6]);
  const trapped = new Uint8Array([7, 8, 9]);
  const trappedBuffer = trapped.buffer;
  for (const key of ['buffer', 'length', 'byteLength', 'byteOffset']) {
    Object.defineProperty(trapped, key, { get() { ran.value = true; trappedBuffer.transfer(); return 0; } });
  }
  const proxied = new Proxy(new Uint8Array([1]), {
    get() { ran.value = true; },
    getPrototypeOf() { ran.value = true; return null; },
  });
  return { ran, list: [
    ['detached', detached],
    ['outOfBounds', outOfBounds],
    ['grown', grown],
    ['shrunk', shrunk],
    ['offset', new Uint8Array(new Uint8Array([9, 8, 7, 6]).buffer, 1, 2)],
    ['shared', shared],
    ['empty', new Uint8Array(0)],
    ['buffer', Buffer.from('hi')],
    ['trapped', trapped],
    ['proxied', proxied],
    ['signed', new Int8Array([1])],
    ['plain', new ArrayBuffer(1)],
  ] };
}
"#;
/// The views of [`BYTE_VIEWS`] the host must copy, in order, with the bytes each holds at the call.
const COPIED_VIEWS: [(&str, &[u8]); 7] = [
    ("grown", &[1, 2, 0, 0, 5]),
    ("shrunk", &[1, 2]),
    ("offset", &[8, 7]),
    ("shared", &[4, 5, 6]),
    ("empty", &[]),
    ("buffer", b"hi"),
    ("trapped", &[7, 8, 9]),
];
/// Views over a detached buffer, or out of bounds of a shrunk one: no bytes to borrow.
const DETACHED_VIEWS: [&str; 2] = ["detached", "outOfBounds"];
/// Not a Uint8Array at all, whatever a proxy claims.
const FOREIGN_VIEWS: [&str; 3] = ["proxied", "signed", "plain"];

// HyperOx (FX-A2): `TypedArray::as_bytes` is unsafe on the new engine, because the slice dies the
// moment script runs. fs.writeFile reads a view's internal slots without running script (getters
// and proxy traps never fire), copies exactly the bytes in bounds at the call (a resized, offset or
// shared view), and refuses a detached or out-of-bounds view before it touches the disk.
#[gpui::test]
fn fs_write_file_copies_the_live_bytes_of_any_view_and_refuses_a_detached_one(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("writer");
    let out = staged.0.join("out");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(root.join("main.js"), r#"
import { View, div } from 'gpui-kit';
import { Buffer } from 'buffer';
import { writeFile } from 'fs/promises';
__VIEWS__
export default class Panel extends View {
  init(_, cx) {
    this.log = [];
    cx.spawn(async cx => {
      const { ran, list } = views();
      for (const [name, view] of list) {
        try { writeFile(__DIR__ + '/' + name + '.bin', view); this.log.push(name + ':queued'); }
        catch (error) { this.log.push(name + ':' + error.message); }
      }
      this.log.push('ran:' + ran.value);
      cx.notify();
    });
  }
  render() { return div().child(this.log.join(' | ')); }
}
"#.replace("__VIEWS__", BYTE_VIEWS).replace("__DIR__", &serde_json::to_string(&out.to_string_lossy()).unwrap())).unwrap();
    let policy = Policy::new().with_capabilities(Capabilities::new().write_roots([out.clone()]));
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let mut session = window.update(cx, |_, window, cx| {
        let mut session = DescriptionSession::load(runtime.clone(), &root, policy, window, cx).unwrap();
        session.describe(window, cx).unwrap();
        session
    }).unwrap();
    cx.run_until_parked();
    let tree = window.update(cx, |_, window, cx| session.describe(window, cx).unwrap()).unwrap();
    assert!(tree.contains("ran:false"), "{tree}");
    for (name, bytes) in COPIED_VIEWS {
        assert!(tree.contains(&format!("{name}:queued")), "{name}: {tree}");
        assert_eq!(std::fs::read(out.join(format!("{name}.bin"))).unwrap(), bytes, "{name}");
    }
    for name in DETACHED_VIEWS {
        assert!(tree.contains(&format!("{name}:fs.writeFile(path, contents) received a detached Uint8Array")), "{name}: {tree}");
        assert!(!out.join(format!("{name}.bin")).exists(), "{name}");
    }
    for name in FOREIGN_VIEWS {
        assert!(tree.contains(&format!("{name}:fs.writeFile(path, contents) expects a string or Uint8Array")), "{name}: {tree}");
        assert!(!out.join(format!("{name}.bin")).exists(), "{name}");
    }
    drop(session);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx (FX-A2): fetch's body goes through the same unsafe borrow. It is read last, after the
// method and headers, so script a getter runs before it (here: a headers getter that detaches the
// body) is seen as a detached body, never as freed bytes. No network: with no grant, a body that was
// read whole reaches the authorization refusal, and one that was not is refused before it.
#[gpui::test]
fn fetch_reads_a_body_view_without_running_script_and_refuses_a_detached_one(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("fetcher");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.js"), r#"
import { View, div } from 'gpui-kit';
import { Buffer } from 'buffer';
__VIEWS__
export default class Panel extends View {
  init() {
    this.log = [];
    const { ran, list } = views();
    const send = (name, options) => {
      try { fetch('http://127.0.0.1:9/', options); this.log.push(name + ':sent'); }
      catch (error) { this.log.push(name + ':' + error.message); }
    };
    for (const [name, view] of list) send(name, { method: 'POST', body: view });
    this.log.push('ran:' + ran.value);
    const victim = new Uint8Array([1, 2, 3]);
    send('reordered', { method: 'POST', get headers() { victim.buffer.transfer(); return {}; }, body: victim });
  }
  render() { return div().child(this.log.join(' | ')); }
}
"#.replace("__VIEWS__", BYTE_VIEWS)).unwrap();
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        let mut session = DescriptionSession::load(runtime.clone(), &root, Policy::new(), window, cx).unwrap();
        let tree = session.describe(window, cx).unwrap();
        assert!(tree.contains("ran:false"), "{tree}");
        for (name, _) in COPIED_VIEWS {
            assert!(tree.contains(&format!("{name}:HTTP request POST http://127.0.0.1:9/ is not granted")), "{name}: {tree}");
        }
        for name in DETACHED_VIEWS.into_iter().chain(["reordered"]) {
            assert!(tree.contains(&format!("{name}:fetch(url, options).body received a detached Uint8Array")), "{name}: {tree}");
        }
        for name in FOREIGN_VIEWS {
            assert!(tree.contains(&format!("{name}:fetch(url, options).body expects a string or Uint8Array")), "{name}: {tree}");
        }
    }).unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx (FX-A2): WebSocket.write goes through the same unsafe borrow. Against a real loopback
// server, each view the host may copy arrives as one binary frame holding exactly its bytes at the
// call; a detached, out-of-bounds or foreign view is refused at the call and sends nothing.
#[gpui::test]
fn websocket_write_sends_the_live_bytes_of_any_view_and_refuses_a_detached_one(cx: &mut TestAppContext) {
    use base64::Engine as _;
    use std::io::{Read as _, Write as _};
    // The handshake's accept key is SHA-1 (RFC 6455 section 4.2.2); the app has no SHA-1 crate.
    fn sha1(data: &[u8]) -> [u8; 20] {
        let mut state: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
        let mut message = data.to_vec();
        message.push(0x80);
        while message.len() % 64 != 56 {
            message.push(0);
        }
        message.extend_from_slice(&(data.len() as u64 * 8).to_be_bytes());
        for block in message.chunks(64) {
            let mut words = [0u32; 80];
            for i in 0..16 {
                words[i] = u32::from_be_bytes(block[4 * i..4 * i + 4].try_into().unwrap());
            }
            for i in 16..80 {
                words[i] = (words[i - 3] ^ words[i - 8] ^ words[i - 14] ^ words[i - 16]).rotate_left(1);
            }
            let [mut a, mut b, mut c, mut d, mut e] = state;
            for (i, word) in words.iter().enumerate() {
                let (f, k) = match i {
                    0..=19 => ((b & c) | (!b & d), 0x5A827999),
                    20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                    40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                    _ => (b ^ c ^ d, 0xCA62C1D6),
                };
                let next = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*word);
                e = d;
                d = c;
                c = b.rotate_left(30);
                b = a;
                a = next;
            }
            for (word, add) in state.iter_mut().zip([a, b, c, d, e]) {
                *word = word.wrapping_add(add);
            }
        }
        let mut digest = [0u8; 20];
        for (i, word) in state.iter().enumerate() {
            digest[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }
    let accept = |key: &str| base64::engine::general_purpose::STANDARD.encode(sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes()));
    assert_eq!(accept("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=", "RFC 6455's own example");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (frames, received) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0u8; 1];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let request = String::from_utf8(request).unwrap();
        let key = request
            .lines()
            .find_map(|line| line.split_once(':').filter(|(name, _)| name.trim().eq_ignore_ascii_case("sec-websocket-key")).map(|(_, key)| key.trim().to_owned()))
            .unwrap();
        let reply = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", accept(&key));
        stream.write_all(reply.as_bytes()).unwrap();
        loop {
            let mut head = [0u8; 2];
            if stream.read_exact(&mut head).is_err() {
                break;
            }
            let mut length = (head[1] & 0x7f) as usize;
            if length == 126 {
                let mut extended = [0u8; 2];
                stream.read_exact(&mut extended).unwrap();
                length = u16::from_be_bytes(extended) as usize;
            } else if length == 127 {
                let mut extended = [0u8; 8];
                stream.read_exact(&mut extended).unwrap();
                length = u64::from_be_bytes(extended) as usize;
            }
            let mut mask = [0u8; 4];
            if head[1] & 0x80 != 0 {
                stream.read_exact(&mut mask).unwrap();
            }
            let mut payload = vec![0u8; length];
            stream.read_exact(&mut payload).unwrap();
            for (i, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[i % 4];
            }
            // Only binary frames; a close (0x8) ends the conversation.
            match head[0] & 0x0f {
                0x2 => {
                    if frames.send(payload).is_err() {
                        break;
                    }
                }
                0x8 => break,
                _ => {}
            }
        }
    });

    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("socket");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.js"), r#"
import { View, div } from 'gpui-kit';
import { Buffer } from 'buffer';
import { WebSocket } from 'websocket';
__VIEWS__
export default class Panel extends View {
  init(_, cx) {
    this.log = [];
    cx.spawn(async cx => {
      try {
        const socket = await WebSocket.connect('__URL__');
        const { ran, list } = views();
        for (const [name, view] of list) {
          try { socket.write(view); this.log.push(name + ':sent'); }
          catch (error) { this.log.push(name + ':' + error.message); }
        }
        this.log.push('ran:' + ran.value);
      } catch (error) {
        this.log.push('failed:' + error.message);
      }
      cx.notify();
    });
  }
  render() { return div().child(this.log.join(' | ')); }
}
"#.replace("__VIEWS__", BYTE_VIEWS).replace("__URL__", &format!("ws://127.0.0.1:{port}/"))).unwrap();
    let policy = Policy::new().with_capabilities(Capabilities::new().network_hosts(["127.0.0.1".to_owned()]));
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let mut session = window.update(cx, |_, window, cx| {
        let mut session = DescriptionSession::load(runtime.clone(), &root, policy, window, cx).unwrap();
        session.describe(window, cx).unwrap();
        session
    }).unwrap();
    cx.run_until_parked();
    let tree = window.update(cx, |_, window, cx| session.describe(window, cx).unwrap()).unwrap();
    assert!(tree.contains("ran:false"), "{tree}");
    for (name, _) in COPIED_VIEWS {
        assert!(tree.contains(&format!("{name}:sent")), "{name}: {tree}");
    }
    for name in DETACHED_VIEWS {
        assert!(tree.contains(&format!("{name}:WebSocket.write(data) received a detached Uint8Array")), "{name}: {tree}");
    }
    for name in FOREIGN_VIEWS {
        assert!(tree.contains(&format!("{name}:WebSocket.write(data) expects a string or Uint8Array")), "{name}: {tree}");
    }
    for (name, bytes) in COPIED_VIEWS {
        let frame = received.recv_timeout(std::time::Duration::from_secs(10)).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(frame, bytes, "{name}");
    }
    drop(session);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx (FX-A2, GHSA-52gx-967m-hxp3): before quickjs-ng 0.16, ArrayBuffer.prototype.transfer
// handed bytes the host allocated (fs.readFile's) to the engine's own realloc: a crash for an empty
// file, a use-after-free otherwise. Now every transfer of host bytes grows, shrinks, empties or
// freezes them safely, zero-fills what it adds, and leaves the old view detached and refused.
#[gpui::test]
fn transferring_the_bytes_of_a_read_file_never_crashes_and_detaches_the_old_view(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("transfer");
    let data = staged.0.join("data");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("empty.bin"), b"").unwrap();
    std::fs::write(data.join("abc.bin"), b"abc").unwrap();
    std::fs::write(root.join("main.js"), r#"
import { View, div } from 'gpui-kit';
import { readFile, writeFile } from 'fs/promises';
const dir = __DIR__;
const bytes = buffer => Array.from(new Uint8Array(buffer)).join(',');
export default class Panel extends View {
  init(_, cx) {
    this.log = [];
    cx.spawn(async cx => {
      try {
        const empty = await readFile(dir + '/empty.bin');
        const grown = empty.buffer.transfer(64);
        this.log.push('grown:' + grown.byteLength + ':' + new Uint8Array(grown).every(byte => byte === 0) + ':' + empty.byteLength);
        try { writeFile(dir + '/detached.bin', empty); this.log.push('detached:queued'); }
        catch (error) { this.log.push('detached:' + error.message); }
        this.log.push('shrunk:' + bytes((await readFile(dir + '/abc.bin')).buffer.transfer(1)));
        this.log.push('fixed:' + bytes((await readFile(dir + '/abc.bin')).buffer.transferToFixedLength(2)));
        this.log.push('zero:' + (await readFile(dir + '/abc.bin')).buffer.transfer(0).byteLength);
        await writeFile(dir + '/grown.bin', new Uint8Array((await readFile(dir + '/abc.bin')).buffer.transfer(8)));
        const frozen = (await readFile(dir + '/abc.bin')).buffer.transferToImmutable();
        await writeFile(dir + '/frozen.bin', new Uint8Array(frozen));
        try { frozen.transfer(1); this.log.push('refrozen:moved'); }
        catch (error) { this.log.push('refrozen:' + error.name); }
        this.log.push('done');
      } catch (error) {
        this.log.push('failed:' + error.message);
      }
      cx.notify();
    });
  }
  render() { return div().child(this.log.join(' | ')); }
}
"#.replace("__DIR__", &serde_json::to_string(&data.to_string_lossy()).unwrap())).unwrap();
    let policy = Policy::new().with_capabilities(Capabilities::new().read_roots([data.clone()]).write_roots([data.clone()]));
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let mut session = window.update(cx, |_, window, cx| {
        let mut session = DescriptionSession::load(runtime.clone(), &root, policy, window, cx).unwrap();
        session.describe(window, cx).unwrap();
        session
    }).unwrap();
    cx.run_until_parked();
    let tree = window.update(cx, |_, window, cx| session.describe(window, cx).unwrap()).unwrap();
    for expected in [
        "grown:64:true:0",
        "detached:fs.writeFile(path, contents) received a detached Uint8Array",
        "shrunk:97",
        "fixed:97,98",
        "zero:0",
        "refrozen:TypeError",
        "done",
    ] {
        assert!(tree.contains(expected), "{expected}: {tree}");
    }
    assert!(!data.join("detached.bin").exists());
    assert_eq!(std::fs::read(data.join("grown.bin")).unwrap(), b"abc\0\0\0\0\0");
    assert_eq!(std::fs::read(data.join("frozen.bin")).unwrap(), b"abc");
    drop(session);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx (FX-A2, GHSA-rxpx-wj25-jhqq): quickjs-ng 0.16 fixed a write past the RegExp bytecode
// buffer when growing it fails. A mod that compiles alternations with its heap at the limit gets
// an out-of-memory error, not a corrupt engine. The shell keeps such a failure out of the code
// cache by its text (an InternalError `out of memory`), so the same unchanged code loads again
// once memory is back; a reworded engine message would cache the failure for good.
#[gpui::test]
fn a_mod_that_runs_out_of_memory_compiling_regexps_fails_cleanly_and_loads_again(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("hog");
    std::fs::create_dir_all(&root).unwrap();
    // Indexed loops only: an iterator allocates, and each step near the limit must be in a try.
    std::fs::write(root.join("main.js"), r#"
import { View, div } from 'gpui-kit';
import { hog } from 'copy';
const sizes = [1 << 24, 1 << 20, 1 << 16, 1 << 12, 1 << 8, 1 << 6];
const patterns = Array.from({ length: 32 }, (_, n) => '(?:' + Array.from({ length: n + 2 }, (_, i) => 'a' + i).join('|') + ')+$');
function exhaust() {
  const held = [];
  let failure = null;
  for (let s = 0; s < sizes.length; s++) {
    try { for (;;) held.push(new ArrayBuffer(sizes[s])); } catch (error) { failure = failure || error; }
    for (let p = 0; p < patterns.length; p++) {
      try { new RegExp(patterns[p], 'u'); } catch (error) { failure = failure || error; }
    }
  }
  return failure;
}
if (hog()) {
  let failure = null;
  try { failure = exhaust(); } catch (error) { failure = failure || error; }
  throw failure;
}
export default class Panel extends View { render() { return div().child('loaded'); } }
"#).unwrap();
    let policy = |hog: bool| Policy::new().with_host_module(HostModule::new("copy").function("hog", move |_| Ok(HostValue::Bool(hog)))).unwrap();
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        let error = DescriptionSession::load(runtime.clone(), &root, policy(true), window, cx)
            .err()
            .map(|error| format!("{error:#}"))
            .expect("a mod that exhausts its heap must not load");
        assert!(error.contains("out of memory"), "{error}");
        let mut loaded = DescriptionSession::load(runtime.clone(), &root, policy(false), window, cx)
            .unwrap_or_else(|error| panic!("the out-of-memory failure was cached: {error:#}"));
        let tree = loaded.describe(window, cx).unwrap();
        assert!(tree.contains("loaded"), "{tree}");
        assert_eq!(runtime.code_cache_counts().0, 1, "one version: the failure was not kept");
    }).unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx: shared code is keyed by content, like the consent hash (ADR-048). With every watcher
// event lost, an edit that keeps the size and modified time, anywhere the code imports (the entry,
// a helper, a file deeper than the watcher's scan, a file replaced by a link), asks again and never
// reuses the old code; a missing import that appears later is picked up without an event.
#[gpui::test]
fn same_size_edits_anywhere_in_the_import_graph_ask_again_and_never_reuse_stale_code(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("graph");
    let nested = ["d"; 12].join("/");
    let deep = format!("lib/{nested}/deep.js");
    std::fs::create_dir_all(root.join(&deep).parent().unwrap()).unwrap();
    let source = |file: &str, version: char| match file {
        "main.js" => format!("import {{ View, div }} from 'gpui-kit'; import {{ mark }} from 'copy'; import {{ helper }} from './lib/helper.js'; mark(); const entry = 'e{version}'; export default class Panel extends View {{ render() {{ return div().child(entry + helper); }} }}"),
        "lib/helper.js" => format!("import {{ deep }} from './{nested}/deep.js'; export const helper = '-h{version}' + deep;"),
        _ => format!("export const deep = '-d{version}';"),
    };
    // Same byte count, and the modified time put back the way a tool that restores times does.
    let rewrite = |file: &str, text: String| {
        let path = root.join(file);
        let before = std::fs::metadata(&path).unwrap();
        assert_eq!(before.len(), text.len() as u64, "{file}");
        std::fs::write(&path, text).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(before.modified().unwrap()).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before.modified().unwrap(), "{file}");
    };
    std::fs::write(root.join("main.js"), source("main.js", '1')).unwrap();
    std::fs::write(root.join("lib/helper.js"), source("lib/helper.js", '1')).unwrap();
    let marks = Rc::new(Cell::new(0));
    let policy = || {
        let marks = marks.clone();
        Policy::new().with_host_module(HostModule::new("copy").function("mark", move |_| {
            marks.set(marks.get() + 1);
            Ok(HostValue::Null)
        })).unwrap()
    };
    let runtime = ShellRuntime::new_isolated().unwrap();
    // The OS watcher missed every event: the mod's revision never moves.
    runtime.set_source_revision_provider(|_| Some(0));
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        let mut text = || match DescriptionSession::load(runtime.clone(), &root, policy(), window, cx) {
            Ok(mut session) => Some(session.describe(window, cx).unwrap()),
            Err(_) => None,
        };
        assert_eq!(text(), None, "the deep import is missing");
        assert_eq!(text(), None, "the deep import is missing");
        std::fs::write(root.join(&deep), source(deep.as_str(), '1')).unwrap();
        let first = text().expect("the missing import appeared");
        assert!(first.contains("e1-h1-d1"), "{first}");
        let evaluations = marks.get();
        assert!(text().unwrap().contains("e1-h1-d1"));
        assert_eq!(marks.get(), evaluations, "unchanged code is shared by copies");
        for (file, shown) in [("main.js", "e2-h1-d1"), ("lib/helper.js", "e2-h2-d1"), (deep.as_str(), "e2-h2-d2")] {
            let consented = content_hash(&root);
            let evaluations = marks.get();
            rewrite(file, source(file, '2'));
            assert_ne!(content_hash(&root), consented, "{file}: the edit must ask again");
            let tree = text().expect(file);
            assert!(tree.contains(shown), "{file}: stale code ran after a same-size edit: {tree}");
            assert_eq!(marks.get(), evaluations + 1, "{file}");
        }
        // The editor config is not code: the answer leaves a JSON one out, and nothing runs it.
        let consented = content_hash(&root);
        std::fs::write(root.join("jsconfig.json"), r#"{"compilerOptions":{"checkJs":false}}"#).unwrap();
        assert_eq!(content_hash(&root), consented);
        assert!(text().unwrap().contains("e2-h2-d2"));
        #[cfg(unix)]
        {
            let path = root.join(&deep);
            let other = root.join("other.js");
            std::fs::write(&other, source(deep.as_str(), '3')).unwrap();
            let before = std::fs::metadata(&path).unwrap().modified().unwrap();
            std::fs::File::options().write(true).open(&other).unwrap().set_modified(before).unwrap();
            let consented = content_hash(&root);
            std::fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(&other, &path).unwrap();
            assert_ne!(content_hash(&root), consented, "a file replaced by a link must ask again");
            let tree = text().unwrap();
            assert!(tree.contains("e2-h2-d3"), "stale code ran after a file became a link: {tree}");
        }
    }).unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx: one folder is one mod. The same files in two folders, folders whose names share a
// prefix (`a`, `ab`), and a folder taken over by another mod after a rename never share code.
#[gpui::test]
fn identical_mods_in_prefix_related_or_renamed_folders_never_share_code(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let main = r#"
import { View, div } from 'gpui-kit';
import { mark } from 'copy';
import { tag } from './part.js';
const evaluation = mark();
export default class Panel extends View {
  render() { return div().child(tag + ':' + evaluation); }
}
"#;
    let write = |name: &str, tag: &str| {
        let root = staged.0.join(name);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("main.js"), main).unwrap();
        std::fs::write(root.join("part.js"), format!("export const tag = '{tag}';")).unwrap();
        root
    };
    let a = write("a", "same");
    let ab = write("ab", "same");
    let marks = Rc::new(Cell::new(0));
    let policy = || {
        let marks = marks.clone();
        Policy::new().with_host_module(HostModule::new("copy").function("mark", move |_| {
            marks.set(marks.get() + 1);
            Ok((marks.get() as f64).into())
        })).unwrap()
    };
    let runtime = ShellRuntime::new_isolated().unwrap();
    // Both folders sit at the same watcher revision.
    runtime.set_source_revision_provider(|_| Some(0));
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        let mut text = |root: &Path| {
            let mut session = DescriptionSession::load(runtime.clone(), root, policy(), window, cx).unwrap();
            session.describe(window, cx).unwrap()
        };
        for _ in 0..2 {
            let tree = text(a.as_path());
            assert!(tree.contains("same:1"), "{tree}");
            let tree = text(ab.as_path());
            assert!(tree.contains("same:2"), "identical files in a and ab are two mods: {tree}");
        }
        assert_eq!(marks.get(), 2);
        let moved = staged.0.join("moved");
        std::fs::rename(&a, &moved).unwrap();
        write("a", "next");
        let tree = text(a.as_path());
        assert!(tree.contains("next:3"), "the new folder a is another mod: {tree}");
        let tree = text(moved.as_path());
        assert!(tree.contains("same:4"), "a moved mod runs from its new folder: {tree}");
        let tree = text(ab.as_path());
        assert!(tree.contains("same:2"), "{tree}");
        assert_eq!(marks.get(), 4);
    }).unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx: on Windows one folder spelled in another case is the same mod (one evaluation), and a
// folder whose name only extends it is not.
#[cfg(windows)]
#[gpui::test]
fn a_mod_folder_spelled_in_another_case_on_windows_is_one_mod(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let main = "import { View, div } from 'gpui-kit'; import { mark } from 'copy'; const evaluation = mark(); export default class Panel extends View { render() { return div().child('case:' + evaluation); } }";
    for name in ["CaseMod", "CaseMod2"] {
        std::fs::create_dir_all(staged.0.join(name)).unwrap();
        std::fs::write(staged.0.join(name).join("main.js"), main).unwrap();
    }
    let marks = Rc::new(Cell::new(0));
    let policy = || {
        let marks = marks.clone();
        Policy::new().with_host_module(HostModule::new("copy").function("mark", move |_| {
            marks.set(marks.get() + 1);
            Ok((marks.get() as f64).into())
        })).unwrap()
    };
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        let spellings = [
            staged.0.join("CaseMod"),
            staged.0.join("casemod"),
            PathBuf::from(staged.0.join("CaseMod").to_string_lossy().to_uppercase()),
        ];
        for root in &spellings {
            let mut session = DescriptionSession::load(runtime.clone(), root, policy(), window, cx).unwrap();
            let tree = session.describe(window, cx).unwrap();
            assert!(tree.contains("case:1"), "{}: {tree}", root.display());
        }
        let mut session = DescriptionSession::load(runtime.clone(), &staged.0.join("CaseMod2"), policy(), window, cx).unwrap();
        let tree = session.describe(window, cx).unwrap();
        assert!(tree.contains("case:2"), "{tree}");
        assert_eq!(marks.get(), 2);
    }).unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx: module code is shared by copies (defaults/AGENTS.md), but a function defined at module
// scope acts for the copy that calls it: one copy's storage is never another copy's, and what the
// module scope wrote once stays with the copy that evaluated it.
#[gpui::test]
fn shared_module_code_reaches_only_the_calling_copys_storage(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("storage");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.js"), r#"
import { View, div } from 'gpui-kit';
import { props } from 'copy';
sessionStorage.setItem('module', props());
const read = key => sessionStorage.getItem(key) ?? 'none';
export default class Panel extends View {
  init() {
    const before = read('who');
    sessionStorage.setItem('who', props());
    this.text = props() + '|' + before + '|' + read('who') + '|' + read('module');
  }
  render() { return div().child(this.text); }
}
"#).unwrap();
    let policy = |name: &'static str| {
        Policy::new().with_host_module(HostModule::new("copy").function("props", move |_| Ok(name.into()))).unwrap()
    };
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        let mut first = DescriptionSession::load(runtime.clone(), &root, policy("A"), window, cx).unwrap();
        let mut second = DescriptionSession::load(runtime.clone(), &root, policy("B"), window, cx).unwrap();
        let tree = first.describe(window, cx).unwrap();
        assert!(tree.contains("A|none|A|A"), "{tree}");
        let tree = second.describe(window, cx).unwrap();
        assert!(tree.contains("B|none|B|none"), "copy B saw copy A's storage: {tree}");
        drop(first);
        let mut third = DescriptionSession::load(runtime.clone(), &root, policy("C"), window, cx).unwrap();
        let tree = third.describe(window, cx).unwrap();
        assert!(tree.contains("C|none|C|none"), "a reopened copy saw an old copy's storage: {tree}");
        let tree = second.describe(window, cx).unwrap();
        assert!(tree.contains("B|none|B|none"), "{tree}");
    }).unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx: the editor-helper refusal (v0.1.12) holds for every way the resolver can name a root
// helper: relative, through `..`, bare, absolute, a `file:` URL, another case, a re-export from a
// sub folder and links; the same names elsewhere in the mod still load.
#[gpui::test]
fn the_editor_helper_refusal_holds_for_every_way_an_import_can_name_it(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("helper-paths");
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("part.js"), "export const value = 1;").unwrap();
    std::fs::write(root.join("jsconfig.json"), "export const value = 2;").unwrap();
    std::fs::write(root.join("lib/jsconfig.json"), "export const value = 4;").unwrap();
    std::fs::write(root.join("lib/again.js"), "export { value } from '../jsconfig.json';").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("jsconfig.json"), root.join("alias.js")).unwrap();
        std::os::unix::fs::symlink(&root, root.join("self")).unwrap();
    }
    let canonical = root.canonicalize().unwrap();
    // A JS string needs Windows separators escaped.
    let absolute = |file: &str| canonical.join(file).display().to_string().replace('\\', "\\\\");
    let entry = |imported: &str| format!("import {{ View, div }} from 'gpui-kit'; import {{ value }} from '{imported}'; export default class Panel extends View {{ render() {{ return div().child('v' + value); }} }}");
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        let mut load = |imported: &str| {
            std::fs::write(root.join("main.js"), entry(imported)).unwrap();
            // Twice: the second answer comes from the shared code.
            let mut answers = Vec::new();
            for _ in 0..2 {
                answers.push(match DescriptionSession::load(runtime.clone(), &root, Policy::new(), window, cx) {
                    Ok(mut session) => Some(session.describe(window, cx).unwrap()),
                    Err(_) => None,
                });
            }
            assert_eq!(answers[0].is_some(), answers[1].is_some(), "{imported}");
            answers.pop().unwrap()
        };
        let mut allowed = vec![("./part.js".to_owned(), "v1"), ("./lib/jsconfig.json".to_owned(), "v4"), (absolute("part.js"), "v1")];
        #[cfg(unix)]
        allowed.push(("./self/part.js".to_owned(), "v1"));
        for (imported, shown) in &allowed {
            let tree = load(imported.as_str()).unwrap_or_else(|| panic!("{imported} must load"));
            assert!(tree.contains(*shown), "{imported}: {tree}");
        }
        let mut refused = vec![
            "./jsconfig.json".to_owned(),
            "./sub/../jsconfig.json".to_owned(),
            "../helper-paths/jsconfig.json".to_owned(),
            "jsconfig.json".to_owned(),
            "./lib/again.js".to_owned(),
            "./JSCONFIG.JSON".to_owned(),
            "./JsConfig.Json".to_owned(),
            absolute("jsconfig.json"),
            format!("file://{}", canonical.join("jsconfig.json").display().to_string().replace('\\', "/")),
        ];
        #[cfg(unix)]
        refused.extend(["./alias.js".to_owned(), "./self/jsconfig.json".to_owned()]);
        for imported in &refused {
            assert_eq!(load(imported.as_str()), None, "{imported} reached the root editor config");
        }
        let tree = load("./part.js").unwrap();
        assert!(tree.contains("v1"), "a refusal must not poison the shared code: {tree}");
    }).unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
}

// HyperOx: editing a mod must not grow what the runtime keeps. Vendor tests of the shell crate do not
// run in CI, so the app gates the code cache here: one evaluated version and one registration per root.
#[gpui::test]
fn editing_a_mod_many_times_keeps_one_cached_version_and_one_registration(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("edited");
    std::fs::create_dir_all(&root).unwrap();
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    for version in 0..20 {
        std::fs::write(root.join("main.js"), format!(
            "import {{ View, div }} from 'gpui-kit'; export default class Panel extends View {{ render() {{ return div().child('v{version:02}'); }} }}"
        )).unwrap();
        window.update(cx, |_, window, cx| {
            for _ in 0..3 {
                let mut session = DescriptionSession::load(runtime.clone(), &root, Policy::new(), window, cx).unwrap();
                assert!(session.describe(window, cx).unwrap().contains(&format!("v{version:02}")));
            }
        }).unwrap();
        // Closed copies release their code when the window settles.
        cx.run_until_parked();
        assert_eq!(runtime.code_cache_counts(), (1, 1), "after edit {version}");
    }
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
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
fn layout_previews_stay_inside_their_cards(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("look");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("layouts".into(), json!(["main"]));
    let tree = describe_with_values(
        cx, &staged, "frame/settings", values,
        Some("this.previews = { main: 'layouts/portrait.png' }; this.previewsOf = 'main';"),
    ).unwrap();
    let lines: Vec<&str> = tree.lines().collect();
    let image = lines.iter().position(|l| l.contains("layouts/portrait.png")).unwrap();
    assert!(lines[image].contains(".h[Number(62.0)]"), "a preview keeps the card's height: {}", lines[image]);
    assert!(lines[image - 1].contains(".overflow_hidden"), "a preview cannot spill out of its card: {}", lines[image - 1]);
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
fn machine_add_needing_input_keeps_the_picker_and_shows_instructions(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new().with_host_module(module(fixture(false, props(), updates("current", "current")), calls.clone())).unwrap();
    let root = staged.wrapper("mods/machine-add", "this.typed = 'host'; this.add('host', cx);");
    let window = cx.add_window(|_, _| Empty);
    let mut session = window.update(cx, |_, window, cx| {
        DescriptionSession::load(ShellRuntime::new_isolated().unwrap(), root, policy, window, cx).unwrap()
    }).unwrap();
    cx.run_until_parked();
    window.update(cx, |_, window, cx| {
        let tree = session.describe(window, cx).unwrap();
        assert!(tree.contains("machine-add"), "{tree}");
        assert!(tree.contains("ma-message-host"), "{tree}");
        assert!(tree.contains("h0x-core machine add host"), "{tree}");
        window.remove_window();
    }).unwrap();
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    assert!(!calls.effects.iter().any(|(name, _)| name == "openOverlay" || name == "closeOverlay"), "the picker stays open without a setup dialog");
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

// Settings → Terminal's scrollback stepper with `person` as settings.json; `steps` presses − and +
// on a copy of it and shows what each would write instead of the value.
fn scrollback_stepper(cx: &mut TestAppContext, staged: &Staged, person: &Value, steps: bool) -> String {
    let mut p = props();
    p["section"] = json!("look");
    let mut setup = "const look = this.look.bind(this); this.look = (t, cx) => { this.person = PERSON; return look(t, cx); };".to_owned();
    if steps {
        setup.push_str(concat!(
            "const stepper = this.stepper.bind(this);",
            "this.stepper = (t, id, value, minus, plus) => {",
            "  if (id !== 'st-scrollback') return stepper(t, id, value, minus, plus);",
            "  const seen = []; const change = this.change;",
            "  this.change = (edit) => { const q = JSON.parse(JSON.stringify(this.person)); edit(q); seen.push(`${q.scrollback_lines}${q.font_ligatures === true ? '+kept' : '+lost'}`); };",
            "  try { minus(null, null); plus(null, null); } finally { this.change = change; }",
            "  return stepper(t, id, `steps ${seen.join(' ')}`, minus, plus);",
            "};",
        ));
    }
    let setup = setup.replace("PERSON", &person.to_string());
    describe(cx, staged, "frame/settings", true, p, updates("current", "current"), Some(&setup)).unwrap()
}

#[gpui::test]
fn scrollback_stepper_moves_to_the_next_stop_keeps_the_ends_and_the_other_settings(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (lines, minus, plus) in [
        // Set by hand between two stops: − and + go to the stops around it.
        (json!(7500), 5000, 10000),
        (json!(1000), 1000, 2000),
        (json!(100000), 50000, 100000),
        // Values the app does not use show and step from the default.
        (json!(999), 5000, 20000),
        (json!(100001), 5000, 20000),
        (json!("7500"), 5000, 20000),
        (json!(2000.5), 5000, 20000),
        (Value::Null, 5000, 20000),
    ] {
        let mut person = json!({"font_ligatures": true});
        if !lines.is_null() {
            person["scrollback_lines"] = lines.clone();
        }
        let tree = scrollback_stepper(cx, &staged, &person, true);
        assert!(tree.contains("st-scrollback-minus") && tree.contains("st-scrollback-plus"), "{lines}: {tree}");
        assert!(tree.contains(&format!("steps {minus}+kept {plus}+kept")), "{lines}: {tree}");
    }
}

#[gpui::test]
fn scrollback_stepper_shows_the_count_with_thousands_separators(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (person, shown) in [(json!({"scrollback_lines": 7500}), "7,500"), (json!({}), "10,000"), (json!({"scrollback_lines": 100000}), "100,000")] {
        let tree = scrollback_stepper(cx, &staged, &person, false);
        assert!(tree.contains(shown), "{person} should show {shown}: {tree}");
    }
}

// Settings → Remote (any page runs init) with settings.json holding `file` (None: missing), after a stepper
// press that sets scrollback_lines to 2000. Returns the page and every write made.
fn press_stepper_with_settings_file(cx: &mut TestAppContext, staged: &Staged, file: Option<&str>) -> (String, Vec<(String, String)>) {
    let path = "/fixture/app/settings.json";
    let replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    let files: Vec<(&str, &str)> = file.map(|f| (path, f)).into_iter().collect();
    let setup = "this.change((p) => { p.scrollback_lines = 2000; }, cx);";
    let (tree, calls) = describe_remote_with(cx, staged, &replies, setup, "/fixture/app", &files);
    let c = calls.borrow();
    if let Some(f) = file.filter(|_| c.writes.is_empty()) {
        assert_eq!(c.files.get(path).map(String::as_str), Some(f), "the file is byte-identical");
    }
    (tree, c.writes.clone())
}

#[gpui::test]
fn settings_never_overwrite_a_file_they_could_not_read_and_say_so(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for bad in ["", "   ", "{not json", "[1, 2]", "null", "\u{feff}{\"font_ligatures\": tru", "\u{feff}"] {
        let (tree, writes) = press_stepper_with_settings_file(cx, &staged, Some(bad));
        assert!(writes.is_empty(), "{bad:?}: nothing is written over it: {writes:?}");
        assert!(tree.contains("can't be read"), "{bad:?}: the page says why nothing changed: {tree}");
    }
}

#[gpui::test]
fn settings_change_keeps_every_other_key_in_a_file_with_a_byte_order_mark(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (tree, writes) = press_stepper_with_settings_file(cx, &staged, Some("\u{feff}{\"font_ligatures\": true, \"zoom\": 1.5}"));
    assert!(!tree.contains("can't be read"), "{tree}");
    assert_eq!(writes.len(), 1, "{writes:?}");
    let written: Value = serde_json::from_str(&writes[0].1).unwrap();
    assert_eq!(written, json!({"font_ligatures": true, "zoom": 1.5, "scrollback_lines": 2000}));
}

#[gpui::test]
fn settings_change_creates_a_missing_file(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (tree, writes) = press_stepper_with_settings_file(cx, &staged, None);
    assert!(!tree.contains("can't be read"), "{tree}");
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(serde_json::from_str::<Value>(&writes[0].1).unwrap(), json!({"scrollback_lines": 2000}));
}

#[gpui::test]
fn settings_sections_and_updates(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let start = Instant::now();
    let staged = Staged::new();
    for section in ["look", "alerts", "mods", "updates", "info"] {
        let mut p = props();
        p["section"] = json!(section);
        let tree = describe(cx, &staged, "frame/settings", true, p, updates("current", "current"), None).unwrap();
        if section == "mods" {
            assert!(tree.contains("No mod has asked yet"), "an empty mods list says so: {tree}");
        }
        if section == "info" {
            assert!(
                ["HyperOx", "Agentic development environment", "Built by", "Team Pavii", "Security & blockchain partner", "Buckleson", "Currently closed source"].iter().all(|t| tree.contains(t)),
                "the Info page names the product, its makers and partner: {tree}"
            );
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

#[gpui::test]
fn settings_info_shows_the_product_makers_and_versions_with_no_source_links(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // A known core version, then none (null, a missing key): the page still renders and says "unknown".
    for (core, shown) in [(Some(json!("0.1.5")), "Core 0.1.5"), (Some(Value::Null), "Core unknown"), (None, "Core unknown")] {
        let mut p = props();
        p["section"] = json!("info");
        let mut u = updates("current", "current");
        let versions = u["core"].as_object_mut().unwrap();
        match &core {
            Some(v) => versions.insert("version".into(), v.clone()),
            None => versions.remove("version"),
        };
        let core = format!("{core:?}");
        let tree = describe(cx, &staged, "frame/settings", true, p, u, None).unwrap_or_else(|e| panic!("{core}: {e:#}"));
        for text in [
            "HyperOx",
            "Agentic development environment",
            "The desktop workspace for AI coding agents.",
            "A secure space to build with AI coding agents.",
            "conversations stay on your own machine.",
            "Learn more",
            "Installation guide",
            "Documentation",
            "Send feedback",
            "Built by",
            "Team Pavii",
            "pavii.tech",
            "Security & blockchain partner",
            "Buckleson",
            "buckleson.com",
            "Currently closed source (open source soon)",
            "Version 0.1.0",
            shown,
            "Open-source notices",
        ] {
            assert!(tree.contains(text), "{core}: the Info page shows {text:?}: {tree}");
        }
        for id in ["st-info-learn", "st-info-install", "st-info-docs", "st-info-feedback", "st-info-pavii", "st-info-buckleson"] {
            assert!(tree.contains(id), "{core}: {id} is on the page: {tree}");
        }
        assert!(!tree.to_ascii_lowercase().contains("github"), "{core}: no source-repo links remain: {tree}");
    }
}

#[gpui::test]
fn settings_info_buttons_open_only_allowed_links_and_feedback_is_coming_soon(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut p = props();
    p["section"] = json!("info");
    let policy = Policy::new()
        .with_capabilities(Capabilities::new().read_roots([staged.0.clone()]))
        .with_host_module(module(fixture(true, p, updates("current", "current")), calls.clone()))
        .unwrap();
    // Presses each Info button once, as a click would.
    let setup = "const real = this.button.bind(this); const pressed = new Set(); \
        this.button = (t, id, label, onClick) => { if (id.startsWith('st-info-') && !pressed.has(id)) { pressed.add(id); onClick(); } return real(t, id, label, onClick); };";
    let root = staged.wrapper("frame/settings", setup);
    let window = cx.add_window(|_, _| Empty);
    let mut session = window
        .update(cx, |_, window, cx| DescriptionSession::load(ShellRuntime::new_isolated().unwrap(), root, policy, window, cx))
        .unwrap()
        .unwrap();
    cx.run_until_parked();
    window.update(cx, |_, window, cx| session.describe(window, cx)).unwrap().unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    let c = calls.borrow();
    assert!(c.unexpected.is_empty(), "no sendFeedback or other unscripted call: {:?}", c.unexpected);
    let links: Vec<&Value> = c.effects.iter().filter(|(n, _)| n == "openLink").map(|(_, a)| &a[0]).collect();
    assert_eq!(links, [&json!("https://www.pavii.tech/products/dev-tools/docs/"); 2], "installation guide and documentation: {:?}", c.effects);
    assert!(links.iter().all(|l| l.as_str().is_some_and(allowed_link)), "{links:?}");
    let notes: Vec<&Value> = c.effects.iter().filter(|(n, _)| n == "notify").map(|(_, a)| &a[0]).collect();
    assert_eq!(notes, [&json!({"title":"The feedback forum is coming soon.","level":"info"})], "feedback shows only the notice, never an error: {:?}", c.effects);
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
        // A file put on disk by the test reads back; anything else falls through to the defaults.
        "readTextFile" => Some(json!(c.files.get(first.as_str()?)?)),
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
        "exec" => {
            let mut args = vec![first.as_str()?.to_owned()];
            args.extend(serde_json::from_value::<Vec<String>>(text?).ok()?);
            let key = args.join(" ");
            let reply = c.replies.iter().filter(|(k, _)| key.starts_with(k.as_str())).max_by_key(|(k, _)| k.len()).map(|(_, v)| v.clone());
            c.execs.push(args);
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

// `entry` on the populated fixture with `props`, once its tasks have finished: `replies` answer core and exec.
fn describe_scripted(cx: &mut TestAppContext, staged: &Staged, entry: &str, props: Value, replies: &[(&str, Value)]) -> (String, Rc<RefCell<Calls>>) {
    let calls = Rc::new(RefCell::new(Calls::default()));
    calls.borrow_mut().replies = replies.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    let policy = Policy::new()
        .with_capabilities(Capabilities::new().read_roots([staged.0.clone()]))
        .with_host_module(module(fixture(true, props, updates("current", "current")), calls.clone()))
        .unwrap();
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let mut session = window
        .update(cx, |_, window, cx| DescriptionSession::load(runtime, staged.0.join(entry), policy, window, cx))
        .unwrap()
        .unwrap();
    cx.run_until_parked();
    let tree = window.update(cx, |_, window, cx| session.describe(window, cx)).unwrap().unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    (tree, calls)
}

fn git_ok(stdout: &str) -> Value {
    json!({"code":0, "stdout":stdout, "stderr":""})
}

#[gpui::test]
fn worktree_remove_skips_submodule_names_and_paths_that_leave_the_repo(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // (name, path, probed): .gitmodules records as `git config -z --get-regexp` prints them.
    let records = [
        ("ok", "libs/ok", true),
        ("my lib", "libs/my lib", true),
        ("./dot", "./dot", true),
        ("a..b", "libs/a..b", true),
        ("", "libs/empty", false),
        ("nopath", "", false),
        ("..", "up", false),
        ("x/..", "xdd", false),
        ("foo/../bar", "foobar", false),
        ("mixed", "a\\..\\b", false),
        ("mixed2", "a/..\\b", false),
        ("trail", "x/..", false),
        ("abs", "/etc", false),
        ("/abs", "abs2", false),
        ("unc", "\\\\server\\x", false),
        ("\\back", "back", false),
        ("drive", "C:foo", false),
        ("C:\\x", "cx", false),
        ("lower", "c:/x", false),
    ];
    let listed: String = records.iter().map(|(name, path, _)| format!("submodule.{name}.path\n{path}\0")).collect();
    let mut p = props();
    p["action"] = json!("remove");
    let (_, calls) = describe_scripted(cx, &staged, "mods/worktree", p, &[
        ("git rev-parse --show-toplevel", git_ok("/r\n")),
        ("git rev-parse --path-format=absolute --git-common-dir", git_ok("/r/.git\n")),
        ("git --no-optional-locks -c core.fsmonitor=false status --porcelain", git_ok("")),
        ("git rev-parse --absolute-git-dir", git_ok("/r/.git\n")),
        ("git --git-dir /r/.git config --file /r/.gitmodules -z --get-regexp", git_ok(&listed)),
        ("git --git-dir /r/.git rev-parse HEAD:", git_ok("abc123\n")),
        // Each probed submodule: checked out, an empty nested .gitmodules, nothing unpushed.
        ("git --git-dir /r/.git/modules/", git_ok("")),
    ]);
    let c = calls.borrow();
    assert!(c.unexpected.is_empty(), "{:?}", c.unexpected);
    let probed: Vec<&str> = c.execs.iter()
        .filter(|a| a.len() == 5 && a[1] == "--git-dir" && a[3..] == ["rev-parse", "--git-dir"])
        .map(|a| a[2].strip_prefix("/r/.git/modules/").unwrap_or(&a[2]))
        .collect();
    let safe: Vec<&str> = records.iter().filter(|r| r.2).map(|r| r.0).collect();
    assert_eq!(probed, safe, "{:?}", c.execs);
    let pinned: Vec<&str> = c.execs.iter().filter_map(|a| a.last()?.strip_prefix("HEAD:")).filter(|p| *p != ".gitmodules").collect();
    let paths: Vec<&str> = records.iter().filter(|r| r.2).map(|r| r.1).collect();
    assert_eq!(pinned, paths, "{:?}", c.execs);
    // The safe ones were checked for unpushed commits; nothing reached outside the repo's modules folder.
    assert_eq!(c.execs.iter().filter(|a| a.contains(&"rev-list".to_string())).count(), safe.len(), "{:?}", c.execs);
    assert!(!c.execs.iter().flatten().any(|a| a.contains("modules/..") || a.contains("modules//") || a.contains("modules/C:")), "{:?}", c.execs);
}

#[gpui::test]
fn changes_panel_diffs_never_run_external_diff_textconv_or_fsmonitor(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    const READ: &str = "git --no-optional-locks -c core.fsmonitor=false -c diff.autoRefreshIndex=false";
    // With a HEAD one numstat diff; on an unborn branch the staged and unstaged ones.
    for (head, numstats) in [(git_ok("abc\n"), 1), (json!({"code":128, "stdout":"", "stderr":"fatal: no HEAD"}), 2)] {
        let (tree, calls) = describe_scripted(cx, &staged, "mods/changes", props(), &[
            ("git rev-parse --show-toplevel", git_ok("/r\n")),
            (format!("{READ} status").as_str(), git_ok(" M a.txt\0")),
            (format!("{READ} rev-parse --verify HEAD").as_str(), head.clone()),
            (format!("{READ} diff").as_str(), git_ok("1\t2\ta.txt\0")),
        ]);
        let c = calls.borrow();
        assert!(tree.contains("a.txt"), "{tree}");
        let diffs: Vec<&Vec<String>> = c.execs.iter().filter(|a| a.iter().any(|x| x == "diff")).collect();
        assert_eq!(diffs.iter().filter(|a| a.iter().any(|x| x == "--numstat")).count(), numstats, "{head}: {:?}", c.execs);
        for args in &diffs {
            let diff = args.iter().position(|x| x == "diff").unwrap();
            let before = &args[..diff];
            assert!(before.windows(2).any(|w| w == ["-c", "core.fsmonitor=false"]), "{args:?}");
            assert!(args[diff..].iter().any(|x| x == "--no-ext-diff") && args[diff..].iter().any(|x| x == "--no-textconv"), "{args:?}");
        }
    }
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
fn settings_remote_page_shows_the_panel_warning_only_when_there_is_one(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let text = "Messages are end-to-end encrypted; the page itself is delivered by Cloudflare, so an attacker who controls that path could change it";
    let ready = |extra: &str| format!("{{\"url\":\"http://10.0.0.2:7000\"{extra}}}\n");
    let shown = |cx: &mut TestAppContext, extra: &str| {
        let mut replies = web_replies(&ready(extra));
        replies.push(("bridge qr", core_ok(json!({"size":1, "rows":["1"]}))));
        describe_remote(cx, &staged, &replies, "").0
    };
    let tree = shown(cx, &format!(",\"warning\":\"{text}\""));
    assert!(tree.contains(text) && tree.contains("Scan with your phone"), "{tree}");
    for extra in [",\"warning\":null", "", ",\"warning\":\"\"", ",\"warning\":7"] {
        let tree = shown(cx, extra);
        assert!(tree.contains("Scan with your phone") && !tree.contains("st-r-warning") && !tree.contains("Cloudflare, so"), "{extra}: {tree}");
    }
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
    let mod_commands = ["ribbon.next", "tabs.todo"];
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
fn sessions_panel_refuses_to_reveal_a_path_explorer_would_split(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let list = core_ok(json!({"sessions":[]}));
    for (platform, path, set) in [
        ("windows", "C:/Users/me/a,/select,C:/evil.jsonl", ""),
        ("windows", "C:/Users/me/\"a\".jsonl", ""),
        ("windows", "", ""),
        ("windows", "x", "this.target.path = null;"),
        ("macos", "x", "this.target.path = null;"),
        ("linux", "", ""),
    ] {
        let host = sessions_host(&[("list", list.clone())], &[]);
        let setup = format!("{} {set} this.command('reveal', cx);", session_target(None, "/fixture/project", "claude", "a1", path));
        describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", platform), &host, &setup);
        let h = host.borrow();
        assert!(h.execs.is_empty(), "{platform} {path:?}: {:?}", h.execs);
        assert!(!h.effects.is_empty(), "{platform} {path:?}: the failure is shown");
    }
}

#[gpui::test]
fn sessions_panel_reveal_refuses_only_what_explorer_splits_and_passes_one_argument_elsewhere(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let list = core_ok(json!({"sessions":[]}));
    // None: refused with a toast and nothing run.
    for (platform, path, expected) in [
        ("windows", "C:/Users/me/a,b/p/a1.jsonl", None),
        ("windows", "C:\\Users\\me\\say \"hi\"\\a1.jsonl", None),
        ("windows", "C:/Users/me/`whoami`/a1.jsonl", None),
        ("windows", "\\\\server\\share\\a,b.jsonl", None),
        ("windows", ",", None),
        ("windows", "\\\\server\\share\\p\\a1.jsonl", Some(json!(["explorer", ["/select,\\\\server\\share\\p\\a1.jsonl"]]))),
        ("windows", "C:/Users/José/プロジェクト/a1.jsonl", Some(json!(["explorer", ["/select,C:\\Users\\José\\プロジェクト\\a1.jsonl"]]))),
        ("windows", "C:/Users/me/my dir;x/a1.jsonl", Some(json!(["explorer", ["/select,C:\\Users\\me\\my dir;x\\a1.jsonl"]]))),
        ("windows", "C:\\Users\\me\\", Some(json!(["explorer", ["/select,C:\\Users\\me\\"]]))),
        ("macos", "/Users/me/a,b/\"q\" `x`.jsonl", Some(json!(["open", ["-R", "/Users/me/a,b/\"q\" `x`.jsonl"]]))),
        ("linux", "/home/me/a,b \"q\"/`x`/a1.jsonl", Some(json!(["xdg-open", ["/home/me/a,b \"q\"/`x`"]]))),
        ("linux", "/a1.jsonl", Some(json!(["xdg-open", ["/"]]))),
    ] {
        let host = sessions_host(&[("list", list.clone())], &[]);
        let setup = format!("{} this.command('reveal', cx);", session_target(None, "/fixture/project", "claude", "a1", path));
        describe_sessions(cx, &staged, "mods/sessions", sessions_values(Value::Null, "/fixture/project", platform), &host, &setup);
        let h = host.borrow();
        let toasts: Vec<_> = h.effects.iter().filter(|(n, _)| n == "notify").map(|(_, a)| a[0]["title"].clone()).collect();
        match expected {
            None => {
                assert!(h.execs.is_empty(), "{platform} {path}: {:?}", h.execs);
                assert_eq!(toasts, vec![json!("Couldn't show the file")], "{platform} {path}");
            }
            Some(expected) => {
                assert_eq!(h.execs, vec![expected], "{platform} {path}");
                assert!(toasts.is_empty(), "{platform} {path}: {toasts:?}");
            }
        }
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
    // Sessions and To-do are each added once to a list stored before they existed.
    let added = json!(["sessions", "todo"]);
    for (before, after) in [
        (json!(["files", "changes"]), json!(["files", "changes", "sessions", "todo"])),
        // The welcome's "nothing picked yet".
        (json!([]), json!(["sessions", "todo"])),
        (json!(["sessions", "files"]), json!(["sessions", "files", "todo"])),
        (json!(["todo", "files"]), json!(["todo", "files", "sessions"])),
    ] {
        let (tree, shared) = tabs(cx, &staged, &[("workspace.panels", before.clone())]);
        assert_eq!(shared["workspace.panels"], after, "{before}");
        assert_eq!(shared["workspace.panels.added"], added, "{before}");
        assert!(tree.contains("panel-sessions"), "{before}: {tree}");
        assert!(tree.contains("panel-todo"), "{before}: {tree}");
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
        assert!(!tree.contains("panel-todo"), "{off}: {tree}");
    }

    // A list the Sessions migration already touched (an older app) gains To-do once; turned
    // off after that, it stays off.
    let older = json!(["sessions"]);
    let (tree, shared) = tabs(cx, &staged, &[("workspace.panels", json!(["files"])), ("workspace.panels.added", older.clone())]);
    assert_eq!(shared["workspace.panels"], json!(["files", "todo"]));
    assert_eq!(shared["workspace.panels.added"], added);
    assert!(tree.contains("panel-todo") && !tree.contains("panel-sessions"), "{tree}");
    let (tree, shared) = tabs(cx, &staged, &[("workspace.panels", json!(["files"])), ("workspace.panels.added", added.clone())]);
    assert_eq!(shared["workspace.panels"], json!(["files"]));
    assert!(!tree.contains("panel-todo"), "{tree}");

    // A fresh install stores nothing and shows all four.
    let (tree, shared) = tabs(cx, &staged, &[]);
    assert!(shared.is_empty(), "{shared:?}");
    assert!(tree.contains("panel-files") && tree.contains("panel-changes") && tree.contains("panel-sessions") && tree.contains("panel-todo"), "{tree}");

    // The welcome's own list already offers Sessions and To-do: the tab bar adds nothing to it.
    let host = sessions_host(&[], &[]);
    let tree = describe_sessions(cx, &staged, "frame/welcome", fixture(true, props(), updates("current", "current")), &host, "this.page = 3;");
    assert!(tree.contains("Choose your panels") && tree.contains("Sessions") && tree.contains("To-do"), "{tree}");
    let welcome = host.borrow().shared.clone();
    assert_eq!(welcome.get("workspace.panels"), Some(&json!([])));
    assert_eq!(welcome.get("workspace.panels.added"), Some(&added));
    // What the welcome stored, with only Files picked: the tab bar shows just Files.
    let stored = welcome.get("workspace.panels.added").cloned().unwrap_or(Value::Null);
    let picked: Vec<(&str, Value)> = vec![("workspace.panels", json!(["files"])), ("workspace.panels.added", stored)];
    let (tree, shared) = tabs(cx, &staged, &picked);
    assert_eq!(shared["workspace.panels"], json!(["files"]));
    assert!(!tree.contains("panel-sessions") && !tree.contains("panel-todo"), "{tree}");
}

// ---- To-do panel --------------------------------------------------------------------------

/// The disk, shared values and host effects the to-do panel (and the tab bar and settings
/// around it) meet: writes reject a stale `expected` like the app's, reads refuse over 1 MiB.
#[derive(Default)]
struct TodoHost {
    files: BTreeMap<String, String>,
    writes: Vec<(String, String, Value)>,
    removes: Vec<String>,
    effects: Vec<(String, Vec<Value>)>,
    shared: BTreeMap<String, Value>,
    commands: std::collections::VecDeque<String>,
    notes: Vec<String>,
}

const TODO_LISTS: &str = "/fixture/app/todo/lists.md";
const TODO_TASKS: &str = "/fixture/app/todo/tasks.md";
const TODO_READ_FAIL: &str = "This list can't be read (over 1 MB or unreadable). It is left untouched and read-only.";

const TODO_SCENARIO: &str = r#"
import Real from './mods/todo/main.js';
import { command, note } from 'fixture';
export default class Scenario extends Real {
  init(props, cx) {
    super.init(props, cx);
    cx.timer.every(50, cx => {
      for (let c = command(); c !== null; c = command()) {
        const at = c.indexOf(' '), verb = at < 0 ? c : c.slice(0, at), arg = c.slice(at + 1);
        const [i, j] = arg.split('.').map(Number);
        const row = arg.includes('.') ? { k: 'step', i, j } : { k: 'task', i };
        if (verb === 'add') this.addTask(arg, cx);
        else if (verb === 'tick') this.tick(row, cx);
        else if (verb === 'delete') this.delete(row, cx);
        else if (verb === 'undo') this.undo(cx);
        else if (verb === 'reload') this.reload(cx);
        else if (verb === 'focus') this.handle.focus();
        else if (verb === 'key') {
          const parts = arg.split('-'), k = parts.pop();
          this.listKey(k, { shift: parts.includes('shift'), alt: parts.includes('alt') }, cx);
        } else if (verb === 'state') {
          if (this.ready) this.current();
          note(JSON.stringify({ ready: this.ready, textLen: this.text.length, text: this.text.length > 16384 ? null : this.text, error: this.error, readonly: this.readonly, sel: this.sel,
            edit: this.edit ? this.edit.input.value() : null, pending: this.pending.length }));
        }
        cx.notify();
      }
    });
  }
}
"#;

fn todo_host(files: &[(&str, &str)], shared: &[(&str, Value)]) -> Rc<RefCell<TodoHost>> {
    Rc::new(RefCell::new(TodoHost {
        files: files.iter().map(|(p, t)| (p.to_string(), t.to_string())).collect(),
        shared: shared.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
        ..Default::default()
    }))
}

fn todo_module(values: BTreeMap<String, Value>, host: Rc<RefCell<TodoHost>>) -> HostModule {
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
    for name in ["run", "openOverlay", "closeOverlay", "notify", "playSound", "openLink", "copyText"] {
        let h = host.clone();
        module = module.function(name, move |a| {
            let args: Vec<Value> = (0..2).filter_map(|i| a.get(i).map(from_host)).collect();
            if name == "run" {
                let command = a.string(0)?;
                if let Some(c) = crate::commands::find(command) {
                    crate::commands::validate_args(c, args.get(1).unwrap_or(&Value::Null)).unwrap();
                } else {
                    // `<mod>.<command>` goes to a shipped mod's nextCommand().
                    let shipped = command.split_once('.').is_some_and(|(m, _)| {
                        Path::new(env!("CARGO_MANIFEST_DIR")).join("defaults/mods").join(m).join("main.js").is_file()
                    });
                    assert!(shipped, "unknown command {command}");
                }
            }
            h.borrow_mut().effects.push((name.into(), args));
            Ok(if name == "openOverlay" || name == "notify" { "fixture-overlay".into() } else { HostValue::Null })
        });
    }
    for name in ["nextCommand", "watchFolder"] {
        module = module.async_function(name, |_| Ok(std::future::pending::<gpui_shell::HostResult>()));
    }
    for name in ["core", "exec"] {
        module = module.async_function(name, |_| Ok(std::future::ready(Ok(to_host(&json!({"code":1,"stdout":"","stderr":"fixture"}))))));
    }
    let h = host.clone();
    module = module.async_function("readTextFile", move |a| {
        let text = h.borrow().files.get(a.string(0)?).cloned();
        Ok(std::future::ready(match text {
            Some(t) if t.len() > 1024 * 1024 => Err(HostError::new("the file is larger than 1 MiB")),
            t => Ok(to_host(&json!(t))),
        }))
    });
    let h = host.clone();
    module = module.async_function("writeTextFile", move |a| {
        let (path, text) = (a.string(0)?.to_owned(), a.string(1)?.to_owned());
        let expected = a.get(2).map(from_host).unwrap_or(Value::Null);
        let mut h = h.borrow_mut();
        h.writes.push((path.clone(), text.clone(), expected.clone()));
        let current = h.files.get(&path).map_or(Value::Null, |t| json!(t));
        let result = if current != expected {
            Err(HostError::new("stale snapshot: the file changed on disk"))
        } else {
            h.files.insert(path, text);
            Ok(HostValue::Null)
        };
        Ok(std::future::ready(result))
    });
    module.async_function("removeAppFile", move |a| {
        let path = a.string(0)?.to_owned();
        let mut h = host.borrow_mut();
        h.files.remove(&path);
        h.removes.push(path);
        Ok(std::future::ready(Ok(HostValue::Null)))
    })
}

fn todo_policy(staged: &Staged, values: BTreeMap<String, Value>, host: &Rc<RefCell<TodoHost>>) -> Policy {
    static NEXT_STORE: AtomicU64 = AtomicU64::new(0);
    let (commands, notes) = (host.clone(), host.clone());
    Policy::new()
        .with_storage_path(staged.0.join(format!("todo-store-{}.json", NEXT_STORE.fetch_add(1, Ordering::Relaxed))))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(todo_module(values, host.clone()))
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
        .unwrap()
}

// The to-do panel driven by TODO_SCENARIO's commands.
fn todo_root(staged: &Staged) -> PathBuf {
    std::fs::write(staged.0.join("main.js"), TODO_SCENARIO).unwrap();
    staged.0.clone()
}

// Loads the mod at `root` on `host` and lets its first tasks (the first load) finish.
fn load_todo(
    cx: &mut TestAppContext,
    staged: &Staged,
    root: PathBuf,
    values: BTreeMap<String, Value>,
    host: &Rc<RefCell<TodoHost>>,
) -> (gpui::WindowHandle<Empty>, DescriptionSession) {
    let policy = todo_policy(staged, values, host);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let session = window
        .update(cx, |_, window, cx| DescriptionSession::load(runtime, root, policy, window, cx))
        .unwrap()
        .unwrap();
    cx.run_until_parked();
    (window, session)
}

// Hands the scenario one command; its 50 ms poll runs it and every write it starts finishes.
fn todo_step(cx: &mut TestAppContext, host: &Rc<RefCell<TodoHost>>, command: &str) {
    host.borrow_mut().commands.push_back(command.into());
    cx.executor().advance_clock(std::time::Duration::from_millis(50));
    cx.run_until_parked();
}

fn todo_state(cx: &mut TestAppContext, host: &Rc<RefCell<TodoHost>>) -> Value {
    todo_step(cx, host, "state");
    serde_json::from_str(host.borrow().notes.last().expect("the scenario reports its state")).unwrap()
}

fn todo_describe(cx: &mut TestAppContext, window: gpui::WindowHandle<Empty>, mut session: DescriptionSession) -> String {
    let tree = window.update(cx, |_, window, cx| session.describe(window, cx)).unwrap().unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    tree
}

fn todo_values() -> BTreeMap<String, Value> {
    fixture(true, props(), updates("current", "current"))
}

// What was written to the tasks list, in order.
fn todo_written(host: &Rc<RefCell<TodoHost>>) -> Vec<String> {
    host.borrow().writes.iter().filter(|(p, _, _)| p == TODO_TASKS).map(|(_, t, _)| t.clone()).collect()
}

fn todo_effects(host: &Rc<RefCell<TodoHost>>, name: &str) -> Vec<Value> {
    host.borrow().effects.iter().filter(|(n, _)| n == name).map(|(_, a)| json!(a)).collect()
}

fn todo_celebrations(host: &Rc<RefCell<TodoHost>>) -> usize {
    todo_effects(host, "openOverlay").iter().filter(|a| a[0]["props"].get("celebrate").is_some()).count()
}

#[gpui::test]
fn todo_first_open_creates_the_index_and_an_empty_tasks_list(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    let tree = todo_describe(cx, window, session);
    assert_eq!(
        host.borrow().writes,
        vec![
            (TODO_TASKS.to_string(), "# Tasks\n".to_string(), Value::Null),
            (TODO_LISTS.to_string(), "- [Tasks](tasks.md)\n".to_string(), Value::Null),
        ],
        "both files are created, never over an existing one"
    );
    for expected in ["Tasks", "todo-add", "No tasks yet. Add one above.", "0 open"] {
        assert!(tree.contains(expected), "missing {expected}: {tree}");
    }
    assert!(!tree.contains("All done for today") && !tree.contains(TODO_READ_FAIL), "{tree}");
    assert_eq!(todo_celebrations(&host), 0, "nothing to celebrate in an empty list");
}

// A lost index (0 lists) is rebuilt; the tasks file already there is kept as it is.
#[gpui::test]
fn todo_lost_index_is_rebuilt_without_touching_an_existing_tasks_file(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tasks = "# Tasks\r\n- [ ] keep me\r\n";
    let host = todo_host(&[(TODO_TASKS, tasks)], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    let tree = todo_describe(cx, window, session);
    assert_eq!(host.borrow().files[TODO_TASKS], tasks, "the existing list is not overwritten");
    assert_eq!(host.borrow().files[TODO_LISTS], "- [Tasks](tasks.md)\n");
    assert!(tree.contains("keep me") && tree.contains("1 open"), "{tree}");
}

#[gpui::test]
fn todo_adding_and_ticking_tasks_writes_the_markdown_over_what_was_read(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] Old\n")], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    assert!(host.borrow().writes.is_empty(), "opening an existing list writes nothing");
    for command in ["add Milk 🥛", "add Ship [ ] it", "add    ", "add two\nlines", "tick 2"] {
        todo_step(cx, &host, command);
    }
    let written = todo_written(&host);
    assert_eq!(
        written,
        [
            "# Tasks\n- [ ] Milk 🥛\n- [ ] Old\n",
            "# Tasks\n- [ ] Ship [ ] it\n- [ ] Milk 🥛\n- [ ] Old\n",
            "# Tasks\n- [ ] two lines\n- [ ] Ship [ ] it\n- [ ] Milk 🥛\n- [ ] Old\n",
            "# Tasks\n- [ ] two lines\n- [ ] Ship [ ] it\n- [x] Milk 🥛\n- [ ] Old\n",
        ],
        "a blank title adds nothing; a newline becomes a space"
    );
    // Each write expects the text the one before it left.
    let writes = host.borrow().writes.clone();
    assert_eq!(writes[0].2, json!("# Tasks\n- [ ] Old\n"));
    for pair in writes.windows(2) {
        assert_eq!(pair[1].2, json!(pair[0].1), "{writes:?}");
    }
    let state = todo_state(cx, &host);
    assert_eq!((state["error"].clone(), state["pending"].clone()), (Value::Null, json!(0)), "{state}");
    let tree = todo_describe(cx, window, session);
    assert!(tree.contains("Completed (1)") && tree.contains("3 open") && tree.contains("Ship [ ] it"), "{tree}");
    assert_eq!(todo_celebrations(&host), 0, "open tasks remain");
}

// AC4: an agent or editor changed the file after we read it; our tick lands on its text.
#[gpui::test]
fn todo_stale_write_rereads_and_reapplies_without_losing_the_other_edit(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] A\n- [ ] B\n")], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    let theirs = "# Tasks\n- [ ] From the agent\n- [ ] A\n  > a note the agent added\n- [ ] B\n";
    host.borrow_mut().files.insert(TODO_TASKS.into(), theirs.into());
    todo_step(cx, &host, "tick 0");
    let merged = "# Tasks\n- [ ] From the agent\n- [x] A\n  > a note the agent added\n- [ ] B\n";
    assert_eq!(host.borrow().files[TODO_TASKS], merged, "both changes are kept");
    let writes = host.borrow().writes.clone();
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert_eq!(writes[0].2, json!("# Tasks\n- [ ] A\n- [ ] B\n"), "the first write is refused as stale");
    assert_eq!(writes[1], (TODO_TASKS.to_string(), merged.to_string(), json!(theirs)));
    let state = todo_state(cx, &host);
    assert_eq!(state["text"], json!(merged), "{state}");
    assert_eq!((state["error"].clone(), state["pending"].clone()), (Value::Null, json!(0)), "{state}");

    // A change made outside appears on the next look, and nothing is written for it.
    let outside = "# Tasks\n- [ ] From the agent\n- [x] A\n  > a note the agent added\n- [ ] B\n- [ ] C\n";
    host.borrow_mut().files.insert(TODO_TASKS.into(), outside.into());
    todo_step(cx, &host, "reload");
    assert_eq!(todo_state(cx, &host)["text"], json!(outside));
    assert_eq!(host.borrow().writes.len(), 2);
    let tree = todo_describe(cx, window, session);
    assert!(tree.contains("From the agent") && tree.contains("C"), "{tree}");
}

// Big lists of ordinary tasks open: every size up to the panel's write limit finishes loading.
#[gpui::test]
fn todo_big_lists_of_many_tasks_open(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut seen = Vec::new();
    for size in [8 * 1024, 32 * 1024, 64 * 1024] {
        let mut text = String::from("# Tasks\n");
        let mut n = 0;
        while text.len() + 40 < size {
            n += 1;
            text.push_str(&format!("- [ ] task number {n}\n"));
        }
        let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, &text)], &[]);
        let (window, _session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
        let mut ready = false;
        for _ in 0..2400 {
            if todo_state(cx, &host)["ready"] == json!(true) {
                ready = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
            cx.run_until_parked();
        }
        let _ = window.update(cx, |_, window, _| window.remove_window());
        seen.push((size, n, ready));
    }
    assert!(seen.iter().all(|s| s.2), "size, tasks, opened: {seen:?}");
}

// AC8: a list over the 1 MiB read limit is shown read-only and never written; at the limit it is
// an editable list that refuses to grow past the limit but still takes same-size changes.
#[gpui::test]
fn todo_list_over_the_read_limit_says_so_and_is_never_written(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let head = "# Tasks\n- [ ] edge\n";
    let full = "This list is full (64 KB, about 2,900 tasks). Start a new list for more tasks.";

    // Exactly at the panel's 64 KB write limit (a few tasks and a long note, so the panel stays
    // small enough to describe; `todo_big_lists_of_many_tasks_open` covers many short tasks):
    // editable; an add would cross the limit and is refused with the reason, a tick is written.
    let size = 64 * 1024;
    let mut big = format!("{head}- [ ] second\n  - [ ] a step\n");
    let pad = size - big.len() - "  > \n".len();
    big.push_str(&format!("  > {}\n", "y".repeat(pad)));
    assert_eq!(big.len(), size);
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, &big)], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    // A big list takes a moment of real time to arrive and parse; changes before that are ignored
    // by design, so wait (in real time, not only on the test clock) until it is open.
    let mut state = todo_state(cx, &host);
    for _ in 0..2400 {
        if state["ready"] == json!(true) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        cx.run_until_parked();
        state = todo_state(cx, &host);
    }
    assert_eq!(state["ready"], json!(true), "the list finished loading");
    assert_eq!((state["readonly"].clone(), state["error"].clone()), (json!(false), Value::Null), "loads editable");
    todo_step(cx, &host, "add New");
    assert!(host.borrow().writes.is_empty(), "an add past the limit writes nothing: {}", host.borrow().writes.len());
    assert_eq!(host.borrow().files[TODO_TASKS], big, "the file is unchanged");
    let state = todo_state(cx, &host);
    assert_eq!(state["readonly"], json!(false), "still editable");
    // The panel's text stays exactly what was read: the refused task is not shown either.
    assert_eq!(state["textLen"], json!(big.chars().count()), "the refused task is not shown either");
    // The reason is the panel's error line (`this.error`); a description renders the panel afresh.
    assert_eq!(state["error"], json!(full), "the reason is shown");
    let tree = todo_describe(cx, window, session);
    assert!(tree.contains("todo-add") && !tree.contains(TODO_READ_FAIL), "the add box stays; not read-only");

    // A same-size change (a tick) at the limit is written over what was read.
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, &big)], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    // Up to a minute: Windows' debug runner can be slow to open a 64 KB list; a tick before that is ignored.
    let mut ready = false;
    for _ in 0..2400 {
        if todo_state(cx, &host)["ready"] == json!(true) {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        cx.run_until_parked();
    }
    assert!(ready, "the list finished loading before the tick");
    todo_step(cx, &host, "tick 0");
    // The write of a 64 KB list lands a moment later on a slow runner: wait for it (real time).
    for _ in 0..200 {
        if !host.borrow().writes.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        cx.run_until_parked();
    }
    let ticked = big.replacen("- [ ] edge", "- [x] edge", 1);
    assert_eq!(ticked.len(), size);
    assert_eq!(todo_written(&host), [ticked.clone()], "a same-size change is written");
    assert_eq!(host.borrow().writes[0].2, json!(big), "over what was read");
    assert_eq!(host.borrow().files[TODO_TASKS], ticked);
    let state = todo_state(cx, &host);
    assert_eq!((state["readonly"].clone(), state["error"].clone()), (json!(false), Value::Null), "{}", state["error"]);
    let tree = todo_describe(cx, window, session);
    assert!(tree.contains("todo-add") && !tree.contains(full) && !tree.contains(TODO_READ_FAIL), "{tree}");

    // Over the limit: read-only, the reason shown, never written.
    for (size, refused) in [(1024 * 1024 + 1, true)] {
        let big = format!("{head}{}\n", "x".repeat(size - head.len() - 1));
        assert_eq!(big.len(), size);
        let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, &big)], &[]);
        let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
        todo_step(cx, &host, "add New");
        todo_step(cx, &host, "tick 0");
        let state = todo_state(cx, &host);
        assert_eq!(state["readonly"], json!(refused), "{size}: {state}");
        let tree = todo_describe(cx, window, session);
        assert_eq!(tree.contains(TODO_READ_FAIL), refused, "{size}: the reason is shown");
        assert_eq!(tree.contains("todo-add"), !refused, "{size}: no add box on a read-only list");
        assert!(host.borrow().writes.is_empty(), "{size}: {:?}", host.borrow().writes.len());
        assert_eq!(host.borrow().files[TODO_TASKS], big, "left untouched");
    }
}

// AC7: the pop-up comes once, on the tick that finishes the list; never on load, a step or a delete.
#[gpui::test]
fn todo_congratulations_open_once_on_the_last_tick_and_never_on_load(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let done = "# Tasks\n- [x] a\n- [x] b\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, done)], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    todo_step(cx, &host, "reload");
    let tree = todo_describe(cx, window, session);
    assert!(tree.contains("All done for today"), "{tree}");
    assert_eq!(todo_celebrations(&host), 0, "a finished list on load is not celebrated");

    let host = todo_host(&[(TODO_LISTS, "- [Groceries](tasks.md)\n"), (TODO_TASKS, "# Groceries\n- [ ] a\n  - [ ] step\n- [x] b\n- [ ] c\n")], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    todo_step(cx, &host, "tick 0.0");
    todo_step(cx, &host, "tick 0");
    assert_eq!(todo_celebrations(&host), 0, "c is still open");
    todo_step(cx, &host, "tick 2");
    assert_eq!(
        todo_effects(&host, "openOverlay"),
        [json!([{"mod":"todo", "kind":"dialog", "width":400, "props":{"celebrate":"Groceries"}}])],
        "the last open task is ticked"
    );
    // Unticking, then deleting the open one, finishes the list without a tick: no second pop-up.
    todo_step(cx, &host, "tick 2");
    todo_step(cx, &host, "delete 2");
    todo_step(cx, &host, "reload");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Groceries\n- [x] a\n  - [x] step\n- [x] b\n");
    assert_eq!(todo_celebrations(&host), 1);
    todo_describe(cx, window, session);

    // The pop-up itself: it reads and writes nothing.
    let host = todo_host(&[], &[]);
    let mut p = props();
    p["celebrate"] = json!("Groceries");
    let values = fixture(true, p, updates("current", "current"));
    let (window, session) = load_todo(cx, &staged, staged.wrapper("mods/todo", ""), values, &host);
    let tree = todo_describe(cx, window, session);
    for expected in ["Congratulations!", "You finished every task in Groceries.", "todo-celebrate", "todo-close"] {
        assert!(tree.contains(expected), "missing {expected}: {tree}");
    }
    assert!(host.borrow().writes.is_empty() && host.borrow().effects.is_empty());
}

// AC2: a delete posts an Undo notice; undo puts the task (with its steps) or the step back once.
#[gpui::test]
fn todo_delete_posts_an_undo_notice_and_undo_restores_it_once(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] a\n- [ ] b\n  - [ ] b1\n  > b's note\n- [ ] c\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    todo_step(cx, &host, "delete 1");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [ ] a\n- [ ] c\n");
    assert_eq!(
        todo_effects(&host, "notify"),
        [json!([{"title":"Task deleted", "body":"b", "timeout":8000, "action":{"name":"todo.undo"}}])]
    );
    todo_step(cx, &host, "undo");
    assert_eq!(host.borrow().files[TODO_TASKS], start, "the task comes back where it was, steps and note included");
    let count = host.borrow().writes.len();
    todo_step(cx, &host, "undo");
    assert_eq!(host.borrow().writes.len(), count, "a second undo does nothing");

    todo_step(cx, &host, "delete 1.0");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [ ] a\n- [ ] b\n- [ ] c\n");
    assert_eq!(todo_effects(&host, "notify")[1][0]["title"], json!("Step deleted"));
    todo_step(cx, &host, "undo");
    assert_eq!(host.borrow().files[TODO_TASKS], start);
    todo_describe(cx, window, session);
}

// AC2 keys, through the panel's key handler: arrows, Tab/Shift+Tab, Alt+Up, Space, Delete, Enter.
#[gpui::test]
fn todo_keys_move_indent_reorder_tick_delete_and_edit(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] one\n- [ ] two\n- [ ] three\n")], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    assert_eq!(todo_state(cx, &host)["sel"], json!({"k":"task", "i":0}));
    todo_step(cx, &host, "key down");
    todo_step(cx, &host, "key tab");
    assert_eq!(todo_state(cx, &host)["sel"], json!({"k":"step", "i":0, "j":0}), "the new step stays selected");
    todo_step(cx, &host, "key shift-tab");
    assert_eq!(todo_state(cx, &host)["sel"], json!({"k":"task", "i":1}));
    todo_step(cx, &host, "key alt-up");
    assert_eq!(todo_state(cx, &host)["sel"], json!({"k":"task", "i":0}), "the moved task stays selected");
    todo_step(cx, &host, "key space");
    // The ticked task leaves the open ones; down twice passes one's empty "Add step" row.
    for command in ["key down", "key down", "key delete"] {
        todo_step(cx, &host, command);
    }
    assert_eq!(
        todo_written(&host),
        [
            "# Tasks\n- [ ] one\n  - [ ] two\n- [ ] three\n",
            "# Tasks\n- [ ] one\n- [ ] two\n- [ ] three\n",
            "# Tasks\n- [ ] two\n- [ ] one\n- [ ] three\n",
            "# Tasks\n- [x] two\n- [ ] one\n- [ ] three\n",
            "# Tasks\n- [x] two\n- [ ] one\n",
        ]
    );
    assert_eq!(todo_effects(&host, "notify")[0][0]["body"], json!("three"));
    todo_step(cx, &host, "key up");
    todo_step(cx, &host, "key up");
    todo_step(cx, &host, "key enter");
    let state = todo_state(cx, &host);
    assert_eq!((state["sel"].clone(), state["edit"].clone()), (json!({"k":"task", "i":1}), json!("one")), "{state}");
    todo_describe(cx, window, session);
}

// Tab under a task that already has steps: the moved task is the one selected, so Space ticks it.
#[gpui::test]
fn todo_tab_under_a_task_with_steps_keeps_the_moved_task_selected(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] one\n  - [ ] a\n- [ ] two\n")], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    for command in ["key down", "key tab", "key space"] {
        todo_step(cx, &host, command);
    }
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [ ] one\n  - [ ] a\n  - [x] two\n");
    todo_describe(cx, window, session);
}

// Real key presses reach the panel once it has the keyboard (todo.focus), and N types a new task.
#[gpui::test]
fn todo_key_presses_tick_and_add_tasks_in_a_mounted_panel(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] one\n- [ ] two\n")], &[]);
    let root = todo_root(&staged);
    gpui_shell::policy::set_default(todo_policy(&staged, todo_values(), &host));
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| FilesMounted(runtime.try_load(&root, window, cx).unwrap()));
    let _root = window.update(cx, |mounted, _, _| mounted.0.clone()).unwrap();
    let mut visual = gpui::VisualTestContext::from_window(*window, cx);
    visual.run_until_parked();
    host.borrow_mut().commands.push_back("focus".into());
    files_wait(&mut visual, 50);
    visual.update(|window, cx| {
        window.activate_window();
        window.draw(cx).clear(cx)
    });
    files_keys(&mut visual, "down space");
    files_wait(&mut visual, 50);
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [ ] one\n- [x] two\n");
    files_keys(&mut visual, "n");
    visual.simulate_input("Milk");
    files_keys(&mut visual, "enter");
    files_wait(&mut visual, 50);
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [ ] Milk\n- [ ] one\n- [x] two\n");
    // The box stays ready for the next one.
    visual.simulate_input("Bread");
    files_keys(&mut visual, "enter");
    files_wait(&mut visual, 50);
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [ ] Bread\n- [ ] Milk\n- [ ] one\n- [x] two\n");
}

// AC1: tabs.todo opens the panel on To-do and hands it the keyboard; pressed on it, it closes.
#[gpui::test]
fn tab_bar_todo_command_opens_focuses_and_closes_the_panel(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let open = [json!(["layout.select", {"stack":"panel-body", "child":"todo"}]), json!(["layout.show", {"id":"panel"}]), json!(["todo.focus"])];
    for (hidden, active, shared, expected) in [
        (true, "files", vec![], open.to_vec()),
        (false, "files", vec![], open.to_vec()),
        (false, "todo", vec![], vec![json!(["layout.hide", {"id":"panel"}])]),
        // Hidden in Settings: the key does nothing.
        (true, "files", vec![("workspace.panels", json!(["files"])), ("workspace.panels.added", json!(["sessions", "todo"]))], vec![]),
    ] {
        let host = todo_host(&[], &shared);
        let mut values = todo_values();
        values.insert("layout".into(), panel_layout(hidden, active));
        let (window, session) = load_todo(cx, &staged, staged.wrapper("mods/tabs", "this.toggleTodo();"), values, &host);
        let tree = todo_describe(cx, window, session);
        assert_eq!(todo_effects(&host, "run"), expected, "hidden {hidden}, on {active}, {shared:?}");
        assert_eq!(tree.contains("panel-todo"), shared.is_empty(), "{tree}");
    }
}

// AC6: Settings → Look turns the To-do button on and off in workspace.panels; the files stay.
#[gpui::test]
fn settings_todo_toggle_edits_the_panel_list_and_keeps_the_files(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let files = [(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] keep\n")];
    let all = json!(["files", "changes", "sessions", "todo"]);
    for (stored, on, active, after, hides) in [
        (Value::Null, false, "todo", json!(["files", "changes", "sessions"]), true),
        (all.clone(), false, "files", json!(["files", "changes", "sessions"]), false),
        (json!(["files"]), true, "files", json!(["files", "todo"]), false),
        (json!(["todo", "files"]), true, "todo", json!(["files", "todo"]), false),
    ] {
        let shared: Vec<(&str, Value)> = if stored.is_null() { vec![] } else { vec![("workspace.panels", stored.clone())] };
        let host = todo_host(&files, &shared);
        let mut p = props();
        p["section"] = json!("look");
        let mut values = fixture(true, p, updates("current", "current"));
        values.insert("layout".into(), panel_layout(false, active));
        let setup = format!("this.setTodoPanel({on});");
        let (window, session) = load_todo(cx, &staged, staged.wrapper("frame/settings", &setup), values, &host);
        let tree = todo_describe(cx, window, session);
        assert!(tree.contains("st-todo") && tree.contains("To-do list"), "{tree}");
        let h = host.borrow();
        assert_eq!(h.shared["workspace.panels"], after, "{stored} → {on}");
        let hid = h.effects.iter().any(|(n, a)| n == "run" && a == &vec![json!("layout.hide"), json!({"id":"panel"})]);
        assert_eq!(hid, hides, "{stored} → {on}: {:?}", h.effects);
        assert!(h.removes.is_empty() && h.writes.iter().all(|(p, _, _)| !p.contains("/todo/")), "files are kept");
        assert_eq!(h.files[TODO_TASKS], "# Tasks\n- [ ] keep\n");
    }
}

// AC6: turned off in Settings on a fresh install, To-do stays off in the tab bar; on again, it is back.
#[gpui::test]
fn todo_turned_off_in_settings_stays_off_in_the_tab_bar(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[], &[]);
    let mut p = props();
    p["section"] = json!("look");
    let (window, session) = load_todo(cx, &staged, staged.wrapper("frame/settings", "this.setTodoPanel(false);"), fixture(true, p.clone(), updates("current", "current")), &host);
    todo_describe(cx, window, session);
    let (window, session) = load_todo(cx, &staged, staged.wrapper("mods/tabs", ""), todo_values(), &host);
    let tree = todo_describe(cx, window, session);
    assert!(!tree.contains("panel-todo") && tree.contains("panel-sessions"), "{:?}: {tree}", host.borrow().shared);
    let (window, session) = load_todo(cx, &staged, staged.wrapper("frame/settings", "this.setTodoPanel(true);"), fixture(true, p, updates("current", "current")), &host);
    todo_describe(cx, window, session);
    let (window, session) = load_todo(cx, &staged, staged.wrapper("mods/tabs", ""), todo_values(), &host);
    let tree = todo_describe(cx, window, session);
    assert!(tree.contains("panel-todo"), "{:?}: {tree}", host.borrow().shared);
}

#[test]
fn todo_key_is_a_shipped_mod_command_on_every_platform() {
    for (text, key) in [(include_str!("../../defaults/keys.json"), "ctrl-shift-j"), (include_str!("../../defaults/keys.macos.json"), "cmd-shift-j")] {
        crate::keys::parse(text).unwrap();
        assert_eq!(text.matches(&format!("\"{key}\"")).count(), 1, "{key} is bound once");
        let sections: BTreeMap<String, BTreeMap<String, String>> = serde_json::from_str(text).unwrap();
        assert_eq!(sections["global"].get(key).map(String::as_str), Some("tabs.todo"), "{key}");
    }
    // Not an app command: it goes to the tab bar mod, which acts on "todo".
    assert!(crate::commands::find("tabs.todo").is_none());
    assert!(include_str!("../../defaults/mods/tabs/main.js").contains("=== \"todo\") this.toggleTodo()"));
}

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

// `entry` with settings.json in `values`' app home holding `settings` (None: missing) and `setup` run after its
// init. A press is what `setup` leaves in `this.press`: a timer runs it with a live cx, as a click would. Describes
// the entry once its tasks have finished, then again after each of `presses` presses; returns each description.
fn describe_pressing(
    cx: &mut TestAppContext,
    staged: &Staged,
    entry: &str,
    values: BTreeMap<String, Value>,
    settings: Option<&str>,
    setup: &str,
    presses: usize,
) -> (Vec<String>, Rc<RefCell<Calls>>) {
    let calls = Rc::new(RefCell::new(Calls::default()));
    // Any scripted reply records the disk; no core or exec call starts with this one.
    calls.borrow_mut().replies.insert("fixture-unused".into(), Value::Null);
    if let Some(text) = settings {
        let home = values["appHome"].as_str().unwrap();
        calls.borrow_mut().files.insert(format!("{home}/settings.json"), text.into());
    }
    let policy = Policy::new()
        .with_capabilities(Capabilities::new().read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    let setup = format!("{setup} cx.timer.every(50, (cx) => {{ const press = this.press; this.press = null; if (press) {{ press(null, cx); cx.notify(); }} }});");
    let root = staged.wrapper(entry, &setup);
    let window = cx.add_window(|_, _| Empty);
    let mut session = window
        .update(cx, |_, window, cx| DescriptionSession::load(ShellRuntime::new_isolated().unwrap(), root, policy, window, cx))
        .unwrap()
        .unwrap();
    let mut trees = Vec::new();
    for _ in 0..=presses {
        advance(cx, 150);
        trees.push(window.update(cx, |_, window, cx| session.describe(window, cx)).unwrap().unwrap());
    }
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    (trees, calls)
}

fn info_values() -> BTreeMap<String, Value> {
    let mut p = props();
    p["section"] = json!("info");
    fixture(true, p, updates("current", "current"))
}

// The written settings.json files, each read back as JSON.
fn written(calls: &Rc<RefCell<Calls>>, path: &str) -> Vec<Value> {
    let c = calls.borrow();
    c.writes
        .iter()
        .map(|(p, text)| {
            assert_eq!(p, path, "only settings.json is written: {:?}", c.writes);
            serde_json::from_str(text).unwrap_or_else(|e| panic!("{text:?}: {e}"))
        })
        .collect()
}

const ANALYTICS_SWITCH_PRESS: &str = "const toggle = this.toggle.bind(this); \
    this.toggle = (t, id, on, onClick) => { if (id === 'st-analytics') this.press = onClick; return toggle(t, id, on, onClick); };";

#[gpui::test]
fn settings_analytics_switch_turns_sharing_off_then_on_again_keeping_every_other_key(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let off = json!({"share_analytics": false});
    // settings.json, then what each of two presses writes: off is share_analytics false, on removes the key.
    // Only false is off; anything else shows (and so turns) as on.
    for (file, first, second) in [
        (None, off.clone(), json!({})),
        (Some("{}"), off.clone(), json!({})),
        (Some(r#"{"share_analytics": true, "zoom": 1.25}"#), json!({"share_analytics": false, "zoom": 1.25}), json!({"zoom": 1.25})),
        (Some(r#"{"share_analytics": false, "zoom": 1.25}"#), json!({"zoom": 1.25}), json!({"share_analytics": false, "zoom": 1.25})),
        (Some(r#"{"share_analytics": null}"#), off.clone(), json!({})),
        (Some(r#"{"share_analytics": "no"}"#), off.clone(), json!({})),
        (Some(r#"{"share_analytics": 0}"#), off.clone(), json!({})),
        (Some("\u{feff}{\"share_analytics\": false}"), json!({}), off.clone()),
    ] {
        let (trees, calls) = describe_pressing(cx, &staged, "frame/settings", info_values(), file, ANALYTICS_SWITCH_PRESS, 2);
        assert_eq!(written(&calls, "/fixture/app/settings.json"), [first, second], "{file:?}");
        for tree in &trees {
            assert!(tree.contains("Share usage analytics") && tree.contains("st-analytics"), "{file:?}: {tree}");
            assert!(!tree.contains("can't be read"), "{file:?}: {tree}");
        }
        assert!(calls.borrow().unexpected.is_empty(), "{file:?}: {:?}", calls.borrow().unexpected);
    }
}

#[gpui::test]
fn settings_analytics_switch_never_overwrites_a_settings_file_it_could_not_read(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for bad in ["", "   ", "{not json", "[]", "null", "\"share_analytics\""] {
        let (trees, calls) = describe_pressing(cx, &staged, "frame/settings", info_values(), Some(bad), ANALYTICS_SWITCH_PRESS, 1);
        assert!(calls.borrow().writes.is_empty(), "{bad:?}: {:?}", calls.borrow().writes);
        assert!(trees[1].contains("can't be read"), "{bad:?}: the page says why nothing changed: {}", trees[1]);
    }
}

const ANALYTICS_TITLE: &str = "How HyperOx handles your data (beta)";
const ANALYTICS_ABOUT: [&str; 3] = [
    "During the beta, HyperOx sends usage data and failure reports so we can fix problems and improve the app. You are identified by a random install ID, not your name or email. We never send what you type, your prompts, agent replies, terminal contents, file or folder names, or your keys. IP addresses are not stored; your approximate location (country and city) is.",
    "After the beta we plan to move data collection to zero-knowledge proofs.",
    "You can turn usage analytics off any time. Failure reports stay on so we can fix crashes and update problems.",
];

#[gpui::test]
fn settings_analytics_about_is_closed_until_asked_and_click_here_opens_only_pavii_tech(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // Closed at first, whatever the switch says.
    for file in [None, Some(r#"{"share_analytics": false}"#)] {
        let (trees, _) = describe_pressing(cx, &staged, "frame/settings", info_values(), file, "", 0);
        assert!(trees[0].contains("st-analytics-info"), "{file:?}: the i button is there: {}", trees[0]);
        assert!(!trees[0].contains(ANALYTICS_TITLE) && !trees[0].contains("st-analytics-more") && !trees[0].contains("Click here"), "{file:?}: {}", trees[0]);
        assert!(ANALYTICS_ABOUT.iter().all(|p| !trees[0].contains(p)), "{file:?}: {}", trees[0]);
    }
    // Open: the title, the three paragraphs word for word, and Click here, which opens pavii.tech.
    let setup = "this.analyticsAbout = true; const button = this.button.bind(this); \
        this.button = (t, id, label, onClick) => { if (id === 'st-analytics-more') this.press = onClick; return button(t, id, label, onClick); };";
    let (trees, calls) = describe_pressing(cx, &staged, "frame/settings", info_values(), None, setup, 1);
    for tree in &trees {
        for text in [ANALYTICS_TITLE, ANALYTICS_ABOUT[0], ANALYTICS_ABOUT[1], ANALYTICS_ABOUT[2], "Click here", "st-analytics-more"] {
            assert!(tree.contains(text), "the open panel shows {text:?}: {tree}");
        }
    }
    let c = calls.borrow();
    let links: Vec<&Value> = c.effects.iter().filter(|(n, _)| n == "openLink").map(|(_, a)| &a[0]).collect();
    assert_eq!(links, [&json!("https://pavii.tech")], "{:?}", c.effects);
    assert!(allowed_link("https://pavii.tech"));
    assert!(!c.effects.iter().any(|(n, _)| n == "notify"), "no \"Can't open the link\": {:?}", c.effects);
    assert!(c.writes.is_empty() && c.unexpected.is_empty(), "{:?} {:?}", c.writes, c.unexpected);
}

const FOUND_VIA_QUESTION: &str = "How did you find HyperOx?";
const FOUND_VIA: [(&str, &str, &str); 6] = [
    ("w-found-friend", "Friend", "friend"),
    ("w-found-x", "X / Twitter", "x"),
    ("w-found-github", "GitHub", "github"),
    ("w-found-search", "Search", "search"),
    ("w-found-other", "Other", "other"),
    ("w-found-skip", "Skip", "skip"),
];

// The welcome on its last page, pressing its button `id`.
fn welcome_pressing(id: &str) -> String {
    format!(
        "this.page = 4; const button = this.button.bind(this); \
        this.button = (t, id, label, primary, onClick, icon) => {{ if (id === '{id}') this.press = onClick; return button(t, id, label, primary, onClick, icon); }};"
    )
}

#[gpui::test]
fn welcome_asks_how_you_found_hyperox_once_and_saves_each_answer_with_the_other_keys(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (id, _, answer) in FOUND_VIA {
        let values = fixture(true, props(), updates("current", "current"));
        let (trees, calls) = describe_pressing(cx, &staged, "frame/welcome", values, Some(r#"{"appearance": "dark", "zoom": 1.25}"#), &welcome_pressing(id), 1);
        assert!(trees[0].contains(FOUND_VIA_QUESTION) && trees[0].contains("Make it yours"), "{id}: {}", trees[0]);
        for (id, label, _) in FOUND_VIA {
            assert!(trees[0].contains(id) && trees[0].contains(label), "{id} {label:?}: {}", trees[0]);
        }
        assert_eq!(written(&calls, "/fixture/app/settings.json"), [json!({"appearance": "dark", "zoom": 1.25, "found_via": answer})], "{id}");
        // Answered: gone, and the rest of the page (and Done) is still there.
        assert!(!trees[1].contains(FOUND_VIA_QUESTION) && !trees[1].contains("w-found-"), "{id}: {}", trees[1]);
        assert!(trees[1].contains("Make it yours") && trees[1].contains("w-start"), "{id}: {}", trees[1]);
        assert!(calls.borrow().unexpected.is_empty(), "{id}: {:?}", calls.borrow().unexpected);
    }
}

#[gpui::test]
fn welcome_asks_only_while_found_via_was_never_set_and_only_on_the_last_page(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (file, asked) in [
        (None, true),
        (Some("{}"), true),
        (Some(r#"{"found_via": null}"#), true),
        (Some(r#"{"zoom": 1.25, "share_analytics": false}"#), true),
        // Any answer counts as asked, even one this version doesn't know.
        (Some(r#"{"found_via": "skip"}"#), false),
        (Some(r#"{"found_via": "github"}"#), false),
        (Some(r#"{"found_via": "carrier-pigeon"}"#), false),
        (Some(r#"{"found_via": ""}"#), false),
        (Some(r#"{"found_via": 0}"#), false),
        // A settings.json the welcome can't read: it doesn't ask.
        (Some(""), false),
        (Some("{not json"), false),
        (Some("null"), false),
    ] {
        let values = fixture(true, props(), updates("current", "current"));
        let (trees, calls) = describe_pressing(cx, &staged, "frame/welcome", values, file, "this.page = 4;", 0);
        assert_eq!(trees[0].contains(FOUND_VIA_QUESTION), asked, "{file:?}: {}", trees[0]);
        assert_eq!(trees[0].contains("w-found-skip"), asked, "{file:?}: {}", trees[0]);
        assert!(trees[0].contains("w-start"), "{file:?}: {}", trees[0]);
        assert!(calls.borrow().writes.is_empty(), "{file:?}: only an answer writes: {:?}", calls.borrow().writes);
    }
    for page in 1..=3 {
        let values = fixture(true, props(), updates("current", "current"));
        let (trees, _) = describe_pressing(cx, &staged, "frame/welcome", values, None, &format!("this.page = {page};"), 0);
        assert!(!trees[0].contains(FOUND_VIA_QUESTION), "page {page}: {}", trees[0]);
    }
}

#[gpui::test]
fn welcome_done_and_open_a_space_work_without_answering_how_you_found_hyperox(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (has_spaces, label) in [(true, "Done"), (false, "Open a space")] {
        let mut values = fixture(true, props(), updates("current", "current"));
        if !has_spaces {
            values.insert("spaces".into(), json!([]));
        }
        let (trees, calls) = describe_pressing(cx, &staged, "frame/welcome", values, None, &welcome_pressing("w-start"), 1);
        assert!(trees[0].contains(FOUND_VIA_QUESTION) && trees[0].contains(label), "{label}: {}", trees[0]);
        let c = calls.borrow();
        assert_eq!(c.shared.get("frame.welcomed"), Some(&json!("finished")), "{label}: {:?}", c.shared);
        assert!(c.effects.iter().any(|(n, a)| n == "closeOverlay" && a.first() == Some(&json!("o1"))), "{label}: {:?}", c.effects);
        assert_eq!(c.effects.iter().any(|(n, a)| n == "run" && a.first() == Some(&json!("space.new"))), !has_spaces, "{label}: {:?}", c.effects);
        assert!(c.writes.is_empty(), "{label}: closing unanswered writes nothing: {:?}", c.writes);
    }
}

#[gpui::test]
fn welcome_appearance_and_answer_edit_settings_json_in_a_windows_app_home_and_keep_each_other(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let home = "C:\\Users\\Zo\u{eb} Q\\AppData\\Roaming\\h0x-ade";
    let path = format!("{home}/settings.json");
    let values = || {
        let mut values = fixture(true, props(), updates("current", "current"));
        values.insert("appHome".into(), json!(home));
        values
    };
    // setAppearance as before the edit() refactor: a value is set, System (null) removes the key, other keys stay.
    let both = r#"{"found_via": "github", "appearance": "light"}"#;
    for (file, value, expected) in [
        (both, "'dark'", Some(json!({"found_via": "github", "appearance": "dark"}))),
        (both, "null", Some(json!({"found_via": "github"}))),
        ("{}", "'light'", Some(json!({"appearance": "light"}))),
        ("{not json", "'dark'", None),
    ] {
        let setup = format!("this.page = 4; this.press = (_e, cx) => this.setAppearance({value}, cx);");
        let (_, calls) = describe_pressing(cx, &staged, "frame/welcome", values(), Some(file), &setup, 0);
        assert_eq!(written(&calls, &path), expected.into_iter().collect::<Vec<_>>(), "{file} {value}");
        assert!(calls.borrow().unexpected.is_empty(), "{file} {value}: {:?}", calls.borrow().unexpected);
    }
    // The answer, written to the same file, keeps the appearance.
    let (trees, calls) = describe_pressing(cx, &staged, "frame/welcome", values(), Some(r#"{"appearance": "dark"}"#), &welcome_pressing("w-found-skip"), 1);
    assert!(trees[0].contains(FOUND_VIA_QUESTION) && !trees[1].contains(FOUND_VIA_QUESTION), "{}\n{}", trees[0], trees[1]);
    assert_eq!(written(&calls, &path), [json!({"appearance": "dark", "found_via": "skip"})]);
    // An unreadable file is never written over, by an answer either.
    let (_, calls) = describe_pressing(cx, &staged, "frame/welcome", values(), Some("{not json"), "this.page = 4; this.press = (_e, cx) => this.setFoundVia('skip', cx);", 0);
    assert!(calls.borrow().writes.is_empty(), "{:?}", calls.borrow().writes);
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
    assert!(tree.contains("My phone") && tree.contains("st-r-unpair-i1") && tree.contains("Bot token ✓ saved"), "{tree}");
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
    assert!(tree.contains("/pair AB12CD") && tree.contains("Send this to your bot in Telegram:"), "{tree}");
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
    let setup = "this.tokenInput.set_value('123456:SECRET-TOKEN'); this.tokenAt = 0; this.saveToken(cx, true);";
    let (tree, calls) = describe_remote(cx, &staged, &replies, setup);
    let c = calls.borrow();
    let path = "/fixture/app/telegram-token.tmp";
    assert_eq!(c.writes, vec![(path.to_string(), "123456:SECRET-TOKEN".to_string())], "written once");
    assert_eq!(c.removes, vec![path.to_string(), path.to_string()], "a leftover is removed first, the file last");
    assert!(c.files.is_empty(), "no file holds the token: {:?}", c.files);
    assert!(c.cores.iter().any(|a| a.join(" ") == format!("bridge telegram token --file {path}")), "--file is called: {:?}", c.cores);
    assert!(c.cores.iter().all(|a| !a.join(" ").contains("SECRET")), "the token is never an argument: {:?}", c.cores);
    let saved: Vec<&Value> = c.effects.iter().filter(|(n, _)| n == "notify").map(|(_, a)| &a[0]).collect();
    assert_eq!(saved, [&json!({"title":"Bot token saved","level":"success"})], "one success toast: {:?}", c.effects);
    assert!(tree.contains("Bot token ✓ saved"), "{tree}");
    assert!(!tree.contains("SECRET"), "the token is never shown: {tree}");
    assert!(c.effects.iter().all(|(_, a)| !a.iter().any(|v| v.to_string().contains("SECRET"))), "the token is never in a toast: {:?}", c.effects);
}

#[gpui::test]
fn settings_remote_token_file_is_deleted_when_core_refuses(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut replies = remote_replies(bridge_ls(json!([]), false, "unset"), json!([]));
    replies.push(("bridge telegram token --file", core_error("invalid")));
    let (tree, calls) = describe_remote(cx, &staged, &replies, "this.tokenInput.set_value('9:BAD-TOKEN'); this.tokenAt = 0; this.saveToken(cx, true);");
    let c = calls.borrow();
    assert_eq!(c.writes.len(), 1, "{:?}", c.writes);
    assert_eq!(c.removes.last().map(String::as_str), Some("/fixture/app/telegram-token.tmp"), "deleted even on failure: {:?}", c.removes);
    assert!(c.files.is_empty(), "no file holds the token: {:?}", c.files);
    assert!(tree.contains("fixture invalid") && !tree.contains("BAD-TOKEN"), "{tree}");
    let toasts: Vec<&Value> = c.effects.iter().filter(|(n, _)| n == "notify").map(|(_, a)| &a[0]).collect();
    assert_eq!(toasts, [&json!({"title":"Couldn't save the bot token","body":"fixture invalid","level":"error"})], "the core's refusal is toasted: {:?}", c.effects);
}

#[gpui::test]
fn settings_remote_token_leftover_file_is_removed_before_and_after(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let path = "/fixture/app/telegram-token.tmp";
    let setup = "this.tokenInput.set_value('8:NEW-TOKEN'); this.tokenAt = 0; this.saveToken(cx, true);";
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
    let setup = "this.tokenInput.set_value('5:THROW-TOKEN'); this.tokenAt = 0; this.saveToken(cx, true);";
    let (tree, calls) = describe_remote(cx, &staged, &replies, setup);
    let c = calls.borrow();
    let path = "/fixture/app/telegram-token.tmp".to_string();
    assert_eq!(c.writes, vec![(path.clone(), "5:THROW-TOKEN".to_string())], "{:?}", c.writes);
    assert!(c.cores.iter().any(|a| a.join(" ") == format!("bridge telegram token --file {path}")), "{:?}", c.cores);
    assert_eq!(c.removes, vec![path.clone(), path], "removed before and after the throw");
    assert!(c.files.is_empty(), "no file holds the token: {:?}", c.files);
    assert!(c.unexpected.iter().all(|m| !m.contains("THROW-TOKEN")), "{:?}", c.unexpected);
    assert!(tree.contains("unexpected core") && !tree.contains("Bot token ✓ saved") && !tree.contains("THROW-TOKEN"), "{tree}");
    let toasts: Vec<&Value> = c.effects.iter().filter(|(n, _)| n == "notify").map(|(_, a)| &a[0]).collect();
    assert!(toasts.len() == 1 && toasts[0]["level"] == "error" && toasts[0]["title"] == "Couldn't save the bot token", "only the failure is toasted: {:?}", c.effects);
    assert!(!toasts[0].to_string().contains("THROW-TOKEN"), "{:?}", toasts);
}

#[gpui::test]
fn settings_remote_token_is_never_written_when_the_leftover_cannot_be_removed(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let path = "/fixture/app/telegram-token.tmp";
    let mut replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    replies.push(("bridge telegram token --file", core_ok(json!({"telegram_token":"set"}))));
    replies.push(("removeAppFile refuses", Value::Null));
    let setup = "this.tokenInput.set_value('6:NEW-TOKEN'); this.tokenAt = 0; this.saveToken(cx, true);";
    let (tree, calls) = describe_remote_with(cx, &staged, &replies, setup, "/fixture/app", &[(path, "1:OLD-TOKEN")]);
    let c = calls.borrow();
    assert!(c.writes.is_empty(), "nothing is written over a leftover: {:?}", c.writes);
    assert!(c.cores.iter().all(|a| a.get(..3) != Some(&["bridge".to_string(), "telegram".to_string(), "token".to_string()][..])), "core is never asked: {:?}", c.cores);
    assert_eq!(c.removes, vec![path.to_string(), path.to_string()], "tried first, and again in the finally");
    assert_eq!(c.files.get(path).map(String::as_str), Some("1:OLD-TOKEN"), "the leftover is untouched, not overwritten");
    // The fixture's core already reports a saved token, so the saved row may show; the failure is shown too.
    assert!(tree.contains("unexpected removeAppFile"), "{tree}");
    let toasts: Vec<&Value> = c.effects.iter().filter(|(n, _)| n == "notify").map(|(_, a)| &a[0]).collect();
    assert!(toasts.len() == 1 && toasts[0]["level"] == "error", "a failure toast, never a saved one: {:?}", c.effects);
    assert!(!toasts[0].to_string().contains("NEW-TOKEN") && !toasts[0].to_string().contains("OLD-TOKEN"), "{:?}", toasts);
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
        assert!(tree.contains("Press Pair my Telegram for a one-time code, then send it to your bot."), "{label}: {tree}");
        assert!(!tree.contains("st-r-pair-copy") && !tree.contains("Send this to your bot"), "no Copy without a live code ({label}): {tree}");
    }
    let (tree, _) = describe_remote(cx, &staged, &replies, "this.pairing = { code: 'DAY123', expires_ms: Date.now() + 86400000 };");
    assert!(tree.contains("/pair DAY123") && tree.contains("Send this to your bot in Telegram:"), "{tree}");
    assert!(tree.contains("works for 86400 s.") || tree.contains("works for 86399 s."), "a day ahead counts whole seconds: {tree}");
    let far = "this.pairing = { code: 'FAR999', expires_ms: Date.now() + 10 * 365 * 86400000 };";
    let (tree, _) = describe_remote(cx, &staged, &replies, far);
    assert!(tree.contains("/pair FAR999") && !tree.contains("NaN") && !tree.contains("Infinity"), "{tree}");
}

#[gpui::test]
fn settings_remote_blank_token_makes_no_call(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let replies = remote_replies(bridge_ls(json!([]), false, "unset"), json!([]));
    // Enter (the submit handler: saves any text) and the pause (shape only) alike.
    for save in ["this.tokenAt = 0; this.saveToken(cx, true);", "this.saveToken(cx);"] {
        for value in ["", "   ", "\\t \\t"] {
            let setup = format!("this.tokenInput.set_value('{value}'); {save}");
            let (tree, calls) = describe_remote(cx, &staged, &replies, &setup);
            let c = calls.borrow();
            assert!(c.writes.is_empty(), "nothing is written for {value:?} ({save}): {:?}", c.writes);
            assert!(c.removes.is_empty(), "nothing is removed for {value:?} ({save}): {:?}", c.removes);
            assert!(c.cores.iter().all(|a| !a.join(" ").starts_with("bridge telegram")), "no call for {value:?} ({save}): {:?}", c.cores);
            assert!(c.effects.iter().all(|(n, _)| n != "notify"), "no toast for {value:?} ({save}): {:?}", c.effects);
            assert!(tree.contains("Not set up") && !tree.contains("Bot token ✓ saved"), "{value:?}: {tree}");
        }
    }
}

#[gpui::test]
fn settings_remote_long_padded_token_is_trimmed_and_never_an_argument(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    replies.push(("bridge telegram token --file", core_ok(json!({"telegram_token":"set"}))));
    let setup = "this.tokenInput.set_value('  ' + 'Z'.repeat(4096) + '  \\n'); this.tokenAt = 0; this.saveToken(cx, true);";
    let (tree, calls) = describe_remote(cx, &staged, &replies, setup);
    let c = calls.borrow();
    let path = "/fixture/app/telegram-token.tmp".to_string();
    assert_eq!(c.writes, vec![(path.clone(), "Z".repeat(4096))], "trimmed, written whole");
    assert_eq!(c.removes.last(), Some(&path), "then deleted");
    assert!(c.files.is_empty(), "no file holds the token: {:?}", c.files);
    assert!(c.cores.iter().all(|a| !a.join(" ").contains("ZZZZ")), "the token is never an argument: {:?}", c.cores);
    assert!(c.effects.iter().any(|(n, a)| n == "notify" && a[0] == json!({"title":"Bot token saved","level":"success"})), "{:?}", c.effects);
    assert!(tree.contains("Bot token ✓ saved") && !tree.contains("ZZZZ"), "{tree}");
}

#[gpui::test]
fn settings_remote_token_file_sits_in_a_windows_app_home(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    replies.push(("bridge telegram token --file", core_ok(json!({"telegram_token":"set"}))));
    let home = r"C:\Users\Ana López\AppData\Roaming\h0x-ade";
    let setup = "this.tokenInput.set_value('7:WIN-TOKEN'); this.tokenAt = 0; this.saveToken(cx, true);";
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
    assert!(c.effects.iter().any(|(n, a)| n == "notify" && a[0] == json!({"title":"Bot token saved","level":"success"})), "{:?}", c.effects);
    assert!(tree.contains("Bot token ✓ saved") && !tree.contains("WIN-TOKEN"), "{tree}");
}

// The Remote page after `ms` more of the clock (its timers run), as describe_remote.
fn describe_remote_after(cx: &mut TestAppContext, staged: &Staged, replies: &[(&str, Value)], setup: &str, ms: u64) -> (String, Rc<RefCell<Calls>>) {
    let calls = Rc::new(RefCell::new(Calls::default()));
    calls.borrow_mut().replies = replies.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    let mut p = props();
    p["section"] = json!("remote");
    let policy = Policy::new()
        .with_capabilities(Capabilities::new().read_roots([staged.0.clone()]))
        .with_host_module(module(fixture(true, p, updates("current", "current")), calls.clone()))
        .unwrap();
    let root = staged.wrapper("frame/settings", setup);
    let window = cx.add_window(|_, _| Empty);
    let mut session = window
        .update(cx, |_, window, cx| DescriptionSession::load(ShellRuntime::new_isolated().unwrap(), root, policy, window, cx))
        .unwrap()
        .unwrap();
    cx.run_until_parked();
    advance(cx, ms);
    let tree = window.update(cx, |_, window, cx| session.describe(window, cx)).unwrap().unwrap();
    window.update(cx, |_, window, _| window.remove_window()).unwrap();
    (tree, calls)
}

// What the token box's "change" handler records (main.js), a second ago: the pause is over.
const TOKEN_PASTED: &str = "this.tokenAt = Date.now() - 1000;";

#[gpui::test]
fn settings_remote_token_saves_itself_after_the_pause_only_when_it_has_the_token_shape(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let path = "/fixture/app/telegram-token.tmp".to_string();
    let mut replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    replies.push(("bridge telegram token --file", core_ok(json!({"telegram_token":"set"}))));
    let tail = "AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw";
    // (typed, saved as): 20 characters after the colon is the shortest token; padding is trimmed.
    for (typed, saved) in [
        (format!("123456789:{tail}"), format!("123456789:{tail}")),
        ("1:abcdefghij_-CDEFGHIJ".to_string(), "1:abcdefghij_-CDEFGHIJ".to_string()),
        (format!("  123456789:{tail} \\t\\n"), format!("123456789:{tail}")),
    ] {
        // A second run of the clock afterwards: the field was cleared, so nothing is saved twice.
        let (tree, calls) = describe_remote_after(cx, &staged, &replies, &format!("this.tokenInput.set_value('{typed}'); {TOKEN_PASTED}"), 1000);
        let c = calls.borrow();
        assert_eq!(c.writes, vec![(path.clone(), saved.clone())], "{typed:?} is saved once, trimmed");
        assert_eq!(c.removes, vec![path.clone(), path.clone()], "{typed:?}: the leftover first, the file last");
        assert!(c.files.is_empty(), "no file holds the token: {:?}", c.files);
        assert!(c.cores.iter().any(|a| *a == ["bridge", "telegram", "token", "--file", path.as_str()]), "{:?}", c.cores);
        assert!(c.cores.iter().all(|a| !a.join(" ").contains("abcdefghij") && !a.join(" ").contains(tail)), "the token is never an argument: {:?}", c.cores);
        let toasts: Vec<&Value> = c.effects.iter().filter(|(n, _)| n == "notify").map(|(_, a)| &a[0]).collect();
        assert_eq!(toasts, [&json!({"title":"Bot token saved","level":"success"})], "{typed:?}: {:?}", c.effects);
        assert!(tree.contains("Bot token ✓ saved") && !tree.contains(tail) && !tree.contains("abcdefghij"), "{tree}");
    }
}

#[gpui::test]
fn settings_remote_partial_or_malformed_token_is_never_saved_on_a_pause(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut replies = remote_replies(bridge_ls(json!([]), false, "unset"), json!([]));
    replies.push(("bridge telegram token --file", core_ok(json!({"telegram_token":"set"}))));
    let tail = "AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw";
    let mut cases: Vec<(String, &str)> = [
        "123", "123:", "123:short", "1:abcdefghij_-CDEFGHI", ":AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw",
        "abc:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw", "12 3:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw",
        "123:AAHdqTcvCH1vGWJxf SeofSAs0K5PALDsaw", "123:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDs$w",
        "1:2:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw",
        "\u{663}\u{664}:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw", "123:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsa\u{e9}",
        "C:\\\\Users\\\\Ana\\\\token.txt",
    ]
    .iter()
    .map(|v| (format!("this.tokenInput.set_value('{v}'); {TOKEN_PASTED}"), *v))
    .collect();
    // A valid token that is not waiting: never changed, still inside the pause, or off the Remote page.
    let valid = format!("this.tokenInput.set_value('123456789:{tail}');");
    cases.push((valid.clone(), "never changed"));
    cases.push((format!("{valid} this.tokenAt = Date.now() + 600000;"), "changed just now"));
    cases.push((format!("{valid} {TOKEN_PASTED} this.section = 'look';"), "on another page"));
    for (setup, label) in &cases {
        let (tree, calls) = describe_remote_after(cx, &staged, &replies, setup, 1000);
        let c = calls.borrow();
        assert!(c.writes.is_empty() && c.removes.is_empty(), "{label:?} is not saved: {:?} {:?}", c.writes, c.removes);
        assert!(c.cores.iter().all(|a| a.get(..2) != Some(&["bridge".to_string(), "telegram".to_string()][..])), "{label:?}: {:?}", c.cores);
        assert!(c.effects.iter().all(|(n, _)| n != "notify"), "{label:?}: no toast: {:?}", c.effects);
    }
}

#[gpui::test]
fn settings_remote_status_line_follows_the_token_and_telegram_pairings(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let id = |n: u32, transport: &str| json!({"id":format!("i{n}"), "transport":transport, "user_id":format!("{n}"), "label":format!("Device {n}"), "paired_at_ms":1700000000000i64});
    for (label, ids, token, setup, status, editing) in [
        ("fresh", json!([]), "unset", "", "Not set up", true),
        ("token only", json!([]), "set", "", "Token saved. Now pair your Telegram.", false),
        ("a web pairing is not Telegram", json!([id(1, "web")]), "set", "", "Token saved. Now pair your Telegram.", false),
        ("one", json!([id(1, "telegram")]), "set", "", "Paired with 1 account", false),
        ("two and a web one", json!([id(1, "telegram"), id(2, "web"), id(3, "telegram")]), "set", "", "Paired with 2 accounts", false),
        ("paired but the token was removed", json!([id(1, "telegram")]), "unset", "", "Paired with 1 account", true),
        ("Change pressed", json!([]), "set", "this.changingToken = true;", "Token saved. Now pair your Telegram.", true),
    ] {
        let (tree, _) = describe_remote(cx, &staged, &remote_replies(bridge_ls(ids, false, token), json!([])), setup);
        assert!(tree.contains(status), "{label}: lacks {status:?}: {tree}");
        assert!(!tree.contains("1 accounts") && !tree.contains("0 account"), "{label}: {tree}");
        assert_eq!(tree.contains("@BotFather") && tree.contains("It saves by itself once you paste it."), editing, "{label}: the input and its hint: {tree}");
        assert_eq!(tree.contains("Bot token ✓ saved"), !editing, "{label}: {tree}");
        assert_eq!(tree.contains("st-r-token-change"), !editing, "{label}: Change only on the saved row: {tree}");
        assert!(!tree.contains("st-r-token\""), "{label}: no Save button: {tree}");
        // Pairing sits beside the token, above the bot's row.
        let pair = tree.find("st-r-pair").unwrap_or_else(|| panic!("{label}: no Pair button: {tree}"));
        assert!(pair < tree.find("st-r-bot-").unwrap(), "{label}: {tree}");
    }
}

#[gpui::test]
fn settings_remote_pair_code_is_large_with_a_copy_button_that_copies_the_command(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    // Presses Copy once, as a click would.
    let press = "const real = this.button.bind(this); let pressed = false; \
        this.button = (t, id, label, onClick) => { if (id === 'st-r-pair-copy' && !pressed) { pressed = true; onClick(); } return real(t, id, label, onClick); };";
    let setup = format!("this.pairing = {{ code: '482913', expires_ms: Date.now() + 600000 }}; {press}");
    let (tree, calls) = describe_remote(cx, &staged, &replies, &setup);
    let c = calls.borrow();
    let copies: Vec<&Vec<Value>> = c.effects.iter().filter(|(n, _)| n == "copyText").map(|(_, a)| a).collect();
    assert_eq!(copies, [&vec![json!("/pair 482913")]], "Copy copies the whole command, once: {:?}", c.effects);
    let toasts: Vec<&Value> = c.effects.iter().filter(|(n, _)| n == "notify").map(|(_, a)| &a[0]).collect();
    assert_eq!(toasts, [&json!({"title":"Copied","level":"success"})], "{:?}", c.effects);
    assert!(tree.contains("st-r-pair-copy") && tree.contains("Send this to your bot in Telegram:") && tree.contains("The code works for "), "{tree}");
    let lines: Vec<&str> = tree.lines().collect();
    let code = lines.iter().position(|l| l.contains("/pair 482913")).unwrap_or_else(|| panic!("{tree}"));
    assert!(lines[code.saturating_sub(1)..=code].iter().any(|l| l.contains(".text_size[Number(24.0)]")), "the command is large: {tree}");
    // An expired code has no Copy, and nothing is copied.
    let setup = format!("this.pairing = {{ code: '482913', expires_ms: Date.now() - 1 }}; {press}");
    let (tree, calls) = describe_remote(cx, &staged, &replies, &setup);
    assert!(!tree.contains("st-r-pair-copy") && !tree.contains("482913"), "{tree}");
    assert!(calls.borrow().effects.iter().all(|(n, _)| n != "copyText"), "{:?}", calls.borrow().effects);
}

#[gpui::test]
fn settings_remote_how_to_use_lists_the_bot_commands_only_when_open(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let replies = remote_replies(bridge_ls(json!([]), false, "set"), json!([]));
    let commands = ["/menu", "/sessions", "/projects", "/help", "/agents", "/read", "/send", "/lock"];
    let (tree, _) = describe_remote(cx, &staged, &replies, "");
    assert!(tree.contains("st-r-help") && tree.contains("How to use") && !tree.contains("Hide how to use"), "{tree}");
    assert!(!tree.contains("List your terminals and pick one.") && !tree.contains("/sessions"), "closed by default: {tree}");
    let (tree, _) = describe_remote(cx, &staged, &replies, "this.showHelp = true;");
    assert!(tree.contains("Hide how to use"), "{tree}");
    let mut at = 0;
    for command in commands {
        let found = tree[at..].find(command).unwrap_or_else(|| panic!("{command} is not listed in order: {tree}"));
        at += found + 1;
    }
}

#[gpui::test]
fn settings_look_lists_the_new_themes_under_dark_and_light(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("look");
    let mut values = fixture(true, p, updates("current", "current"));
    let colors = values["theme"].clone();
    let dark = ["graphite", "amber-minimal", "candyland", "claymorphism", "darkmatter"];
    let light = ["paper", "soft-pop", "neo-brutalism"];
    let list: Vec<Value> = dark.iter().map(|n| (n, false)).chain(light.iter().map(|n| (n, true))).map(|(n, l)| json!({"name":n, "light":l, "colors":colors})).collect();
    values.insert("themes".into(), json!(list));
    let tree = describe_with_values(cx, &staged, "frame/settings", values, None).unwrap();
    let at = |name: &str| tree.find(&format!("st-t-{name}")).unwrap_or_else(|| panic!("{name} is not offered: {tree}"));
    let last_dark = dark.iter().map(|n| at(n)).max().unwrap();
    let first_light = light.iter().map(|n| at(n)).min().unwrap();
    assert!(last_dark < first_light, "Dark comes first, then Light: {tree}");
    for label in ["amber minimal", "soft pop", "neo brutalism"] {
        assert!(tree.contains(label), "{label} is shown with spaces: {tree}");
    }
}

#[gpui::test]
fn brand_logo_is_the_same_foreground_coloured_svg_mark_on_every_chrome(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let file_chrome = |text: &str| serde_json::from_str::<Value>(text).unwrap()["colors"]["chrome"].as_str().unwrap().to_owned();
    let paper = crate::settings::themes().into_iter().find(|(n, _)| n == "paper").unwrap().1["chrome"].clone();
    let graphite = crate::settings::themes().into_iter().find(|(n, _)| n == "graphite").unwrap().1["chrome"].clone();
    let cases = [
        ("graphite", graphite, false),
        ("amber-minimal", file_chrome(include_str!("../../defaults/themes/amber-minimal.json")), false),
        ("candyland", file_chrome(include_str!("../../defaults/themes/candyland.json")), false),
        ("claymorphism", file_chrome(include_str!("../../defaults/themes/claymorphism.json")), false),
        ("darkmatter", file_chrome(include_str!("../../defaults/themes/darkmatter.json")), false),
        ("black", "#000000".to_owned(), false),
        ("translucent dark", "#0f0f0f80".to_owned(), false),
        ("paper", paper, true),
        ("soft-pop", file_chrome(include_str!("../../defaults/themes/soft-pop.json")), true),
        ("neo-brutalism", file_chrome(include_str!("../../defaults/themes/neo-brutalism.json")), true),
        ("white", "#ffffff".to_owned(), true),
        ("translucent light", "#f0f0f0ff".to_owned(), true),
    ];
    let mark = "mods/workspace-chrome/hyperox-mark.svg";
    for (label, chrome, light) in cases {
        let mut values = fixture(true, props(), updates("current", "current"));
        values.get_mut("theme").unwrap()["chrome"] = json!(chrome);
        // Each chrome gets a readable foreground: dark text on a light chrome, light on a dark one.
        values.get_mut("theme").unwrap()["foreground"] = json!(if light { "#1c1c1c" } else { "#ededed" });
        values.get_mut("layout").unwrap()["children"][0]["size"]["px"] = json!(268);
        let tree = describe_with_values(cx, &staged, "mods/workspace-chrome", values.clone(), None).unwrap();
        let lines: Vec<&str> = tree.lines().collect();
        let logos: Vec<&&str> = lines.iter().filter(|l| l.contains(mark)).collect();
        assert_eq!(logos.len(), 1, "{label} ({chrome}): one logo: {tree}");
        let logo = logos[0];
        assert!(logo.trim_start().starts_with("svg"), "{label}: an svg, not an image: {logo}");
        assert!(logo.contains(".size[Number(20.0)]"), "{label}: 20 px: {logo}");
        let color = format!(".text_color[Str({})]", values["theme"]["foreground"]);
        assert!(logo.contains(&color), "{label}: coloured by the theme's foreground {color}: {logo}");
        assert!(!tree.contains("#111111") && !tree.contains(".png"), "{label} ({chrome}): no tile, no PNG: {tree}");
        // The rail hides the brand.
        values.get_mut("layout").unwrap()["children"][0]["size"]["px"] = json!(44);
        let tree = describe_with_values(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
        assert!(!tree.contains(mark) && !tree.contains(".png"), "{label}: rail: {tree}");
    }
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
        else if (verb === 'openfile') this.openFile(arg, cx);
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
    for name in ["nextCommand", "watchFolder"] {
        module = module.async_function(name, |_| Ok(std::future::pending::<gpui_shell::HostResult>()));
    }
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

// A terminal link click opens a file of this machine by its full path: inside the listed folder
// by its relative name, outside it (or a folder that only shares the prefix) by the full path.
#[gpui::test]
fn files_panel_opens_a_clicked_local_file_inside_and_outside_its_folder(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (notes, other, sibling) = ("/fixture/project/src/notes.txt", "/elsewhere/a b.md", "/fixture/project-old/a.txt");
    let disk = files_disk(&[(notes, "mine\n"), (other, "other\n"), (sibling, "old\n")]);
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, files_values(json!([])), &disk);
    for (clicked, file, value) in [(notes, "src/notes.txt", "mine\n"), (other, other, "other\n"), (sibling, sibling, "old\n")] {
        files_step(&mut visual, &disk, &format!("openfile {clicked}"));
        let state = files_state(&mut visual, &disk);
        assert_eq!(
            (state["file"].clone(), state["value"].clone(), state["editable"].clone(), state["hint"].clone()),
            (json!(file), json!(value), json!(true), Value::Null),
            "{clicked}: {state}"
        );
        assert_eq!(disk.borrow().reads.last(), Some(&json!([clicked, {"on":null}])), "{clicked}");
    }
}

// The panel lists another machine, but the click was in a terminal of this one: the file is
// read here (no `on`), whole path kept, and it is editable rather than a remote preview.
#[gpui::test]
fn files_panel_opens_a_clicked_local_file_here_while_it_lists_a_remote_machine(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let notes = "/fixture/project/notes.txt";
    let disk = files_disk(&[(notes, "local\n")]);
    let mut values = files_values(json!([]));
    values.get_mut("spaces").unwrap()[0]["machine"] = json!("remote");
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, values, &disk);
    files_step(&mut visual, &disk, &format!("openfile {notes}"));
    let state = files_state(&mut visual, &disk);
    assert_eq!(
        (state["file"].clone(), state["value"].clone(), state["editable"].clone(), state["hint"].clone()),
        (json!(notes), json!("local\n"), json!(true), Value::Null),
        "{state}"
    );
    assert_eq!(disk.borrow().reads.last(), Some(&json!([notes, {"on":null}])));
    // A plain open from the list still reads the remote machine.
    files_step(&mut visual, &disk, "open notes.txt");
    assert_eq!(disk.borrow().reads.last(), Some(&json!([notes, {"on":"remote"}])));
}

// Windows: backslashes, mixed slashes, spaces and a lowercase drive. Inside the folder the
// relative name joins the folder as listed; outside it the path is read with forward slashes.
#[gpui::test]
fn files_panel_opens_clicked_windows_paths_with_spaces_and_mixed_slashes(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let folder = r"C:\Users\Me\My Project";
    let inside = format!(r"{folder}/sub/notes file.txt");
    let disk = files_disk(&[(inside.as_str(), "in\r\n"), ("D:/Other Dir/a.txt", "out\n"), ("c:/x/b.md", "low\n")]);
    let mut values = files_values(json!([]));
    values.insert(
        "window".into(),
        json!({"platform":"windows","titlebar":"none","buttons":true,"maximized":false,"fullscreen":false,"active":true}),
    );
    values.get_mut("spaces").unwrap()[0]["folder"] = json!(folder);
    values.get_mut("panes").unwrap()[0]["cwd"] = json!(folder);
    values.get_mut("focused").unwrap()["cwd"] = json!(folder);
    let (mut visual, _runtime, _root) = mount_files(cx, &staged, values, &disk);
    for (clicked, file, read) in [
        (r"C:\Users\Me\My Project\sub\notes file.txt", "sub/notes file.txt", inside.as_str()),
        (r"C:\Users\Me\My Project/sub\notes file.txt", "sub/notes file.txt", inside.as_str()),
        (r"D:\Other Dir\a.txt", "D:/Other Dir/a.txt", "D:/Other Dir/a.txt"),
        (r"c:\x\b.md", "c:/x/b.md", "c:/x/b.md"),
    ] {
        files_step(&mut visual, &disk, &format!("openfile {clicked}"));
        let state = files_state(&mut visual, &disk);
        assert_eq!((state["file"].clone(), state["editable"].clone()), (json!(file), json!(true)), "{clicked}: {state}");
        assert_eq!(disk.borrow().reads.last(), Some(&json!([read, {"on":null}])), "{clicked}");
    }
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
    // `cut`: commands the sandbox cut off before they finished (a script call over its 500 ms
    // real-time budget is stopped uncatchably), in order, as each next command starts; `slowMs`: the
    // slowest finished command.
    this.cut = []; this.running = null; this.slowMs = 0;
    cx.timer.every(50, cx => {
      for (let c = command(); c !== null; c = command()) {
        if (this.running !== null) this.cut.push(this.running);
        this.running = c;
        const started = Date.now();
        const at = c.indexOf(' '), verb = at < 0 ? c : c.slice(0, at), arg = c.slice(at + 1);
        const [i, j] = arg.split('.').map(Number);
        const row = arg.includes('.') ? { k: 'step', i, j } : { k: 'task', i };
        if (verb === 'add') this.addTask(arg, cx);
        else if (verb === 'tick') this.tick(row, cx);
        else if (verb === 'delete') this.delete(row, cx);
        else if (verb === 'undo') this.undo(cx);
        else if (verb === 'reload') this.reload(cx);
        else if (verb === 'focus') this.handle.focus();
        // `wide <row>`: the full-title toggle a click on the row makes, without the click.
        else if (verb === 'wide') this.toggleWide(this.item(row), row.k === 'step');
        // `stuck <ms ago> [writing]`: a save flag left set, as if a cut-off write loop never reset it.
        else if (verb === 'stuck') {
          const [ago, writing] = arg.split(' ');
          this.flushing = true; this.writing = writing === 'writing'; this.flushAt = Date.now() - Number(ago);
        }
        else if (verb === 'key') {
          const parts = arg.split('-'), k = parts.pop();
          this.listKey(k, { shift: parts.includes('shift'), alt: parts.includes('alt') }, cx);
        } else if (verb === 'state') {
          if (this.ready) this.current();
          note(JSON.stringify({ ready: this.ready, textLen: this.text.length, text: this.text.length > 16384 ? null : this.text, error: this.error, readonly: this.readonly, sel: this.sel,
            edit: this.edit ? this.edit.input.value() : null, pending: this.pending.length, cur: this.cur, badIndex: Boolean(this.badIndex),
            confirming: this.confirming, naming: Boolean(this.naming), flushing: Boolean(this.flushing),
            menu: this.menu, menuAt: this.menuAt, nameValue: this.naming ? this.naming.input.value() : null,
            firstDone: this.list?.tasks[0]?.done ?? null, cut: this.cut, slowMs: this.slowMs, wide: [...this.wide], open: [...this.open],
            hover: this.hover, press: Boolean(this.press), drag: this.drag,
            lines: this.ready ? this.rows().map((r) => [r, this.lineOf(r)]).filter((l) => l[1] !== null) : [] }));
        }
        this.running = null;
        this.slowMs = Math.max(this.slowMs, Date.now() - started);
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

// A fresh report, never the one before: a "state" the sandbox cut off (see TODO_SCENARIO's `cut`)
// notes nothing, so it is asked again.
fn todo_state(cx: &mut TestAppContext, host: &Rc<RefCell<TodoHost>>) -> Value {
    let before = host.borrow().notes.len();
    for _ in 0..20 {
        todo_step(cx, host, "state");
        if host.borrow().notes.len() > before {
            break;
        }
    }
    let host = host.borrow();
    assert!(host.notes.len() > before, "the scenario reports its state");
    serde_json::from_str(host.notes.last().unwrap()).unwrap()
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

// The described lines of the element with this id and of everything inside it.
fn todo_element(tree: &str, id: &str) -> Vec<String> {
    let lines: Vec<&str> = tree.lines().collect();
    let tag = format!(":id[Str({id:?})]");
    let at = lines.iter().position(|l| l.contains(&tag)).unwrap_or_else(|| panic!("no {id}: {tree}"));
    let depth = |l: &str| l.len() - l.trim_start().len();
    let inside = lines[at + 1..].iter().take_while(|l| depth(l) > depth(lines[at]));
    std::iter::once(&lines[at]).chain(inside).map(|l| l.to_string()).collect()
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
    assert!(tree.contains("Completed") && tree.contains("todo-done") && tree.contains("3 open") && tree.contains("1 of 4 done") && tree.contains("Ship [ ] it"), "{tree}");
    // The Completed header carries its count (formerly "Completed (1)") in its pill.
    let head = todo_element(&tree, "todo-done");
    assert!(head.iter().any(|l| l.trim() == "text \"Completed\"") && head.iter().any(|l| l.trim() == "text \"1\""), "{head:#?}");
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
    // The sandbox stops a script call after 500 ms of real time, uncatchably (gpui-shell's task
    // budget). Parsing, serializing and sizing a 64 KB list can take that long on a loaded Windows
    // debug runner: CI saw the tick below never reach its write. A command the scenario reports as
    // cut off did not run to its end (the panel makes the change only after the slow part), so it
    // is sent again; a command that finished is checked as it is.
    let cuts = |state: &Value, command: &str| state["cut"].as_array().map_or(0, |c| c.iter().filter(|c| *c == command).count());
    for _ in 0..20 {
        let before = cuts(&state, "add New");
        todo_step(cx, &host, "add New");
        state = todo_state(cx, &host);
        if cuts(&state, "add New") == before {
            break;
        }
    }
    assert!(host.borrow().writes.is_empty(), "an add past the limit writes nothing: {} ({state})", host.borrow().writes.len());
    assert_eq!(host.borrow().files[TODO_TASKS], big, "the file is unchanged");
    assert_eq!(state["readonly"], json!(false), "still editable: {state}");
    // The panel's text stays exactly what was read: the refused task is not shown either.
    assert_eq!(state["textLen"], json!(big.chars().count()), "the refused task is not shown either: {state}");
    // The reason is the panel's error line (`this.error`); a description renders the panel afresh.
    assert_eq!(state["error"], json!(full), "the reason is shown: {state}");
    let tree = todo_describe(cx, window, session);
    assert!(tree.contains("todo-add") && !tree.contains(TODO_READ_FAIL), "the add box stays; not read-only");

    // A same-size change (a tick) at the limit is written over what was read.
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, &big)], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    // Up to a minute: Windows' debug runner can be slow to open a 64 KB list; a tick before that is ignored.
    let mut state = todo_state(cx, &host);
    for _ in 0..2400 {
        if state["ready"] == json!(true) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        cx.run_until_parked();
        state = todo_state(cx, &host);
    }
    assert_eq!(state["ready"], json!(true), "the list finished loading before the tick: {state}");
    // The panel's write starts in the same script call as the tick, so it is there once the tick
    // ran; a tick cut off before it changed anything (still not done, nothing written) is sent again.
    for _ in 0..20 {
        let before = cuts(&state, "tick 0");
        todo_step(cx, &host, "tick 0");
        state = todo_state(cx, &host);
        let untouched = state["firstDone"] == json!(false) && host.borrow().writes.is_empty();
        if cuts(&state, "tick 0") == before || !untouched {
            break;
        }
    }
    // A slow runner can cut the save itself off (500 ms budget): the tick is shown and pending with the save
    // flag left set. The panel heals that after 10 s of wall-clock time on its next look at the disk.
    if host.borrow().writes.is_empty() && state["flushing"] == json!(true) && state["pending"] != json!(0) {
        std::thread::sleep(std::time::Duration::from_millis(10_500));
        todo_step(cx, &host, "reload");
        state = todo_state(cx, &host);
    }
    let ticked = big.replacen("- [ ] edge", "- [x] edge", 1);
    assert_eq!(ticked.len(), size);
    assert_eq!(todo_written(&host), [ticked.clone()], "a same-size change is written; the panel: {state}");
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
    // Ticking a task's only step finishes the task (a and b done); c is still open.
    todo_step(cx, &host, "tick 0.0");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Groceries\n- [x] a\n  - [x] step\n- [x] b\n- [ ] c\n");
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

// A task and its steps tick together both ways; a task without steps ticks alone.
#[gpui::test]
fn todo_a_task_and_its_steps_tick_together_and_write_the_markdown(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] A\n  > a note\n  - [ ] a1\n  - [x] a2\n  - [ ] a3\n- [ ] B\n- [ ] C\n  - [ ] c1\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    for command in ["tick 0", "tick 0", "tick 0.0", "tick 0.1", "tick 0.2", "tick 0.1", "tick 1", "tick 1"] {
        todo_step(cx, &host, command);
    }
    let rest = "- [ ] C\n  - [ ] c1\n";
    assert_eq!(
        todo_written(&host),
        [
            format!("# Tasks\n- [x] A\n  > a note\n  - [x] a1\n  - [x] a2\n  - [x] a3\n- [ ] B\n{rest}"),
            format!("# Tasks\n- [ ] A\n  > a note\n  - [ ] a1\n  - [ ] a2\n  - [ ] a3\n- [ ] B\n{rest}"),
            format!("# Tasks\n- [ ] A\n  > a note\n  - [x] a1\n  - [ ] a2\n  - [ ] a3\n- [ ] B\n{rest}"),
            format!("# Tasks\n- [ ] A\n  > a note\n  - [x] a1\n  - [x] a2\n  - [ ] a3\n- [ ] B\n{rest}"),
            format!("# Tasks\n- [x] A\n  > a note\n  - [x] a1\n  - [x] a2\n  - [x] a3\n- [ ] B\n{rest}"),
            format!("# Tasks\n- [ ] A\n  > a note\n  - [x] a1\n  - [ ] a2\n  - [x] a3\n- [ ] B\n{rest}"),
            format!("# Tasks\n- [ ] A\n  > a note\n  - [x] a1\n  - [ ] a2\n  - [x] a3\n- [x] B\n{rest}"),
            format!("# Tasks\n- [ ] A\n  > a note\n  - [x] a1\n  - [ ] a2\n  - [x] a3\n- [ ] B\n{rest}"),
        ],
        "the task carries its steps; the last step closes it and any unticked step opens it; B has no steps"
    );
    let state = todo_state(cx, &host);
    assert_eq!((state["error"].clone(), state["pending"].clone()), (Value::Null, json!(0)), "{state}");
    assert_eq!(todo_celebrations(&host), 0, "C is open the whole time");
    todo_describe(cx, window, session);
}

// AC7 with steps: the step that finishes the last open task finishes the list, once; unticks never do.
#[gpui::test]
fn todo_congratulations_open_once_when_the_last_step_finishes_the_list(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [x] a\n  - [x] a1\n- [ ] b\n  - [x] b1\n  - [ ] b2\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    todo_step(cx, &host, "tick 1.0");
    todo_step(cx, &host, "tick 1.0");
    assert_eq!(host.borrow().files[TODO_TASKS], start, "b2 is still open");
    assert_eq!(todo_celebrations(&host), 0);
    todo_step(cx, &host, "tick 1.1");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [x] a\n  - [x] a1\n- [x] b\n  - [x] b1\n  - [x] b2\n");
    assert_eq!(
        todo_effects(&host, "openOverlay"),
        [json!([{"mod":"todo", "kind":"dialog", "width":400, "props":{"celebrate":"Tasks"}}])],
        "the last step of the last open task finishes the list"
    );
    // Unticking a step of a done task, a whole task, and a reload: no second pop-up.
    for command in ["tick 1.1", "tick 0", "reload"] {
        todo_step(cx, &host, command);
    }
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [ ] a\n  - [ ] a1\n- [ ] b\n  - [x] b1\n  - [ ] b2\n");
    assert_eq!(todo_celebrations(&host), 1);
    todo_describe(cx, window, session);

    // Ticking the only open task, steps and all, finishes the list on that one tick.
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] a\n  - [ ] a1\n  - [ ] a2\n")], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    todo_step(cx, &host, "tick 0");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [x] a\n  - [x] a1\n  - [x] a2\n");
    assert_eq!(todo_celebrations(&host), 1);
    todo_step(cx, &host, "tick 0");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [ ] a\n  - [ ] a1\n  - [ ] a2\n", "toggled twice is back to the start");
    assert_eq!(todo_celebrations(&host), 1, "an untick is never celebrated");
    todo_describe(cx, window, session);
}

// AC4 with steps: a stale write is replayed by title on the agent's text, steps following their task.
#[gpui::test]
fn todo_stale_write_replays_task_and_step_ticks_onto_the_agents_steps(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] A\n  - [ ] a1\n")], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    // The agent adds a done task above and a step to A; ticking A ticks the agent's step too.
    let theirs = "# Tasks\n- [x] Agent done\n- [ ] A\n  - [ ] a1\n  - [ ] from the agent\n";
    host.borrow_mut().files.insert(TODO_TASKS.into(), theirs.into());
    todo_step(cx, &host, "tick 0");
    let merged = "# Tasks\n- [x] Agent done\n- [x] A\n  - [x] a1\n  - [x] from the agent\n";
    assert_eq!(host.borrow().files[TODO_TASKS], merged);
    assert_eq!(host.borrow().writes.len(), 2, "one refused write, one replayed");
    assert_eq!(todo_celebrations(&host), 1, "one tick, one pop-up");

    // The agent unticks its step; our untick of a1 lands on that text and A stays open.
    let theirs = "# Tasks\n- [x] Agent done\n- [ ] A\n  - [x] a1\n  - [ ] from the agent\n";
    host.borrow_mut().files.insert(TODO_TASKS.into(), theirs.into());
    todo_step(cx, &host, "tick 1.0");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [x] Agent done\n- [ ] A\n  - [ ] a1\n  - [ ] from the agent\n");

    // The agent ticks its step; our tick of a1 is then the last step, so A is done on disk.
    let theirs = "# Tasks\n- [x] Agent done\n- [ ] A\n  - [ ] a1\n  - [x] from the agent\n";
    host.borrow_mut().files.insert(TODO_TASKS.into(), theirs.into());
    todo_step(cx, &host, "tick 1.0");
    assert_eq!(host.borrow().files[TODO_TASKS], merged);
    let writes = host.borrow().writes.clone();
    assert_eq!(writes.len(), 6, "{writes:?}");
    assert_eq!(writes[5], (TODO_TASKS.to_string(), merged.to_string(), json!(theirs)));
    let state = todo_state(cx, &host);
    assert_eq!(state["text"], json!(merged), "{state}");
    assert_eq!((state["error"].clone(), state["pending"].clone()), (Value::Null, json!(0)), "{state}");
    todo_describe(cx, window, session);
}

// The pop-up follows the list as written after a stale-write replay, not the view before it: another program
// finishing the rest makes our tick the last one; another program adding an open task means it is not.
#[gpui::test]
fn todo_congratulations_follow_the_list_as_written_after_a_stale_write_replay(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] a\n- [ ] b\n")], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    // b is still open in the view, but the agent ticked it on disk.
    host.borrow_mut().files.insert(TODO_TASKS.into(), "# Tasks\n- [ ] a\n- [x] b\n".into());
    todo_step(cx, &host, "tick 0");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [x] a\n- [x] b\n");
    assert_eq!(
        todo_effects(&host, "openOverlay"),
        [json!([{"mod":"todo", "kind":"dialog", "width":400, "props":{"celebrate":"Tasks"}}])],
        "our tick finished the list as written"
    );
    todo_step(cx, &host, "reload");
    assert_eq!(todo_celebrations(&host), 1, "once");
    todo_describe(cx, window, session);

    // The view is all done after the tick, but the agent added an open task: no pop-up until our tick finishes it.
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] a\n- [x] b\n")], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    host.borrow_mut().files.insert(TODO_TASKS.into(), "# Tasks\n- [ ] a\n- [x] b\n- [ ] from the agent\n".into());
    todo_step(cx, &host, "tick 0");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [x] a\n- [x] b\n- [ ] from the agent\n");
    assert_eq!(todo_celebrations(&host), 0, "the list as written still has an open task");
    let state = todo_state(cx, &host);
    assert_eq!(state["text"], json!("# Tasks\n- [x] a\n- [x] b\n- [ ] from the agent\n"), "{state}");
    todo_step(cx, &host, "tick 2");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [x] a\n- [x] b\n- [x] from the agent\n");
    assert_eq!(todo_celebrations(&host), 1, "the tick that finishes it as written");
    todo_describe(cx, window, session);

    // The agent already ticked everything, ours included: the replayed write changes nothing, so no pop-up.
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] a\n- [x] b\n")], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    host.borrow_mut().files.insert(TODO_TASKS.into(), "# Tasks\n- [x] a\n- [x] b\n".into());
    todo_step(cx, &host, "tick 0");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [x] a\n- [x] b\n");
    assert_eq!(host.borrow().writes.len(), 2, "one refused write, one replayed over identical text");
    assert_eq!(todo_celebrations(&host), 0, "our write did not change the list");
    let state = todo_state(cx, &host);
    assert_eq!((state["error"].clone(), state["pending"].clone()), (Value::Null, json!(0)), "{state}");
    todo_step(cx, &host, "reload");
    assert_eq!(todo_celebrations(&host), 0);
    todo_describe(cx, window, session);
}

// A failed save is retried by itself, without any user action: after 5 s, then 10, 20 s and so on; one timer at a
// time however many changes fail meanwhile; a successful save starts the wait at 5 s again; and a timer that comes
// due with nothing pending writes nothing.
#[gpui::test]
fn todo_a_failed_save_is_retried_by_itself_after_a_growing_wait(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] a\n- [ ] b\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    let big = "x".repeat(1024 * 1024 + 1);
    let wait = |cx: &mut TestAppContext, ms: u64| {
        cx.executor().advance_clock(std::time::Duration::from_millis(ms));
        cx.run_until_parked();
    };
    let writes = |host: &Rc<RefCell<TodoHost>>| host.borrow().writes.len();
    // At T the tick fails (the file changed and can't be re-read); at T+2.5 s an add fails too, starting no timer.
    host.borrow_mut().files.insert(TODO_TASKS.into(), big.clone());
    todo_step(cx, &host, "tick 0");
    assert_eq!(writes(&host), 1);
    wait(cx, 2450);
    todo_step(cx, &host, "add c");
    assert_eq!(writes(&host), 2);
    wait(cx, 2450);
    assert_eq!(writes(&host), 2, "not before 5 s");
    wait(cx, 100);
    assert_eq!(writes(&host), 3, "the first retry, at 5 s; it fails again");
    wait(cx, 9900);
    assert_eq!(writes(&host), 3, "one timer, now 10 s");
    wait(cx, 100);
    assert_eq!(writes(&host), 4, "the second retry, at 10 s more");
    // The file is back as we read it: the next retry, 20 s later, saves both changes.
    host.borrow_mut().files.insert(TODO_TASKS.into(), start.into());
    wait(cx, 19900);
    assert_eq!(writes(&host), 4, "not before 20 s");
    wait(cx, 100);
    let saved = "# Tasks\n- [ ] c\n- [x] a\n- [ ] b\n";
    assert_eq!(writes(&host), 5);
    assert_eq!(host.borrow().files[TODO_TASKS], saved);
    let state = todo_state(cx, &host);
    assert_eq!((state["error"].clone(), state["pending"].clone()), (Value::Null, json!(0)), "{state}");
    // After a save the wait starts at 5 s again.
    host.borrow_mut().files.insert(TODO_TASKS.into(), big);
    todo_step(cx, &host, "tick 2");
    assert_eq!(writes(&host), 6);
    wait(cx, 4900);
    assert_eq!(writes(&host), 6);
    wait(cx, 200);
    assert_eq!(writes(&host), 7, "5 s, not 40 s");
    // The file is back; untick b by hand, which saves at once. The 10 s timer then finds nothing pending.
    host.borrow_mut().files.insert(TODO_TASKS.into(), saved.into());
    todo_step(cx, &host, "tick 2");
    assert_eq!(writes(&host), 8);
    assert_eq!(host.borrow().files[TODO_TASKS], saved);
    wait(cx, 60_000);
    assert_eq!(writes(&host), 8, "nothing pending, nothing written");
    let state = todo_state(cx, &host);
    assert_eq!((state["error"].clone(), state["pending"].clone()), (Value::Null, json!(0)), "{state}");
    todo_describe(cx, window, session);
}

// A tick whose write failed (the file changed and could not be re-read) is kept and not celebrated; when it reaches
// the disk with the next change and the list as written is done, the pop-up comes once.
#[gpui::test]
fn todo_a_finishing_tick_whose_write_failed_celebrates_once_when_it_is_saved(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] a\n- [x] b\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    host.borrow_mut().files.insert(TODO_TASKS.into(), "x".repeat(1024 * 1024 + 1));
    todo_step(cx, &host, "tick 0");
    let state = todo_state(cx, &host);
    assert!(state["error"].as_str().is_some_and(|e| e.starts_with("Not saved yet")), "{state}");
    assert_eq!(state["pending"], json!(1), "{state}");
    assert_eq!(todo_celebrations(&host), 0, "nothing was saved");
    // The file is back as we read it; deleting the done task sends both changes.
    host.borrow_mut().files.insert(TODO_TASKS.into(), start.into());
    todo_step(cx, &host, "delete 1");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [x] a\n");
    assert_eq!(todo_written(&host), ["# Tasks\n- [x] a\n- [x] b\n", "# Tasks\n- [x] a\n"]);
    let state = todo_state(cx, &host);
    assert_eq!((state["error"].clone(), state["pending"].clone()), (Value::Null, json!(0)), "{state}");
    assert_eq!(todo_celebrations(&host), 1);
    todo_step(cx, &host, "reload");
    assert_eq!(todo_celebrations(&host), 1, "at most once");
    todo_describe(cx, window, session);
}

// Front matter above the heading stays above it on every write: an add, a tick and a stale-write replay onto an agent's
// edit of the front matter itself.
#[gpui::test]
fn todo_front_matter_stays_above_the_heading_on_every_write(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "---\ntitle: x\n---\n# Tasks\n- [ ] Old\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    todo_step(cx, &host, "reload");
    assert!(host.borrow().writes.is_empty(), "opening writes nothing");
    todo_step(cx, &host, "add Milk");
    todo_step(cx, &host, "tick 1");
    let theirs = "---\ntitle: y\n---\n# Tasks\n- [ ] Milk\n- [x] Old\n- [ ] agent\n";
    host.borrow_mut().files.insert(TODO_TASKS.into(), theirs.into());
    todo_step(cx, &host, "tick 0");
    assert_eq!(
        todo_written(&host),
        [
            "---\ntitle: x\n---\n# Tasks\n- [ ] Milk\n- [ ] Old\n",
            "---\ntitle: x\n---\n# Tasks\n- [ ] Milk\n- [x] Old\n",
            "---\ntitle: x\n---\n# Tasks\n- [x] Milk\n- [x] Old\n",
            "---\ntitle: y\n---\n# Tasks\n- [x] Milk\n- [x] Old\n- [ ] agent\n",
        ],
        "the stale write is refused, then replayed on the agent's front matter"
    );
    let state = todo_state(cx, &host);
    assert_eq!(state["text"], json!("---\ntitle: y\n---\n# Tasks\n- [x] Milk\n- [x] Old\n- [ ] agent\n"), "{state}");
    assert_eq!(todo_celebrations(&host), 0, "the agent's task is open");
    todo_describe(cx, window, session);

    // No heading: closed front matter stays above the `# Untitled` written under it; an unclosed fence is a plain line,
    // as before front matter was known.
    for (text, written) in [
        ("---\ntitle: x\n---\n- [ ] a\n- [ ] b\n", "---\ntitle: x\n---\n# Untitled\n- [x] a\n- [ ] b\n"),
        ("---\ntitle: x\n- [ ] a\n- [ ] b\n", "# Untitled\n---\ntitle: x\n- [x] a\n- [ ] b\n"),
    ] {
        let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, text)], &[]);
        let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
        todo_step(cx, &host, "tick 0");
        assert_eq!(todo_written(&host), [written]);
        todo_describe(cx, window, session);
    }
}

// AC3/AC4: done tasks and steps are struck through and dimmed; titles and the add box use the
// handwriting only when the font is loaded.
#[gpui::test]
fn todo_done_items_are_struck_through_and_titles_are_handwritten_when_the_font_is_there(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] Open task\n  - [x] Done step\n  - [ ] Open step\n- [x] Done task\n- [ ] Plain\n";
    let hand = ".font_family[Str(\"Patrick Hand\")]";
    for loaded in [true, false] {
        let mut values = todo_values();
        if loaded {
            values.insert("fonts".into(), json!(["Monospace", "Sans", "Patrick Hand"]));
        }
        let (muted, fg) = (values["theme"]["muted"].clone(), values["theme"]["foreground"].clone());
        assert_ne!(muted, fg);
        let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
        let (window, session) = load_todo(cx, &staged, todo_root(&staged), values, &host);
        // The panel opens on the first open task; two down reach the Completed header.
        assert_eq!(todo_state(cx, &host)["sel"], json!({"k":"task", "i":0}));
        todo_step(cx, &host, "key down");
        todo_step(cx, &host, "key down");
        assert_eq!(todo_state(cx, &host)["sel"], json!({"k":"head"}));
        // Show the completed task, unfold Open task's steps, then select Done task.
        for command in ["key enter", "key up", "key up", "key right", "key down", "key down", "key down", "key down", "key down", "key down"] {
            todo_step(cx, &host, command);
        }
        assert_eq!(todo_state(cx, &host)["sel"], json!({"k":"task", "i":1}));
        assert!(host.borrow().writes.is_empty(), "looking writes nothing");
        let tree = todo_describe(cx, window, session);
        let lines: Vec<&str> = tree.lines().collect();
        for (title, done) in [("Open task", false), ("Done step", true), ("Open step", false), ("Done task", true), ("Plain", false)] {
            let at = lines.iter().position(|l| l.trim() == format!("text {title:?}")).unwrap_or_else(|| panic!("no {title}: {tree}"));
            let color = format!(".text_color[Str({})]", if done { &muted } else { &fg });
            assert!(lines[at - 1].contains(&color), "{title} is {}: {}", if done { "dimmed" } else { "not dimmed" }, lines[at - 1]);
            let struck = lines.get(at + 1).is_some_and(|l| l.contains(".absolute") && l.contains(".inset[Number(0.0)]"));
            assert_eq!(struck, done, "{title} struck through: {:?}", lines.get(at + 1));
            assert_eq!(lines[at - 1].contains(hand), loaded, "{title}: {}", lines[at - 1]);
        }
        assert_eq!(todo_element(&tree, "todo-add").iter().any(|l| l.contains(hand)), loaded, "the add box");
        if !loaded {
            assert!(!tree.contains("Patrick Hand"), "no font asked for that is not there: {tree}");
        }
        for id in ["todo-switch", "todo-add", "todo-row-0.", "todo-tick-0.", "todo-fold-0.", "todo-row-0.0", "todo-tick-0.1", "todo-addstep-0", "todo-row-2.", "todo-done", "todo-row-1.", "todo-tick-1.", "todo-del-1."] {
            assert!(tree.contains(&format!(":id[Str({id:?})]")), "missing {id}: {tree}");
        }
        assert!(tree.contains("text \"Add step\"") && tree.contains("text \"1/2\"") && tree.contains("2 open") && tree.contains("1 of 3 done"), "{tree}");
        assert!(tree.contains(".w[Str(\"33%\")]"), "a third of the bar: {tree}");
        assert_eq!(todo_celebrations(&host), 0);
    }
}

// AC5: the progress header from no tasks to all done, with the Completed count and empty states.
#[gpui::test]
fn todo_progress_header_and_completed_count_from_no_tasks_to_all_done(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // (file, open, progress, bar, completed count, the note under the list)
    let cases = [
        ("# Tasks\n", "0 open", None, None, None, Some(("square-pen", "No tasks yet. Add one above."))),
        ("# Tasks\n- [ ] a\n  - [x] a1\n- [ ] b\n", "2 open", Some("0 of 2 done"), Some("0%"), None, None),
        ("# Tasks\n- [x] a\n- [ ] b\n- [ ] c\n", "2 open", Some("1 of 3 done"), Some("33%"), Some("1"), None),
        ("# Tasks\n- [x] a\n  - [x] a1\n- [x] b\n", "0 open", Some("2 of 2 done"), Some("100%"), Some("2"), Some(("circle-check", "All done for today"))),
    ];
    for (file, open, progress, bar, count, note) in cases {
        let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, file)], &[]);
        let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
        let tree = todo_describe(cx, window, session);
        assert!(tree.contains(&format!("text {open:?}")), "{file:?}: {tree}");
        match progress {
            Some(progress) => assert!(tree.contains(&format!("text {progress:?}")), "{file:?}: {tree}"),
            None => assert!(!tree.contains(" done\"") && !tree.contains(".w[Str("), "no progress for an empty list: {tree}"),
        }
        if let Some(bar) = bar {
            assert!(tree.contains(&format!(".w[Str({bar:?})]")), "{file:?}: {tree}");
        }
        match count {
            Some(count) => {
                let head = todo_element(&tree, "todo-done");
                assert!(head.iter().any(|l| l.trim() == format!("text {count:?}")), "{file:?}: {head:#?}");
            }
            None => assert!(!tree.contains("todo-done"), "no Completed header without done tasks: {tree}"),
        }
        for (icon, text) in [("square-pen", "No tasks yet. Add one above."), ("circle-check", "All done for today")] {
            let shown = note == Some((icon, text));
            assert_eq!(tree.contains(text), shown, "{file:?}: {tree}");
            assert_eq!(tree.contains(&format!("mods/workspace-chrome/{icon}.svg")), shown, "{file:?}: {tree}");
        }
        assert!(host.borrow().writes.is_empty() && todo_celebrations(&host) == 0, "opening writes and celebrates nothing");
    }
}

// AC2: a delete posts an Undo notice; undo puts the task (with its steps) or the step back once.
// UPD-D1 AC3: the notice shows 6 s with an Undo button (todo.undo) and an OK button that only closes it.
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
        [json!([{"title":"Task deleted", "body":"b", "timeout":6000, "actions":[{"label":"Undo", "name":"todo.undo"}, {"label":"OK"}]}])]
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

// UPD-D1 AC3: the notice's 6 s is only how long it shows: once it has gone (OK pressed or timed out) the task stays
// deleted and nothing runs, and todo.undo still restores it. Undo has no clock: it holds the latest delete only.
#[gpui::test]
fn todo_undo_after_the_notice_has_gone_still_restores_only_the_latest_delete(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] a\n  - [ ] a1\n- [ ] b\n- [ ] c\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), todo_values(), &host);
    todo_step(cx, &host, "delete 0");
    let deleted = "# Tasks\n- [ ] b\n- [ ] c\n";
    assert_eq!(host.borrow().files[TODO_TASKS], deleted);
    let writes = host.borrow().writes.len();
    cx.executor().advance_clock(std::time::Duration::from_millis(6001));
    cx.run_until_parked();
    assert_eq!(host.borrow().files[TODO_TASKS], deleted, "the task stays deleted after the notice's 6 s");
    assert_eq!(host.borrow().writes.len(), writes, "nothing is written when the notice goes");
    assert!(todo_effects(&host, "run").is_empty(), "nothing runs when the notice goes: {:?}", host.borrow().effects);
    todo_step(cx, &host, "undo");
    assert_eq!(host.borrow().files[TODO_TASKS], start, "undo after the notice has gone restores the task with its step");

    // Two deletes (a, then b): undo brings back b only, and a second undo does nothing.
    todo_step(cx, &host, "delete 0");
    todo_step(cx, &host, "delete 0");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [ ] c\n");
    todo_step(cx, &host, "undo");
    assert_eq!(host.borrow().files[TODO_TASKS], deleted, "the latest delete comes back");
    let writes = host.borrow().writes.len();
    todo_step(cx, &host, "undo");
    assert_eq!(host.borrow().writes.len(), writes, "the earlier delete is not undone");
    assert_eq!(host.borrow().files[TODO_TASKS], deleted);
    todo_describe(cx, window, session);
}

// UPD-D1 AC1/AC2: the toast draws one button per `actions` entry with a label, in order (entries without a label, null
// or not objects are skipped); an empty list, a list that is not an array, an old single-action notice and a bare one
// draw none.
#[gpui::test]
fn toast_draws_one_button_per_labelled_action_and_none_for_old_or_bare_notices(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let cases = [
        (json!({"title":"Task deleted", "body":"b", "timeout":6000, "actions":[{"label":"Undo", "name":"todo.undo"}, {"label":"OK"}]}), vec!["Undo", "OK"]),
        (
            json!({"title":"t", "actions":[{"name":"app.help"}, null, 7, {"label":""}, {"label":"Later"}, {"label":"Go", "name":"agent.focus", "args":{"terminal":"t1"}}]}),
            vec!["Later", "Go"],
        ),
        (json!({"title":"t", "actions":[]}), vec![]),
        (json!({"title":"t", "actions":{"label":"Undo", "name":"todo.undo"}}), vec![]),
        (json!({"title":"t", "actions":null}), vec![]),
        (json!({"title":"Done", "body":"Build complete", "action":{"name":"app.help"}}), vec![]),
        (json!({"title":"Done"}), vec![]),
        (json!({}), vec![]),
    ];
    for (notification, labels) in cases {
        let mut p = notification.clone();
        p["overlay"] = json!("o1");
        let tree = describe(cx, &staged, "mods/toast", true, p, updates("current", "current"), None).unwrap_or_else(|e| panic!("{notification}: {e:#}"));
        for (i, label) in labels.iter().enumerate() {
            let button = todo_element(&tree, &format!("toast-action-{i}"));
            assert!(button.iter().any(|l| l.trim() == format!("text {label:?}")), "{notification}: button {i} is {label}: {button:#?}");
        }
        let extra = format!(":id[Str({:?})]", format!("toast-action-{}", labels.len()));
        assert!(!tree.contains(&extra), "{notification}: {} buttons only: {tree}", labels.len());
    }
}

// UPD-D1 AC1/AC2/AC3 with real clicks across the middle of a mounted toast, right to left, each click on a fresh frame:
// what each click ran, with repeats of the same outcome merged. A button runs its own command (with its args) or none
// and closes the toast, never the whole-toast action; a click on the body (padding, gaps, text) runs that action and
// closes it, as before `actions` existed.
#[gpui::test]
fn toast_buttons_run_only_their_own_command_and_the_body_still_runs_the_toast_action(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let close = json!(["closeOverlay", ["o1"]]);
    let body = json!([["run", ["app.help", {}]], close]);
    let only_close = json!([close]);
    let cases = [
        // The to-do delete notice, plus a whole-toast action: OK only closes, Undo runs todo.undo.
        (
            json!({"title":"Task deleted", "body":"b", "timeout":6000, "action":{"name":"app.help"}, "actions":[{"label":"Undo", "name":"todo.undo"}, {"label":"OK"}]}),
            vec![body.clone(), only_close.clone(), body.clone(), json!([["run", ["todo.undo", {}]], close]), body.clone()],
        ),
        // Args reach the command; an entry without a label is no button (its command would show in a click).
        (
            json!({"title":"t", "action":{"name":"app.help"}, "actions":[{"label":"Go", "name":"agent.focus", "args":{"terminal":"t1"}}, {"name":"todo.undo"}, {"label":"Later"}]}),
            vec![body.clone(), only_close.clone(), body.clone(), json!([["run", ["agent.focus", {"terminal":"t1"}]], close]), body.clone()],
        ),
        // An old single-action toast, and one with neither: the whole toast is one target.
        (json!({"title":"Done", "action":{"name":"agent.focus", "args":{"terminal":"t2"}}}), vec![json!([["run", ["agent.focus", {"terminal":"t2"}]], close])]),
        (json!({"title":"Done"}), vec![only_close.clone()]),
        // No buttons for an empty list or one that is not an array.
        (json!({"title":"t", "action":{"name":"app.help"}, "actions":[]}), vec![body.clone()]),
        (json!({"title":"t", "actions":{"label":"Undo", "name":"todo.undo"}}), vec![only_close.clone()]),
    ];
    for (notification, want) in cases {
        let host = todo_host(&[], &[]);
        let mut values = todo_values();
        let mut p = notification.clone();
        p["overlay"] = json!("o1");
        values.insert("props".into(), p);
        gpui_shell::policy::set_default(todo_policy(&staged, values, &host));
        let runtime = ShellRuntime::new_isolated().unwrap();
        let root = staged.0.join("mods/toast");
        let window = cx.add_window(|window, cx| FilesMounted(runtime.try_load(&root, window, cx).unwrap()));
        let mut visual = gpui::VisualTestContext::from_window(*window, cx);
        visual.run_until_parked();
        visual.update(|window, _| window.activate_window());
        let size = visual.update(|window, _| window.viewport_size());
        let y = size.height * 0.5;
        let mut seen: Vec<Value> = Vec::new();
        for from_right in (2..(f32::from(size.width) * 0.5) as i32).step_by(3) {
            host.borrow_mut().effects.clear();
            visual.update(|window, cx| window.draw(cx).clear(cx));
            visual.simulate_click(gpui::point(size.width - gpui::px(from_right as f32), y), gpui::Modifiers::none());
            visual.run_until_parked();
            let ran = json!(host.borrow().effects.iter().map(|(n, a)| json!([n, a])).collect::<Vec<_>>());
            if seen.last() != Some(&ran) {
                seen.push(ran);
            }
        }
        assert_eq!(seen, want, "{notification}");
    }
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

const TODO_WORK: &str = "/fixture/app/todo/work.md";
const TODO_TWO_LISTS: &str = "- [Tasks](tasks.md)\n- [Work](work.md)\n";
const TODO_WORK_TEXT: &str = "# Work\n- [ ] ship\n- [ ] test\n- [x] plan\n";

// The to-do panel (TODO_SCENARIO) mounted with real input and holding the keyboard, its first load done. Keys,
// typing and clicks reach the panel's own handlers, whose cx the runtime retires when the call ends.
fn mount_todo(
    cx: &mut TestAppContext,
    staged: &Staged,
    host: &Rc<RefCell<TodoHost>>,
) -> (gpui::VisualTestContext, Rc<ShellRuntime>, gpui::Entity<gpui_shell::ShellRoot>) {
    let root = todo_root(staged);
    gpui_shell::policy::set_default(todo_policy(staged, todo_values(), host));
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| FilesMounted(runtime.try_load(&root, window, cx).unwrap()));
    let shell = window.update(cx, |mounted, _, _| mounted.0.clone()).unwrap();
    let mut visual = gpui::VisualTestContext::from_window(*window, cx);
    visual.run_until_parked();
    host.borrow_mut().commands.push_back("focus".into());
    files_wait(&mut visual, 50);
    visual.update(|window, cx| {
        window.activate_window();
        window.draw(cx).clear(cx)
    });
    (visual, runtime, shell)
}

fn todo_mounted_state(visual: &mut gpui::VisualTestContext, host: &Rc<RefCell<TodoHost>>) -> Value {
    host.borrow_mut().commands.push_back("state".into());
    files_wait(visual, 50);
    serde_json::from_str(host.borrow().notes.last().expect("the scenario reports its state")).unwrap()
}

// The panel's elements carry no test selectors, so this clicks down a column 20 px in from its right edge (where the
// confirm and new-index bars keep their right-hand button), from just under the 46 px header, until `landed` says a
// click did something. Nothing else in that column takes a click above those bars. Each click meets a fresh frame.
fn todo_click_right_column(visual: &mut gpui::VisualTestContext, landed: impl Fn() -> bool) -> gpui::Point<gpui::Pixels> {
    let x = visual.update(|window, _| window.viewport_size().width) - gpui::px(20.);
    for y in (48..400).step_by(2) {
        let at = gpui::point(x, gpui::px(y as f32));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.simulate_click(at, gpui::Modifiers::none());
        if landed() {
            return at;
        }
    }
    panic!("no click in the panel's right-hand column landed");
}

// After deleting the open list (index entry and file, once), the next list is open, editable and without an error.
fn assert_tasks_list_deleted_once(visual: &mut gpui::VisualTestContext, host: &Rc<RefCell<TodoHost>>) {
    files_wait(visual, 100);
    {
        let h = host.borrow();
        assert_eq!(h.removes, [TODO_TASKS], "the list file is removed once");
        assert!(!h.files.contains_key(TODO_TASKS), "{:?}", h.files);
        assert_eq!(h.files[TODO_LISTS], "- [Work](work.md)\n");
        assert_eq!(h.writes.iter().filter(|(p, _, _)| p == TODO_LISTS).count(), 1, "one edit of lists.md: {:?}", h.writes);
        assert_eq!(h.files[TODO_WORK], TODO_WORK_TEXT, "the next list is untouched");
    }
    let state = todo_mounted_state(visual, host);
    assert_eq!(state["error"], Value::Null, "{state}");
    assert_eq!(
        (state["cur"].clone(), state["text"].clone(), state["readonly"].clone(), state["ready"].clone(), state["confirming"].clone()),
        (json!("work"), json!(TODO_WORK_TEXT), json!(false), json!(true), json!(false)),
        "{state}"
    );
}

// Delete list from the list menu with real keys: Enter confirms. A second Enter, pressed before the delete has
// finished, deletes nothing more.
#[gpui::test]
fn todo_delete_list_confirmed_with_enter_opens_the_next_list_editable(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, TODO_TWO_LISTS), (TODO_TASKS, "# Tasks\n- [ ] a\n- [ ] b\n"), (TODO_WORK, TODO_WORK_TEXT)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    // Tasks, Work, New list, Rename list, Delete list.
    files_keys(&mut visual, "ctrl-l down down down down enter");
    assert_eq!(todo_mounted_state(&mut visual, &host)["confirming"], json!(true), "Delete list asks first");
    assert!(host.borrow().removes.is_empty() && host.borrow().files.contains_key(TODO_TASKS), "nothing is deleted before the answer");
    files_keys(&mut visual, "enter enter");
    assert_tasks_list_deleted_once(&mut visual, &host);
}

// The same through the confirm bar's Delete button; a second click on the same spot deletes nothing more.
#[gpui::test]
fn todo_delete_list_confirmed_with_the_delete_button_opens_the_next_list_editable(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, TODO_TWO_LISTS), (TODO_TASKS, "# Tasks\n- [ ] a\n- [ ] b\n"), (TODO_WORK, TODO_WORK_TEXT)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    files_keys(&mut visual, "ctrl-l down down down down enter");
    assert_eq!(todo_mounted_state(&mut visual, &host)["confirming"], json!(true));
    let at = todo_click_right_column(&mut visual, || !host.borrow().removes.is_empty());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_click(at, gpui::Modifiers::none());
    assert_tasks_list_deleted_once(&mut visual, &host);
}

// Switching lists from the menu with real keys opens the other list editable, with no error, and writes nothing.
#[gpui::test]
fn todo_switching_lists_with_keys_opens_the_other_list_editable(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, TODO_TWO_LISTS), (TODO_TASKS, "# Tasks\n- [ ] a\n- [ ] b\n"), (TODO_WORK, TODO_WORK_TEXT)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    files_keys(&mut visual, "ctrl-l down enter");
    files_wait(&mut visual, 100);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(state["error"], Value::Null, "{state}");
    assert_eq!(
        (state["cur"].clone(), state["text"].clone(), state["readonly"].clone(), state["ready"].clone()),
        (json!("work"), json!(TODO_WORK_TEXT), json!(false), json!(true)),
        "{state}"
    );
    assert!(host.borrow().writes.is_empty() && host.borrow().removes.is_empty(), "switching writes nothing");
    // Still live: Space ticks Work's first task and saves it.
    files_keys(&mut visual, "space");
    files_wait(&mut visual, 50);
    assert_eq!(host.borrow().files[TODO_WORK], "# Work\n- [x] ship\n- [ ] test\n- [x] plan\n");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [ ] a\n- [ ] b\n");
}

// New list through the name box: an empty name does nothing; a name already taken gets the next free file, which is
// created (never over an existing one), added to lists.md and opened.
#[gpui::test]
fn todo_new_list_from_the_name_box_creates_and_opens_it(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // An empty open list: the panel's selection is the (absent) Completed row, so nothing else answers Enter.
    let host = todo_host(&[(TODO_LISTS, TODO_TWO_LISTS), (TODO_TASKS, "# Tasks\n"), (TODO_WORK, TODO_WORK_TEXT)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    files_keys(&mut visual, "ctrl-l down down enter");
    assert_eq!(todo_mounted_state(&mut visual, &host)["naming"], json!(true));
    files_keys(&mut visual, "enter");
    files_wait(&mut visual, 50);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["naming"].clone(), state["error"].clone(), state["cur"].clone()), (json!(false), Value::Null, json!("tasks")), "{state}");
    assert!(host.borrow().writes.is_empty(), "an empty name creates nothing: {:?}", host.borrow().writes);

    // key() drops an Enter within 150 ms (wall clock, Date.now) of a name-box submit: the submit's own Enter also
    // reaches the panel. A person pauses longer than that before the next menu pick; the fake test clock doesn't.
    std::thread::sleep(std::time::Duration::from_millis(200));
    files_keys(&mut visual, "ctrl-l down down enter");
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["naming"].clone(), state["menu"].clone()), (json!(true), json!(false)), "New list opens the name box again: {state}");
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_input("Work");
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(state["nameValue"], json!("Work"), "typing reaches the name box: {state}");
    files_keys(&mut visual, "enter");
    files_wait(&mut visual, 100);
    let new = "/fixture/app/todo/work-2.md";
    {
        let h = host.borrow();
        assert_eq!(h.files[TODO_LISTS], "- [Tasks](tasks.md)\n- [Work](work.md)\n- [Work](work-2.md)\n");
        assert_eq!(h.files[new], "# Work\n");
        assert!(h.writes.iter().any(|w| w == &(new.to_string(), "# Work\n".to_string(), Value::Null)), "created, never over a file: {:?}", h.writes);
        assert_eq!(h.files[TODO_WORK], TODO_WORK_TEXT, "the list with the same name is kept");
    }
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(state["error"], Value::Null, "{state}");
    assert_eq!(
        (state["cur"].clone(), state["text"].clone(), state["readonly"].clone(), state["ready"].clone()),
        (json!("work-2"), json!("# Work\n"), json!(false), json!(true)),
        "{state}"
    );
}

// Rename through the name box renames lists.md's entry and the list's heading, and leaves the panel saving: the next
// tick is written too.
#[gpui::test]
fn todo_rename_list_from_the_name_box_renames_it_and_keeps_saving(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, TODO_TWO_LISTS), (TODO_TASKS, "# Tasks\n- [ ] a\n- [x] b\n"), (TODO_WORK, TODO_WORK_TEXT)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    // Select the Completed row first, so the Enter that ends the name box opens no task.
    files_keys(&mut visual, "down");
    assert_eq!(todo_mounted_state(&mut visual, &host)["sel"], json!({"k":"head"}));
    files_keys(&mut visual, "ctrl-l down down down enter");
    visual.update(|window, cx| window.draw(cx).clear(cx));
    // The old name is selected, so typing replaces it.
    visual.simulate_input("Chores");
    files_keys(&mut visual, "enter");
    files_wait(&mut visual, 100);
    assert_eq!(host.borrow().files[TODO_LISTS], "- [Chores](tasks.md)\n- [Work](work.md)\n");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Chores\n- [ ] a\n- [x] b\n");
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(
        (state["error"].clone(), state["flushing"].clone(), state["pending"].clone(), state["readonly"].clone()),
        (Value::Null, json!(false), json!(0), json!(false)),
        "{state}"
    );
    files_keys(&mut visual, "up space");
    files_wait(&mut visual, 100);
    assert_eq!(host.borrow().files[TODO_TASKS], "# Chores\n- [x] a\n- [x] b\n", "a tick after the rename is saved");
    assert_eq!(todo_mounted_state(&mut visual, &host)["pending"], json!(0));
}

// Renaming a list with front matter renames the heading under it; the front matter stays on top, and the real key
// press that finishes the list opens the pop-up once.
#[gpui::test]
fn todo_rename_keeps_front_matter_above_the_renamed_heading(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // The front matter holds a `# ` comment: it is never taken for the heading, so the rename leaves it alone.
    let start = "---\n# note\ntitle: x\n---\n# Tasks\n- [ ] a\n- [x] b\n";
    let host = todo_host(&[(TODO_LISTS, TODO_TWO_LISTS), (TODO_TASKS, start), (TODO_WORK, TODO_WORK_TEXT)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    files_keys(&mut visual, "down");
    assert_eq!(todo_mounted_state(&mut visual, &host)["sel"], json!({"k":"head"}));
    files_keys(&mut visual, "ctrl-l down down down enter");
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_input("Chores");
    files_keys(&mut visual, "enter");
    files_wait(&mut visual, 100);
    assert_eq!(host.borrow().files[TODO_TASKS], "---\n# note\ntitle: x\n---\n# Chores\n- [ ] a\n- [x] b\n");
    assert_eq!(host.borrow().files[TODO_LISTS], "- [Chores](tasks.md)\n- [Work](work.md)\n");
    assert_eq!(todo_celebrations(&host), 0);
    files_keys(&mut visual, "up space");
    files_wait(&mut visual, 100);
    assert_eq!(host.borrow().files[TODO_TASKS], "---\n# note\ntitle: x\n---\n# Chores\n- [x] a\n- [x] b\n");
    assert_eq!(
        todo_effects(&host, "openOverlay"),
        [json!([{"mod":"todo", "kind":"dialog", "width":400, "props":{"celebrate":"Chores"}}])]
    );
}

// A save flag left set with nothing writing for over 10 s is reset by the next tick, and that tick is saved (and,
// finishing the list, celebrated once).
#[gpui::test]
fn todo_a_stuck_save_is_reset_by_the_next_tick_and_the_tick_is_saved(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] one\n- [x] two\n")], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    host.borrow_mut().commands.push_back("stuck 11000".into());
    files_wait(&mut visual, 50);
    assert_eq!(todo_mounted_state(&mut visual, &host)["flushing"], json!(true), "the flag is set");
    files_keys(&mut visual, "space");
    files_wait(&mut visual, 50);
    assert_eq!(todo_written(&host), ["# Tasks\n- [x] one\n- [x] two\n"], "saved once");
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(
        (state["error"].clone(), state["flushing"].clone(), state["pending"].clone()),
        (Value::Null, json!(false), json!(0)),
        "{state}"
    );
    assert_eq!(todo_celebrations(&host), 1);
}

// A write that never ended (the loop was cut off after it started writing) is reset after 60 s: the next tick is saved
// once, and the panel saves normally afterwards.
#[gpui::test]
fn todo_a_save_stuck_writing_for_over_60_s_is_reset_by_the_next_tick(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] one\n- [ ] two\n")], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    host.borrow_mut().commands.push_back("stuck 61000 writing".into());
    files_wait(&mut visual, 50);
    files_keys(&mut visual, "space");
    files_wait(&mut visual, 50);
    assert_eq!(todo_written(&host), ["# Tasks\n- [x] one\n- [ ] two\n"], "saved once");
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(
        (state["error"].clone(), state["flushing"].clone(), state["pending"].clone()),
        (Value::Null, json!(false), json!(0)),
        "{state}"
    );
    // "one" moved to Completed, so the selection is now on "two": the next Space ticks it and is saved too.
    files_keys(&mut visual, "space");
    files_wait(&mut visual, 50);
    assert_eq!(todo_written(&host), ["# Tasks\n- [x] one\n- [ ] two\n", "# Tasks\n- [x] one\n- [x] two\n"], "and keeps saving");
}

// Under 10 s, or while a write has been in progress for under 60 s, the flag is never reset and the change waits; past
// 10 s with nothing writing, the next look at the disk resets it and saves the waiting change.
#[gpui::test]
fn todo_a_save_flag_is_not_reset_under_10_s_or_while_writing_and_a_reload_heals_it(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] one\n- [ ] two\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    let waiting = |visual: &mut gpui::VisualTestContext, why: &str| {
        let state = todo_mounted_state(visual, &host);
        assert_eq!((state["flushing"].clone(), state["pending"].clone()), (json!(true), json!(1)), "{why}: {state}");
        assert!(host.borrow().writes.is_empty() && host.borrow().files[TODO_TASKS] == start, "{why}: nothing is written");
    };
    host.borrow_mut().commands.push_back("stuck 9000".into());
    files_wait(&mut visual, 50);
    files_keys(&mut visual, "space");
    files_wait(&mut visual, 50);
    waiting(&mut visual, "a tick under 10 s");
    host.borrow_mut().commands.extend(["stuck 9000", "reload"].map(String::from));
    files_wait(&mut visual, 50);
    waiting(&mut visual, "a reload under 10 s");
    host.borrow_mut().commands.extend(["stuck 59000 writing", "reload"].map(String::from));
    files_wait(&mut visual, 50);
    waiting(&mut visual, "a write in progress for 59 s");
    host.borrow_mut().commands.extend(["stuck 11000", "reload"].map(String::from));
    files_wait(&mut visual, 50);
    assert_eq!(todo_written(&host), ["# Tasks\n- [x] one\n- [ ] two\n"], "the waiting tick is saved once");
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(
        (state["error"].clone(), state["flushing"].clone(), state["pending"].clone()),
        (Value::Null, json!(false), json!(0)),
        "{state}"
    );
}

// Switching lists while a stuck flag holds a waiting change resets it, saves the change to its own list, and opens the
// other list editable.
#[gpui::test]
fn todo_switching_lists_heals_a_stuck_save_and_saves_to_the_list_it_was_made_in(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, TODO_TWO_LISTS), (TODO_TASKS, "# Tasks\n- [ ] a\n- [ ] b\n"), (TODO_WORK, TODO_WORK_TEXT)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    host.borrow_mut().commands.push_back("stuck 0".into());
    files_wait(&mut visual, 50);
    files_keys(&mut visual, "space");
    files_wait(&mut visual, 50);
    assert!(host.borrow().writes.is_empty(), "the tick waits behind the flag");
    host.borrow_mut().commands.push_back("stuck 11000".into());
    files_wait(&mut visual, 50);
    files_keys(&mut visual, "ctrl-l down enter");
    files_wait(&mut visual, 200);
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [x] a\n- [ ] b\n");
    assert_eq!(host.borrow().files[TODO_WORK], TODO_WORK_TEXT, "nothing lands in the other list");
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(
        (state["cur"].clone(), state["text"].clone(), state["readonly"].clone(), state["error"].clone(), state["flushing"].clone()),
        (json!("work"), json!(TODO_WORK_TEXT), json!(false), Value::Null, json!(false)),
        "{state}"
    );
}

// TD-WRAP AC1/AC4: a wide row wraps its full title (no truncate, the title takes the rest of the row, no spacer, the
// row top-aligned with 8 px above and below) and a wide done item has no strike line but stays dimmed; collapsed rows
// keep today's look, strike line included. A step with its task's title stays collapsed when only the task is wide,
// and an empty title widens like any other. Widening writes nothing.
#[gpui::test]
fn todo_wide_rows_wrap_their_title_without_a_strike_and_collapsed_rows_keep_todays_look(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] Same\n  - [ ] Same\n  - [x] Done step\n- [x] Done task\n- [ ] \n- [x] Old\n";
    let values = todo_values();
    let (muted, fg) = (values["theme"]["muted"].clone(), values["theme"]["foreground"].clone());
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (window, session) = load_todo(cx, &staged, todo_root(&staged), values, &host);
    assert_eq!(todo_state(cx, &host)["sel"], json!({"k":"task", "i":0}));
    // Unfold Same's steps, walk past them, Add step and the empty task to Completed, and show the done tasks.
    for command in ["key right", "key down", "key down", "key down", "key down", "key down"] {
        todo_step(cx, &host, command);
    }
    assert_eq!(todo_state(cx, &host)["sel"], json!({"k":"head"}));
    todo_step(cx, &host, "key enter");
    for command in ["wide 0", "wide 0.1", "wide 1", "wide 2"] {
        todo_step(cx, &host, command);
    }
    assert_eq!(todo_state(cx, &host)["wide"], json!(["t:Same", "s:Done step", "t:Done task", "t:"]));
    assert!(host.borrow().writes.is_empty(), "widening writes nothing");
    let tree = todo_describe(cx, window, session);
    let depth = |l: &str| l.len() - l.trim_start().len();
    // (row, title, wide, done)
    let rows = [
        ("todo-row-0.", "Same", true, false),
        ("todo-row-0.0", "Same", false, false),
        ("todo-row-0.1", "Done step", true, true),
        ("todo-row-1.", "Done task", true, true),
        ("todo-row-2.", "", true, false),
        ("todo-row-3.", "Old", false, true),
    ];
    for (id, title, wide, done) in rows {
        let row = todo_element(&tree, id);
        assert_eq!(row[0].contains(".items_start") && row[0].contains(".py[Number(8.0)]"), wide, "{id} top-aligned: {}", row[0]);
        let spacer = row.iter().any(|l| depth(l) == depth(&row[0]) + 2 && l.split_whitespace().eq(["div", ".flex_1"]));
        assert_eq!(spacer, !wide, "{id} spacer: {row:#?}");
        if title.is_empty() {
            continue;
        }
        let at = row.iter().position(|l| l.trim() == format!("text {title:?}")).unwrap_or_else(|| panic!("no {title} in {id}: {row:#?}"));
        assert_eq!(row[at - 1].contains(".truncate"), !wide, "{id} truncated: {}", row[at - 1]);
        assert_eq!(row[at - 1].contains(".flex_1"), wide, "{id} title takes the rest of the row: {}", row[at - 1]);
        let color = format!(".text_color[Str({})]", if done { &muted } else { &fg });
        assert!(row[at - 1].contains(&color), "{id} dimmed only when done: {}", row[at - 1]);
        let struck = row.get(at + 1).is_some_and(|l| l.contains(".absolute") && l.contains(".inset[Number(0.0)]"));
        assert_eq!(struck, done && !wide, "{id} struck through: {:?}", row.get(at + 1));
    }
}

// Presses the left button at `at` on a fresh frame; a `click_count` of 2 is the second press of a double click.
fn todo_press(visual: &mut gpui::VisualTestContext, at: gpui::Point<gpui::Pixels>, click_count: usize) {
    let (modifiers, button) = (gpui::Modifiers::none(), gpui::MouseButton::Left);
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_event(gpui::MouseDownEvent { position: at, modifiers, button, click_count, first_mouse: false });
    visual.simulate_event(gpui::MouseUpEvent { position: at, modifiers, button, click_count });
}

fn todo_wide(visual: &mut gpui::VisualTestContext, host: &Rc<RefCell<TodoHost>>) -> Value {
    todo_mounted_state(visual, host)["wide"].clone()
}

// Presses down the middle of the panel (clear of the tick circle on the left and a row's controls on the right) from
// `from` px until the full titles read `want`; returns that press, which is within 2 px of the row's top edge. Above
// the rows (header, progress, add box) nothing a press reaches toggles a title.
fn todo_press_down_the_middle(visual: &mut gpui::VisualTestContext, host: &Rc<RefCell<TodoHost>>, from: f32, want: Value) -> gpui::Point<gpui::Pixels> {
    let x = visual.update(|window, _| window.viewport_size().width) * 0.5;
    for y in (from as i32..800).step_by(2) {
        let at = gpui::point(x, gpui::px(y as f32));
        todo_press(visual, at, 1);
        if todo_wide(visual, host) == want {
            return at;
        }
    }
    panic!("no press down the middle of the panel from {from} px made the full titles {want}");
}

// TD-WRAP AC1/AC2/AC5 through the row's own handler: one click widens a task's full title, a second collapses it, a
// third widens it again; an empty title toggles like any other. A double click on the wide row edits the title and
// leaves it wide. Nothing is written.
#[gpui::test]
fn todo_a_click_toggles_a_tasks_full_title_and_a_double_click_on_a_wide_row_edits_it_still_wide(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] Same\n- [ ] \n")], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    let found = todo_press_down_the_middle(&mut visual, &host, 48., json!(["t:Same"]));
    // Well inside the row (44 px collapsed), so every press below meets it however tall it is.
    let task = gpui::point(found.x, found.y + gpui::px(10.));
    todo_press(&mut visual, task, 1);
    assert_eq!(todo_wide(&mut visual, &host), json!([]), "a second click collapses it");
    todo_press(&mut visual, task, 1);
    assert_eq!(todo_wide(&mut visual, &host), json!(["t:Same"]), "a third widens it again");
    let empty = todo_press_down_the_middle(&mut visual, &host, f32::from(found.y) + 46., json!(["t:Same", "t:"]));
    todo_press(&mut visual, gpui::point(empty.x, empty.y + gpui::px(10.)), 1);
    assert_eq!(todo_wide(&mut visual, &host), json!(["t:Same"]), "the empty title collapses again");
    // A double click on the wide row: its first press collapses it, its second widens it again and edits.
    todo_press(&mut visual, task, 1);
    todo_press(&mut visual, task, 2);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(
        (state["wide"].clone(), state["edit"].clone(), state["sel"].clone()),
        (json!(["t:Same"]), json!("Same"), json!({"k":"task", "i":0})),
        "{state}"
    );
    assert!(host.borrow().writes.is_empty(), "toggling and starting an edit write nothing: {:?}", host.borrow().writes);
}

// TD-WRAP AC1/AC2/AC5: a step and its task with the same title are separate rows: a click on the step widens only the
// step, a second collapses only the step. A double click on the collapsed step edits it and leaves it collapsed.
#[gpui::test]
fn todo_a_step_with_its_tasks_title_toggles_on_its_own_and_a_double_click_edits_it_collapsed(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] Same\n  - [ ] Same\n- [ ] two\n")], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    files_keys(&mut visual, "right");
    files_wait(&mut visual, 50);
    assert_eq!(todo_mounted_state(&mut visual, &host)["open"], json!(["Same"]), "Same's steps are shown");
    let task = todo_press_down_the_middle(&mut visual, &host, 48., json!(["t:Same"]));
    let found = todo_press_down_the_middle(&mut visual, &host, f32::from(task.y) + 46., json!(["t:Same", "s:Same"]));
    let step = gpui::point(found.x, found.y + gpui::px(10.));
    todo_press(&mut visual, step, 1);
    assert_eq!(todo_wide(&mut visual, &host), json!(["t:Same"]), "the second click collapses only the step");
    todo_press(&mut visual, gpui::point(task.x, task.y + gpui::px(10.)), 1);
    assert_eq!(todo_wide(&mut visual, &host), json!([]));
    todo_press(&mut visual, step, 1);
    todo_press(&mut visual, step, 2);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(
        (state["wide"].clone(), state["edit"].clone(), state["sel"].clone()),
        (json!([]), json!("Same"), json!({"k":"step", "i":0, "j":0})),
        "{state}"
    );
    assert!(host.borrow().writes.is_empty(), "{:?}", host.borrow().writes);
}

// TD-WRAP AC3: presses on the fold chevron, the delete × and the tick circle act and never toggle the full title.
// Each control is found by pressing along the row's middle; a press that toggles the row itself is undone at once.
#[gpui::test]
fn todo_presses_on_the_fold_delete_and_tick_controls_never_toggle_the_full_title(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] one\n  - [ ] a\n- [ ] two\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    let found = todo_press_down_the_middle(&mut visual, &host, 48., json!(["t:one"]));
    // The collapsed row's middle, where its controls sit; the press leaves "one" selected, so its × shows.
    let y = found.y + gpui::px(22.);
    todo_press(&mut visual, gpui::point(found.x, y), 1);
    assert_eq!(todo_wide(&mut visual, &host), json!([]));
    let width = f32::from(visual.update(|window, _| window.viewport_size().width));
    let (mut folds, mut deleted) = (0, false);
    for x in ((width - 100.) as i32..width as i32).step_by(2) {
        let at = gpui::point(gpui::px(x as f32), y);
        let open = todo_mounted_state(&mut visual, &host)["open"].clone();
        todo_press(&mut visual, at, 1);
        let state = todo_mounted_state(&mut visual, &host);
        if host.borrow().files[TODO_TASKS] != start {
            assert_eq!(state["wide"], json!([]), "the delete × at {x} toggled the title");
            deleted = true;
            break;
        }
        if state["open"] != open {
            assert_eq!(state["wide"], json!([]), "the fold chevron at {x} toggled the title");
            folds += 1;
        } else if state["wide"] != json!([]) {
            todo_press(&mut visual, at, 1);
            assert_eq!(todo_wide(&mut visual, &host), json!([]), "the row at {x} toggles back");
        }
    }
    assert!(folds > 0 && deleted, "the fold chevron ({folds} presses) and the delete × ({deleted}) were both pressed");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [ ] two\n");
    // "two" is the first row now; from the left edge the press reaches its tick circle.
    let mut ticked = false;
    for x in (0..100).step_by(2) {
        let at = gpui::point(gpui::px(x as f32), y);
        todo_press(&mut visual, at, 1);
        let wide = todo_wide(&mut visual, &host);
        if host.borrow().files[TODO_TASKS] != "# Tasks\n- [ ] two\n" {
            assert_eq!(wide, json!([]), "the tick circle at {x} toggled the title");
            ticked = true;
            break;
        }
        if wide != json!([]) {
            todo_press(&mut visual, at, 1);
            assert_eq!(todo_wide(&mut visual, &host), json!([]), "the row at {x} toggles back");
        }
    }
    assert!(ticked, "a press reached the tick circle");
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [x] two\n");
}

// TD-WRAP AC5: a wide title stays wide when its tick moves it under Completed and across a reload of an outside
// edit; switching lists clears it, even for a same-titled task in the other list, and switching back does not bring
// it back. Only the tick is written.
#[gpui::test]
fn todo_a_wide_title_survives_a_tick_and_a_reload_and_a_list_switch_clears_it(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let work = "# Work\n- [ ] same\n";
    let host = todo_host(&[(TODO_LISTS, TODO_TWO_LISTS), (TODO_TASKS, "# Tasks\n- [ ] same\n- [ ] other\n"), (TODO_WORK, work)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    todo_press_down_the_middle(&mut visual, &host, 48., json!(["t:same"]));
    files_keys(&mut visual, "space");
    files_wait(&mut visual, 50);
    assert_eq!(host.borrow().files[TODO_TASKS], "# Tasks\n- [x] same\n- [ ] other\n");
    assert_eq!(todo_wide(&mut visual, &host), json!(["t:same"]), "ticked and moved under Completed, still wide");
    let outside = "# Tasks\n- [x] same\n- [ ] other\n- [ ] new\n";
    host.borrow_mut().files.insert(TODO_TASKS.into(), outside.into());
    host.borrow_mut().commands.push_back("reload".into());
    files_wait(&mut visual, 50);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["text"].clone(), state["wide"].clone()), (json!(outside), json!(["t:same"])), "{state}");
    files_keys(&mut visual, "ctrl-l down enter");
    files_wait(&mut visual, 100);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["cur"].clone(), state["wide"].clone()), (json!("work"), json!([])), "{state}");
    files_keys(&mut visual, "ctrl-l up enter");
    files_wait(&mut visual, 100);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["cur"].clone(), state["text"].clone(), state["wide"].clone()), (json!("tasks"), json!(outside), json!([])), "{state}");
    assert_eq!(host.borrow().writes.len(), 1, "only the tick is written: {:?}", host.borrow().writes);
    assert_eq!(host.borrow().files[TODO_WORK], work);
}

// TD-WRAP AC1: in a narrow panel a wide title of 300 letters and no spaces wraps over several lines, so the row grows
// past where the next row began; collapsed, it is one line again and the same press meets the next row.
#[gpui::test]
fn todo_a_long_title_without_spaces_wraps_over_several_lines_when_wide(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let long = "x".repeat(300);
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, &format!("# Tasks\n- [ ] {long}\n- [ ] next\n"))], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    visual.simulate_resize(gpui::size(gpui::px(320.), gpui::px(800.)));
    visual.run_until_parked();
    let found = todo_press_down_the_middle(&mut visual, &host, 48., json!([format!("t:{long}")]));
    // 64 px below the long row's top: inside it while it is wide, in "next" (4 px gap after 44 px) once it is not.
    let below = gpui::point(found.x, found.y + gpui::px(64.));
    todo_press(&mut visual, below, 1);
    assert_eq!(todo_wide(&mut visual, &host), json!([]), "the wide row reaches 64 px down and collapses");
    todo_press(&mut visual, below, 1);
    assert_eq!(todo_wide(&mut visual, &host), json!(["t:next"]), "collapsed, the long row is one line and the press meets next");
    assert!(host.borrow().writes.is_empty(), "{:?}", host.borrow().writes);
}

// UPD-D1 AC4 (TD4 part 2) with real keys in the edit box: select all, copy, cut and paste (secondary = Cmd on macOS,
// Ctrl elsewhere) reach the box, and the panel's own keys (Delete) edit the text instead of the list. Nothing is
// written until Enter, which saves a pasted Windows path as it is.
#[gpui::test]
fn todo_edit_box_takes_select_all_copy_cut_and_paste_and_writes_only_on_enter(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] Milk run\n- [ ] two\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    files_keys(&mut visual, "enter");
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["edit"].clone(), state["sel"].clone()), (json!("Milk run"), json!({"k":"task", "i":0})), "{state}");
    let clipboard = |visual: &gpui::VisualTestContext| visual.read_from_clipboard().and_then(|c| c.text());
    // The box opens with its title selected; End drops that, so the copy below is select all's.
    files_keys(&mut visual, "end secondary-a secondary-c");
    assert_eq!(clipboard(&visual).as_deref(), Some("Milk run"));
    assert_eq!(todo_mounted_state(&mut visual, &host)["edit"], json!("Milk run"), "copy leaves the text");
    files_keys(&mut visual, "secondary-x");
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["edit"].clone(), state["menu"].clone()), (json!(""), json!(false)), "cut empties the box: {state}");
    assert_eq!(clipboard(&visual).as_deref(), Some("Milk run"));
    files_keys(&mut visual, "secondary-v secondary-v");
    assert_eq!(todo_mounted_state(&mut visual, &host)["edit"], json!("Milk runMilk run"));
    // Delete at the start removes a letter, never the task.
    files_keys(&mut visual, "home delete");
    assert_eq!(todo_mounted_state(&mut visual, &host)["edit"], json!("ilk runMilk run"));
    let path = r"C:\Users\me\notes.md";
    visual.write_to_clipboard(gpui::ClipboardItem::new_string(path.to_string()));
    files_keys(&mut visual, "secondary-a secondary-v");
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["edit"].clone(), state["sel"].clone()), (json!(path), json!({"k":"task", "i":0})), "{state}");
    {
        let h = host.borrow();
        assert!(h.writes.is_empty(), "cut and paste change only the box until Enter: {:?}", h.writes);
        assert_eq!(h.files[TODO_TASKS], start);
        assert!(h.effects.iter().all(|(n, _)| n != "notify"), "no delete notice: {:?}", h.effects);
    }
    files_keys(&mut visual, "enter");
    files_wait(&mut visual, 100);
    assert_eq!(todo_written(&host), [format!("# Tasks\n- [ ] {path}\n- [ ] two\n")]);
    assert_eq!(todo_mounted_state(&mut visual, &host)["edit"], Value::Null);
}

// UPD-D1 AC4 (TD4 part 2) with the mouse in the edit box opened by a double click: a drag selects part of the text and
// a double click selects a word (each copied), and neither ends the edit nor loses its unsaved text. Escape then
// writes nothing.
#[gpui::test]
fn todo_mouse_selection_in_the_edit_box_keeps_the_edit_and_its_unsaved_text(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // Long enough that the middle of the panel is over the text.
    let title = (0..60).map(|i| format!("word{i:02}")).collect::<Vec<_>>().join(" ");
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, &format!("# Tasks\n- [ ] {title}\n- [ ] two\n"))], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    let found = todo_press_down_the_middle(&mut visual, &host, 48., json!([format!("t:{title}")]));
    // 20 px below the row's top: inside the edit box whether the row is wide (box from 8 px) or not (centred in 44 px).
    let at = gpui::point(found.x, found.y + gpui::px(20.));
    todo_press(&mut visual, at, 1);
    todo_press(&mut visual, at, 2);
    assert_eq!(todo_mounted_state(&mut visual, &host)["edit"], json!(title), "the double click edits the row");
    files_keys(&mut visual, "end");
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_input(" zz");
    let typed = format!("{title} zz");
    assert_eq!(todo_mounted_state(&mut visual, &host)["edit"], json!(typed));
    let clipboard = |visual: &gpui::VisualTestContext| visual.read_from_clipboard().and_then(|c| c.text());
    let copied_part = |visual: &mut gpui::VisualTestContext, what: &str| {
        visual.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".into()));
        files_keys(visual, "secondary-c");
        let copied = clipboard(&*visual).unwrap_or_default();
        assert!(copied != "sentinel" && !copied.is_empty() && copied.len() < typed.len() && typed.contains(&copied), "{what} selects part of the text: {copied:?}");
    };

    // A drag 80 px to the left.
    let (modifiers, button) = (gpui::Modifiers::none(), gpui::MouseButton::Left);
    let to = gpui::point(at.x - gpui::px(80.), at.y);
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_event(gpui::MouseDownEvent { position: at, modifiers, button, click_count: 1, first_mouse: false });
    visual.simulate_event(gpui::MouseMoveEvent { position: to, pressed_button: Some(button), modifiers });
    visual.simulate_event(gpui::MouseUpEvent { position: to, modifiers, button, click_count: 1 });
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["edit"].clone(), state["sel"].clone()), (json!(typed), json!({"k":"task", "i":0})), "a drag keeps the edit: {state}");
    copied_part(&mut visual, "the drag");

    // A double click on a word, at the drag's end.
    todo_press(&mut visual, to, 1);
    todo_press(&mut visual, to, 2);
    assert_eq!(todo_mounted_state(&mut visual, &host)["edit"], json!(typed), "a double click keeps the edit and its text");
    copied_part(&mut visual, "the double click");

    assert!(host.borrow().writes.is_empty(), "selecting writes nothing: {:?}", host.borrow().writes);
    files_keys(&mut visual, "escape");
    files_wait(&mut visual, 100);
    assert!(host.borrow().writes.is_empty(), "Escape drops the edit: {:?}", host.borrow().writes);
    assert_eq!(todo_mounted_state(&mut visual, &host)["edit"], Value::Null);
}

// The left button goes down at `at` on a fresh frame; a `click_count` of 2 is the second press of a double click.
fn todo_down(visual: &mut gpui::VisualTestContext, at: gpui::Point<gpui::Pixels>, click_count: usize) {
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let (modifiers, button) = (gpui::Modifiers::none(), gpui::MouseButton::Left);
    visual.simulate_event(gpui::MouseDownEvent { position: at, modifiers, button, click_count, first_mouse: false });
}

fn todo_up(visual: &mut gpui::VisualTestContext, at: gpui::Point<gpui::Pixels>) {
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let (modifiers, button) = (gpui::Modifiers::none(), gpui::MouseButton::Left);
    visual.simulate_event(gpui::MouseUpEvent { position: at, modifiers, button, click_count: 1 });
}

// The pointer moves to `at` on a fresh frame, the left button held when `held`.
fn todo_move(visual: &mut gpui::VisualTestContext, at: gpui::Point<gpui::Pixels>, held: bool) {
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let pressed_button = held.then_some(gpui::MouseButton::Left);
    visual.simulate_event(gpui::MouseMoveEvent { position: at, pressed_button, modifiers: gpui::Modifiers::none() });
}

// A drag not yet released: the left button goes down at `from`, moves 6 px down (past the 4 px that make a press a drag,
// still inside a pressed row of 30 px or more when `from` is its middle), then over `to`.
fn todo_drag_over(visual: &mut gpui::VisualTestContext, from: gpui::Point<gpui::Pixels>, to: gpui::Point<gpui::Pixels>) {
    todo_down(visual, from, 1);
    todo_move(visual, gpui::point(from.x, from.y + gpui::px(6.)), true);
    todo_move(visual, to, true);
}

// The task and step rows down the middle of the panel from 48 px to `until`, found by hovering with no button held: each
// row's hover key (`i.` a task, `i.j` a step) with the first and last y that hovered it, top to bottom. Add step rows and
// the Completed head have no hover key; they are in the gaps between these.
fn todo_rows(visual: &mut gpui::VisualTestContext, host: &Rc<RefCell<TodoHost>>, until: i32) -> Vec<(String, f32, f32)> {
    let x = visual.update(|window, _| window.viewport_size().width) * 0.5;
    let mut rows: Vec<(String, f32, f32)> = Vec::new();
    for y in (48..until).step_by(2) {
        todo_move(visual, gpui::point(x, gpui::px(y as f32)), false);
        let state = todo_mounted_state(visual, host);
        let Some(key) = state["hover"].as_str() else { continue };
        match rows.last_mut() {
            Some(row) if row.0 == key => row.2 = y as f32,
            _ => rows.push((key.to_string(), y as f32, y as f32)),
        }
    }
    rows
}

fn todo_row(rows: &[(String, f32, f32)], key: &str) -> (f32, f32) {
    let row = rows.iter().find(|r| r.0 == key).unwrap_or_else(|| panic!("no row {key} in {rows:?}"));
    (row.1, row.2)
}

// A point down the middle of the panel in row `key`, `at` of the way from its top (0.0) to its bottom (1.0).
fn todo_at(visual: &mut gpui::VisualTestContext, rows: &[(String, f32, f32)], key: &str, at: f32) -> gpui::Point<gpui::Pixels> {
    let x = visual.update(|window, _| window.viewport_size().width) * 0.5;
    let (top, bottom) = todo_row(rows, key);
    gpui::point(x, gpui::px(top + (bottom - top) * at))
}

fn todo_row_keys(rows: &[(String, f32, f32)]) -> Vec<&str> {
    rows.iter().map(|r| r.0.as_str()).collect()
}

// UPD-D2 AC1/AC5: a task dragged with the mouse shows a drop line where it would land and, released, moves among the open
// tasks (stepping over the done task between them, its note line going with it): one write per completed drag, the
// moved task selected, the wide toggle of each press undone. Dropping past the last open task works; a drop that leaves
// the order as it is (only a done task in between) or on its own row writes nothing.
#[gpui::test]
fn todo_dragging_a_task_moves_it_among_the_open_tasks_with_one_write_per_drop(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] one\n- [x] done\n- [ ] two\n- [ ] three\n  > note\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    let rows = todo_rows(&mut visual, &host, 400);
    assert_eq!(todo_row_keys(&rows), ["0.", "2.", "3."], "{rows:?}");

    // three up to the top of one.
    let (from, to) = (todo_at(&mut visual, &rows, "3.", 0.5), todo_at(&mut visual, &rows, "0.", 0.2));
    todo_drag_over(&mut visual, from, to);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(state["drag"]["slot"], json!({"k":"task", "i":0, "title":"one", "after":false}), "{state}");
    assert_eq!(state["lines"], json!([[{"k":"task", "i":0}, "top"]]), "the drop line is at the top of one: {state}");
    assert!(host.borrow().writes.is_empty(), "nothing is written before the release");
    todo_up(&mut visual, to);
    files_wait(&mut visual, 50);
    let first = "# Tasks\n- [ ] three\n  > note\n- [ ] one\n- [x] done\n- [ ] two\n";
    assert_eq!(todo_written(&host), [first]);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(
        (state["sel"].clone(), state["drag"].clone(), state["press"].clone(), state["lines"].clone()),
        (json!({"k":"task", "i":0}), Value::Null, json!(false), json!([])),
        "the moved task is selected and the drag is over: {state}"
    );

    // one past the last open task: the bottom half of two, over the done task.
    let rows = todo_rows(&mut visual, &host, 400);
    assert_eq!(todo_row_keys(&rows), ["0.", "1.", "3."], "{rows:?}");
    let (from, to) = (todo_at(&mut visual, &rows, "1.", 0.5), todo_at(&mut visual, &rows, "3.", 0.8));
    todo_drag_over(&mut visual, from, to);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(state["drag"]["slot"], json!({"k":"task", "i":3, "title":"two", "after":true}), "{state}");
    assert_eq!(state["lines"], json!([[{"k":"task", "i":3}, "bottom"]]), "the drop line is under two: {state}");
    todo_up(&mut visual, to);
    files_wait(&mut visual, 50);
    let second = "# Tasks\n- [ ] three\n  > note\n- [x] done\n- [ ] two\n- [ ] one\n";
    assert_eq!(todo_written(&host), [first, second]);
    assert_eq!(todo_mounted_state(&mut visual, &host)["sel"], json!({"k":"task", "i":3}));

    // two onto the bottom half of three: only the done task lies between, so nothing changes and nothing is written.
    let rows = todo_rows(&mut visual, &host, 400);
    assert_eq!(todo_row_keys(&rows), ["0.", "2.", "3."], "{rows:?}");
    let (from, to) = (todo_at(&mut visual, &rows, "2.", 0.5), todo_at(&mut visual, &rows, "0.", 0.8));
    todo_drag_over(&mut visual, from, to);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(state["drag"]["slot"], json!({"k":"task", "i":0, "title":"three", "after":true}), "{state}");
    todo_up(&mut visual, to);
    files_wait(&mut visual, 50);
    assert_eq!(todo_written(&host), [first, second], "the drop at its own place writes nothing");

    // one over its own row: a drag with no slot and no line; released, nothing is written.
    let (from, to) = (todo_at(&mut visual, &rows, "3.", 0.5), todo_at(&mut visual, &rows, "3.", 0.9));
    todo_drag_over(&mut visual, from, to);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["drag"]["slot"].clone(), state["lines"].clone()), (Value::Null, json!([])), "{state}");
    assert!(!state["drag"].is_null(), "it is a drag: {state}");
    todo_up(&mut visual, to);
    files_wait(&mut visual, 50);
    assert_eq!(todo_written(&host), [first, second]);
    assert_eq!(host.borrow().files[TODO_TASKS], second);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["drag"].clone(), state["press"].clone(), state["wide"].clone()), (Value::Null, json!(false), json!([])), "{state}");
}

// UPD-D2 AC5 at the 4 px boundary: a press that moves 3 px is a click (the full title toggles, no drag, nothing written);
// one that moves exactly 4 px is a drag that undoes the toggle its press made. A real drag afterwards still moves the
// task, and its title stays as wide as before the press.
#[gpui::test]
fn todo_a_press_moving_under_4_px_is_a_click_and_at_4_px_a_drag_that_undoes_the_wide_toggle(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] one\n- [ ] two\n")], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    let rows = todo_rows(&mut visual, &host, 300);
    assert_eq!(todo_row_keys(&rows), ["0.", "1."], "{rows:?}");
    let at = todo_at(&mut visual, &rows, "0.", 0.5);
    let by = |dy: f32| gpui::point(at.x, at.y + gpui::px(dy));

    todo_down(&mut visual, at, 1);
    todo_move(&mut visual, by(3.), true);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(
        (state["press"].clone(), state["drag"].clone(), state["wide"].clone()),
        (json!(true), Value::Null, json!(["t:one"])),
        "3 px is still a click: {state}"
    );
    todo_up(&mut visual, by(3.));
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(
        (state["press"].clone(), state["wide"].clone(), state["sel"].clone()),
        (json!(false), json!(["t:one"]), json!({"k":"task", "i":0})),
        "the click toggled the full title: {state}"
    );

    todo_down(&mut visual, at, 1);
    assert_eq!(todo_wide(&mut visual, &host), json!([]), "the press toggles it back at once");
    todo_move(&mut visual, by(4.), true);
    let state = todo_mounted_state(&mut visual, &host);
    assert!(!state["drag"].is_null(), "4 px starts a drag: {state}");
    assert_eq!((state["wide"].clone(), state["drag"]["slot"].clone()), (json!(["t:one"]), Value::Null), "the drag undid the press's toggle: {state}");
    todo_up(&mut visual, by(4.));
    assert_eq!(todo_wide(&mut visual, &host), json!(["t:one"]));
    assert!(host.borrow().writes.is_empty(), "{:?}", host.borrow().writes);

    let to = todo_at(&mut visual, &rows, "1.", 0.8);
    todo_drag_over(&mut visual, at, to);
    todo_up(&mut visual, to);
    files_wait(&mut visual, 50);
    assert_eq!(todo_written(&host), ["# Tasks\n- [ ] two\n- [ ] one\n"]);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["wide"].clone(), state["sel"].clone()), (json!(["t:one"]), json!({"k":"task", "i":1})), "{state}");
}

// UPD-D2 AC2: a step moves within its task before or after a step by the row half it is dropped on, onto another task's
// row (folded: it becomes its last step and the task opens) or onto its Add step row (the last step), keeping its done
// state, with the selection following it. A step dropped on its own task's row, right next to itself, or on its own
// task's Add step row when it is already last writes nothing.
#[gpui::test]
fn todo_dragging_a_step_moves_it_within_its_task_or_to_the_end_of_another_keeping_its_done_state(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] A\n  - [ ] a1\n  - [x] a2\n  - [ ] a3\n- [ ] B\n  - [ ] b1\n- [ ] C\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    files_keys(&mut visual, "right");
    files_wait(&mut visual, 50);
    assert_eq!(todo_mounted_state(&mut visual, &host)["open"], json!(["A"]));
    let rows = todo_rows(&mut visual, &host, 560);
    assert_eq!(todo_row_keys(&rows), ["0.", "0.0", "0.1", "0.2", "1.", "2."], "{rows:?}");

    // a3 to the top half of a1.
    let (from, to) = (todo_at(&mut visual, &rows, "0.2", 0.5), todo_at(&mut visual, &rows, "0.0", 0.2));
    todo_drag_over(&mut visual, from, to);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(state["drag"]["slot"], json!({"k":"step", "i":0, "title":"A", "j":0, "beforeTitle":"a1"}), "{state}");
    assert_eq!(state["lines"], json!([[{"k":"step", "i":0, "j":0}, "top"]]), "{state}");
    todo_up(&mut visual, to);
    files_wait(&mut visual, 50);
    let w1 = "# Tasks\n- [ ] A\n  - [ ] a3\n  - [ ] a1\n  - [x] a2\n- [ ] B\n  - [ ] b1\n- [ ] C\n";
    assert_eq!(todo_written(&host), [w1]);
    assert_eq!(todo_mounted_state(&mut visual, &host)["sel"], json!({"k":"step", "i":0, "j":0}));

    // The done a2 onto folded B's row: B's last step, still done; B opens.
    let (from, to) = (todo_at(&mut visual, &rows, "0.2", 0.5), todo_at(&mut visual, &rows, "1.", 0.5));
    todo_drag_over(&mut visual, from, to);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(state["drag"]["slot"], json!({"k":"step", "i":1, "title":"B", "j":1, "beforeTitle":null}), "{state}");
    assert_eq!(state["lines"], json!([[{"k":"task", "i":1}, "bottom"]]), "{state}");
    todo_up(&mut visual, to);
    files_wait(&mut visual, 50);
    let w2 = "# Tasks\n- [ ] A\n  - [ ] a3\n  - [ ] a1\n- [ ] B\n  - [ ] b1\n  - [x] a2\n- [ ] C\n";
    assert_eq!(todo_written(&host), [w1, w2]);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["open"].clone(), state["sel"].clone()), (json!(["A", "B"]), json!({"k":"step", "i":1, "j":1})), "{state}");

    // a1 onto B's Add step row (between B's last step and C).
    let rows = todo_rows(&mut visual, &host, 560);
    assert_eq!(todo_row_keys(&rows), ["0.", "0.0", "0.1", "1.", "1.0", "1.1", "2."], "{rows:?}");
    let add_b = gpui::point(todo_at(&mut visual, &rows, "2.", 0.).x, gpui::px((todo_row(&rows, "1.1").1 + todo_row(&rows, "2.").0) / 2.));
    let from = todo_at(&mut visual, &rows, "0.1", 0.5);
    todo_drag_over(&mut visual, from, add_b);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(state["drag"]["slot"], json!({"k":"step", "i":1, "title":"B", "j":2, "beforeTitle":null}), "{state}");
    assert_eq!(state["lines"], json!([[{"k":"add", "i":1}, "top"]]), "{state}");
    todo_up(&mut visual, add_b);
    files_wait(&mut visual, 50);
    let w3 = "# Tasks\n- [ ] A\n  - [ ] a3\n- [ ] B\n  - [ ] b1\n  - [x] a2\n  - [ ] a1\n- [ ] C\n";
    assert_eq!(todo_written(&host), [w1, w2, w3]);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["sel"].clone(), state["edit"].clone()), (json!({"k":"step", "i":1, "j":2}), Value::Null), "no Add step edit opens: {state}");

    // No-ops: a3 onto its own task's row; a1 onto the bottom half of a2 (where it already is); a1 onto B's Add step row
    // when it is already B's last step (a slot is shown, but nothing changes).
    let rows = todo_rows(&mut visual, &host, 560);
    assert_eq!(todo_row_keys(&rows), ["0.", "0.0", "1.", "1.0", "1.1", "1.2", "2."], "{rows:?}");
    let add_b = gpui::point(todo_at(&mut visual, &rows, "2.", 0.).x, gpui::px((todo_row(&rows, "1.2").1 + todo_row(&rows, "2.").0) / 2.));
    let cases = [
        ("a3 onto A", todo_at(&mut visual, &rows, "0.0", 0.5), todo_at(&mut visual, &rows, "0.", 0.5), false),
        ("a1 below a2", todo_at(&mut visual, &rows, "1.2", 0.5), todo_at(&mut visual, &rows, "1.1", 0.8), false),
        ("a1 onto B's Add step", todo_at(&mut visual, &rows, "1.2", 0.5), add_b, true),
    ];
    for (what, from, to, slot) in cases {
        todo_drag_over(&mut visual, from, to);
        let state = todo_mounted_state(&mut visual, &host);
        assert!(!state["drag"].is_null(), "{what} is a drag: {state}");
        assert_eq!(!state["drag"]["slot"].is_null(), slot, "{what}: {state}");
        todo_up(&mut visual, to);
        files_wait(&mut visual, 50);
        assert_eq!(todo_written(&host), [w1, w2, w3], "{what} writes nothing");
    }
    assert_eq!(host.borrow().files[TODO_TASKS], w3);
}

// UPD-D2 AC3: no drag starts from a done task, a step of a done task, the Completed head, an Add step row, the second
// press of a double click or the row being edited (its edit stays open); nor on a read-only list. A step over a done
// task and a task over the Completed head have nowhere to land. Nothing is written, no Add step edit opens and the done
// tasks stay shown.
#[gpui::test]
fn todo_done_rows_the_completed_head_add_step_rows_double_clicks_edits_and_read_only_lists_never_drag(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] one\n  - [ ] s\n- [x] old\n  - [x] os\n- [ ] two\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    // Open one; walk past s, Add step and two to Completed and show it; open old.
    files_keys(&mut visual, "right down down down down enter down right");
    files_wait(&mut visual, 50);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["open"].clone(), state["sel"].clone()), (json!(["one", "old"]), json!({"k":"task", "i":1})), "{state}");
    let rows = todo_rows(&mut visual, &host, 640);
    assert_eq!(todo_row_keys(&rows), ["0.", "0.0", "2.", "1.", "1.0"], "{rows:?}");
    let x = todo_at(&mut visual, &rows, "0.", 0.).x;
    let between = |a: &str, b: &str| gpui::point(x, gpui::px((todo_row(&rows, a).1 + todo_row(&rows, b).0) / 2.));
    let (add_one, head) = (between("0.0", "2."), between("2.", "1."));
    let one_top = todo_at(&mut visual, &rows, "0.", 0.2);
    let two = todo_at(&mut visual, &rows, "2.", 0.5);
    let no_drag = [
        ("the done task", todo_at(&mut visual, &rows, "1.", 0.5), one_top),
        ("the done task's step", todo_at(&mut visual, &rows, "1.0", 0.5), two),
        ("the Completed head", head, one_top),
        ("the Add step row", add_one, one_top),
    ];
    for (what, from, to) in no_drag {
        todo_drag_over(&mut visual, from, to);
        let state = todo_mounted_state(&mut visual, &host);
        assert_eq!((state["press"].clone(), state["drag"].clone(), state["lines"].clone()), (json!(false), Value::Null, json!([])), "{what}: {state}");
        todo_up(&mut visual, to);
        let state = todo_mounted_state(&mut visual, &host);
        assert_eq!((state["edit"].clone(), state["open"].clone()), (Value::Null, json!(["one", "old"])), "{what}: {state}");
    }
    let nowhere = [
        ("s over the done task", todo_at(&mut visual, &rows, "0.0", 0.5), todo_at(&mut visual, &rows, "1.", 0.5)),
        ("two over the Completed head", two, head),
    ];
    for (what, from, to) in nowhere {
        todo_drag_over(&mut visual, from, to);
        let state = todo_mounted_state(&mut visual, &host);
        assert!(!state["drag"].is_null(), "{what} is a drag: {state}");
        assert_eq!((state["drag"]["slot"].clone(), state["lines"].clone()), (Value::Null, json!([])), "{what}: {state}");
        todo_up(&mut visual, to);
    }
    files_wait(&mut visual, 50);
    assert!(host.borrow().writes.is_empty(), "{:?}", host.borrow().writes);

    // A double click's second press edits two and starts no drag.
    todo_press(&mut visual, two, 1);
    todo_down(&mut visual, two, 2);
    todo_move(&mut visual, gpui::point(two.x, two.y + gpui::px(6.)), true);
    todo_move(&mut visual, one_top, true);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["press"].clone(), state["drag"].clone(), state["edit"].clone()), (json!(false), Value::Null, json!("two")), "{state}");
    todo_up(&mut visual, one_top);
    // The row being edited leaves the press to its edit box: dragging from it onto one moves nothing, the edit stays.
    todo_drag_over(&mut visual, two, one_top);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["press"].clone(), state["drag"].clone(), state["edit"].clone()), (json!(false), Value::Null, json!("two")), "{state}");
    todo_up(&mut visual, one_top);
    files_wait(&mut visual, 50);
    assert_eq!(todo_mounted_state(&mut visual, &host)["edit"], json!("two"));
    files_keys(&mut visual, "escape");
    files_wait(&mut visual, 50);
    assert_eq!(todo_mounted_state(&mut visual, &host)["edit"], Value::Null);
    assert!(host.borrow().writes.is_empty(), "{:?}", host.borrow().writes);

    // The list turns read-only (now over the read limit) with its rows still shown: no press becomes a drag.
    let big = "x".repeat(1024 * 1024 + 1);
    host.borrow_mut().files.insert(TODO_TASKS.into(), big.clone());
    host.borrow_mut().commands.push_back("reload".into());
    files_wait(&mut visual, 50);
    assert_eq!(todo_mounted_state(&mut visual, &host)["readonly"], json!(true));
    let rows = todo_rows(&mut visual, &host, 640);
    assert_eq!(todo_row_keys(&rows), ["0.", "0.0", "2.", "1.", "1.0"], "{rows:?}");
    let (from, to) = (todo_at(&mut visual, &rows, "2.", 0.5), todo_at(&mut visual, &rows, "0.", 0.2));
    todo_drag_over(&mut visual, from, to);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["press"].clone(), state["drag"].clone()), (json!(false), Value::Null), "read-only: {state}");
    todo_up(&mut visual, to);
    files_wait(&mut visual, 50);
    assert!(host.borrow().writes.is_empty(), "{:?}", host.borrow().writes.len());
    assert_eq!(host.borrow().files[TODO_TASKS], big, "left untouched");
}

// UPD-D2 AC3: presses on the fold chevron, the delete × and the tick circle act and never start a drag, while presses on
// the rest of the row do. Each spot along the first row is pressed, moved 4 px and released: right of the title from the
// left (the chevron folds, the × deletes), then from the left edge (the circle ticks). Only the delete and the tick write.
#[gpui::test]
fn todo_presses_on_the_tick_fold_and_delete_controls_never_start_a_drag(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, "# Tasks\n- [ ] one\n  - [ ] s\n- [ ] two\n- [ ] three\n")], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    let rows = todo_rows(&mut visual, &host, 300);
    let y = todo_at(&mut visual, &rows, "0.", 0.5).y;
    let width = f32::from(visual.update(|window, _| window.viewport_size().width));
    let (mut body, mut folds, mut deleted, mut ticked) = (0, 0, false, false);
    let right = ((width - 100.) as i32..width as i32).step_by(2);
    let left = (0..100).step_by(2);
    for (side, x) in right.map(|x| ("right", x)).chain(left.map(|x| ("left", x))) {
        if (side == "right" && deleted) || ticked {
            continue;
        }
        let (at, moved) = (gpui::point(gpui::px(x as f32), y), gpui::point(gpui::px(x as f32), y + gpui::px(4.)));
        let open = todo_mounted_state(&mut visual, &host)["open"].clone();
        let text = host.borrow().files[TODO_TASKS].clone();
        todo_down(&mut visual, at, 1);
        todo_move(&mut visual, moved, true);
        let held = todo_mounted_state(&mut visual, &host);
        todo_up(&mut visual, moved);
        files_wait(&mut visual, 50);
        let state = todo_mounted_state(&mut visual, &host);
        let file = host.borrow().files[TODO_TASKS].clone();
        if file != text || state["open"] != open {
            assert_eq!((held["press"].clone(), held["drag"].clone()), (json!(false), Value::Null), "the control at {x} started a drag: {held}");
            if file.matches("- [").count() < text.matches("- [").count() {
                deleted = true;
            } else if file.contains("[x]") {
                ticked = true;
            } else {
                folds += 1;
            }
        } else if !held["drag"].is_null() {
            body += 1;
        }
    }
    assert!(body > 0 && folds > 0 && deleted && ticked, "row body {body}, chevron {folds}, × {deleted}, circle {ticked}");
    assert_eq!(todo_written(&host), ["# Tasks\n- [ ] two\n- [ ] three\n", "# Tasks\n- [x] two\n- [ ] three\n"]);
    assert_eq!(todo_wide(&mut visual, &host), json!([]), "every drag undid its press's toggle");
}

// UPD-D2 AC4: Esc while dragging cancels it (and does nothing else Esc would do, such as leaving the panel); leaving the
// list area clears the slot and coming back shows it again; the pointer leaving the panel cancels the drag. Moving on and
// releasing afterwards writes nothing.
#[gpui::test]
fn todo_escape_or_leaving_the_panel_cancels_a_drag_and_leaving_the_list_clears_its_slot(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] one\n- [ ] two\n- [ ] three\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    let rows = todo_rows(&mut visual, &host, 300);
    assert_eq!(todo_row_keys(&rows), ["0.", "1.", "2."], "{rows:?}");
    let (one_top, two, three) =
        (todo_at(&mut visual, &rows, "0.", 0.2), todo_at(&mut visual, &rows, "1.", 0.5), todo_at(&mut visual, &rows, "2.", 0.5));
    let slot = json!({"k":"task", "i":0, "title":"one", "after":false});

    todo_drag_over(&mut visual, three, one_top);
    assert_eq!(todo_mounted_state(&mut visual, &host)["drag"]["slot"], slot);
    files_keys(&mut visual, "escape");
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["press"].clone(), state["drag"].clone(), state["lines"].clone()), (json!(false), Value::Null, json!([])), "{state}");
    todo_move(&mut visual, two, true);
    assert_eq!(todo_mounted_state(&mut visual, &host)["drag"], Value::Null, "moving on does not start it again");
    todo_up(&mut visual, two);
    files_wait(&mut visual, 50);
    assert!(host.borrow().writes.is_empty(), "{:?}", host.borrow().writes);
    let left_panel = todo_effects(&host, "run").iter().any(|a| a[0] == json!("pane.go"));
    assert!(!left_panel, "Esc only cancelled the drag: {:?}", todo_effects(&host, "run"));

    // Above the list (over the add box) there is no slot; back over one it returns; released above, nothing is written.
    let above = gpui::point(one_top.x, gpui::px(todo_row(&rows, "0.").0 - 20.));
    todo_drag_over(&mut visual, three, one_top);
    todo_move(&mut visual, above, true);
    let state = todo_mounted_state(&mut visual, &host);
    assert!(!state["drag"].is_null(), "still dragging: {state}");
    assert_eq!((state["drag"]["slot"].clone(), state["lines"].clone()), (Value::Null, json!([])), "{state}");
    todo_move(&mut visual, one_top, true);
    assert_eq!(todo_mounted_state(&mut visual, &host)["drag"]["slot"], slot);
    todo_move(&mut visual, above, true);
    todo_up(&mut visual, above);
    files_wait(&mut visual, 50);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["press"].clone(), state["drag"].clone()), (json!(false), Value::Null), "{state}");
    assert!(host.borrow().writes.is_empty(), "{:?}", host.borrow().writes);

    // Out of the panel (past the window's right edge): cancelled; back over one and released there, nothing is written.
    let width = visual.update(|window, _| window.viewport_size().width);
    todo_drag_over(&mut visual, three, one_top);
    todo_move(&mut visual, gpui::point(width + gpui::px(40.), one_top.y), true);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["press"].clone(), state["drag"].clone()), (json!(false), Value::Null), "{state}");
    todo_move(&mut visual, one_top, true);
    assert_eq!(todo_mounted_state(&mut visual, &host)["drag"], Value::Null);
    todo_up(&mut visual, one_top);
    files_wait(&mut visual, 50);
    assert!(host.borrow().writes.is_empty(), "{:?}", host.borrow().writes);
    assert_eq!(host.borrow().files[TODO_TASKS], start);
}

// UPD-D2 AC1 edge: with one open task (the other done) a drag has nowhere to land: not on its own row, the Completed head,
// the empty list below or the shown done task. Nothing is written.
#[gpui::test]
fn todo_a_list_with_one_open_task_has_nowhere_to_drop_it(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] only\n- [x] old\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    let rows = todo_rows(&mut visual, &host, 300);
    assert_eq!(todo_row_keys(&rows), ["0."], "{rows:?}");
    let only = todo_at(&mut visual, &rows, "0.", 0.5);
    let bottom = todo_row(&rows, "0.").1;
    // Completed sits 4 px (gap) + 6 px (margin) under the row and is 32 px tall; the list is empty further down.
    let (head, empty) = (gpui::point(only.x, gpui::px(bottom + 26.)), gpui::point(only.x, gpui::px(bottom + 120.)));
    for (what, to) in [("its own row", todo_at(&mut visual, &rows, "0.", 0.9)), ("Completed", head), ("the empty list", empty)] {
        todo_drag_over(&mut visual, only, to);
        let state = todo_mounted_state(&mut visual, &host);
        assert!(!state["drag"].is_null(), "{what}: {state}");
        assert_eq!((state["drag"]["slot"].clone(), state["lines"].clone()), (Value::Null, json!([])), "{what}: {state}");
        todo_up(&mut visual, to);
        files_wait(&mut visual, 50);
    }
    files_keys(&mut visual, "down enter");
    files_wait(&mut visual, 50);
    let rows = todo_rows(&mut visual, &host, 300);
    assert_eq!(todo_row_keys(&rows), ["0.", "1."], "{rows:?}");
    let (from, to) = (todo_at(&mut visual, &rows, "0.", 0.5), todo_at(&mut visual, &rows, "1.", 0.5));
    todo_drag_over(&mut visual, from, to);
    assert_eq!(todo_mounted_state(&mut visual, &host)["drag"]["slot"], Value::Null, "a done task is no place to drop");
    todo_up(&mut visual, to);
    files_wait(&mut visual, 50);
    assert!(host.borrow().writes.is_empty(), "{:?}", host.borrow().writes);
    assert_eq!(host.borrow().files[TODO_TASKS], start);
}

// UPD-D2 edge: two tasks share a title. A drop made while someone else appended to the file is written once more over the
// fresh text, finding its tasks by title near their old place, so the right one moves and the outside line stays. The
// selection follows the moved task, not the first task with its title.
#[gpui::test]
fn todo_a_drop_among_duplicate_titles_replays_over_an_outside_edit_and_selects_the_moved_task(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] a\n  - [ ] x\n- [ ] b\n- [ ] a\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    let rows = todo_rows(&mut visual, &host, 300);
    assert_eq!(todo_row_keys(&rows), ["0.", "1.", "2."], "{rows:?}");
    let outside = format!("{start}- [ ] new\n");
    host.borrow_mut().files.insert(TODO_TASKS.into(), outside.clone());
    let (from, to) = (todo_at(&mut visual, &rows, "2.", 0.5), todo_at(&mut visual, &rows, "1.", 0.2));
    todo_drag_over(&mut visual, from, to);
    todo_up(&mut visual, to);
    files_wait(&mut visual, 100);
    let merged = "# Tasks\n- [ ] a\n  - [ ] x\n- [ ] a\n- [ ] b\n- [ ] new\n";
    assert_eq!(
        todo_written(&host),
        ["# Tasks\n- [ ] a\n  - [ ] x\n- [ ] a\n- [ ] b\n", merged],
        "the stale write is refused and the drop replayed once"
    );
    assert_eq!(host.borrow().writes[1].2, json!(outside), "over the fresh text");
    assert_eq!(host.borrow().files[TODO_TASKS], merged);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["text"].clone(), state["error"].clone(), state["pending"].clone()), (json!(merged), Value::Null, json!(0)), "{state}");
    assert_eq!(state["sel"], json!({"k":"task", "i":1}), "the moved a (no steps) is selected, not the first a: {state}");
}

// UPD-D2 edge: drops while the save queue is busy (TD3 stuck flag) wait, are shown, and are written together once the
// flag heals: nothing lost, nothing written twice. The next drop is written on its own.
#[gpui::test]
fn todo_drops_while_the_save_queue_is_busy_are_written_once_when_it_frees(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let start = "# Tasks\n- [ ] one\n- [ ] two\n- [ ] three\n";
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, start)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    let rows = todo_rows(&mut visual, &host, 300);
    assert_eq!(todo_row_keys(&rows), ["0.", "1.", "2."], "{rows:?}");
    let (first_top, second_top, third) =
        (todo_at(&mut visual, &rows, "0.", 0.2), todo_at(&mut visual, &rows, "1.", 0.2), todo_at(&mut visual, &rows, "2.", 0.5));
    host.borrow_mut().commands.push_back("stuck 0".into());
    files_wait(&mut visual, 50);
    // three to the top, then two (now last) above one.
    todo_drag_over(&mut visual, third, first_top);
    todo_up(&mut visual, first_top);
    files_wait(&mut visual, 50);
    todo_drag_over(&mut visual, third, second_top);
    todo_up(&mut visual, second_top);
    files_wait(&mut visual, 50);
    let both = "# Tasks\n- [ ] three\n- [ ] two\n- [ ] one\n";
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["text"].clone(), state["pending"].clone(), state["flushing"].clone()), (json!(both), json!(2), json!(true)), "{state}");
    assert!(host.borrow().writes.is_empty(), "nothing is written while the flag is set: {:?}", host.borrow().writes);
    host.borrow_mut().commands.extend(["stuck 11000", "reload"].map(String::from));
    files_wait(&mut visual, 50);
    assert_eq!(todo_written(&host), [both], "both drops in one write");
    assert_eq!(host.borrow().writes[0].2, json!(start));
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["pending"].clone(), state["flushing"].clone(), state["error"].clone()), (json!(0), json!(false), Value::Null), "{state}");
    todo_drag_over(&mut visual, third, first_top);
    todo_up(&mut visual, first_top);
    files_wait(&mut visual, 50);
    assert_eq!(todo_written(&host), [both, "# Tasks\n- [ ] one\n- [ ] three\n- [ ] two\n"]);
}

// UPD-D2 edge: in a list already over the 64 KB write limit (it still opens editable) a drop is refused with the reason:
// nothing is written or shown moved, and the selection stays on the pressed task.
#[gpui::test]
fn todo_a_drop_in_a_list_over_the_64_kb_limit_is_refused_and_writes_nothing(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let full = "This list is full (64 KB, about 2,900 tasks). Start a new list for more tasks.";
    let big = format!("# Tasks\n- [ ] one\n- [ ] two\n  > {}\n", "y".repeat(65 * 1024));
    let host = todo_host(&[(TODO_LISTS, "- [Tasks](tasks.md)\n"), (TODO_TASKS, &big)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    // A big list takes a moment of real time to arrive and parse.
    let mut state = todo_mounted_state(&mut visual, &host);
    for _ in 0..2400 {
        if state["ready"] == json!(true) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        visual.run_until_parked();
        state = todo_mounted_state(&mut visual, &host);
    }
    assert_eq!((state["ready"].clone(), state["readonly"].clone()), (json!(true), json!(false)), "{state}");
    let rows = todo_rows(&mut visual, &host, 300);
    assert_eq!(todo_row_keys(&rows), ["0.", "1."], "{rows:?}");
    let (from, to) = (todo_at(&mut visual, &rows, "1.", 0.5), todo_at(&mut visual, &rows, "0.", 0.2));
    // A release cut off by the sandbox's 500 ms budget before it refused changed nothing, so it is dragged again.
    for _ in 0..5 {
        todo_drag_over(&mut visual, from, to);
        todo_up(&mut visual, to);
        files_wait(&mut visual, 50);
        state = todo_mounted_state(&mut visual, &host);
        if state["error"] == json!(full) {
            break;
        }
    }
    assert_eq!(state["error"], json!(full), "the reason is shown: {}", state["error"]);
    assert!(host.borrow().writes.is_empty(), "{}", host.borrow().writes.len());
    assert_eq!(host.borrow().files[TODO_TASKS], big);
    assert_eq!(
        (state["textLen"].clone(), state["sel"].clone(), state["drag"].clone()),
        (json!(big.chars().count()), json!({"k":"task", "i":1}), Value::Null),
        "not shown moved; the selection stays on two"
    );
}

// An unreadable lists.md is left alone until "Start a new index" is clicked; then the index and an empty Tasks list are
// written (the index over exactly the text that was read) and opened, editable and without an error.
#[gpui::test]
fn todo_start_a_new_index_button_writes_and_opens_a_fresh_index(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let unreadable = "My lists:\nTasks, Work\n";
    let host = todo_host(&[(TODO_LISTS, unreadable)], &[]);
    let (mut visual, _runtime, _shell) = mount_todo(cx, &staged, &host);
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!((state["badIndex"].clone(), state["ready"].clone()), (json!(true), json!(false)), "{state}");
    assert!(host.borrow().writes.is_empty(), "nothing is written before the click");
    todo_click_right_column(&mut visual, || host.borrow().writes.iter().any(|(p, _, _)| p == TODO_LISTS));
    files_wait(&mut visual, 100);
    {
        let h = host.borrow();
        assert_eq!(h.files[TODO_LISTS], "- [Tasks](tasks.md)\n");
        assert_eq!(h.files[TODO_TASKS], "# Tasks\n");
        let index: Vec<_> = h.writes.iter().filter(|(p, _, _)| p == TODO_LISTS).collect();
        assert_eq!(index.len(), 1, "{:?}", h.writes);
        assert_eq!(index[0].2, json!(unreadable), "the new index replaces exactly what was read");
    }
    let state = todo_mounted_state(&mut visual, &host);
    assert_eq!(state["error"], Value::Null, "{state}");
    assert_eq!(
        (state["badIndex"].clone(), state["cur"].clone(), state["readonly"].clone(), state["ready"].clone()),
        (json!(false), json!("tasks"), json!(false), json!(true)),
        "{state}"
    );
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

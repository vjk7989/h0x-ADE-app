//! What runs in a terminal, as people see it: a name, an icon and a tile.
//! One table for every mod: `defaults/programs.json` (built in), with the
//! person's `programs.json` in the home folder on top (entry by entry). The
//! shipped icons are written to `icons/` in the home folder (the app's;
//! rewritten when they change). Mods read the result as `program` on
//! panes, agents and terminals.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;

const BUILT_IN: &str = include_str!("../defaults/programs.json");
include!(concat!(env!("OUT_DIR"), "/default_icons.rs"));

#[derive(Deserialize, Default)]
struct Table {
    #[serde(default)]
    shells: Vec<String>,
    #[serde(default)]
    interpreters: Vec<String>,
    #[serde(default)]
    programs: BTreeMap<String, Entry>,
    /// Keys only the person's `programs.json` has (not in the built-in table): their own agents.
    #[serde(skip)]
    custom: std::collections::BTreeSet<String>,
}

#[derive(Deserialize, Serialize, Default, Clone)]
struct Entry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// A path in the home folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    /// A one-color glyph, drawn in `color` (or the text color); else a picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    /// Other process names (or agents' 7501 app names) for this entry.
    #[serde(default, rename = "match", skip_serializing_if = "Vec::is_empty")]
    also: Vec<String>,
    /// How to start it, for an agent people can start from a menu (`knownAgents()`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    run: Vec<String>,
    /// Where one of the person's own agents sits among the others in the number-key slots (lower first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    order: Option<i64>,
}

impl Entry {
    fn over(&mut self, mine: Entry) {
        self.name = mine.name.or(self.name.take());
        self.icon = mine.icon.or(self.icon.take());
        self.tint = mine.tint.or(self.tint);
        self.tile = mine.tile.or(self.tile.take());
        self.color = mine.color.or(self.color.take());
        if !mine.also.is_empty() {
            self.also = mine.also;
        }
        if !mine.run.is_empty() {
            self.run = mine.run;
        }
        self.order = mine.order.or(self.order);
    }
}

/// The built-in agents that hold slots 1-4, in order; the person's own follow.
const SLOT_AGENTS: &[&str] = &["claude", "codex", "opencode", "gemini"];
/// `agent.new` and its keys count slots 1 to this.
const SLOTS: usize = 9;

/// The keys of the agents in the number-key slots (slot 1 first): the fixed ones that can start,
/// then the person's own, by `order` and then key. At most `SLOTS`.
fn slot_keys(t: &Table) -> Vec<String> {
    let runs = |k: &str| t.programs.get(k).is_some_and(|e| !e.run.is_empty());
    let mut keys: Vec<String> = SLOT_AGENTS.iter().filter(|k| runs(k)).map(|k| k.to_string()).collect();
    let mut own: Vec<&String> = t.custom.iter().filter(|k| runs(k) && !SLOT_AGENTS.contains(&k.as_str())).collect();
    own.sort_by_key(|k| (t.programs[*k].order.unwrap_or(i64::MAX), k.to_string()));
    keys.extend(own.into_iter().cloned());
    keys.truncate(SLOTS);
    keys
}

/// The slot (1-9) `agent.new` was asked for: a whole number, or digits as text.
pub fn slot_number(index: Option<&Value>) -> Result<usize, String> {
    let n = match index {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    };
    n.filter(|n| (1..=SLOTS as u64).contains(n)).map(|n| n as usize).ok_or_else(|| format!("agent.new needs index: a slot from 1 to {SLOTS}"))
}

/// The agents people pick most, first in menus; the rest follow by name.
const AGENTS_FIRST: &[&str] = &["claude", "codex", "pi", "opencode", "cursor", "copilot", "amp", "gemini", "grok"];

/// Every agent the table can start (entries with `run`), most picked first:
/// `{ key, name, icon, tint, tile, color, run, slot, custom }`.
pub fn startable() -> Vec<Value> {
    TABLE.with(|t| {
        let t = t.borrow();
        let mut keys: Vec<&String> = t.programs.iter().filter(|(_, e)| !e.run.is_empty()).map(|(k, _)| k).collect();
        keys.sort_by_key(|k| (AGENTS_FIRST.iter().position(|f| f == k).unwrap_or(AGENTS_FIRST.len()), k.to_string()));
        keys.into_iter().map(|k| startable_entry(&t, k)).collect()
    })
}

/// The agents holding the number-key slots, slot 1 first, shaped as `startable()` entries.
pub fn slots() -> Vec<Value> {
    TABLE.with(|t| {
        let t = t.borrow();
        slot_keys(&t).iter().map(|k| startable_entry(&t, k)).collect()
    })
}

/// `{ key, name, icon, tint, tile, color, run, slot, custom }`: `slot` is 1-9 or null.
fn startable_entry(t: &Table, key: &str) -> Value {
    let e = &t.programs[key];
    let mut v = shown(key, e, None, false, true);
    v["run"] = json!(e.run);
    v["slot"] = slot_keys(t).iter().position(|k| k == key).map_or(Value::Null, |i| json!(i + 1));
    v["custom"] = json!(t.custom.contains(key));
    v
}

/// Which of `commands` this machine can start, asked of the shell agents start
/// in (so an app opened from the Dock finds what the person's terminal finds).
/// On another machine (`on`): its login/interactive shell, as remote agents start.
pub fn installed(commands: &[String], on: Option<&str>) -> Vec<String> {
    if commands.is_empty() {
        return Vec::new();
    }
    // Each found one as a marked line: an interactive shell may print its own
    // things (a prompt's escapes, a greeting) around the answer.
    let out = match on {
        Some(m) => {
            let argv = crate::remote_shell::check(commands);
            let result = match crate::mods::run_remote(m, &argv[0], &argv[1..], None) {
                Err(e) if e.contains("cannot start sh: program not found") => {
                    crate::mods::run_remote(m, "powershell.exe", &installed_powershell(commands), None)
                }
                r => r,
            };
            result.map(|(_, out, _)| String::from_utf8_lossy(&out).into_owned()).unwrap_or_default()
        }
        None if cfg!(windows) => {
            // The core refreshes user/system PATH; the app may have inherited only the system PATH.
            let mut args = vec!["exec".into(), "--".into(), crate::window::shell()];
            args.extend(installed_powershell(commands));
            crate::mods::run_here(&crate::core_bin::path().to_string_lossy(), &args).unwrap_or_default()
        }
        None => {
            let shell = crate::window::shell();
            let line = installed_line(commands, base(&shell) == "fish");
            crate::mods::run_here(&shell, &["-ic".into(), line]).unwrap_or_default()
        }
    };
    let found: std::collections::HashSet<&str> = out.split("h0x-has:").skip(1).filter_map(|s| s.split_whitespace().next()).collect();
    commands.iter().filter(|c| found.contains(c.as_str())).cloned().collect()
}

fn installed_powershell(commands: &[String]) -> Vec<String> {
    let list = commands.iter().map(|c| powershell_quote(c)).collect::<Vec<_>>().join(",");
    let script = format!(
        "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); foreach($c in @({list})){{ if(Get-Command $c -ErrorAction SilentlyContinue){{ \"h0x-has:$c\" }} }}"
    );
    vec!["-NoLogo".into(), "-Command".into(), script]
}

/// Check in the shell itself for functions, but not aliases (`"$@"` cannot run them).
fn installed_line(commands: &[String], fish: bool) -> String {
    commands
        .iter()
        .map(|c| {
            let word = format!("'{}'", c.replace('\'', "'\\''"));
            let unalias = if fish { String::new() } else { format!("unalias -- {word} 2>/dev/null; ") };
            format!("{unalias}command -v {word} >/dev/null 2>&1 && printf 'h0x-has:%s\\n' {word};")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

thread_local! {
    static TABLE: RefCell<Table> = RefCell::new(serde_json::from_str(BUILT_IN).expect("defaults/programs.json"));
}

pub fn path() -> std::path::PathBuf {
    crate::mods::app_dir().join("programs.json")
}

/// Writes the shipped icons into the home folder.
pub fn install_icons() {
    let dir = crate::mods::app_dir().join("icons");
    let _ = std::fs::create_dir_all(&dir);
    for (name, bytes) in DEFAULT_ICONS {
        let file = dir.join(name);
        if std::fs::read(&file).ok().as_deref() != Some(*bytes) {
            let _ = crate::persist::write_atomic(&file, bytes);
        }
    }
}

/// Reads the person's table on top of the built-in one. Err: what's wrong with their file.
pub fn load() -> Result<(), String> {
    let mut table: Table = serde_json::from_str(BUILT_IN).expect("defaults/programs.json");
    let result = match std::fs::read_to_string(path()) {
        Err(_) => Ok(()),
        Ok(text) => match serde_json::from_str::<Table>(&text) {
            Err(e) => Err(format!("programs.json: {e}")),
            Ok(mine) => {
                merge(&mut table, mine);
                Ok(())
            }
        },
    };
    TABLE.with(|t| *t.borrow_mut() = table);
    result
}

/// The person's table on top of the built-in one, entry by entry.
fn merge(table: &mut Table, mine: Table) {
    table.shells.extend(mine.shells);
    table.interpreters.extend(mine.interpreters);
    for (key, entry) in mine.programs {
        if !table.programs.contains_key(&key) {
            table.custom.insert(key.clone());
        }
        table.programs.entry(key).or_default().over(entry);
    }
}

/// How a terminal shows: from the agent's own report (`app`), else the
/// program it runs (`argv`). `{ key, name, icon, tint, tile, color, shell, agent }`.
pub fn of(app: Option<&str>, argv: &[String]) -> Value {
    TABLE.with(|t| resolve(&t.borrow(), app, argv))
}

/// The agent `argv` starts (its key; an entry people can start), else none.
pub fn agent(argv: &[String]) -> Option<String> {
    TABLE.with(|t| {
        let t = t.borrow();
        let key = resolve(&t, None, argv)["key"].as_str()?.to_owned();
        t.programs.get(&key).filter(|e| !e.run.is_empty()).map(|_| key)
    })
}

/// How the app calls an agent ("Kiro finished"), from its 7501 `app`: the
/// name `of` shows for it (the person's `programs.json` included), else the `app`.
pub fn name(app: &str) -> String {
    TABLE.with(|t| find(&t.borrow(), app).map(|(key, e)| e.name.clone().unwrap_or_else(|| key.to_owned()))).unwrap_or_else(|| app.to_owned())
}

fn base(s: &str) -> &str {
    let s = s.rsplit(['/', '\\']).next().unwrap_or(s);
    let s = s.trim_start_matches('-');
    // Windows: `pwsh.exe` is `pwsh`.
    match s.len().checked_sub(4) {
        Some(i) if s.is_char_boundary(i) && s[i..].eq_ignore_ascii_case(".exe") => &s[..i],
        _ => s,
    }
}

/// PowerShell's single quotes, which include the curly ones.
const PS_QUOTES: [char; 5] = ['\'', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'];

/// What `powershell_line` puts before the program.
const PS_START: &str = "& { $PSNativeCommandArgumentPassing = 'Legacy'; $p = ";
/// What `powershell_line` puts after the arguments: `.cmd`/`.bat` targets run in
/// cmd.exe, which ends a command line at the first line break.
const PS_END: &str = r"); if ((Get-Command $p -ErrorAction Ignore | Select-Object -First 1).Extension -in '.cmd', '.bat') { $a = $a -replace '\r?\n', ' ' }; & $p @a }";

/// A command for PowerShell's `-Command`: `prog` then its arguments, each word
/// quoted. PowerShell hands a native program its arguments joined into one
/// command line, and in Windows PowerShell (and pwsh's `Legacy` mode, set here
/// so every version behaves alike) it does not escape an inner `"`: so each
/// argument goes in already quoted for that command line.
pub fn powershell_line(cmd: &[String]) -> String {
    let (prog, args) = cmd.split_first().map_or(("", &[][..]), |(p, a)| (p.as_str(), a));
    let args = args.iter().map(|a| powershell_quote(&windows_arg(a))).collect::<Vec<_>>().join(", ");
    format!("{PS_START}{}; $a = @({args}{PS_END}", powershell_quote(prog))
}

/// One argument of a Windows command line, as C runtimes since 2008 read it.
/// A `"` inside quotes is written `""`, which keeps cmd.exe in quotes (`&`,
/// `|`, `>` stay text) and Windows PowerShell from quoting it again.
fn windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{0B}', '"']) {
        return arg.to_owned();
    }
    let mut s = String::from('"');
    let mut slashes = 0;
    for ch in arg.chars() {
        match ch {
            '\\' => {
                slashes += 1;
                continue;
            }
            // Backslashes before a quote are doubled.
            '"' => s.extend(std::iter::repeat_n('\\', 2 * slashes).chain(['"', '"'])),
            _ => s.extend(std::iter::repeat_n('\\', slashes).chain([ch])),
        }
        slashes = 0;
    }
    s.extend(std::iter::repeat_n('\\', 2 * slashes));
    s + "\""
}

/// The argument back from `windows_arg`.
fn windows_unquote(arg: &str) -> String {
    let (mut s, mut quoted, mut slashes) = (String::new(), false, 0);
    let mut chars = arg.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        if ch == '"' {
            s.extend(std::iter::repeat_n('\\', slashes / 2));
            if slashes % 2 == 1 {
                s.push('"');
            } else if quoted && chars.peek() == Some(&'"') {
                s.push(chars.next().unwrap_or(ch));
            } else {
                quoted = !quoted;
            }
        } else {
            s.extend(std::iter::repeat_n('\\', slashes).chain([ch]));
        }
        slashes = 0;
    }
    s.extend(std::iter::repeat_n('\\', slashes));
    s
}

pub(crate) fn powershell_quote(word: &str) -> String {
    let mut s = String::from("'");
    for ch in word.chars() {
        if PS_QUOTES.contains(&ch) {
            s.push(ch);
        }
        s.push(ch);
    }
    s + "'"
}

/// The words back from `powershell_line`.
fn powershell_words(line: &str) -> Option<Vec<String>> {
    let rest = line.strip_prefix(PS_START)?.strip_suffix(PS_END)?;
    let (prog, rest) = powershell_word(rest)?;
    let mut rest = rest.strip_prefix("; $a = @(")?;
    let mut words = vec![prog];
    while !rest.is_empty() {
        if words.len() > 1 {
            rest = rest.strip_prefix(", ")?;
        }
        let (arg, after) = powershell_word(rest)?;
        words.push(windows_unquote(&arg));
        rest = after;
    }
    Some(words)
}

/// The quoted word `s` starts with, and what follows it.
fn powershell_word(s: &str) -> Option<(String, &str)> {
    let mut chars = s.char_indices().peekable();
    if !chars.next().is_some_and(|(_, c)| PS_QUOTES.contains(&c)) {
        return None;
    }
    let mut w = String::new();
    while let Some((i, ch)) = chars.next() {
        if PS_QUOTES.contains(&ch) {
            match chars.next_if(|(_, n)| PS_QUOTES.contains(n)) {
                Some((_, n)) => w.push(n),
                None => return Some((w, &s[i + ch.len_utf8()..])),
            }
            continue;
        }
        w.push(ch);
    }
    None
}

fn find<'a>(t: &'a Table, name: &str) -> Option<(&'a str, &'a Entry)> {
    let lower = name.to_lowercase();
    t.programs
        .iter()
        .find(|(k, _)| k.as_str() == name || k.to_lowercase() == lower)
        .or_else(|| t.programs.iter().find(|(_, e)| e.also.iter().any(|a| a == name || a.to_lowercase() == lower)))
        .map(|(k, e)| (k.as_str(), e))
}

fn resolve(t: &Table, app: Option<&str>, argv: &[String]) -> Value {
    if let Some(app) = app.filter(|a| !a.is_empty()) {
        return match find(t, app) {
            Some((key, e)) => shown(key, e, None, false, true),
            None => shown("agent", t.programs.get("agent").unwrap_or(&Entry::default()), Some(app), false, true),
        };
    }
    // The app starts commands in the person's shell: `zsh -ic '"$@"; exec "$0" -i' zsh <cmd>...`.
    if argv.len() > 4
        && t.shells.iter().any(|s| s == base(&argv[0]))
        && argv[1].starts_with('-')
        && argv[1].ends_with('c')
        && (argv[2].contains("\"$@\"") || argv[2] == "command $argv[2..]; exec $argv[1] -i")
    {
        return resolve(t, None, &argv[4..]);
    }
    // On Windows: `pwsh -NoExit -Command <powershell_line>`.
    if argv.len() == 4
        && t.shells.iter().any(|s| s == base(&argv[0]))
        && argv[1] == "-NoExit"
        && argv[2] == "-Command"
        && let Some(words) = powershell_words(&argv[3])
    {
        return resolve(t, None, &words);
    }
    let Some(first) = argv.first().map(|a| base(a)) else {
        return shown("shell", t.programs.get("shell").unwrap_or(&Entry::default()), None, true, false);
    };
    // `node .../vite.js dev`, `npx vite`, `uv run pytest`: the program they run, when it's known.
    if t.interpreters.iter().any(|i| i == first) {
        for arg in argv[1..].iter().filter(|a| !a.starts_with('-')) {
            // A script's name without its extension (`vite.js`), never a module path (`http.server`).
            let name = base(arg);
            let stem = [".js", ".mjs", ".cjs", ".ts", ".py", ".rb"].iter().find_map(|x| name.strip_suffix(x)).unwrap_or(name);
            if let Some((key, e)) = find(t, stem).filter(|(k, _)| !t.interpreters.iter().any(|i| i == k)) {
                return shown(key, e, None, false, false);
            }
        }
    }
    if let Some((key, e)) = find(t, first) {
        return shown(key, e, None, false, false);
    }
    if t.shells.iter().any(|s| s == first) {
        return shown("shell", t.programs.get("shell").unwrap_or(&Entry::default()), Some(first), true, false);
    }
    let mut e = t.programs.get("program").cloned().unwrap_or_default();
    e.tile = Some(hash_color(first).into());
    e.color = Some("#ffffff".into());
    shown("program", &e, Some(first), false, false)
}

fn shown(key: &str, e: &Entry, name: Option<&str>, shell: bool, agent: bool) -> Value {
    json!({
        "key": key,
        "name": name.map(str::to_owned).or_else(|| e.name.clone()).unwrap_or_else(|| key.to_owned()),
        "icon": e.icon,
        "tint": e.tint.unwrap_or(true),
        "tile": e.tile,
        "color": e.color,
        "shell": shell,
        "agent": agent,
    })
}

/// A color for a program nobody listed, the same every time.
fn hash_color(s: &str) -> &'static str {
    const PALETTE: [&str; 8] = ["#6d5bd0", "#2f80ed", "#0f9d8a", "#d9822b", "#c2417d", "#3a7d44", "#8a5a44", "#4e5d94"];
    let h = s.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    PALETTE[(h % PALETTE.len() as u32) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Table {
        serde_json::from_str(BUILT_IN).unwrap()
    }
    fn key(app: Option<&str>, argv: &[&str]) -> String {
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        resolve(&t(), app, &argv)["key"].as_str().unwrap().to_owned()
    }

    fn with_mine(mine: &str) -> Table {
        let mut table = t();
        merge(&mut table, serde_json::from_str(mine).unwrap());
        table
    }

    #[test]
    fn slots_are_the_four_agents_then_the_persons_own() {
        assert_eq!(slot_keys(&t()), ["claude", "codex", "opencode", "gemini"]);
        let table = with_mine(r#"{ "programs": { "zed": { "run": ["zed-agent"] }, "aider": { "run": ["aider", "--yes"] }, "idle": { "name": "No run" } } }"#);
        assert_eq!(slot_keys(&table), ["claude", "codex", "opencode", "gemini", "aider", "zed"], "customs by key; no run: no slot");
        let entries: Vec<Value> = slot_keys(&table).iter().map(|k| startable_entry(&table, k)).collect();
        assert_eq!(entries[0]["slot"], json!(1));
        assert_eq!(entries[0]["custom"], json!(false));
        assert_eq!(entries[4]["slot"], json!(5));
        assert_eq!(entries[4]["custom"], json!(true));
        assert_eq!(entries[4]["run"], json!(["aider", "--yes"]));
        // An agent outside the slots still lists, with no slot.
        assert_eq!(startable_entry(&table, "pi")["slot"], Value::Null);
        assert_eq!(startable_entry(&table, "pi")["custom"], json!(false));
    }

    #[test]
    fn custom_agents_sort_by_order_then_key_and_stop_at_nine() {
        let table = with_mine(r#"{ "programs": {
            "c": { "run": ["c"], "order": 2 }, "b": { "run": ["b"], "order": 2 }, "a": { "run": ["a"] },
            "first": { "run": ["f"], "order": -5 }, "neg": { "run": ["n"], "order": 1 }
        } }"#);
        assert_eq!(slot_keys(&table), ["claude", "codex", "opencode", "gemini", "first", "neg", "b", "c", "a"], "ties by key; no order last");
        let many: Vec<String> = (0..20).map(|i| format!(r#""x{i:02}": {{ "run": ["x{i}"] }}"#)).collect();
        let table = with_mine(&format!(r#"{{ "programs": {{ {} }} }}"#, many.join(",")));
        let keys = slot_keys(&table);
        assert_eq!(keys.len(), 9, "at most nine");
        assert_eq!(&keys[4..], ["x00", "x01", "x02", "x03", "x04"]);
    }

    #[test]
    fn overriding_a_built_in_agent_keeps_its_slot_and_is_not_custom() {
        let table = with_mine(r#"{ "programs": { "codex": { "run": ["codex", "--fast"], "order": 1 }, "claude": { "name": "Mine" } } }"#);
        assert_eq!(slot_keys(&table), ["claude", "codex", "opencode", "gemini"]);
        let codex = startable_entry(&table, "codex");
        assert_eq!(codex["run"], json!(["codex", "--fast"]));
        assert_eq!(codex["custom"], json!(false));
        assert_eq!(codex["slot"], json!(2));
        // A mine-only entry whose run is removed again by an empty list keeps nothing to start.
        let table = with_mine(r#"{ "programs": { "gone": { "name": "x" } } }"#);
        assert_eq!(slot_keys(&table).len(), 4);
    }

    #[test]
    fn slot_numbers_are_one_to_nine() {
        for (given, want) in [(json!(1), 1), (json!(9), 9), (json!("3"), 3), (json!(" 4 "), 4)] {
            assert_eq!(slot_number(Some(&given)), Ok(want), "{given}");
        }
        for bad in [json!(0), json!(10), json!(-1), json!(1.5), json!("x"), json!(""), json!(null), json!([1]), json!(true)] {
            assert!(slot_number(Some(&bad)).is_err(), "{bad}");
        }
        assert!(slot_number(None).is_err());
    }

    #[test]
    fn powershell_words_survive_quotes() {
        let words: Vec<String> = ["pi", "it's \u{2019}fine\u{2019}", "", "a  b"].map(String::from).to_vec();
        let line = powershell_line(&words);
        assert_eq!(line, format!("{PS_START}'pi'; $a = @('\"it''s \u{2019}\u{2019}fine\u{2019}\u{2019}\"', '\"\"', '\"a  b\"'{PS_END}"));
        assert_eq!(powershell_words(&line), Some(words));
        assert_eq!(powershell_line(&["pi".into()]), format!("{PS_START}'pi'; $a = @({PS_END}"));
        assert_eq!(powershell_words(&powershell_line(&["pi".into()])), Some(vec!["pi".to_owned()]));
        assert_eq!(powershell_words("& 'pi' 'x'"), None);
    }

    /// Tasks given to an agent at its start: quotes, line breaks and PowerShell
    /// syntax are text, never code.
    const TASKS: [&str; 12] = [
        "a \"- [ ] \" b",
        "Fix these:\n- [ ] \"one\"\r\n- [ ] two\n",
        "it's \u{2018}curly\u{2019} \u{201C}double\u{201D} $(x) `$y $env:PATH %PATH% & calc | x > y; exit",
        "a trailing backslash \\",
        "C:\\dir\\",
        "a\\\"b c \\\\\"d\"",
        "\"",
        "\"\"",
        "'",
        "$(x)",
        "-",
        "",
    ];

    #[test]
    fn powershell_line_quotes_arguments_for_the_windows_command_line() {
        // An inner `"` is `""` inside quotes: Windows PowerShell passes it unchanged.
        assert_eq!(powershell_line(&["claude".into(), TASKS[0].into()]), format!("{PS_START}'claude'; $a = @('\"a \"\"- [ ] \"\" b\"'{PS_END}"));
        for (arg, quoted) in [
            ("plain", "plain"),
            ("", "\"\""),
            ("\"", "\"\"\"\""),
            ("a\nb", "\"a\nb\""),
            ("$(x)", "$(x)"),
            ("C:\\dir\\", "C:\\dir\\"),
            ("a trailing backslash \\", "\"a trailing backslash \\\\\""),
            ("a\\\"b c", "\"a\\\\\"\"b c\""),
            ("C:\\a b\\x", "\"C:\\a b\\x\""),
        ] {
            assert_eq!(windows_arg(arg), quoted, "{arg:?}");
        }
        for task in TASKS {
            assert_eq!(windows_unquote(&windows_arg(task)), task, "{task:?}");
            let cmd = vec!["claude".to_owned(), "--flag".to_owned(), task.to_owned()];
            let line = powershell_line(&cmd);
            assert_eq!(powershell_words(&line), Some(cmd), "{line}");
        }
        // PowerShell code in a task stays inside single quotes.
        assert!(powershell_line(&["claude".into(), "$(x)".into()]).contains("@('$(x)')"));
        assert!(powershell_line(&["claude".into(), "it's".into()]).contains("@('it''s')"));
    }

    /// Runs PowerShell on `powershell_line` for this test binary (filtered to
    /// this test) with every task: the child checks each arrives intact.
    #[cfg(windows)]
    #[test]
    fn powershell_line_reaches_a_native_program_intact() {
        const NAME: &str = "programs::tests::powershell_line_reaches_a_native_program_intact";
        let want = serde_json::to_string(&TASKS).unwrap();
        if let Ok(sent) = std::env::var("H0X_ADE_TEST_ARGS") {
            // The child: after `--` come this test's name, then the tasks.
            let got: Vec<String> = std::env::args().skip_while(|a| a != "--").skip(2).collect();
            assert_eq!(serde_json::to_string(&got).unwrap(), sent);
            return;
        }
        let mut cmd: Vec<String> = [&*std::env::current_exe().unwrap().to_string_lossy(), "--exact", "--nocapture", "--test-threads=1", "--", NAME].map(String::from).to_vec();
        cmd.extend(TASKS.map(String::from));
        let out = std::process::Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", &powershell_line(&cmd)])
            .env("H0X_ADE_TEST_ARGS", &want)
            .output()
            .unwrap();
        let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success() && log.contains("1 passed"), "{log}");
    }

    #[test]
    fn powershell_checks_keep_command_names_literal() {
        let argv = installed_powershell(&["claude".into(), "missing'command; $x".into(), "O’Brien".into()]);
        assert_eq!(argv[..2], ["-NoLogo", "-Command"]);
        assert!(argv[2].contains("@('claude','missing''command; $x','O’’Brien')"));
        assert!(argv[2].contains("[Console]::OutputEncoding"));
    }

    #[test]
    fn powershell_quote_doubles_straight_and_curly_quotes() {
        for (word, quoted) in [
            ("", "''"),
            ("claude", "'claude'"),
            ("'", "''''"),
            ("\u{2019}", "'\u{2019}\u{2019}'"),
            ("\u{2018}x\u{2019}", "'\u{2018}\u{2018}x\u{2019}\u{2019}'"),
            ("\u{201A}\u{201B}", "'\u{201A}\u{201A}\u{201B}\u{201B}'"),
            ("'\u{2019}", "'''\u{2019}\u{2019}'"),
            // Single-quoted PowerShell strings are verbatim: no expansion, escapes or line ends.
            ("$(calc)", "'$(calc)'"),
            ("`n`$x", "'`n`$x'"),
            ("a\nb", "'a\nb'"),
            ("\"x\"", "'\"x\"'"),
            (r"C:\Program Files\a b.exe", r"'C:\Program Files\a b.exe'"),
        ] {
            assert_eq!(powershell_quote(word), quoted, "{word:?}");
            assert_eq!(powershell_words(&powershell_line(&[word.to_owned()])), Some(vec![word.to_owned()]), "{word:?}");
        }
    }

    #[test]
    fn remote_agent_checks_pass_names_as_arguments_not_source() {
        assert!(installed(&[], Some("remote-host")).is_empty());
        let commands = ["claude".to_owned(), "x'; rm -rf ~".to_owned(), r"C:\Tools\a b.exe".to_owned()];
        let argv = crate::remote_shell::check(&commands);
        assert_eq!(argv[..2], ["sh", "-c"]);
        assert_eq!(argv[3], "sh");
        assert_eq!(argv[4..], commands);
        assert!(!argv[2].contains("rm -rf") && !argv[2].contains("Tools"));
        // The fallback for a remote Windows machine without sh: names stay quoted literals.
        let fallback = installed_powershell(&commands);
        assert!(fallback[2].contains(r"@('claude','x''; rm -rf ~','C:\Tools\a b.exe')"), "{}", fallback[2]);
    }

    #[cfg(windows)]
    #[test]
    fn powershell_finds_installed_commands() {
        let out = crate::mods::run_here("powershell.exe", &installed_powershell(&["cmd.exe".into(), "h0x_missing_agent_440".into()])).unwrap();
        assert!(out.contains("h0x-has:cmd.exe"));
        assert!(!out.contains("h0x-has:h0x_missing_agent_440"));
    }

    #[test]
    fn agents_shells_tools_and_what_interpreters_run() {
        assert_eq!(key(Some("claude-code"), &["/bin/zsh"]), "claude");
        assert_eq!(key(Some("grok-build"), &["/bin/zsh"]), "grok");
        assert_eq!(key(Some("dsh-tui"), &["/bin/zsh"]), "dsh-tui");
        assert_eq!(key(None, &["dsh-tui"]), "dsh-tui");
        assert_eq!(key(Some("newagent"), &[]), "agent");
        assert_eq!(key(None, &["/bin/zsh"]), "shell");
        assert_eq!(key(None, &["-zsh"]), "shell");
        assert_eq!(key(None, &["nvim", "x.rs"]), "nvim");
        assert_eq!(key(None, &["vi"]), "vim");
        assert_eq!(key(None, &["node", "/x/node_modules/.bin/vite", "dev"]), "vite");
        assert_eq!(key(None, &["npx", "vitest"]), "vitest");
        assert_eq!(key(None, &["node", "server.js"]), "node");
        assert_eq!(key(None, &["python3", "-m", "http.server", "8799"]), "python");
        assert_eq!(key(None, &["uv", "run", "pytest"]), "pytest");
        assert_eq!(key(None, &["mytool"]), "program");
        assert_eq!(key(None, &["/bin/zsh", "-ic", "\"$@\"; exec \"$0\" -i", "/bin/zsh", "vi", "x"]), "vim");
        assert_eq!(key(None, &["/usr/bin/fish", "-ic", "command $argv[2..]; exec $argv[1] -i", "/usr/bin/fish", "vi", "x"]), "vim");
        let line = powershell_line(&["codex".into(), "fix it".into()]);
        assert_eq!(key(None, &["pwsh.exe", "-NoExit", "-Command", &line]), "codex");
        assert_eq!(key(None, &[r"C:\Windows\powershell.exe"]), "shell");
    }

    #[test]
    fn kimi_code_names_map_to_kimi_and_near_misses_do_not() {
        for app in ["kimi", "kimi-code", "kimicodecli", "Kimi-Code", "KIMICODECLI"] {
            assert_eq!(key(Some(app), &["/bin/zsh"]), "kimi", "{app}");
        }
        assert_eq!(key(None, &["kimi-code", "--resume"]), "kimi");
        assert_eq!(key(None, &["/usr/local/bin/kimicodecli"]), "kimi");
        assert_eq!(key(None, &[r"C:\Users\me\.local\bin\kimi-code.exe"]), "kimi");
        assert_eq!(key(Some("kimi-coder"), &[]), "agent");
        assert_eq!(key(None, &["kimicode"]), "program");
        // An empty app name is no agent name: the program decides.
        assert_eq!(key(Some(""), &["kimi-code"]), "kimi");
    }

    #[test]
    fn no_commands_checks_nothing_on_this_machine() {
        assert!(installed(&[], None).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn installed_checks_functions_and_quoted_names_but_not_aliases() {
        let commands = ["issue57_function", "issue57_alias", "printf", "missing'command; false"].map(String::from);
        let line = installed_line(&commands, false);
        let out = std::process::Command::new("sh")
            .args(["-c", &format!("issue57_function() {{ :; }}; alias issue57_alias='printf alias'; {line}")])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(out.stdout).unwrap(), "h0x-has:issue57_function\nh0x-has:printf\n");
    }

    #[test]
    fn every_entry_has_its_icon() {
        for (k, e) in t().programs {
            let icon = e.icon.unwrap_or_else(|| panic!("{k} has no icon"));
            let file = icon.strip_prefix("icons/").unwrap();
            assert!(DEFAULT_ICONS.iter().any(|(n, _)| *n == file), "{k}: {icon} not shipped");
        }
    }
}

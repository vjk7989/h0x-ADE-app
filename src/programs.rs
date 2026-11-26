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
        None => return installed_here(&crate::window::shell(), commands),
    };
    installed_found(commands, &out)
}

/// The check in the person's shell; a shell that can't run it (nushell) leaves no
/// completion mark, so sh searches the app's PATH instead, which is where agents for
/// such shells start (`window::unix_agent_argv`).
fn installed_here(shell: &str, commands: &[String]) -> Vec<String> {
    let mark = "printf 'h0x-check-complete\\n'";
    let run = |shell: &str, args: [String; 2]| crate::mods::run_here(shell, &args).filter(|out| out.lines().any(|l| l == "h0x-check-complete"));
    let out = run(shell, ["-ic".into(), format!("{} {mark}", installed_line(commands, base(shell) == "fish"))])
        .or_else(|| run("/bin/sh", ["-c".into(), format!("{} {mark}", installed_line(commands, false))]))
        .unwrap_or_default();
    installed_found(commands, &out)
}

fn installed_found(commands: &[String], out: &str) -> Vec<String> {
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

/// The `H0X_TOKEN_SAVER` value (`1` on, `0` off) that the launch of `argv` carries in its
/// environment: only a Claude Code start. The core's `claude` shim adds the hooks
/// (`--settings`) itself, unless the person gave their own (ADR-129).
pub fn token_saver_env(on: bool, argv: &[String]) -> Option<&'static str> {
    starts_claude(argv).then_some(if on { "1" } else { "0" })
}

pub(crate) fn starts_claude(argv: &[String]) -> bool {
    argv.first().is_some_and(|a| base(a).eq_ignore_ascii_case("claude")) && agent(argv).as_deref() == Some("claude")
}

/// A random UUID v4 for one Claude launch. Not a secret: it only names the log file.
fn new_session_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // `RandomState` is seeded randomly per process; two keyed hashes give 128 bits.
    let part = |salt: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(nanos);
        h.write_u64(n ^ salt);
        h.write_u32(std::process::id());
        h.finish()
    };
    let (hi, lo) = (part(1), part(2));
    let hi = (hi & !0xF000) | 0x4000; // version 4
    let lo = (lo & !(0b11 << 62)) | (0b10 << 62); // variant 10
    format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", hi >> 32, (hi >> 16) & 0xFFFF, hi & 0xFFFF, lo >> 48, lo & 0xFFFF_FFFF_FFFF)
}

const SESSION_FLAGS: [&str; 2] = ["--session-id", "--resume"];

/// `claude --session-id <new uuid> <the person's args>`, so HyperOx knows which log is its
/// session (agent_usage reads that one only). A Claude that already chose its session
/// (`--session-id`, `--resume`/`-r`, `--continue`/`-c`, `--from-pr`) stays exactly as asked;
/// `session_of` then reads the explicit id, if any. Anything else stays as asked.
pub fn with_session_id(mut argv: Vec<String>) -> Vec<String> {
    if starts_claude(&argv) && !gave(&argv, &["--session-id", "--resume", "-r", "--continue", "-c", "--from-pr"]) {
        argv.splice(1..1, ["--session-id".to_owned(), new_session_id()]);
    }
    argv
}

/// The explicit Claude session id in `argv` (`--session-id`, `--resume`, `-r`; as `flag id` or
/// `flag=id`), when it is a valid UUID. A `--continue`, a search term or no flag: none, so
/// nothing is recorded for that terminal.
pub fn session_of(argv: &[String]) -> Option<String> {
    // A fork writes to a new id we do not know: record nothing, never the old log.
    if !starts_claude(argv) || argv.iter().any(|a| a == "--fork-session") {
        return None;
    }
    let mut found = None;
    let mut it = argv.iter().skip(1);
    while let Some(a) = it.next() {
        let value = if SESSION_FLAGS.contains(&a.as_str()) || a == "-r" {
            it.next().map(String::as_str)
        } else {
            SESSION_FLAGS.iter().find_map(|f| a.strip_prefix(f).and_then(|r| r.strip_prefix('=')))
        };
        // Claude uses the last one given.
        if let Some(v) = value.filter(|v| crate::agent_usage::valid_session_id(v)) {
            found = Some(v.to_owned());
        }
    }
    found
}

/// The core "route" output for a session: which tools it needs, whether the code
/// map helps, and a cheaper model for read-only work. Other fields are ignored.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Route {
    pub graft: bool,
    pub tools: Vec<String>,
    pub model: Option<String>,
}

/// Tells the agent about the code map, which the core binary serves.
const MAP_HINT: &str = r#"Code map: `"$H0X_CORE_EXE" map` (Bash), use before reading code.
- `"$H0X_CORE_EXE" map callers <sym>`: callers with file:line and enclosing fn
- `"$H0X_CORE_EXE" map grep <word>`: hits by enclosing symbol
Then Read only the needed lines (offset/limit)."#;

/// Whether the person already gave a flag, as `--flag value` or `--flag=value`.
fn gave(argv: &[String], flags: &[&str]) -> bool {
    argv.iter().any(|a| flags.iter().any(|f| a == f || a.strip_prefix(f).is_some_and(|r| r.starts_with('='))))
}
/// `argv` for a Claude Code terminal, with the session's `route` applied: `--tools` (never
/// `--allowedTools`: permission prompts stay), `--model haiku` and the code map hint. A flag the
/// person already gave is left alone. Anything that is not `claude` stays exactly as asked. No
/// `--settings`: the core's `claude` shim adds the hooks, the handoff included.
/// that is not `claude` stays exactly as asked.
pub fn with_routing(route: &Route, mut argv: Vec<String>) -> Vec<String> {
    if !starts_claude(&argv) {
        return argv;
    }
    let mut add: Vec<String> = Vec::new();
    if !route.tools.is_empty() && !gave(&argv, &["--tools", "--allowedTools", "--allowed-tools", "--disallowedTools", "--disallowed-tools"]) {
        add.extend(["--tools".to_owned(), route.tools.join(",")]);
    }
    if route.model.as_deref() == Some("haiku") && !gave(&argv, &["--model", "--agent"]) {
        add.extend(["--model".to_owned(), "haiku".to_owned()]);
    }
    if route.graft && !argv.iter().any(|a| a.starts_with("--append-system-prompt")) {
        add.extend(["--append-system-prompt".to_owned(), MAP_HINT.to_owned()]);
    }
    argv.splice(1..1, add);
    argv
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
    // The app starts commands in the person's shell: `zsh -ic '"$@"; exec "$0" -i' zsh <cmd>...`,
    // or fish's `command $argv[2..]; exec $argv[1] -i`; on macOS both as login shells (`-lic`, `-li`).
    if argv.len() > 4
        && t.shells.iter().any(|s| s == base(&argv[0]))
        && argv[1].starts_with('-')
        && argv[1].ends_with('c')
        && (argv[2].contains("\"$@\"") || matches!(argv[2].as_str(), "command $argv[2..]; exec $argv[1] -i" | "command $argv[2..]; exec $argv[1] -li"))
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
        // A cold PowerShell start on a busy CI runner can miss the exec limit now and then; the
        // check itself is the same on every try.
        let args = installed_powershell(&["cmd.exe".into(), "h0x_missing_agent_440".into()]);
        let out = (0..3).find_map(|_| crate::mods::run_here("powershell.exe", &args)).expect("powershell answered within three tries");
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
        // macOS starts agents from a login shell (`-lic`, then `exec fish -li`).
        assert_eq!(key(None, &["/bin/zsh", "-lic", "\"$@\"; exec \"$0\" -li", "/bin/zsh", "cursor-agent"]), "cursor");
        assert_eq!(key(None, &["/opt/homebrew/bin/fish", "-lic", "command $argv[2..]; exec $argv[1] -li", "/opt/homebrew/bin/fish", "cursor-agent"]), "cursor");
        let line = powershell_line(&["codex".into(), "fix it".into()]);
        assert_eq!(key(None, &["pwsh.exe", "-NoExit", "-Command", &line]), "codex");
        assert_eq!(key(None, &[r"C:\Program Files\PowerShell\7\pwsh.exe", "-NoExit", "-Command", &line]), "codex");
        assert_eq!(key(None, &[r"C:\Program Files\PowerShell\7\pwsh.exe"]), "shell");
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

    #[cfg(unix)]
    #[test]
    fn the_check_runs_in_the_persons_shell_and_falls_back_to_sh_only_when_it_cannot() {
        use std::os::unix::fs::PermissionsExt;
        let temp = if cfg!(target_os = "linux") { std::path::PathBuf::from("/var/tmp") } else { std::env::temp_dir() };
        let dir = temp.join(format!("h0x-other-shell-check-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("rc")).unwrap();
        let script = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path.to_string_lossy().into_owned()
        };
        // Only the person's shell can find it: its rc adds `rc` to PATH.
        script("rc/issue240-rc-agent", "");
        // A sh-compatible shell under a name nothing knows; and nushell, which rejects sh syntax.
        let posix = script("mksh-custom", &format!("PATH={}:$PATH exec /bin/sh \"$@\"", dir.join("rc").display()));
        let nu = script("nu", "echo \"Error: nu::parser::shell_outerr\" >&2\nexit 1");
        let commands = ["sh", "issue240-rc-agent", "issue240-missing-agent"].map(String::from);
        let (posix, nu) = (installed_here(&posix, &commands), installed_here(&nu, &commands));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(posix, ["sh", "issue240-rc-agent"]);
        assert_eq!(nu, ["sh"]);
    }

    #[test]
    fn found_commands_are_whole_words_in_the_asked_order() {
        let commands = ["claude", "codex", "gemini"].map(String::from);
        assert_eq!(installed_found(&commands, ""), Vec::<String>::new());
        assert_eq!(installed_found(&[], "h0x-has:codex\n"), Vec::<String>::new());
        // A longer name is not the command, nor is a bare mention of it.
        assert_eq!(installed_found(&commands, "h0x-has:codexx\nsee codex\n"), Vec::<String>::new());
        // Order follows the question; PowerShell's CRLF and a chatty profile change nothing.
        assert_eq!(installed_found(&commands, "Welcome!\r\nh0x-has:gemini\r\nh0x-has:claude\r\nh0x-has:claude\r\n"), ["claude", "gemini"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_shell_that_cannot_start_or_stops_early_falls_back_to_sh() {
        use std::os::unix::fs::PermissionsExt;
        let temp = if cfg!(target_os = "linux") { std::path::PathBuf::from("/var/tmp") } else { std::env::temp_dir() };
        let dir = temp.join(format!("h0x-early-shell-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Claims an agent, then fails before the completion mark: its answer is not trusted.
        let liar = dir.join("liar");
        std::fs::write(&liar, "#!/bin/sh\nprintf 'h0x-has:h0x-fake-agent\\n'\nexit 1\n").unwrap();
        std::fs::set_permissions(&liar, std::fs::Permissions::from_mode(0o755)).unwrap();
        let commands = ["h0x-fake-agent", "sh"].map(String::from);
        let liar = installed_here(&liar.to_string_lossy(), &commands);
        let missing = installed_here(&dir.join("no-such-shell").to_string_lossy(), &commands);
        let empty = installed_here("", &commands);
        let nothing = installed_here("/bin/sh", &[]);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(liar, ["sh"]);
        assert_eq!(missing, ["sh"]);
        assert_eq!(empty, ["sh"]);
        assert!(nothing.is_empty());
    }

    #[test]
    fn agents_in_the_sh_fallback_for_other_shells_are_named() {
        // How `window::unix_agent_argv` starts an agent under nushell and the like.
        let argv = ["/bin/sh", "-c", "PATH='/usr/bin:/bin' \"$@\"; exec \"$0\" -i", "/usr/bin/nu", "cursor-agent"];
        assert_eq!(key(None, &argv), "cursor");
        // Fish's wrapper only in its two exact forms.
        assert_eq!(key(None, &["/usr/bin/fish", "-ic", "command $argv[2..]; exec $argv[1] -li", "/usr/bin/fish", "vi", "x"]), "vim");
        assert_ne!(key(None, &["/usr/bin/fish", "-ic", "command $argv[2..]; exec $argv[1] -x", "/usr/bin/fish", "vi", "x"]), "vim");
    }

    #[test]
    fn every_entry_has_its_icon() {
        for (k, e) in t().programs {
            let icon = e.icon.unwrap_or_else(|| panic!("{k} has no icon"));
            let file = icon.strip_prefix("icons/").unwrap();
            assert!(DEFAULT_ICONS.iter().any(|(n, _)| *n == file), "{k}: {icon} not shipped");
        }
    }

    // ---- 0.1.31 edge tests (A31.1) ----

    #[test]
    fn past_nine_slots_the_extra_own_agents_list_with_no_slot() {
        let many: Vec<String> = (0..12).map(|i| format!(r#""own{i:02}": {{ "run": ["own{i}"], "order": {} }}"#, 100 - i)).collect();
        let table = with_mine(&format!(r#"{{ "programs": {{ {} }} }}"#, many.join(",")));
        let keys = slot_keys(&table);
        // Highest order last: own11 (order 89) is first among the person's own.
        assert_eq!(&keys[4..], ["own11", "own10", "own09", "own08", "own07"]);
        assert_eq!(startable_entry(&table, "own07")["slot"], json!(9));
        for left_out in ["own06", "own00"] {
            let e = startable_entry(&table, left_out);
            assert_eq!(e["slot"], Value::Null, "{left_out}");
            assert_eq!(e["custom"], json!(true), "{left_out}");
        }
    }

    #[test]
    fn extreme_and_missing_orders_sort_without_overflow_and_ties_fall_back_to_the_key() {
        let table = with_mine(&format!(
            r#"{{ "programs": {{
                "max": {{ "run": ["m"], "order": {} }}, "none": {{ "run": ["n"] }}, "min": {{ "run": ["i"], "order": {} }},
                "zero": {{ "run": ["z"], "order": 0 }}, "alpha": {{ "run": ["a"] }}
            }} }}"#,
            i64::MAX,
            i64::MIN
        ));
        // An explicit i64::MAX ties with "no order": the key decides between them.
        assert_eq!(slot_keys(&table), ["claude", "codex", "opencode", "gemini", "min", "zero", "alpha", "max", "none"]);
    }

    #[test]
    fn an_order_that_is_not_a_whole_number_is_a_programs_json_error_not_a_silent_default() {
        for bad in [r#"{ "programs": { "x": { "run": ["x"], "order": 1.5 } } }"#, r#"{ "programs": { "x": { "run": ["x"], "order": "1" } } }"#, r#"{ "programs": { "x": { "run": ["x"], "order": 1e30 } } }"#] {
            assert!(serde_json::from_str::<Table>(bad).is_err(), "{bad}");
        }
        // null is the same as leaving it out.
        let table = with_mine(r#"{ "programs": { "x": { "run": ["x"], "order": null } } }"#);
        assert_eq!(slot_keys(&table).last().map(String::as_str), Some("x"));
    }

    #[test]
    fn a_persons_entry_for_a_built_in_outside_the_four_gets_no_slot_and_is_not_their_own() {
        // pi is in the built-in table: giving it an order does not make it one of the person's agents.
        let table = with_mine(r#"{ "programs": { "pi": { "run": ["pi", "--fast"], "order": -100 }, "mine": { "run": ["mine"] } } }"#);
        assert_eq!(slot_keys(&table), ["claude", "codex", "opencode", "gemini", "mine"]);
        let pi = startable_entry(&table, "pi");
        assert_eq!((pi["slot"].clone(), pi["custom"].clone(), pi["run"].clone()), (Value::Null, json!(false), json!(["pi", "--fast"])));
    }

    #[test]
    fn a_built_in_cannot_lose_its_slot_by_an_empty_run_and_a_key_differing_in_case_is_its_own() {
        let table = with_mine(r#"{ "programs": { "gemini": { "run": [] }, "Claude": { "name": "My Claude", "run": ["claude", "--mine"] } } }"#);
        assert_eq!(slot_keys(&table), ["claude", "codex", "opencode", "gemini", "Claude"]);
        assert_eq!(startable_entry(&table, "gemini")["slot"], json!(4));
        assert!(!startable_entry(&table, "gemini")["run"].as_array().unwrap().is_empty(), "the built-in command stays");
        let mine = startable_entry(&table, "Claude");
        assert_eq!((mine["slot"].clone(), mine["custom"].clone()), (json!(5), json!(true)));
        assert_eq!(startable_entry(&table, "claude")["custom"], json!(false));
    }

    #[test]
    fn own_agents_with_unicode_keys_or_no_run_are_handled_and_never_panic() {
        let table = with_mine(r#"{ "programs": {
            "éclair": { "run": ["eclair"] }, "zebra": { "run": ["z"] }, "日本": { "run": ["nihon"] },
            "empty": { "run": [] }, "nameonly": { "name": "N" }
        } }"#);
        // By key, as bytes: ASCII first, then the accented and the CJK keys.
        assert_eq!(slot_keys(&table), ["claude", "codex", "opencode", "gemini", "zebra", "\u{e9}clair", "\u{65e5}\u{672c}"]);
        for k in ["empty", "nameonly"] {
            assert!(!slot_keys(&table).contains(&k.to_string()), "{k} has nothing to start");
        }
    }

    #[test]
    fn a_table_with_no_programs_at_all_still_has_the_four_slots() {
        for mine in ["{}", r#"{ "programs": {} }"#, r#"{ "shells": [], "interpreters": [] }"#] {
            assert_eq!(slot_keys(&with_mine(mine)), ["claude", "codex", "opencode", "gemini"], "{mine}");
        }
    }

    #[test]
    fn key_shorthands_for_agent_new_reach_only_slots_one_to_nine() {
        // `agent.new:<n>` in keys.json goes through the shorthand, then the slot check.
        let command = crate::commands::find("agent.new").expect("agent.new is a command");
        let slot = |text: &str| slot_number(crate::commands::shorthand(command, text).get("index"));
        for (text, want) in [("1", 1), ("9", 9), ("5", 5), ("03", 3)] {
            assert_eq!(slot(text), Ok(want), "{text:?}");
        }
        for bad in ["0", "10", "-1", "1.5", "abc", "", " ", "99999999999999999999999", "\u{0661}", "\u{ff11}"] {
            let got = slot(bad);
            assert!(got.is_err(), "{bad:?}: {got:?}");
            assert!(got.unwrap_err().contains("1 to 9"), "{bad:?}: the message names the range");
        }
        // A mod's run("agent.new", { index: 3 }) arrives as a whole JS number.
        assert_eq!(slot_number(Some(&crate::mods::from_host(&gpui_shell::HostValue::Number(3.0)))), Ok(3));
        assert!(slot_number(Some(&crate::mods::from_host(&gpui_shell::HostValue::Number(3.5)))).is_err());
        assert!(slot_number(Some(&crate::mods::from_host(&gpui_shell::HostValue::Number(f64::NAN)))).is_err());
        assert!(slot_number(Some(&json!(u64::MAX))).is_err());
        assert!(slot_number(Some(&json!({ "index": 1 }))).is_err());
    }

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn claude_gets_its_own_session_id_unless_the_person_chose_a_session() {
        let id = "0b0c3f5e-1a2b-4c3d-8e4f-5a6b7c8d9e0f";
        let out = with_session_id(argv(&["claude", "go"]));
        assert_eq!((out[0].as_str(), out[1].as_str(), out[3].as_str()), ("claude", "--session-id", "go"));
        assert!(crate::agent_usage::valid_session_id(&out[2]), "{out:?}");
        assert_eq!(session_of(&out).as_deref(), Some(out[2].as_str()));
        assert_ne!(with_session_id(argv(&["claude"]))[2], out[2], "one id per launch");
        // Version 4, variant 10.
        assert_eq!((out[2].as_bytes()[14], matches!(out[2].as_bytes()[19], b'8' | b'9' | b'a' | b'b')), (b'4', true));
        // The person's own choice is respected: argv unchanged, an explicit id is used.
        for a in [argv(&["claude", "--session-id", id]), argv(&["claude", "--resume", id]), argv(&["claude", "-r", id]), argv(&["claude", &format!("--session-id={id}")]), argv(&["claude", &format!("--resume={id}")])] {
            assert_eq!(with_session_id(a.clone()), a);
            assert_eq!(session_of(&a).as_deref(), Some(id), "{a:?}");
        }
        for a in [argv(&["claude", "--continue"]), argv(&["claude", "-c"]), argv(&["claude", "--resume"]), argv(&["claude", "--resume", "fix the bug"]), argv(&["claude", "--from-pr", "7"]), argv(&["claude", "--session-id", "../x"]), argv(&["claude", "--resume", r"..\..\x"])] {
            assert_eq!(with_session_id(a.clone()), a);
            assert_eq!(session_of(&a), None, "{a:?}");
        }
        // Not Claude: unchanged, no session.
        for a in [argv(&["codex"]), argv(&["bash", "--session-id", id]), argv(&[]), argv(&[""])] {
            assert_eq!(with_session_id(a.clone()), a);
            assert_eq!(session_of(&a), None, "{a:?}");
        }
        // The env flag still applies to the launch with its session id.
        assert_eq!((token_saver_env(true, &out), token_saver_env(false, &out)), (Some("1"), Some("0")));
    }

    #[test]
    fn new_session_ids_are_uuid_v4_shaped_and_unique_across_ten_thousand_calls_and_threads() {
        let check = |id: &str| {
            let b = id.as_bytes();
            assert!(crate::agent_usage::valid_session_id(id), "{id}");
            assert!(id.bytes().all(|c| c == b'-' || c.is_ascii_digit() || (b'a'..=b'f').contains(&c)), "lowercase hex: {id}");
            assert_eq!(b[14], b'4', "version nibble: {id}");
            assert!(matches!(b[19], b'8' | b'9' | b'a' | b'b'), "variant 10: {id}");
        };
        let mut seen = std::collections::HashSet::new();
        for _ in 0..10_000 {
            let id = new_session_id();
            check(&id);
            assert!(seen.insert(id.clone()), "repeated id {id}");
        }
        // Each thread seeds its own `RandomState` keys; ids from parallel launches never repeat either.
        let handles: Vec<_> = (0..4).map(|_| std::thread::spawn(|| (0..2_500).map(|_| new_session_id()).collect::<Vec<_>>())).collect();
        for h in handles {
            for id in h.join().unwrap() {
                check(&id);
                assert!(seen.insert(id.clone()), "repeated id across threads {id}");
            }
        }
        assert_eq!(seen.len(), 20_000);
    }

    #[test]
    fn session_flags_edge_cases_keep_argv_and_record_only_an_explicit_valid_id() {
        let id = "0b0c3f5e-1a2b-4c3d-8e4f-5a6b7c8d9e0f";
        let other = "1c1d4a6f-2b3c-4d4e-9f50-6b7c8d9e0f1a";
        let unchanged = |a: Vec<String>| assert_eq!(with_session_id(a.clone()), a, "{a:?}");
        // Empty values and a value-less flag at the end: the person chose a session flag, nothing recorded.
        for a in [
            argv(&["claude", "--session-id="]),
            argv(&["claude", "--resume="]),
            argv(&["claude", "go", "-r"]),
            argv(&["claude", "--session-id"]),
            argv(&["claude", "--session-id", ""]),
            argv(&["claude", "--from-pr=7"]),
            argv(&["claude", "-r", "--continue"]),
        ] {
            unchanged(a.clone());
            assert_eq!(session_of(&a), None, "{a:?}");
        }
        // `-r=<id>` is not a form Claude takes; the flag is respected but no id is recorded.
        let a = argv(&["claude", &format!("-r={id}")]);
        unchanged(a.clone());
        assert_eq!(session_of(&a), None);
        // `--resume` given twice: a search term then an id records the id.
        let a = argv(&["claude", "--resume", "fix it", "--resume", id]);
        unchanged(a.clone());
        assert_eq!(session_of(&a).as_deref(), Some(id));
        // Two ids: one of the person's own ids, never a fresh one.
        let a = argv(&["claude", "--resume", id, "--resume", other]);
        unchanged(a.clone());
        assert_eq!(session_of(&a).as_deref(), Some(other), "the last one wins");
        let a = argv(&["claude", "--session-id", other, "--resume", id]);
        assert_eq!(session_of(&a).as_deref(), Some(id));
        // A fork writes a new, unknown id: nothing recorded, argv kept.
        for a in [argv(&["claude", "-r", id, "--fork-session"]), argv(&["claude", "--fork-session", "--resume", id]), argv(&["claude", "--session-id", id, "--fork-session"])] {
            unchanged(a.clone());
            assert_eq!(session_of(&a), None, "{a:?}");
        }
        // `--continue` / `-c` combined with an explicit id: argv kept, that id recorded.
        for a in [
            argv(&["claude", "--continue", "--session-id", id]),
            argv(&["claude", "--session-id", id, "--continue"]),
            argv(&["claude", "-c", &format!("--session-id={id}")]),
        ] {
            unchanged(a.clone());
            assert_eq!(session_of(&a).as_deref(), Some(id), "{a:?}");
        }
        // Non-canonical UUID spellings are respected but never recorded.
        for v in [
            format!("{{{id}}}"),
            format!("urn:uuid:{id}"),
            id.replace('-', ""),
            format!(" {id}"),
            format!("{id} "),
            format!("{id}\n"),
            format!("{id}.jsonl"),
            id[..35].to_owned(),
            format!("{id}0"),
            id.replacen('-', "_", 1),
            "\u{ff10}".repeat(36),
        ] {
            for a in [argv(&["claude", "--session-id", &v]), argv(&["claude", &format!("--resume={v}")]), argv(&["claude", "-r", &v])] {
                unchanged(a.clone());
                assert_eq!(session_of(&a), None, "{a:?}");
            }
        }
        // Claude's logs are named in lower case: upper or mixed case is respected but never recorded.
        let upper = id.to_ascii_uppercase();
        assert_eq!(session_of(&argv(&["claude", "--session-id", &upper])), None);
        // The nil UUID is never recorded.
        assert_eq!(session_of(&argv(&["claude", "--session-id", "00000000-0000-0000-0000-000000000000"])), None);
        // A flag that only starts like ours is not ours: Claude still gets its own id.
        let out = with_session_id(argv(&["claude", "--session-idx", id]));
        assert_eq!((out[1].as_str(), out.len()), ("--session-id", 5));
        assert_ne!(out[2], id);
        // Windows and Unix paths to Claude get an id too, after argv[0].
        for exe in [r"C:\Program Files\claude\claude.exe", r"\\server\share\CLAUDE.EXE", "/usr/local/bin/claude"] {
            let out = with_session_id(argv(&[exe, ""]));
            assert_eq!((out[0].as_str(), out[1].as_str(), out[3].as_str()), (exe, "--session-id", ""), "{out:?}");
            assert_eq!(session_of(&out).as_deref(), Some(out[2].as_str()));
        }
    }

    /// The env flag a launch carries; Token Saver adds nothing to the argv (the core's `claude`
    /// shim adds the hooks, ADR-129).
    fn saver(on: bool, a: &[&str]) -> Option<&'static str> {
        token_saver_env(on, &argv(a))
    }

    #[test]
    fn token_saver_sets_the_env_flag_for_claude_only_and_adds_no_settings() {
        assert_eq!((saver(true, &["claude"]), saver(false, &["claude", "go"])), (Some("1"), Some("0")));
        // On or off, argv is never rewritten: no `--settings`, no hook, no scope or secret.
        for a in [argv(&["claude", "go"]), argv(&["/usr/local/bin/claude", "--resume", "fix it"]), argv(&["claude.exe"]), argv(&[r"C:\Program Files\claude\claude.exe"])] {
            assert!(token_saver_env(true, &a).is_some() && token_saver_env(false, &a).is_some(), "{a:?}");
            assert_eq!(with_routing(&Route::default(), a.clone()), a, "an empty route adds nothing");
            assert!(!a.iter().any(|x| x.contains("settings") || x.contains("H0X_SCOPE")), "{a:?}");
        }
    }

    #[test]
    fn token_saver_flag_leaves_everything_else_alone() {
        // Other programs get no flag; the person's own `--settings` Claude still gets it (the shim
        // then adds nothing) and its argv stays exactly as given.
        for a in [argv(&[]), argv(&["codex"]), argv(&["opencode", "x"]), argv(&["pwsh"])] {
            assert_eq!((token_saver_env(true, &a), token_saver_env(false, &a)), (None, None), "{a:?}");
        }
        for a in [argv(&["claude", "--settings", "{}"]), argv(&["claude", "--settings=x.json"])] {
            assert_eq!(with_routing(&Route::default(), a.clone()), a);
            assert_eq!(token_saver_env(true, &a), Some("1"));
            assert_eq!(token_saver_env(false, &a), Some("0"));
        }
    }

    #[test]
    fn token_saver_matches_claude_in_any_case_like_agent() {
        // Windows file names ignore case and so does `agent`: the flag goes to any casing.
        for name in ["CLAUDE.EXE", "Claude", "Claude.exe", r"C:\Tools\CLAUDE.EXE", "claude.EXE", r"C:\Tools\claude.Exe", "/opt/claude/bin/claude", r"\\server\share\claude.exe"] {
            assert_eq!(agent(&argv(&[name])).as_deref(), Some("claude"), "{name}");
            assert_eq!(saver(true, &[name]), Some("1"), "{name}");
            assert_eq!(saver(false, &[name]), Some("0"), "{name}");
        }
    }

    #[test]
    fn token_saver_skips_other_programs_that_run_or_resemble_claude() {
        // `agent` calls these Claude (its process name, or the program an interpreter runs),
        // but the command is not `claude` itself: no flag.
        for a in [argv(&["claude-code"]), argv(&["npx", "claude"])] {
            assert_eq!(agent(&a).as_deref(), Some("claude"), "{a:?}");
            assert_eq!(token_saver_env(true, &a), None);
        }
        for a in [argv(&["claude-dev"]), argv(&["claudex"]), argv(&[r"C:\Users\me\AppData\Roaming\npm\claude.cmd"]), argv(&[""]), argv(&["", "claude"]), argv(&["pwsh.exe", "-c", "claude"])] {
            assert_eq!(token_saver_env(true, &a), None, "{a:?}");
        }
    }

    #[test]
    fn any_settings_word_is_left_untouched_even_a_prompt() {
        // `-p --settings` is a prompt, a bare `--settings` or one after `--` is theirs: argv stays as given.
        for a in [argv(&["claude", "-p", "--settings"]), argv(&["claude", "--settings"]), argv(&["claude", "--", "--settings=x"]), argv(&["claude", "--settings="]), argv(&["claude", "--settingsx"]), argv(&["claude", "--Settings", "x.json"]), argv(&["claude", "-p", "use --settings=x"])] {
            assert_eq!(with_routing(&Route::default(), a.clone()), a, "{a:?}");
            assert_eq!(token_saver_env(true, &a), Some("1"), "{a:?}");
        }
    }

    #[test]
    fn a_task_with_quotes_and_dollars_survives_the_powershell_line() {
        let task = "say \"hi\" to $env:USERNAME's \u{2018}box\u{2019}; `$(Get-Date)` & | > done";
        let mut cmd = argv(&["claude"]);
        cmd.push(task.to_owned());
        assert_eq!(powershell_words(&powershell_line(&cmd)), Some(cmd.clone()));
        assert_eq!(windows_unquote(&windows_arg(task)), task);
    }

    fn route(tools: &[&str], graft: bool, model: Option<&str>) -> Route {
        Route { graft, tools: tools.iter().map(|s| s.to_string()).collect(), model: model.map(str::to_owned) }
    }
    fn full() -> Route {
        route(&["Bash", "Read"], true, Some("haiku"))
    }
    /// Every word that is a `--settings` flag of any spelling.
    fn settings_words(v: &[String]) -> usize {
        v.iter().filter(|a| *a == "--settings" || a.starts_with("--settings=")).count()
    }

    #[test]
    fn routing_adds_tools_model_and_hint_and_no_settings() {
        let out = with_routing(&full(), argv(&["claude", "go"]));
        assert_eq!(settings_words(&out), 0);
        assert_eq!(out, argv(&["claude", "--tools", "Bash,Read", "--model", "haiku", "--append-system-prompt", MAP_HINT, "go"]));
        assert!(!out.iter().any(|a| a == "--allowedTools"), "permission prompts stay");
        assert!(!out.iter().any(|a| a.contains("SessionStart") || a.contains("handoff") || a.contains("gate")), "the shim owns the hooks");
        assert!(MAP_HINT.contains("map callers <sym>") && MAP_HINT.contains("map grep <word>") && !MAP_HINT.contains("graft"));
    }

    #[test]
    fn routing_without_flags_changes_nothing() {
        assert_eq!(with_routing(&route(&[], false, None), argv(&["claude", "go"])), argv(&["claude", "go"]));
    }

    #[test]
    fn routing_flags_follow_the_profile() {
        let out = with_routing(&route(&[], false, None), argv(&["claude"]));
        assert!(!out.iter().any(|a| a == "--tools" || a == "--model" || a == "--append-system-prompt"), "empty tools, no model, no graft: {out:?}");
        for model in [None, Some("sonnet"), Some("Haiku"), Some("")] {
            let out = with_routing(&route(&["Read"], false, model), argv(&["claude"]));
            assert!(!out.iter().any(|a| a == "--model"), "{model:?}");
            assert!(!out.iter().any(|a| a == "--append-system-prompt"));
            assert!(out.iter().any(|a| a == "--tools"));
        }
        let out = with_routing(&route(&["Read"], true, None), argv(&["claude"]));
        assert!(out.iter().any(|a| a == "--append-system-prompt"));
    }

    #[test]
    fn routing_adds_no_hooks_when_the_person_gave_settings() {
        for given in [&["--settings", "{}"][..], &["--settings=x.json"], &["--settings"]] {
            let mut a = argv(&["claude"]);
            a.extend(argv(given));
            let out = with_routing(&route(&["Read"], false, None), a.clone());
            assert!(!out.iter().any(|x| x.contains("SessionStart")), "{given:?}");
            assert_eq!(settings_words(&out), 1);
            assert_eq!(out[1..3], argv(&["--tools", "Read"])[..], "flags still apply");
            assert_eq!(out[3..], a[1..], "{given:?}");
        }
        assert!(!with_routing(&full(), argv(&["claude", "--settings=x.json"])).iter().any(|x| x.contains("SessionStart")));
    }

    #[test]
    fn routing_matches_claude_in_any_case_and_windows_paths() {
        for name in ["CLAUDE.EXE", "Claude", "claude.exe", r"C:\Program Files\claude\claude.exe", r"C:\Tools\CLAUDE.EXE", r"\\server\share\claude.exe", "/usr/local/bin/claude"] {
            let out = with_routing(&full(), argv(&[name, "go"]));
            assert_eq!(out[0], name);
            assert_eq!(out[1], "--tools", "{name}");
            assert_eq!(out.last().unwrap(), "go");
            assert_eq!(settings_words(&out), 0);
        }
    }

    #[test]
    fn routing_prompt_and_hint_survive_the_powershell_line() {
        let cmd = with_routing(&full(), argv(&[r"C:\Program Files\claude\claude.exe", "do \"it\" $x\nnext"]));
        assert_eq!(powershell_words(&powershell_line(&cmd)), Some(cmd.clone()));
        let hint = cmd.iter().position(|a| a == "--append-system-prompt").unwrap() + 1;
        assert_eq!(cmd[hint], MAP_HINT);
        assert!(cmd[hint].contains('"') && cmd[hint].contains('$') && cmd[hint].contains('\n'));
        assert_eq!(windows_unquote(&windows_arg(MAP_HINT)), MAP_HINT);
    }

    #[test]
    fn routing_on_bare_claude_gives_the_exact_argv() {
        let out = with_routing(&full(), argv(&["claude"]));
        assert_eq!(out, argv(&["claude", "--tools", "Bash,Read", "--model", "haiku", "--append-system-prompt", MAP_HINT]));
        // Token Saver on or off: the same argv; only the env flag differs.
        assert_eq!((saver(true, &["claude"]), saver(false, &["claude"])), (Some("1"), Some("0")));
    }

    #[test]
    fn routing_twice_adds_nothing_more() {
        for start in [argv(&["claude", "go"]), argv(&["claude", "--model", "opus", "go"]), argv(&["claude", "--settings", "{}"])] {
            let once = with_routing(&full(), start.clone());
            let twice = with_routing(&full(), once.clone());
            assert_eq!(twice, once, "{start:?}");
            assert_eq!(settings_words(&twice), settings_words(&start));
            assert!(!twice.iter().any(|a| a.contains("SessionStart")));
        }
    }

    #[test]
    fn routing_keeps_the_persons_args_in_order_after_ours() {
        let person = argv(&["--resume", "abc", "--verbose", "-p", "fix it", "--", "--tail"]);
        let mut a = argv(&["/usr/local/bin/claude"]);
        a.extend(person.clone());
        let out = with_routing(&full(), a.clone());
        assert_eq!(out[0], "/usr/local/bin/claude");
        assert_eq!(out[out.len() - person.len()..], person[..], "{out:?}");
        assert_eq!(out.len(), 1 + 6 + person.len());
        // With the person's own model, ours are only the missing ones and theirs keep place.
        let out = with_routing(&full(), argv(&["claude", "go", "--model", "opus", "--verbose"]));
        assert_eq!(out[out.len() - 4..], argv(&["go", "--model", "opus", "--verbose"])[..]);
        assert_eq!(out.iter().filter(|a| *a == "--model").count(), 1);
    }

    #[test]
    fn routing_leaves_remote_and_wrapped_claude_alone() {
        // Only the local `claude` command is changed; a remote or wrapped one is not ours to rewrite.
        for a in [argv(&["ssh", "host", "claude"]), argv(&["ssh", "-t", "me@host", "claude", "--model", "opus"]), argv(&["wsl", "claude"]), argv(&["wsl.exe", "-e", "claude"]), argv(&[r"C:\Windows\System32\OpenSSH\ssh.exe", "host", "claude"]), argv(&[""])] {
            assert_eq!(with_routing(&full(), a.clone()), a, "{a:?}");
            assert_eq!(token_saver_env(true, &a), None, "{a:?}");
        }
    }

    #[test]
    fn routing_defers_to_flags_the_person_gave() {
        for given in [&["--model", "opus"][..], &["--model=opus"], &["--agent", "x"], &["--agent=x"]] {
            let mut a = argv(&["claude"]);
            a.extend(argv(given));
            let out = with_routing(&route(&[], false, Some("haiku")), a);
            assert!(!out.iter().any(|a| a == "haiku"), "{given:?}");
        }
        for given in [&["--tools", "Bash"][..], &["--tools="], &["--allowedTools", "Bash"], &["--allowedTools=Bash"], &["--disallowedTools", "Edit"], &["--disallowed-tools=Edit"]] {
            let mut a = argv(&["claude"]);
            a.extend(argv(given));
            let out = with_routing(&route(&["Read"], false, None), a);
            assert!(!out.iter().any(|a| a == "Read"), "{given:?}");
        }
        for given in [&["--append-system-prompt", "x"][..], &["--append-system-prompt=x"], &["--append-system-prompt-file", "f"]] {
            let mut a = argv(&["claude"]);
            a.extend(argv(given));
            let out = with_routing(&route(&[], true, None), a);
            assert!(!out.iter().any(|a| a.contains("Code map")), "{given:?}");
        }
        // Each flag is judged alone: the person's model does not block our tools.
        let out = with_routing(&route(&["Read"], false, Some("haiku")), argv(&["claude", "--model", "opus"]));
        assert!(out.iter().any(|a| a == "Read"));
        assert!(!out.iter().any(|a| a == "haiku"));
    }

    #[test]
    fn routing_leaves_other_programs_alone() {
        for a in [argv(&[]), argv(&["codex"]), argv(&["pwsh"]), argv(&["claude-code"]), argv(&["npx", "claude"]), argv(&["claudex"]), argv(&[r"C:\Users\me\npm\claude.cmd"])] {
            assert_eq!(with_routing(&full(), a.clone()), a, "{a:?}");
        }
    }

    #[test]
    fn route_reads_the_core_json_and_ignores_the_rest() {
        let r: Route = serde_json::from_str(r#"{"kind":"code","code":true,"read_only":false,"nav":true,"graft":true,"tools":["Bash","Read"],"model":null}"#).unwrap();
        assert!(r.graft && r.model.is_none());
        assert_eq!(r.tools, ["Bash", "Read"]);
        let r: Route = serde_json::from_str(r#"{"tools":[],"model":"haiku"}"#).unwrap();
        assert!(!r.graft && r.tools.is_empty() && r.model.as_deref() == Some("haiku"));
    }

    #[test]
    fn routing_counts_empty_and_kebab_flag_values_as_given() {
        // `--flag=` with an empty value is still the person's flag.
        for given in ["--model=", "--agent="] {
            let out = with_routing(&route(&[], false, Some("haiku")), argv(&["claude", given]));
            assert_eq!(out[3..], argv(&[given])[..], "{given}");
        }
        for given in [&["--allowed-tools", "Bash"][..], &["--allowed-tools="], &["--allowedTools="], &["--disallowedTools="], &["--disallowed-tools", "Edit"]] {
            let mut a = argv(&["claude"]);
            a.extend(argv(given));
            let out = with_routing(&route(&["Read"], false, None), a);
            assert!(!out.iter().any(|x| x == "--tools"), "{given:?}");
        }
        let out = with_routing(&route(&[], true, None), argv(&["claude", "--append-system-prompt="]));
        assert!(!out.iter().any(|a| a.contains("Code map")));
        // Look-alikes are not the flag: `--agents` defines agents but picks none, so haiku still applies.
        for other in ["--agents", "--modelx", "--toolsx"] {
            let out = with_routing(&route(&["Read"], false, Some("haiku")), argv(&["claude", other, "{}"]));
            assert!(out.iter().any(|a| a == "haiku"), "{other}");
            assert!(out.iter().any(|a| a == "--tools"), "{other}");
        }
    }

    #[test]
    fn routing_passes_a_tools_list_with_spaces_as_one_argument() {
        let cmd = with_routing(&route(&["Bash(git log:*)", "Read"], false, None), argv(&[r"C:\Program Files\claude\claude.exe", "go"]));
        let i = cmd.iter().position(|a| a == "--tools").unwrap();
        assert_eq!(cmd[i + 1], "Bash(git log:*),Read");
        assert_eq!(cmd[i + 2], "go");
        assert_eq!(powershell_words(&powershell_line(&cmd)), Some(cmd.clone()));
        let one = with_routing(&route(&["Read"], false, None), argv(&["claude"]));
        assert_eq!(one[one.len() - 2..], argv(&["--tools", "Read"])[..], "one tool, no trailing comma");
    }

    /// A Windows command line split the way C runtimes since 2008 read it (`""` inside quotes is one `"`).
    fn crt_split(line: &str) -> Vec<String> {
        let (mut out, mut cur, mut in_word, mut quoted, mut slashes) = (Vec::new(), String::new(), false, false, 0);
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\\' => {
                    slashes += 1;
                    in_word = true;
                    continue;
                }
                '"' => {
                    cur.extend(std::iter::repeat_n('\\', slashes / 2));
                    if slashes % 2 == 1 {
                        cur.push('"');
                    } else if quoted && chars.peek() == Some(&'"') {
                        chars.next();
                        cur.push('"');
                    } else {
                        quoted = !quoted;
                    }
                    in_word = true;
                }
                ' ' | '\t' if !quoted => {
                    cur.extend(std::iter::repeat_n('\\', slashes));
                    if in_word {
                        out.push(std::mem::take(&mut cur));
                        in_word = false;
                    }
                }
                _ => {
                    cur.extend(std::iter::repeat_n('\\', slashes).chain([ch]));
                    in_word = true;
                }
            }
            slashes = 0;
        }
        cur.extend(std::iter::repeat_n('\\', slashes));
        if in_word {
            out.push(cur);
        }
        out
    }


    #[test]
    fn every_claude_spelling_gets_the_env_flag_on_and_off() {
        // Full Windows paths with spaces, `claude.exe`, WSL-style paths and npm's extensionless global shim.
        for name in [
            r"C:\Program Files\Anthropic Claude\claude.exe",
            r"C:\Users\Jane Doe\AppData\Roaming\npm\claude",
            "claude.exe",
            "/mnt/c/Users/Jane Doe/AppData/Roaming/npm/claude",
            r"\\wsl.localhost\Ubuntu\home\me\.npm-global\bin\claude",
            "/home/me/.npm-global/bin/claude",
        ] {
            for on in [true, false] {
                assert_eq!(saver(on, &[name, "go"]), Some(if on { "1" } else { "0" }), "{name} {on}");
            }
        }
    }

    #[test]
    fn the_flag_follows_claude_through_paths_case_and_windows_spellings() {
        for name in [r"C:\Users\Jane Doe\.local\bin\claude", "C:/Tools/Claude.EXE", "cLaUdE.ExE", r"D:\x\CLAUDE", "/usr/local/bin/claude", r"\\server\share\Claude.Exe"] {
            for rest in [&[][..], &["go"], &["--resume", "fix it"]] {
                let mut a = argv(&[name]);
                a.extend(argv(rest));
                assert_eq!((token_saver_env(true, &a), token_saver_env(false, &a)), (Some("1"), Some("0")), "{a:?}");
                // The flag is the whole change: argv is not rewritten by an empty route.
                assert_eq!(with_routing(&Route::default(), a.clone()), a, "{a:?}");
            }
        }
    }

    #[test]
    fn shims_wrappers_and_look_alikes_get_no_flag_on_or_off() {
        for a in [
            argv(&["claude.cmd"]),
            argv(&["CLAUDE.CMD", "go"]),
            argv(&[r"C:\Users\Jane Doe\AppData\Roaming\npm\claude.cmd"]),
            argv(&["claude.ps1"]),
            argv(&["npx", "claude"]),
            argv(&["npx.cmd", "claude", "go"]),
            argv(&["claude-code"]),
            argv(&["claude-code.exe", "go"]),
            argv(&[" claude"]),
            argv(&["claude ", "go"]),
            argv(&["", ""]),
            argv(&["sh", "-c", "claude"]),
        ] {
            for on in [true, false] {
                assert_eq!(token_saver_env(on, &a), None, "{a:?} {on}");
            }
        }
    }

    #[test]
    fn a_persons_settings_in_equals_form_passes_through_untouched() {
        let own = r#"--settings={"hooks":{"PostToolUse":[]}}"#;
        for a in [argv(&["claude", own]), argv(&["claude", "go", own]), argv(&["claude", "--settings=C:\\Users\\Jane Doe\\s.json"]), argv(&["claude", "--settings=/home/me/s.json", "--model", "opus"])] {
            assert_eq!(with_routing(&Route::default(), a.clone()), a, "{a:?}");
            let out = with_routing(&full(), a.clone());
            assert_eq!(settings_words(&out), 1, "{out:?}");
            assert_eq!(out[out.len() - (a.len() - 1)..], a[1..], "theirs keep their place and text: {out:?}");
            assert!(!out.iter().any(|x| x.contains("SessionStart") || x.contains("$H0X_CORE_EXE\" gate")), "{out:?}");
            // The shim then adds nothing, but the flag still says what the setting is.
            assert_eq!((token_saver_env(true, &a), token_saver_env(false, &a)), (Some("1"), Some("0")), "{a:?}");
        }
    }

    #[test]
    fn token_saver_toggled_between_two_launches_only_changes_the_flag() {
        let a = argv(&["claude", "go"]);
        let (first, second, third) = (token_saver_env(true, &a), token_saver_env(false, &a), token_saver_env(true, &a));
        assert_eq!((first, second, third), (Some("1"), Some("0"), Some("1")));
        assert_eq!(with_routing(&full(), a.clone()), with_routing(&full(), a.clone()), "argv is the same either way");
        assert_eq!(a, argv(&["claude", "go"]), "argv is only read");
    }

    #[test]
    fn session_id_handling_is_unchanged_under_routing_and_the_flag() {
        let id = "0b0c3f5e-1a2b-4c3d-8e4f-5a6b7c8d9e0f";
        let out = with_session_id(argv(&["claude", "--settings={}", "go"]));
        assert_eq!(out[1], "--session-id", "the person's settings do not stop the session id: {out:?}");
        let sid = session_of(&out).expect("a session id");
        let routed = with_routing(&full(), out.clone());
        assert_eq!(session_of(&routed).as_deref(), Some(sid.as_str()), "{routed:?}");
        assert_eq!(routed.iter().filter(|a| *a == "--session-id").count(), 1);
        assert_eq!(token_saver_env(false, &routed), Some("0"));
        for a in [argv(&["claude", "--resume", id]), argv(&["claude", &format!("--session-id={id}")])] {
            let routed = with_routing(&full(), with_session_id(a.clone()));
            assert_eq!(session_of(&routed).as_deref(), Some(id), "{routed:?}");
            assert_eq!(settings_words(&routed), 0, "{routed:?}");
        }
        // Not Claude: no session, no flag.
        for a in [argv(&["npx", "claude", "--session-id", id]), argv(&["claude.cmd", "--session-id", id])] {
            assert_eq!(with_session_id(a.clone()), a);
            assert_eq!((session_of(&a), token_saver_env(true, &a)), (None, None), "{a:?}");
        }
    }

    #[test]
    fn a_shim_that_is_not_claude_itself_gets_no_flag() {
        // `claude.cmd`/`claude.ps1` are skipped (programs.rs base() only drops `.exe`).
        for a in [argv(&["claude.cmd"]), argv(&[r"C:\Users\Jane Doe\AppData\Roaming\npm\claude.cmd", "go"]), argv(&["claude.ps1"])] {
            for on in [true, false] {
                assert_eq!(token_saver_env(on, &a), None, "{a:?} {on}");
            }
        }
        assert!(token_saver_env(true, &[]).is_none() && token_saver_env(true, &argv(&[""])).is_none() && token_saver_env(true, &argv(&["codex"])).is_none());
    }

    #[test]
    fn a_settings_flag_with_no_value_or_an_empty_value_is_the_persons_own() {
        for a in [argv(&["claude", "go", "--settings"]), argv(&["claude", "--settings="]), argv(&["claude", "--settings=", "go"]), argv(&["claude", "--model", "opus", "--settings"])] {
            // Routing adds its flags but never a second `--settings` nor any hook.
            let out = with_routing(&full(), a.clone());
            assert_eq!(settings_words(&out), 1, "{out:?}");
            assert!(!out.iter().any(|x| x.contains("SessionStart") || x.contains("hook session")), "{out:?}");
        }
    }

    #[test]
    fn routed_argv_never_carries_hooks_scope_or_secret() {
        for name in ["claude", r"C:\Program Files\Anthropic Claude\claude.exe"] {
            let out = with_routing(&full(), argv(&[name, "go"]));
            assert_eq!(settings_words(&out), 0);
            // Only the code map hint mentions the core; no scope or secret in any other word.
            assert!(!out.iter().filter(|a| *a != MAP_HINT).any(|a| a.contains("SCOPE") || a.contains("TOKEN") || a.contains("SECRET") || a.contains("H0X_CORE_EXE")), "{out:?}");
        }
    }

    #[test]
    fn the_routed_argv_survives_the_final_windows_command_line() {
        let task = "say \"hi\" to C:\\dir\\ & | > ^ done\\";
        for prog in ["claude", r"C:\Program Files\Anthropic Claude\claude.exe"] {
            for routed in [false, true] {
                let mut cmd = argv(&[prog]);
                if routed {
                    cmd = with_routing(&full(), cmd);
                }
                cmd.push(task.to_owned());
                // PowerShell's words, then the native command line it hands Claude, parsed back.
                let words = powershell_words(&powershell_line(&cmd)).expect("powershell line");
                assert_eq!(words, cmd, "{prog} {routed}");
                let line = words.iter().map(|w| windows_arg(w)).collect::<Vec<_>>().join(" ");
                assert_eq!(crt_split(&line), cmd, "{line}");
            }
        }
    }
}

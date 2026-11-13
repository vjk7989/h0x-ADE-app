//! Links in terminal text: Ghostty's URL and file path pattern, what a match
//! points to, and handing it to the system to open.

use fancy_regex::Regex;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Ghostty's default link pattern (`src/config/url.zig`), unchanged.
static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    let schemes = r"https?://|mailto:|ftp://|file:|ssh:|git://|ssh://|tel:|magnet:|ipfs://|ipns://|gemini://|gopher://|news:";
    let ipv6 = r"(?:\[[:0-9a-fA-F]+(?:[:0-9a-fA-F]*)+\](?::[0-9]+)?)";
    let scheme_chars = r"[\w\-.~:/?#@!$&*+,;=%]";
    let path_chars = r"[\w\-.~:\/?#@!$&*+;=%]";
    let bracketed = r"(?:[\(\[]\w*[\)\]])?";
    let no_trailing_punctuation = r"(?<![,.])";
    let no_trailing_colon = r"(?<!:)";
    let dotted = r"(?=[\w\-.~:\/?#@!$&*+;=%]*\.)";
    let undotted = r"(?![\w\-.~:\/?#@!$&*+;=%]*\.)";
    let dotted_spaces = r"(?:(?<!:) (?!\w+:\/\/)(?!\.{0,2}\/)(?!~\/)[\w\-.~:\/?#@!$&*+;=%]*[\/.])*";
    let any_spaces = r"(?:(?<!:) (?!\w+:\/\/)(?!\.{0,2}\/)(?!~\/)[\w\-.~:\/?#@!$&*+;=%]+)*";
    let rooted = r"(?:\.\.\/|\.\/|(?<!\w)~\/|(?:[\w][\w\-.]*\/)*(?<!\w)\$[A-Za-z_]\w*\/|\.[\w][\w\-.]*\/|(?<![\w~\/])\/(?!\/))";
    let scheme_url = format!("(?:{schemes})(?:{ipv6}|{scheme_chars}+{bracketed})+{no_trailing_punctuation}");
    let rooted_path = format!(
        "{rooted}(?:{dotted}{path_chars}+{dotted_spaces}{no_trailing_colon}|{undotted}{path_chars}+{any_spaces}{no_trailing_colon})"
    );
    let bare_path = format!(r"{dotted}(?<!\$\d*)(?<!\w)[\w][\w\-.]*\/{path_chars}+{no_trailing_colon}");
    Regex::new(&format!("{scheme_url}|{rooted_path}|{bare_path}")).expect("link pattern")
});

/// The link in `text` that covers byte `at`.
pub fn find(text: &str, at: usize) -> Option<Range<usize>> {
    for m in PATTERN.find_iter(text) {
        let m = m.ok()?;
        if m.start() > at {
            return None;
        }
        if at < m.end() {
            return Some(m.range());
        }
    }
    None
}

/// The address-shaped word under the pointer, excluding surrounding punctuation.
pub fn word(text: &str, at: usize) -> Option<Range<usize>> {
    word_in(text, at, false)
}

/// A bare file name under the pointer (`plan.md`, `docs\plan.md` on Windows) that
/// is a file in the terminal's folder; the text detector only sees names with a slash.
pub fn bare_file(text: &str, at: usize, cwd: Option<&Path>) -> Option<(Range<usize>, Target)> {
    // A backslash is always part of the word: a separator on Windows, an ordinary
    // character in a file name elsewhere (so `x\note.md` is never read as `note.md`).
    let mut range = word_in(text, at, true)?;
    if cfg!(windows) {
        // The word loses what it trims or cannot hold: `.\` and `..\` name folders, and
        // `C:` names the drive. A rooted path with no drive would mean the pane's own.
        while range.start > 0 && text.as_bytes()[range.start - 1] == b'.' {
            range.start -= 1;
        }
        if text[range.clone()].starts_with(['/', '\\']) {
            let before = text[..range.start].as_bytes();
            let drive = before.len() >= 2 && before[before.len() - 1] == b':' && before[before.len() - 2].is_ascii_alphabetic();
            let word_before = before.len() >= 3 && (before[before.len() - 3].is_ascii_alphanumeric() || before[before.len() - 3] == b'_');
            if !drive || word_before {
                return None;
            }
            range.start -= 2;
        }
    }
    let name = text[range.clone()].rsplit(['/', '\\']).next()?;
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || !(1..=10).contains(&ext.len()) || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    match resolve(&text[range.clone()], cwd)? {
        target @ Target::File(_) if matches!(&target, Target::File(path) if path.is_file()) => Some((range, target)),
        _ => None,
    }
}

fn word_in(text: &str, at: usize, backslash: bool) -> Option<Range<usize>> {
    let allowed = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.') || (backslash && c == '\\');
    if !text.get(at..)?.chars().next().is_some_and(allowed) {
        return None;
    }
    let start = text[..at].char_indices().rev().find(|(_, c)| !allowed(*c)).map_or(0, |(i, c)| i + c.len_utf8());
    let end = text[at..].char_indices().find(|(_, c)| !allowed(*c)).map_or(text.len(), |(i, _)| at + i);
    let trimmed = text[start..end].trim_matches('.');
    let start = start + text[start..end].len() - text[start..end].trim_start_matches('.').len();
    let end = start + trimmed.len();
    (start <= at && at < end).then_some(start..end)
}

/// Bare addresses refer to the hovered terminal's machine, never its names.
pub fn terminal_address<'a>(word: &str, source: &str, live: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let address = if word.contains('/') {
        word.to_owned()
    } else {
        crate::mirror::qualify(word, source.split_once('/').map(|(machine, _)| machine))
    };
    live.into_iter().any(|a| a == address).then_some(address)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Url(String),
    File(PathBuf),
}

/// What a link opens. File paths count only when they exist here; relative
/// ones need the terminal's folder.
pub fn resolve(link: &str, cwd: Option<&Path>) -> Option<Target> {
    resolve_with_line(link, cwd).map(|(target, _)| target)
}

/// `resolve`, and the line a `:12` or `:12:5` suffix names in the file (the column is dropped).
/// A relative path missing from the folder is tried from the git root above it.
pub fn resolve_with_line(link: &str, cwd: Option<&Path>) -> Option<(Target, Option<u32>)> {
    if link.get(..5).is_some_and(|s| s.eq_ignore_ascii_case("file:")) {
        return file_uri_path(link).and_then(|path| resolve_with_line(&path, cwd));
    }
    // `file.rs:12` looks like a scheme ("file.rs:") but is a file and a line: a dotted name before
    // a colon and a digit is a path.
    let scheme = link.split_once(':').is_some_and(|(s, rest)| {
        s.len() > 1
            && s.starts_with(|c: char| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            && !(s.contains('.') && rest.starts_with(|c: char| c.is_ascii_digit()))
    });
    if scheme {
        return Some((Target::Url(link.to_owned()), None));
    }
    let home = std::env::home_dir();
    let expanded = if let Some(rest) = link.strip_prefix("~/") {
        home?.join(rest)
    } else if let Some(var) = link.strip_prefix('$') {
        let (name, rest) = var.split_once('/').unwrap_or((var, ""));
        let base = match name {
            "PWD" => cwd?.to_path_buf(),
            "HOME" => home?,
            _ => PathBuf::from(std::env::var_os(name)?),
        };
        base.join(rest)
    } else {
        PathBuf::from(link)
    };
    let relative = !expanded.is_absolute();
    let full = if relative { cwd?.join(&expanded) } else { expanded.clone() };
    // Checking a share path for existence already reaches out to that machine.
    if network(Path::new(link)) || network(&full) {
        tracing::warn!("refusing network path link {full:?}");
        return None;
    }
    if let Some((path, line)) = existing(&full) {
        return Some((Target::File(path), line));
    }
    if relative && let Some(root) = cwd.and_then(git_root) {
        let full = root.join(&expanded);
        if !network(&full) && let Some((path, line)) = existing(&full) {
            return Some((Target::File(path), line));
        }
    }
    None
}

/// `full` if it exists, else the path left after the period ending a sentence and a
/// `:42` / `:42:7` naming a place in the file, with that line.
fn existing(full: &Path) -> Option<(PathBuf, Option<u32>)> {
    // Windows ignores a trailing period when checking whether a path exists.
    if !(cfg!(windows) && full.extension().is_some_and(|ext| ext.is_empty())) && full.exists() {
        return Some((full.to_path_buf(), None));
    }
    let mut bare = full.to_str()?.strip_suffix('.').unwrap_or(full.to_str()?);
    let mut line = None;
    for _ in 0..2 {
        match bare.rsplit_once(':') {
            Some((head, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
                bare = head;
                line = n.parse().ok();
            }
            _ => break,
        }
    }
    Path::new(bare).exists().then(|| (PathBuf::from(bare), line))
}

/// The nearest folder above `cwd` (or itself) holding `.git`, a folder or a file. No git is run.
fn git_root(cwd: &Path) -> Option<PathBuf> {
    if network(cwd) {
        return None;
    }
    cwd.ancestors().find(|a| a.join(".git").exists()).map(Path::to_path_buf)
}

/// Where a Windows drive path (`C:\a b\c.rs`, `C:/x/y.rs`) starts at or before byte `at`, and the
/// ends it may have, longest first: the end of the text or just before a blank (a path may hold
/// spaces, so the caller keeps the longest that exists). Not after a slash or `?`, which
/// would make it the tail of a UNC, NT or device path.
fn drive_path_ends(text: &str, at: usize) -> Option<(usize, Vec<usize>)> {
    let b = text.as_bytes();
    if b.len() < 3 {
        return None;
    }
    let start = (0..=at.min(b.len().saturating_sub(3))).rev().find(|&i| {
        b[i].is_ascii_alphabetic()
            && b[i + 1] == b':'
            && matches!(b[i + 2], b'\\' | b'/')
            && (i == 0 || text[..i].chars().next_back().is_some_and(|c| c.is_whitespace() || "\"'`([<{=>|,;".contains(c)))
    })?;
    // Blanks end a candidate; at most this many words after the first.
    const WORDS: usize = 8;
    let mut ends: Vec<usize> = text[start..].match_indices(' ').map(|(i, _)| start + i).filter(|&e| e > at && !text[start..e].ends_with(' ')).take(WORDS).collect();
    ends.push(text.trim_end().len());
    ends.retain(|&e| e > at && e > start + 3);
    ends.sort_unstable();
    ends.dedup();
    ends.reverse();
    Some((start, ends))
}

/// A Windows drive path under byte `at` that exists: the longest of the words after the drive
/// that names a file (a trailing `,` `)` and the like dropped, a `:12` line kept apart).
pub fn drive_file(text: &str, at: usize, cwd: Option<&Path>) -> Option<(Range<usize>, Target, Option<u32>)> {
    if !cfg!(windows) {
        return None;
    }
    let (start, ends) = drive_path_ends(text, at)?;
    for end in ends {
        let whole = &text[start..end];
        let trimmed = whole.trim_end_matches([',', ';', ')', ']', '\'', '"', '`']);
        for slice in [whole, trimmed] {
            if slice.len() > 3
                && let Some((target @ Target::File(_), line)) = resolve_with_line(slice, cwd)
            {
                return Some((start..start + slice.len(), target, line));
            }
        }
    }
    None
}

/// The local path a `file:` URI names: no host or `localhost`, percent-decoded.
/// Any other host is another machine and is refused.
fn file_uri_path(uri: &str) -> Option<String> {
    let rest = uri.get(5..)?.strip_prefix("//")?;
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
        tracing::warn!("refusing file link to another machine {host:?}");
        return None;
    }
    let path = path.split(['?', '#']).next()?;
    let raw = path.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' {
            let hex = raw.get(i + 1..i + 3)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            bytes.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    // `/C:/dir/file` is a drive path on Windows.
    if cfg!(windows)
        && let Some(drive) = path.strip_prefix('/')
        && drive.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && drive.as_bytes().get(1) == Some(&b':')
    {
        return Some(drive.to_owned());
    }
    Some(path)
}

/// UNC, `//host`, device and NT object paths (`\??\UNC\host\…`) in any slash
/// mix, which can name another machine. On Windows any prefix but a drive
/// letter is refused too.
pub(crate) fn network(path: &Path) -> bool {
    let text = path.as_os_str().to_string_lossy();
    let slash = ['/', '\\'];
    let unc_or_nt = text
        .strip_prefix(slash)
        .is_some_and(|rest| rest.starts_with(slash) || rest.strip_prefix("??").is_some_and(|rest| rest.starts_with(slash)));
    #[cfg(windows)]
    if let Some(std::path::Component::Prefix(prefix)) = path.components().next() {
        return unc_or_nt || !matches!(prefix.kind(), std::path::Prefix::Disk(_));
    }
    unc_or_nt
}

/// Only web and mail links reach the system opener: any other scheme can name
/// a local program or a protocol handler that starts one.
fn web_or_mail(url: &str) -> bool {
    // Invisible and bidi format characters disguise where a link goes.
    let hidden = |c: char| {
        matches!(c, '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
    };
    // Explorer splits its argument on commas; a properly encoded URL carries
    // these only percent-encoded.
    let unencoded = |c: char| matches!(c, ',' | '"' | '\\' | '`');
    if url.chars().any(|c| c.is_whitespace() || c.is_control() || hidden(c) || unencoded(c)) {
        return false;
    }
    url.split_once(':').is_some_and(|(scheme, rest)| {
        scheme.eq_ignore_ascii_case("mailto")
            || ((scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) && rest.starts_with("//"))
    })
}

/// Extensions that open in HyperOx's editor (scripts too: they are read, never run).
const TEXT: &[&str] = &[
    "txt", "md", "markdown", "json", "jsonc", "toml", "yaml", "yml", "rs", "js", "mjs", "cjs", "ts", "tsx", "jsx", "py", "go", "java", "kt", "c", "h", "cpp",
    "hpp", "cs", "rb", "php", "sh", "bash", "zsh", "fish", "html", "htm", "css", "scss", "xml", "csv", "log", "ini", "cfg", "conf",
];

/// Executables, installers and shortcuts: shown in their folder, never opened.
const PROGRAMS: &[&str] = &[
    "exe", "bat", "cmd", "com", "ps1", "psm1", "vbs", "vbe", "wsf", "msi", "msp", "lnk", "scr", "reg", "app", "dmg", "pkg", "deb", "rpm", "appimage", "jar", "url",
    "hta", "jse", "wsh", "wsc", "pif", "cpl", "msc", "inf", "scf", "application", "appx", "msix", "command", "run",
];

/// What a click on a file does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Opens in HyperOx's editor.
    Text,
    /// Opens in the system's default app.
    System,
    /// Shown in its folder: folders, programs and files without a plain extension.
    Reveal,
}

pub fn kind(path: &Path) -> Kind {
    fn by_extension(path: &Path) -> Kind {
        // An extension with anything but letters and digits (`a.txt:stream`, `a.`) is not one.
        let Some(ext) = path.extension().and_then(|e| e.to_str()).filter(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_alphanumeric())) else {
            return Kind::Reveal;
        };
        let ext = ext.to_ascii_lowercase();
        if PROGRAMS.contains(&ext.as_str()) {
            Kind::Reveal
        } else if TEXT.contains(&ext.as_str()) {
            Kind::Text
        } else {
            Kind::System
        }
    }
    // Windows ignores a trailing dot or space, so `a.exe.` is `a.exe`.
    let trailing = path.file_name().and_then(|n| n.to_str()).is_none_or(|n| n.ends_with(['.', ' ']));
    if trailing || path.is_dir() {
        return Kind::Reveal;
    }
    let kind = by_extension(path);
    // A link named like a document can lead to a program.
    match std::fs::canonicalize(path) {
        Ok(real) if by_extension(&real) == Kind::Reveal => Kind::Reveal,
        Ok(_) => kind,
        Err(_) => Kind::Reveal,
    }
}

/// How a target reaches the system.
enum Opener {
    /// A system command: xdg-open or open, or explorer showing a file.
    Command(std::process::Command),
    /// A web or mail link for Windows' default handler. Explorer would read
    /// `=` in a query as its own arguments; the handler gets the literal URL.
    #[cfg(windows)]
    Shell(String),
}

/// How the system is handed a target, or `None` when it could start a program.
/// Files are only ever shown in their folder, never opened.
fn opener(target: &Target) -> Option<Opener> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer.exe"
    } else {
        "xdg-open"
    };
    let mut command = std::process::Command::new(opener);
    match target {
        Target::Url(url) if web_or_mail(url) => {
            #[cfg(windows)]
            return Some(Opener::Shell(url.clone()));
            #[cfg(not(windows))]
            command.arg(url);
        }
        Target::Url(url) => {
            tracing::warn!("refusing to open link {url:?}: only http, https and mailto links open");
            return None;
        }
        Target::File(path) if network(path) => {
            tracing::warn!("refusing network path link {path:?}");
            return None;
        }
        Target::File(path) => {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                // Explorer parses `/select,"path"` itself, not as one quoted argument.
                let mut select = std::ffi::OsString::from("/select,\"");
                select.push(path.components().collect::<PathBuf>());
                select.push("\"");
                command.raw_arg(select);
            }
            #[cfg(target_os = "macos")]
            command.arg("-R").arg(path);
            #[cfg(not(any(windows, target_os = "macos")))]
            command.arg(path.parent().unwrap_or(path));
        }
    }
    Some(Opener::Command(command))
}

/// Hands a web or mail link to the system's default handler, or shows a file
/// selected in its folder.
pub fn open(target: &Target) {
    match opener(target) {
        None => {}
        Some(Opener::Command(command)) => spawn(command),
        #[cfg(windows)]
        Some(Opener::Shell(url)) => open_shell(std::ffi::OsStr::new(&url)),
    }
}

/// Opens a file in the system's default app. Only for `Kind::System` files.
pub fn open_default(path: &Path) {
    if network(path) {
        tracing::warn!("refusing network path link {path:?}");
        return;
    }
    #[cfg(windows)]
    open_shell(path.components().collect::<PathBuf>().as_os_str());
    #[cfg(not(windows))]
    {
        let mut command = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" });
        command.arg(path);
        spawn(command);
    }
}

/// A text file the editor may not read: the system opens it, except scripts,
/// which their handler could run (`.js`, `.py`): those are shown in their folder.
pub fn open_text_elsewhere(path: &Path) {
    const SCRIPTS: &[&str] = &["js", "mjs", "cjs", "py", "sh", "bash", "zsh", "fish", "rb", "php"];
    let script = path.extension().and_then(|e| e.to_str()).is_some_and(|e| SCRIPTS.iter().any(|s| e.eq_ignore_ascii_case(s)));
    if script {
        open(&Target::File(path.to_path_buf()));
    } else {
        open_default(path);
    }
}

fn spawn(mut command: std::process::Command) {
    let spawned = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        // Reaped off the UI thread so it never lingers as a zombie.
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(e) => tracing::warn!("cannot run {:?}: {e}", command.get_program()),
    }
}

/// Opens a web or mail link (already checked by `web_or_mail`) or a file
/// (`Kind::System`) with its registered handler, without a command line in
/// between. Neither has a NUL.
#[cfg(windows)]
fn open_shell(target: &std::ffi::OsStr) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    };

    let wide: Vec<u16> = target.encode_wide().chain([0]).collect();
    // ShellExecuteW can pump messages; never reenter GPUI while it handles a click.
    std::thread::spawn(move || {
        let initialized = unsafe { CoInitializeEx(std::ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32) };
        let result = unsafe {
            ShellExecuteW(std::ptr::null_mut(), std::ptr::null(), wide.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
        } as isize;
        if result <= 32 {
            tracing::warn!("cannot open link: ShellExecuteW returned {result}");
        }
        if initialized >= 0 {
            unsafe { CoUninitialize() };
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(text: &str) -> Option<&str> {
        PATTERN.find(text).unwrap().map(|m| m.as_str())
    }

    #[test]
    fn a_suffix_names_the_line() {
        let dir = std::env::temp_dir().join(format!("h0x-links-line-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("file.rs");
        std::fs::write(&file, "x\n").unwrap();
        std::fs::write(dir.join("README.md"), "x\n").unwrap();
        let line = |link: &str| resolve_with_line(link, Some(&dir)).map(|(t, l)| (t, l));
        assert_eq!(line("file.rs:12"), Some((Target::File(file.clone()), Some(12))));
        assert_eq!(line("file.rs:12:5"), Some((Target::File(file.clone()), Some(12))));
        assert_eq!(line("file.rs:12."), Some((Target::File(file.clone()), Some(12))));
        assert_eq!(line("file.rs"), Some((Target::File(file.clone()), None)));
        assert_eq!(line("README.md"), Some((Target::File(dir.join("README.md")), None)));
        assert_eq!(line("file.rs:").and_then(|(_, l)| l), None, "a colon alone names no line");
        assert_eq!(line("file.rs:x"), None);
        assert_eq!(resolve("file.rs:12", Some(&dir)), Some(Target::File(file.clone())), "resolve still drops the suffix");
        if cfg!(windows) {
            let abs = format!("{}:7", file.display());
            assert_eq!(resolve_with_line(&abs, None), Some((Target::File(file.clone()), Some(7))), "a drive colon is no line");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_relative_path_is_tried_from_the_git_root() {
        let dir = std::env::temp_dir().join(format!("h0x-links-root-{}", std::process::id()));
        let sub = dir.join("crates").join("app");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join(".git"), "gitdir: elsewhere\n").unwrap();
        std::fs::write(dir.join("src").join("x.rs"), "x\n").unwrap();
        std::fs::write(sub.join("near.rs"), "x\n").unwrap();
        assert_eq!(git_root(&sub), Some(dir.clone()), ".git may be a file");
        assert_eq!(resolve("src/x.rs", Some(&sub)), Some(Target::File(dir.join("src").join("x.rs"))));
        assert_eq!(resolve_with_line("src/x.rs:3", Some(&sub)).and_then(|(_, l)| l), Some(3));
        assert_eq!(resolve("near.rs", Some(&sub)), Some(Target::File(sub.join("near.rs"))), "the folder's own file wins");
        assert_eq!(resolve("src/none.rs", Some(&sub)), None);
        std::fs::remove_file(dir.join(".git")).unwrap();
        std::fs::create_dir(dir.join(".git")).unwrap();
        assert_eq!(git_root(&sub), Some(dir.clone()), ".git may be a folder");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn drive_paths_start_at_a_drive_and_never_inside_unc_or_nt_paths() {
        let ends = |t: &str, at: usize| drive_path_ends(t, at);
        let t = r"see C:\a b\c.rs now";
        let (start, e) = ends(t, 5).unwrap();
        assert_eq!(start, 4);
        assert_eq!(e.iter().map(|&e| &t[start..e]).collect::<Vec<_>>(), [r"C:\a b\c.rs now", r"C:\a b\c.rs", r"C:\a"]);
        // Hovering before the end of a shorter candidate drops the candidates left of it.
        assert_eq!(ends(t, 12).unwrap().1.iter().map(|&e| &t[start..e]).collect::<Vec<_>>(), [r"C:\a b\c.rs now", r"C:\a b\c.rs"]);
        let (start, e) = ends("C:/x/y.rs", 0).unwrap();
        assert_eq!((start, e), (0, vec![9]));
        assert_eq!(ends(r"open (D:\proj\file.md)", 12).unwrap().0, 6);
        assert_eq!(ends(r"\\?\C:\x\y.rs", 5), None, "NT path");
        assert_eq!(ends(r"\\host\share\C:\x", 15), None, "UNC");
        assert_eq!(ends("//host/C:/x", 8), None);
        assert_eq!(ends(r"\\.\C:\x", 5), None, "device path");
        assert_eq!(ends(r"abC:\x", 3), None, "a drive letter is a letter on its own");
        assert_eq!(ends("C:", 0), None);
        assert_eq!(ends("plain text", 3), None);
    }

    #[test]
    #[cfg(windows)]
    fn a_drive_path_with_spaces_is_found_when_it_exists() {
        let dir = std::env::temp_dir().join(format!("h0x links drive {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("c d.rs");
        std::fs::write(&file, "x\n").unwrap();
        let shown = format!("error in {}:12, see above", file.display());
        let at = shown.find("links").unwrap();
        let (range, target, line) = drive_file(&shown, at, None).unwrap();
        assert_eq!(target, Target::File(file.clone()));
        assert_eq!(line, Some(12));
        assert_eq!(&shown[range], format!("{}:12", file.display()));
        let slashes = format!("{}", file.display()).replace('\\', "/");
        assert_eq!(drive_file(&slashes, 3, None).map(|(_, t, _)| t), Some(Target::File(file.clone())));
        assert_eq!(drive_file(r"C:\no\such\file.rs", 3, None), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Ghostty's own cases (`src/config/url.zig`).
    #[test]
    fn matches_like_ghostty() {
        let cases = [
            ("hello https://example.com world", "https://example.com"),
            ("https://example.com/foo(bar) more", "https://example.com/foo(bar)"),
            ("https://example.com/foo(bar)baz more", "https://example.com/foo(bar)baz"),
            ("Link inside (https://example.com) parens", "https://example.com"),
            ("Link period https://example.com. More text.", "https://example.com"),
            ("Link trailing comma https://example.com, more text.", "https://example.com"),
            ("Link in double quotes \"https://example.com\" and more", "https://example.com"),
            ("Link in single quotes 'https://example.com' and more", "https://example.com"),
            ("some file with https://google.com https://duckduckgo.com links.", "https://google.com"),
            ("and links in it. links https://yahoo.com mailto:test@example.com ssh://1.2.3.4", "https://yahoo.com"),
            ("also match http://example.com non-secure links", "http://example.com"),
            ("match tel://+12123456789 phone numbers", "tel://+12123456789"),
            ("match with query url https://example.com?query=1&other=2 and more text.", "https://example.com?query=1&other=2"),
            (
                "url with dashes [mode 2027](https://github.com/contour-terminal/terminal-unicode-core) for better unicode support",
                "https://github.com/contour-terminal/terminal-unicode-core",
            ),
            ("dot.http://example.com", "http://example.com"),
            ("weird characters https://example.com/~user/?query=1&other=2#hash and more", "https://example.com/~user/?query=1&other=2#hash"),
            ("square brackets https://example.com/[foo] and more", "https://example.com/[foo]"),
            (
                "[13]:TooManyStatements: TempFile#assign_temp_file_to_entity has approx 7 statements [https://example.com/docs/Too-Many-Statements.md]",
                "https://example.com/docs/Too-Many-Statements.md",
            ),
            ("match ftp://example.com ftp links", "ftp://example.com"),
            ("match file://example.com file links", "file://example.com"),
            ("match ssh://example.com ssh links", "ssh://example.com"),
            ("match git://example.com git links", "git://example.com"),
            ("/tmp/test.txt http://www.google.com", "/tmp/test.txt"),
            ("match tel:+18005551234 tel links", "tel:+18005551234"),
            ("match magnet:?xt=urn:btih:1234567890 magnet links", "magnet:?xt=urn:btih:1234567890"),
            ("match ipfs://QmSomeHashValue ipfs links", "ipfs://QmSomeHashValue"),
            ("match ipns://QmSomeHashValue ipns links", "ipns://QmSomeHashValue"),
            ("match gemini://example.com gemini links", "gemini://example.com"),
            ("match gopher://example.com gopher links", "gopher://example.com"),
            ("match news:comp.infosystems.www.servers.unix news links", "news:comp.infosystems.www.servers.unix"),
            ("/Users/ghostty.user/code/example.py", "/Users/ghostty.user/code/example.py"),
            ("/Users/ghostty.user/code/../example.py", "/Users/ghostty.user/code/../example.py"),
            ("/Users/ghostty.user/code/../example.py hello world", "/Users/ghostty.user/code/../example.py"),
            ("../example.py", "../example.py"),
            ("../example.py ", "../example.py"),
            ("first time ../example.py contributor ", "../example.py"),
            ("[link](/home/user/ghostty.user/example)", "/home/user/ghostty.user/example"),
            ("Serving HTTP on :: port 8000 (http://[::]:8000/)", "http://[::]:8000/"),
            ("IPv6 address https://[2001:db8::1]:8080/path", "https://[2001:db8::1]:8080/path"),
            ("IPv6 localhost http://[::1]:3000", "http://[::1]:3000"),
            ("Complex IPv6 https://[2001:db8:85a3:8d3:1319:8a2e:370:7348]:443/", "https://[2001:db8:85a3:8d3:1319:8a2e:370:7348]:443/"),
            ("IPv6 with path https://[2001:db8::1]/path/to/resource", "https://[2001:db8::1]/path/to/resource"),
            ("IPv6 with query https://[2001:db8::1]:8080/api?param=value&other=123", "https://[2001:db8::1]:8080/api?param=value&other=123"),
            ("IPv6 compressed http://[2001:db8::]:80/", "http://[2001:db8::]:80/"),
            ("IPv6 multiple zeros http://[2001:0:0:0:0:0:0:1]", "http://[2001:0:0:0:0:0:0:1]"),
            ("IPv6 link-local https://[fe80::1234:5678:9abc]", "https://[fe80::1234:5678:9abc]"),
            ("IPv6 multicast http://[ff02::1]/stream", "http://[ff02::1]/stream"),
            ("IPv6 in markdown [link](http://[2001:db8::1]/docs)", "http://[2001:db8::1]/docs"),
            ("./spaces-end.   ", "./spaces-end."),
            ("./space middle", "./space middle"),
            ("../test folder/file.txt", "../test folder/file.txt"),
            ("/tmp/test folder/file.txt", "/tmp/test folder/file.txt"),
            ("/tmp/test  folder/file.txt", "/tmp/test"),
            ("diff --git a/src/font/shaper/harfbuzz.zig b/src/font/shaper/harfbuzz.zig", "a/src/font/shaper/harfbuzz.zig"),
            ("/tmp/foo /tmp/bar", "/tmp/foo"),
            ("/tmp/foo.txt /tmp/bar.txt", "/tmp/foo.txt"),
            ("src/config/url.zig", "src/config/url.zig"),
            ("app/folder/file.rb:1", "app/folder/file.rb:1"),
            ("modified:   src/config/url.zig", "src/config/url.zig"),
            ("lib/ghostty/terminal.zig:42:10", "lib/ghostty/terminal.zig:42:10"),
            ("some-pkg/src/file.txt more text", "some-pkg/src/file.txt"),
            ("src/foo.c,baz.txt", "src/foo.c"),
            ("~/foo/bar.txt", "~/foo/bar.txt"),
            ("open ~/Documents/notes.md please", "~/Documents/notes.md"),
            ("~/.config/ghostty/config", "~/.config/ghostty/config"),
            ("directory: ~/src/ghostty-org/ghostty", "~/src/ghostty-org/ghostty"),
            ("$HOME/src/config/url.zig", "$HOME/src/config/url.zig"),
            ("space dir: $PWD/src/ghostty/main.zig", "$PWD/src/ghostty/main.zig"),
            ("foo/$BAR/baz", "foo/$BAR/baz"),
            (".foo/bar/$VAR", ".foo/bar/$VAR"),
            (".config/ghostty/config", ".config/ghostty/config"),
            ("loaded from .local/share/ghostty/state.db now", ".local/share/ghostty/state.db"),
            ("../some/where", "../some/where"),
            ("  - shared/src/foo/SomeItem.m:12, shared/src/", "shared/src/foo/SomeItem.m:12"),
            ("foo.local/share", "foo.local/share"),
            ("2024/report.txt", "2024/report.txt"),
            ("./foo bar,baz", "./foo bar"),
            ("/tmp/foo bar,baz", "/tmp/foo bar"),
            ("./.config/ghostty: Needs upstream (main)", "./.config/ghostty"),
            ("./Downloads: Operation not permitted", "./Downloads"),
        ];
        for (input, expect) in cases {
            assert_eq!(first(input), Some(expect), "{input:?}");
        }
        for input in ["input/output", "foo/bar", "$10/bar", "$10/$20", "$10/bar.txt", "foo/bar,baz.txt", "foo$BAR/baz.txt", "foo~/bar.txt", "// foo bar", "//foo"] {
            assert_eq!(first(input), None, "{input:?}");
        }
    }

    #[test]
    fn finds_the_link_under_the_pointer() {
        let text = "see https://a.com and /tmp/x.txt.";
        assert_eq!(find(text, 0), None);
        assert_eq!(find(text, 4).map(|r| &text[r]), Some("https://a.com"));
        assert_eq!(find(text, 16).map(|r| &text[r]), Some("https://a.com"));
        assert_eq!(find(text, 17), None);
        assert_eq!(find(text, 25).map(|r| &text[r]), Some("/tmp/x.txt."));
    }

    #[test]
    fn extracts_address_words_at_the_pointer() {
        for (text, expected) in [
            ("<t1hg>", "t1hg"),
            ("[\"macbook/t2w\"]", "macbook/t2w"),
            ("terminal=t1hg,", "t1hg"),
            ("'my-machine_2/t2w'.", "my-machine_2/t2w"),
            ("before t1hg after", "t1hg"),
            ("é t1hg", "t1hg"),
            ("<build.box/t1>.", "build.box/t1"),
            ("1.2.3.4/t1", "1.2.3.4/t1"),
            ("...t1hg.", "t1hg"),
        ] {
            let at = text.find(expected).unwrap();
            for (i, _) in expected.char_indices() {
                assert_eq!(word(text, at + i).map(|r| &text[r]), Some(expected), "{text}");
            }
        }
        for (text, at) in [("<t1>", 0), ("t1 ", 2), ("", 0), ("t1", 2), ("é", 1)] {
            assert_eq!(word(text, at), None);
        }
    }

    #[test]
    fn matches_only_live_addresses_relative_to_the_source_machine() {
        let live = ["t1", "t2", "macbook/t1", "macbook/t2w", "my-machine_2/t3", "build.box/t1", "box/t1"];
        for (word, source, expected) in [
            ("t1", "t2", Some("t1")),
            ("t1", "macbook/t2w", Some("macbook/t1")),
            ("t1", "build.box/t2", Some("build.box/t1")),
            ("t2", "macbook/t1", None),
            ("macbook/t2w", "t1", Some("macbook/t2w")),
            ("my-machine_2/t3", "macbook/t1", Some("my-machine_2/t3")),
            ("t99", "t1", None),
            ("Build", "t1", None),
            ("t", "t1", None),
            ("other/t1", "t1", None),
            ("macbook/t1/extra", "t1", None),
        ] {
            assert_eq!(terminal_address(word, source, live), expected.map(str::to_owned), "{word} in {source}");
        }
    }

    #[test]
    fn dotted_machines_never_match_a_different_suffix_machine() {
        let text = "<build.box/t1>.";
        let range = word(text, text.find("t1").unwrap()).unwrap();
        assert_eq!(&text[range.clone()], "build.box/t1");
        assert_eq!(terminal_address(&text[range.clone()], "t2", ["box/t1"]), None);
        assert_eq!(terminal_address(&text[range], "t2", ["box/t1", "build.box/t1"]), Some("build.box/t1".into()));
    }

    #[test]
    fn files_resolve_only_when_they_exist() {
        let dir = std::env::temp_dir().join(format!("h0x-links-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        let file = Target::File(dir.join("src/main.rs"));
        assert_eq!(resolve("src/main.rs", Some(&dir)), Some(file.clone()));
        assert_eq!(resolve("src/main.rs:42:7", Some(&dir)), Some(file.clone()));
        assert_eq!(resolve("src/main.rs.", Some(&dir)), Some(file.clone()));
        assert_eq!(resolve("src/.", Some(&dir)), Some(Target::File(dir.join("src/."))));
        assert_eq!(resolve("src/..", Some(&dir)), Some(Target::File(dir.join("src/.."))));
        assert_eq!(resolve("$PWD/src/main.rs", Some(&dir)), Some(file.clone()));
        assert_eq!(resolve(dir.join("src/main.rs").to_str().unwrap(), None), Some(file));
        assert_eq!(resolve("src/gone.rs", Some(&dir)), None);
        assert_eq!(resolve("src/main.rs", None), None);
        assert_eq!(resolve("https://a.com/x", None), Some(Target::Url("https://a.com/x".into())));
        #[cfg(unix)]
        {
            let dotted = dir.join("src/main.rs.");
            std::fs::write(&dotted, "").unwrap();
            assert_eq!(resolve("src/main.rs.", Some(&dir)), Some(Target::File(dotted)));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn opener_args(target: &Target) -> Option<Vec<String>> {
        opener(target).map(|o| match o {
            Opener::Command(c) => c.get_args().map(|a| a.to_string_lossy().into_owned()).collect(),
            #[cfg(windows)]
            Opener::Shell(url) => vec![url],
        })
    }

    #[test]
    fn query_links_keep_their_full_target() {
        for url in ["https://example.com/?a=1&b=2", "https://localhost:17480/login?t=abc%3D%3D"] {
            let text = format!("Dashboard: {url}");
            let range = find(&text, text.find(url).unwrap()).unwrap();
            assert_eq!(&text[range.clone()], url);
            assert_eq!(resolve(&text[range], None), Some(Target::Url(url.into())));
        }
        // A file: link is never a Url: it names a local file that exists, or nothing.
        let winver = "file:///C:/Windows/System32/winver.exe";
        let text = format!("Dashboard: {winver}");
        let range = find(&text, text.find(winver).unwrap()).unwrap();
        assert_eq!(&text[range.clone()], winver);
        let expected = (cfg!(windows) && Path::new("C:/Windows/System32/winver.exe").exists()).then(|| Target::File(PathBuf::from("C:/Windows/System32/winver.exe")));
        assert_eq!(resolve(&text[range], None), expected);
        // The whole query reaches the handler; a file: link still never does.
        for url in ["https://example.com/?a=1&b=2", "https://localhost:17480/login?t=abc%3D%3D"] {
            assert_eq!(opener_args(&Target::Url(url.into())), Some(vec![url.to_owned()]), "{url}");
        }
        assert_eq!(opener_args(&Target::Url("file:///C:/Windows/System32/winver.exe".into())), None);
    }

    #[test]
    fn only_web_and_mail_links_reach_the_opener() {
        for url in ["https://a.com/x", "HTTP://A.COM", "Https://a.com", "mailto:a@b.c", "MAILTO:a@b.c"] {
            assert_eq!(opener_args(&Target::Url(url.into())), Some(vec![url.to_owned()]), "{url}");
        }
        for url in [
            "",
            "javascript:alert(1)",
            "file:///C:/Windows/System32/calc.exe",
            "FILE:///etc/passwd",
            "smb://server/share/a.bat",
            "ms-settings:",
            "ms-msdt:/id",
            "search-ms:query=x",
            "vscode://file/x",
            "ftp://a.com/x",
            "http:a.com",
            "https:\\\\server\\share",
            "\\\\server\\share\\a.bat",
            "https://a.com/x y",
            "https://a.com/x\ty",
            "https://a.com/\u{7}",
            "https://a.com/\n",
            "mailto:a@b.c\r\n",
            " https://a.com",
        ] {
            assert_eq!(opener_args(&Target::Url(url.into())), None, "{url:?}");
        }
    }

    #[test]
    fn links_with_invisible_or_bidi_characters_never_reach_the_opener() {
        assert!(web_or_mail("https://a.com/gpj.exe"));
        for c in ['\u{200B}', '\u{200F}', '\u{2028}', '\u{2029}', '\u{202A}', '\u{202E}', '\u{2060}', '\u{2064}', '\u{2066}', '\u{2069}', '\u{FEFF}'] {
            for url in [format!("https://a.com/{c}gpj.exe"), format!("{c}https://a.com/"), format!("mailto:a{c}@b.c")] {
                assert!(!web_or_mail(&url), "{url:?}");
                assert_eq!(opener_args(&Target::Url(url.clone())), None, "{url:?}");
            }
        }
        assert!(!web_or_mail("https://a.com/\u{202E}gpj.exe"));
        assert!(!web_or_mail("https://a\u{200B}.com/"));
        // Just outside the refused ranges.
        for c in ['\u{200A}', '\u{2010}', '\u{2027}', '\u{202F}', '\u{2065}', '\u{206A}', '\u{FEFE}'] {
            let url = format!("https://a.com/{c}");
            assert_eq!(web_or_mail(&url), !c.is_whitespace() && !c.is_control(), "{url:?}");
        }
    }

    #[test]
    fn files_are_shown_in_their_folder_never_opened() {
        let dir = std::env::temp_dir();
        for name in ["y.exe", "a.bat", "a.cmd", "a.ps1", "a.sh", "a.lnk", "A.app", "notes.txt"] {
            let path = dir.join(name);
            let args = opener_args(&Target::File(path.clone())).unwrap();
            // macOS passes the path after `-R`, which reveals it in Finder; the exact args are asserted below.
            #[cfg(not(target_os = "macos"))]
            assert!(!args.iter().any(|a| Path::new(a) == path.as_path()), "{name}: {args:?}");
            #[cfg(windows)]
            assert_eq!(args, [format!("/select,\"{}\"", path.components().collect::<PathBuf>().display())]);
            #[cfg(target_os = "macos")]
            assert_eq!(args, ["-R".to_owned(), path.display().to_string()]);
            #[cfg(not(any(windows, target_os = "macos")))]
            assert_eq!(args, [path.parent().unwrap().display().to_string()]);
        }
        #[cfg(windows)]
        {
            assert_eq!(opener_args(&Target::File(PathBuf::from(r"C:\x\y.exe"))), Some(vec![r#"/select,"C:\x\y.exe""#.to_owned()]));
            assert_eq!(opener_args(&Target::File(PathBuf::from(r"C:\x/src/y.bat"))), Some(vec![r#"/select,"C:\x\src\y.bat""#.to_owned()]));
        }
    }

    #[test]
    fn network_paths_are_refused() {
        for path in [r"\\server\share\a.bat", "//server/share/a.bat", r"\\?\UNC\server\share\a.exe"] {
            assert_eq!(opener_args(&Target::File(PathBuf::from(path))), None, "{path}");
            assert_eq!(resolve(path, None), None, "{path}");
            assert_eq!(resolve(path, Some(&std::env::temp_dir())), None, "{path}");
        }
        assert_eq!(resolve("", None), None);
    }

    #[test]
    fn spoofed_and_injected_links_are_refused() {
        for url in [
            "mailto:a@b.c\nBcc:evil@x.com",
            "mailto:a@b.c\r\nBcc:evil@x.com",
            "https://a.com/\u{85}",
            "https://a.com/\u{a0}x",
            "https://a.com/\u{2028}x",
            "https://a.com/\u{3000}x",
            "https://a.com/\u{0}",
            "https://a.com/\u{9b}",
            "https:",
            "https:/a.com",
            "https ://a.com",
            "http//a.com",
        ] {
            assert_eq!(opener_args(&Target::Url(url.into())), None, "{url:?}");
        }
        // Hosts are the browser's business: any https link without whitespace opens as given.
        for url in ["https://xn--e1afmkfd.xn--p1ai/", "https://bücher.de/", "https://a.com?q=1", "https://a.com#top", "MailTo:a@b.c?subject=hi%0Athere"] {
            assert_eq!(opener_args(&Target::Url(url.into())), Some(vec![url.to_owned()]), "{url}");
        }
    }

    #[test]
    fn mixed_slash_network_paths_and_remote_folders_are_refused() {
        for path in [r"\/server/share/a.bat", r"/\server\share\a.bat", r"\\.\PIPE\x", "//?/UNC/server/share/a.exe"] {
            assert_eq!(opener_args(&Target::File(PathBuf::from(path))), None, "{path}");
            assert_eq!(resolve(path, None), None, "{path}");
            assert_eq!(resolve(path, Some(&std::env::temp_dir())), None, "{path}");
        }
        // A relative link in a terminal whose folder is a share names the other machine too.
        for cwd in [r"\\server\share\project", "//server/share/project"] {
            for link in ["a.txt", "src/main.rs:42", "$PWD/a.txt"] {
                assert_eq!(resolve(link, Some(Path::new(cwd))), None, "{cwd} {link}");
            }
        }
    }

    #[test]
    fn nt_object_paths_are_refused() {
        for path in [
            r"\??\UNC\host\share\x",
            "/??/UNC/host/x",
            r"\??\GLOBALROOT\Device\x",
            r"/??\UNC\host\share\x",
            r"\??/UNC/host/share/x",
            r"\??\C:\x",
        ] {
            assert!(network(Path::new(path)), "{path}");
            assert_eq!(opener_args(&Target::File(PathBuf::from(path))), None, "{path}");
            assert_eq!(resolve(path, None), None, "{path}");
            assert_eq!(resolve(path, Some(&std::env::temp_dir())), None, "{path}");
            // A remote pane's POSIX folder: the link replaces it whole on Windows.
            assert_eq!(resolve(path, Some(Path::new("/home/u"))), None, "{path}");
            assert!(network(&Path::new("/home/u").join(path)) || cfg!(not(windows)), "{path}");
        }
        // Lookalikes that are ordinary names stay local.
        for path in ["/home/u/??/x", "??/x", "/?/x", r"\?x", "/home/u/src/a.rs", "src/a.rs"] {
            assert!(!network(Path::new(path)), "{path}");
        }
        assert!(!network(&Path::new("/home/u").join("src/a.rs")));
        #[cfg(windows)]
        {
            for path in [r"C:\x\y.txt", "C:/x", "C:x", r"\x\y"] {
                assert!(!network(Path::new(path)), "{path}");
            }
            for path in [r"\\?\C:\x", r"\\?\UNC\host\share\x", r"\\.\C:\x", r"\\?\GLOBALROOT\Device\x", r"\\host\share\x"] {
                assert!(network(Path::new(path)), "{path}");
            }
        }
    }

    #[test]
    fn urls_explorer_would_split_are_refused() {
        for url in ["https://x/,C:/a.cmd", "https://x/\"a", r"https://x/a\b", "https://x/`a`", "mailto:a@b.c,d@e.f"] {
            assert!(!web_or_mail(url), "{url}");
            assert_eq!(opener_args(&Target::Url(url.into())), None, "{url}");
        }
        for url in ["https://x/%2CC:/a.cmd", "https://x/%22a", "https://x/a%5Cb"] {
            assert!(web_or_mail(url), "{url}");
            assert_eq!(opener_args(&Target::Url(url.into())), Some(vec![url.to_owned()]), "{url}");
        }
    }

    #[test]
    fn file_uris_name_a_path_of_this_machine_only() {
        for (uri, path) in [
            ("file:///tmp/a.txt", "/tmp/a.txt"),
            ("FILE:///tmp/a.txt", "/tmp/a.txt"),
            ("File://localhost/tmp/a.txt", "/tmp/a.txt"),
            ("file://LOCALHOST/tmp/a.txt", "/tmp/a.txt"),
            ("file:///tmp/My%20Notes/a%20b.txt", "/tmp/My Notes/a b.txt"),
            ("file:///tmp/a.txt?line=4#L4", "/tmp/a.txt"),
            ("file:///tmp/a.txt#?x", "/tmp/a.txt"),
            // Encoded `#` and `?` are part of the name, decoded only after the query is cut.
            ("file:///tmp/a%23b%3fc.txt", "/tmp/a#b?c.txt"),
            ("file:///tmp/%C3%BC%c3%bc.txt", "/tmp/üü.txt"),
            ("file:///tmp/ü.txt", "/tmp/ü.txt"),
            ("file:///", "/"),
        ] {
            assert_eq!(file_uri_path(uri).as_deref(), Some(path), "{uri}");
        }
        // `/C:/…` is a drive path on Windows only.
        for (uri, drive) in [("file:///C:/Users/a%20b/x.txt", "C:/Users/a b/x.txt"), ("file://localhost/c:/x.txt", "c:/x.txt")] {
            let expected = if cfg!(windows) { drive.to_owned() } else { format!("/{drive}") };
            assert_eq!(file_uri_path(uri), Some(expected), "{uri}");
        }
        for uri in [
            // Another machine, however it is spelled.
            "file://server/share/a.txt",
            "file://evil.com/etc/passwd",
            "file://localhost.evil.com/tmp/a.txt",
            "file://127.0.0.1/tmp/a.txt",
            "file://user@localhost/tmp/a.txt",
            "file://localhost:80/tmp/a.txt",
            "file://localhost?x",
            // Not `file://`.
            "file:",
            "file:/tmp/a.txt",
            "file:tmp/a.txt",
            // No path.
            "file://",
            "file://localhost",
            "file://?/tmp/a.txt",
            // Bad escapes, NUL and bytes that are not UTF-8.
            "file:///tmp/%zz.txt",
            "file:///tmp/%2",
            "file:///tmp/%",
            "file:///tmp/%g0.txt",
            "file:///tmp/a%00.txt",
            "file:///%00",
            "file:///tmp/%ff.txt",
            "file:///tmp/%C3.txt",
            "file:///tmp/%C3%28.txt",
        ] {
            assert_eq!(file_uri_path(uri), None, "{uri}");
            assert_eq!(resolve(uri, Some(&std::env::temp_dir())), None, "{uri}");
        }
    }

    #[test]
    fn file_uris_resolve_to_files_that_exist_here_and_never_to_shares() {
        let dir = std::env::temp_dir().join(format!("h0x-file-uri-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("My Notes")).unwrap();
        let file = dir.join("My Notes").join("a b.txt");
        std::fs::write(&file, "").unwrap();
        // `file:///C:/…` on Windows, `file:///tmp/…` elsewhere.
        let uri = |path: &Path| {
            let text = path.to_str().unwrap().replace('\\', "/").replace(' ', "%20");
            if text.starts_with('/') { format!("file://{text}") } else { format!("file:///{text}") }
        };
        let found = Some(Target::File(file.clone()));
        assert_eq!(resolve(&uri(&file), None), found);
        assert_eq!(resolve(&format!("{}?x=1#top", uri(&file)), None), found);
        assert_eq!(resolve(&uri(&file).replacen("file://", "file://localhost", 1), None), found);
        assert_eq!(resolve(&uri(&file).replacen("file://", "FILE://", 1), Some(Path::new("/elsewhere"))), found);
        assert_eq!(resolve(&uri(&dir), None), Some(Target::File(dir.clone())));
        assert_eq!(resolve(&uri(&dir.join("gone.txt")), None), None);
        assert_eq!(resolve(&uri(&file).replacen("file://", "file://server", 1), None), None);
        // A file: link never becomes a web link handed to the opener.
        assert!(!matches!(resolve(&uri(&file), None), Some(Target::Url(_))));
        // Shares hidden in the path: an empty host, then `//server` or encoded backslashes.
        for uri in [
            "file:////server/share/a.txt",
            "file://localhost//server/share/a.txt",
            "file:///%5C%5Cserver%5Cshare%5Ca.txt",
            "file:///%2F%2Fserver/share/a.txt",
            "file:///%5C%3F%3F%5CUNC%5Cserver%5Cshare%5Ca.txt",
        ] {
            assert_eq!(resolve(uri, None), None, "{uri}");
            assert_eq!(resolve(uri, Some(&dir)), None, "{uri}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn text_files_open_in_the_editor_programs_are_revealed_and_the_rest_go_to_the_system() {
        let dir = std::env::temp_dir().join(format!("h0x-kind-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let touch = |name: &str| {
            let path = dir.join(name);
            std::fs::write(&path, "").unwrap();
            path
        };
        for name in [
            "a.txt", "a.md", "a.markdown", "a.json", "a.jsonc", "Cargo.toml", "a.yaml", "a.yml", "a.rs", "a.js", "a.mjs", "a.cjs", "a.ts", "a.tsx", "a.jsx", "a.py",
            "a.go", "a.java", "a.kt", "a.c", "a.h", "a.cpp", "a.hpp", "a.cs", "a.rb", "a.php", "a.sh", "a.bash", "a.zsh", "a.fish", "a.html", "a.htm", "a.css",
            "a.scss", "a.xml", "a.csv", "a.log", "a.ini", "a.cfg", "a.conf", "UPPER.TXT", "Mixed.Md", "release.notes.rs", "a b.txt",
        ] {
            assert_eq!(kind(&touch(name)), Kind::Text, "{name}");
        }
        // Windows mixed slashes.
        std::fs::write(dir.join("sub").join("b.md"), "").unwrap();
        assert_eq!(kind(&dir.join("sub/b.md")), Kind::Text);
        for ext in PROGRAMS {
            for name in [format!("x.{ext}"), format!("X.{}", ext.to_ascii_uppercase()), format!("notes.txt.{ext}")] {
                assert_eq!(kind(&touch(&name)), Kind::Reveal, "{name}");
            }
        }
        for name in ["Run.Bat", "a.AppImage", "SETUP.Exe", "a.MsI"] {
            assert_eq!(kind(&touch(name)), Kind::Reveal, "{name}");
        }
        for name in ["a.png", "a.PDF", "a.docx", "a.svg", "a.mp4", "a.zip", "a.xlsx", "a.tar.gz"] {
            assert_eq!(kind(&touch(name)), Kind::System, "{name}");
        }
        // No plain extension: none, an empty one, a dotfile, a stream, a lookalike letter.
        for name in ["Makefile", ".bashrc", ".txt", "a.txt:stream", "a.txt::$DATA", "a.еxe", "a.tхt", "a.exe ", "a.txt.", "a.exe.", "a."] {
            let path = dir.join(name);
            let _ = std::fs::write(&path, "");
            assert_eq!(kind(&path), Kind::Reveal, "{name:?}");
        }
        // A folder, even one named like a document, and a file that is gone.
        std::fs::create_dir_all(dir.join("folder.txt")).unwrap();
        assert_eq!(kind(&dir.join("folder.txt")), Kind::Reveal);
        assert_eq!(kind(&dir), Kind::Reveal);
        assert_eq!(kind(&dir.join("gone.txt")), Kind::Reveal);
        assert_eq!(kind(Path::new("")), Kind::Reveal);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_link_named_like_a_document_that_leads_to_a_program_is_only_revealed() {
        let dir = std::env::temp_dir().join(format!("h0x-kind-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for target in ["prog.exe", "setup.MSI", "script"] {
            std::fs::write(dir.join(target), "").unwrap();
        }
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        #[cfg(unix)]
        let link = |target: &str, name: &str| std::os::unix::fs::symlink(dir.join(target), dir.join(name)).is_ok();
        // Symbolic links need Developer Mode or elevation on Windows: skipped without.
        #[cfg(windows)]
        let link = |target: &str, name: &str| {
            if target == "folder" {
                std::os::windows::fs::symlink_dir(dir.join(target), dir.join(name)).is_ok()
            } else {
                std::os::windows::fs::symlink_file(dir.join(target), dir.join(name)).is_ok()
            }
        };
        for (target, name) in [("prog.exe", "notes.txt"), ("setup.MSI", "photo.png"), ("script", "readme.md"), ("folder", "doc.txt")] {
            if link(target, name) {
                assert_eq!(kind(&dir.join(name)), Kind::Reveal, "{name} -> {target}");
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_forward_slash_files_are_selected_with_backslashes() {
        assert_eq!(opener_args(&Target::File(PathBuf::from("C:/x/y.exe"))), Some(vec![r#"/select,"C:\x\y.exe""#.to_owned()]));
        assert_eq!(opener_args(&Target::File(PathBuf::from("C:/My Files/a b.txt"))), Some(vec![r#"/select,"C:\My Files\a b.txt""#.to_owned()]));
    }

    #[cfg(windows)]
    #[test]
    fn only_checked_web_and_mail_links_reach_the_windows_shell_handler() {
        for url in ["https://a.com/?a=1&b=2", "HTTP://A.COM", "mailto:a@b.c"] {
            assert!(matches!(opener(&Target::Url(url.into())), Some(Opener::Shell(u)) if u == url), "{url}");
        }
        for url in [
            "",
            "file:///C:/Windows/System32/calc.exe",
            "FILE://server/share/a.exe",
            r"C:\Windows\System32\calc.exe",
            "C:/x/y.exe",
            r"\\server\share\a.bat",
            r"\\?\C:\x.exe",
            r"\\.\PIPE\x",
            r"\??\C:\x.exe",
            "ms-settings:",
            "https://x/,C:/a.cmd",
            "https://a.com/\u{202E}gpj.exe",
        ] {
            assert!(opener(&Target::Url(url.into())).is_none(), "{url:?}");
        }
        // A file, even one named like a link, is only shown selected by explorer.
        for (path, select) in [
            (r"C:\x\y.exe", r#"/select,"C:\x\y.exe""#),
            ("C:/x/https:/a.com", r#"/select,"C:\x\https:\a.com""#),
            (r"C:\x\mailto:a@b.c", r#"/select,"C:\x\mailto:a@b.c""#),
        ] {
            match opener(&Target::File(PathBuf::from(path))) {
                Some(Opener::Command(c)) => {
                    assert_eq!(c.get_program(), std::ffi::OsStr::new("explorer.exe"), "{path}");
                    let args: Vec<String> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
                    assert_eq!(args, [select.to_owned()], "{path}");
                }
                _ => panic!("{path}: not shown in its folder"),
            }
        }
        for path in [r"\\server\share\a.bat", r"\\?\C:\x.exe", r"\\.\C:\x", r"\\.\COM1", r"\??\C:\x", r"\\?\GLOBALROOT\Device\x"] {
            assert!(opener(&Target::File(PathBuf::from(path))).is_none(), "{path}");
        }
    }

    #[test]
    fn bare_file_names_link_when_they_exist_in_the_folder() {
        let dir = std::env::temp_dir().join(format!("h0x-bare-file-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("plan.md"), "").unwrap();
        std::fs::write(dir.join("docs").join("plan2.md"), "").unwrap();
        std::fs::write(dir.join("my plan.md"), "").unwrap();
        std::fs::write(dir.join("t3"), "").unwrap();
        let hit = |text: &str, at: usize| bare_file(text, at, Some(&dir)).map(|(r, t)| (text[r].to_owned(), t));
        let plan = Target::File(dir.join("plan.md"));
        assert_eq!(hit("see plan.md now", 6), Some(("plan.md".into(), plan.clone())));
        assert_eq!(hit("plan.md,", 2), Some(("plan.md".into(), plan.clone())));
        assert_eq!(hit("(plan.md)", 3), Some(("plan.md".into(), plan)));
        assert_eq!(hit("see missing.md now", 6), None);
        assert_eq!(hit("t3", 0), None, "no extension");
        assert_eq!(hit(".md", 1), None, "leading dot only");
        assert_eq!(hit("plan.verylongext1", 2), None);
        assert_eq!(hit("see plan.md now", 0), None);
        assert_eq!(bare_file("plan.md", 0, None), None);
        // A name with spaces is not joined: only the word under the pointer counts.
        assert_eq!(hit("my plan.md", 0), None);
        assert_eq!(hit("my plan.md", 5), Some(("plan.md".into(), Target::File(dir.join("plan.md")))));
        if cfg!(windows) {
            let text = r"open docs\plan2.md.";
            assert_eq!(hit(text, 8), Some((r"docs\plan2.md".into(), Target::File(dir.join(r"docs\plan2.md")))));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bare_file_names_with_odd_cases_punctuation_and_lengths() {
        let dir = std::env::temp_dir().join(format!("h0x-bare-odd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["UPPER.MD", "café.md", "日本.txt", "archive.tar.gz", "a.abcdefghij", "a.abcdefghijk", "a.m-d", "note.md"] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        let hit = |text: &str, at: usize| bare_file(text, at, Some(&dir)).map(|(r, t)| (text[r].to_owned(), t));
        let file = |name: &str| Some((name.to_owned(), Target::File(dir.join(name))));
        assert_eq!(hit("UPPER.MD", 0), file("UPPER.MD"), "an uppercase extension");
        assert_eq!(hit("open café.md", 6), file("café.md"));
        assert_eq!(hit("日本.txt", 0), file("日本.txt"));
        assert_eq!(hit("x archive.tar.gz y", 4), file("archive.tar.gz"));
        assert_eq!(hit("a.abcdefghij", 0), file("a.abcdefghij"), "a ten-letter extension");
        assert_eq!(hit("a.abcdefghijk", 0), None, "eleven letters is no extension");
        assert_eq!(hit("a.m-d", 0), None, "an extension of letters and digits only");
        for text in ["note.md.", "note.md,", "note.md;", "note.md:", "'note.md'", "`note.md`", "[note.md]", "\"note.md\"", "note.md!"] {
            let at = text.find('n').unwrap();
            assert_eq!(hit(text, at), file("note.md"), "{text:?}");
        }
        assert_eq!(hit("note.md", 7), None, "just past the end");
        assert_eq!(hit("note.md", 99), None, "far past the end");
        assert_eq!(hit("", 0), None, "empty text");
        assert_eq!(hit("note.md", 2), file("note.md"));
        assert_eq!(bare_file("note.md", 0, Some(Path::new(""))), None, "an empty folder name is no folder");
        if !cfg!(windows) {
            // Only Windows reads a backslash as a separator.
            assert_eq!(hit(r"x\note.md", 2), None);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_named_like_a_file_is_not_a_bare_file_link() {
        let dir = std::env::temp_dir().join(format!("h0x-bare-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs.md")).unwrap();
        assert_eq!(bare_file("see docs.md now", 4, Some(&dir)), None, "a folder");
        assert_eq!(bare_file("see docs.md/ now", 4, Some(&dir)), None, "a folder with its slash");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_bare_names_keep_their_drive_and_dot_folders() {
        let dir = std::env::temp_dir().join(format!("h0x-bare-win-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("plan.md"), "").unwrap();
        std::fs::write(dir.join("sub").join("plan.md"), "").unwrap();
        let same = |found: Option<(Range<usize>, Target)>, want: PathBuf| match found {
            Some((_, Target::File(path))) => path.canonicalize().ok() == want.canonicalize().ok(),
            _ => false,
        };
        let sub = dir.join("sub");
        // `.\` and `..\` name the pane's folder and its parent, never the drive's root.
        assert!(same(bare_file(r"open .\plan.md", 7, Some(&sub)), sub.join("plan.md")), r".\plan.md");
        assert!(same(bare_file(r"open ..\plan.md", 8, Some(&sub)), dir.join("plan.md")), r"..\plan.md");
        // Another drive's path is that drive's: not the same folders on the pane's drive.
        let full = dir.join("plan.md").display().to_string();
        let drive = full.as_bytes()[0];
        let other = if drive.eq_ignore_ascii_case(&b'Q') { 'R' } else { 'Q' };
        let elsewhere = format!("{other}{}", &full[1..]);
        let at = elsewhere.rfind("plan").unwrap();
        assert_eq!(bare_file(&elsewhere, at, Some(&sub)), None, "{elsewhere}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

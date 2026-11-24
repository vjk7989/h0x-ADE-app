//! Start HyperOx at login (Settings): opens the HyperOx window when the person logs in.
//! Windows: a value in the per-user Run key. macOS: a LaunchAgent. Linux: an autostart entry.
//! The builders are pure; `apply` picks the platform and is the only part that touches the system.
#![cfg_attr(not(test), allow(dead_code))] // each platform uses only its own builders

use std::path::{Path, PathBuf};

/// The macOS bundle id (scripts/package) names the LaunchAgent.
const BUNDLE_ID: &str = "tech.pavii.h0xade";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// `reg add` argv (after `reg`). The path is quoted inside the value: an unquoted path with
/// spaces would run a program at the first space.
pub fn reg_add_args(exe: &Path) -> Vec<String> {
    ["add", RUN_KEY, "/v", "HyperOx", "/t", "REG_SZ", "/d"].iter().map(|s| s.to_string()).chain([format!("\"{}\"", exe.display()), "/f".into()]).collect()
}

/// `reg delete` argv (after `reg`).
pub fn reg_delete_args() -> Vec<String> {
    ["delete", RUN_KEY, "/v", "HyperOx", "/f"].iter().map(|s| s.to_string()).collect()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The LaunchAgent: opens the app bundle once at login.
pub fn launch_agent_plist(app: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\"><dict>\n\
    <key>Label</key><string>{BUNDLE_ID}</string>\n\
    <key>ProgramArguments</key><array><string>/usr/bin/open</string><string>-a</string><string>{}</string></array>\n\
    <key>RunAtLoad</key><true/>\n\
</dict></plist>\n",
        xml_escape(&app.display().to_string())
    )
}

/// The autostart entry. `Exec` quotes the path as the Desktop Entry spec says: `%` is `%%`;
/// inside the quotes `"`, backtick, `$` and `\` take a backslash, and the file's own string
/// escaping then doubles every backslash (so `$` is written `\\$` and `\` is `\\\\`).
pub fn desktop_entry(exe: &Path) -> String {
    let exe: String = exe
        .display()
        .to_string()
        .chars()
        .map(|c| match c {
            '%' => "%%".to_owned(),
            '"' | '`' | '$' => format!(r"\\{c}"),
            '\\' => r"\\\\".to_owned(),
            c => c.to_string(),
        })
        .collect();
    format!("[Desktop Entry]\nType=Application\nName=HyperOx\nExec=\"{exe}\"\nTerminal=false\n")
}

/// `<App>.app` from the executable inside it (`<App>.app/Contents/MacOS/<bin>`).
pub fn app_bundle(exe: &Path) -> Option<PathBuf> {
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

pub fn launch_agent_path(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents").join(format!("{BUNDLE_ID}.plist"))
}

/// `config` is `$XDG_CONFIG_HOME`, or `<home>/.config`.
pub fn autostart_path(config: &Path) -> PathBuf {
    config.join("autostart/hyperox.desktop")
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn remove_file(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Writes (on) or removes (off) the macOS LaunchAgent under `home`.
pub fn write_launch_agent(home: &Path, exe: &Path, on: bool) -> Result<(), String> {
    let path = launch_agent_path(home);
    if !on {
        return remove_file(&path);
    }
    let app = app_bundle(exe).ok_or_else(|| format!("{} is not inside a HyperOx.app", exe.display()))?;
    write_file(&path, &launch_agent_plist(&app))
}

/// Writes (on) or removes (off) the Linux autostart entry under `config`.
pub fn write_autostart(config: &Path, exe: &Path, on: bool) -> Result<(), String> {
    let path = autostart_path(config);
    if on { write_file(&path, &desktop_entry(exe)) } else { remove_file(&path) }
}

/// Turns start at login on or off for this platform.
pub fn apply(on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let args = if on { reg_add_args(&exe) } else { reg_delete_args() };
        let out = std::process::Command::new("reg").args(&args).creation_flags(0x0800_0000).output().map_err(|e| format!("reg: {e}"))?;
        // Turning off what was never on is fine.
        if out.status.success() || !on { return Ok(()); }
        return Err(format!("reg: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").ok_or("no HOME")?;
        return write_launch_agent(Path::new(&home), &exe, on);
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            Some(c) => PathBuf::from(c),
            None => PathBuf::from(std::env::var_os("HOME").ok_or("no HOME")?).join(".config"),
        };
        write_autostart(&config, &exe, on)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("h0x-login-start-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn windows_add_quotes_the_path_with_spaces_inside_the_value() {
        let args = reg_add_args(Path::new(r"C:\Program Files\HyperOx\h0x-ade.exe"));
        assert_eq!(args, ["add", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "HyperOx", "/t", "REG_SZ", "/d", r#""C:\Program Files\HyperOx\h0x-ade.exe""#, "/f"]);
    }

    #[test]
    fn windows_delete_names_the_value_and_forces() {
        assert_eq!(reg_delete_args(), ["delete", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "HyperOx", "/f"]);
    }

    #[test]
    fn the_launch_agent_opens_the_app_at_load_and_escapes_the_path() {
        let plist = launch_agent_plist(Path::new("/Applications/Hyper & Ox.app"));
        assert!(plist.contains("<key>RunAtLoad</key><true/>") && plist.contains("<string>tech.pavii.h0xade</string>"), "{plist}");
        assert!(plist.contains("<string>/usr/bin/open</string><string>-a</string><string>/Applications/Hyper &amp; Ox.app</string>"), "{plist}");
    }

    #[test]
    fn the_bundle_is_found_from_the_exe_and_a_bare_binary_has_none() {
        assert_eq!(app_bundle(Path::new("/Applications/HyperOx.app/Contents/MacOS/HyperOx")), Some(PathBuf::from("/Applications/HyperOx.app")));
        assert_eq!(app_bundle(Path::new("/usr/local/bin/h0x-ade")), None);
        assert_eq!(app_bundle(Path::new("h0x-ade")), None);
        assert_eq!(app_bundle(Path::new("")), None);
    }

    #[test]
    fn the_desktop_entry_quotes_exec() {
        assert_eq!(desktop_entry(Path::new("/opt/Hyper Ox/h0x-ade")), "[Desktop Entry]\nType=Application\nName=HyperOx\nExec=\"/opt/Hyper Ox/h0x-ade\"\nTerminal=false\n");
        assert!(desktop_entry(Path::new(r#"/o/a"b"#)).contains(r#"Exec="/o/a\\"b""#));
    }

    #[test]
    fn the_launch_agent_is_written_and_removed_under_the_given_home_only() {
        let home = scratch("mac");
        let exe = Path::new("/Applications/HyperOx.app/Contents/MacOS/HyperOx");
        write_launch_agent(&home, exe, true).unwrap();
        let file = home.join("Library/LaunchAgents/tech.pavii.h0xade.plist");
        assert!(std::fs::read_to_string(&file).unwrap().contains("/Applications/HyperOx.app"));
        write_launch_agent(&home, exe, false).unwrap();
        assert!(!file.exists());
        write_launch_agent(&home, exe, false).unwrap();
        assert!(write_launch_agent(&home, Path::new("/usr/bin/h0x-ade"), true).is_err(), "not in a bundle");
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_autostart_entry_is_written_and_removed_under_the_given_config_only() {
        let config = scratch("linux");
        let exe = Path::new("/opt/hyperox/h0x-ade");
        write_autostart(&config, exe, true).unwrap();
        let file = config.join("autostart/hyperox.desktop");
        assert!(std::fs::read_to_string(&file).unwrap().contains("Exec=\"/opt/hyperox/h0x-ade\""));
        write_autostart(&config, exe, false).unwrap();
        assert!(!file.exists());
        write_autostart(&config, exe, false).unwrap();
        let _ = std::fs::remove_dir_all(&config);
    }

    #[test]
    fn a_write_that_cannot_happen_is_an_error() {
        let config = scratch("blocked");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("autostart"), "a file where the folder goes").unwrap();
        assert!(write_autostart(&config, Path::new("/x/h0x-ade"), true).is_err());
        let _ = std::fs::remove_dir_all(&config);
    }

    // A2a edge cases (test writer).

    const FIXED: [&str; 7] = ["add", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "HyperOx", "/t", "REG_SZ", "/d"];

    #[test]
    fn windows_add_quotes_every_kind_of_path_whole_inside_the_one_value() {
        for exe in [
            r"C:\Program Files\HyperOx\h0x-ade.exe",
            r"C:\Users\O'Brien\AppData\Local\HyperOx\h0x-ade.exe",
            r"C:\Tools & More\h0x-ade.exe",
            r"C:\100%\%PATH%\h0x-ade.exe",
            "C:\\Users\\Jos\u{e9} \u{5f20}\u{4e09}\\HyperOx\\h0x-ade.exe",
            r"\\?\C:\Program Files\HyperOx\h0x-ade.exe",
            r"\\server\share name\HyperOx\h0x-ade.exe",
            r"C:\h0x-ade.exe",
        ] {
            let args = reg_add_args(Path::new(exe));
            assert_eq!(args.len(), 9, "{exe}: {args:?}");
            assert_eq!(args[..7], FIXED, "{exe}");
            // One argv item, quoted whole, the path untouched (no expansion, no escaping).
            assert_eq!(args[7], format!("\"{exe}\""), "{exe}");
            assert_eq!(args[8], "/f", "{exe}");
        }
    }

    #[test]
    fn windows_delete_never_carries_a_path_or_data() {
        let args = reg_delete_args();
        assert_eq!(args.len(), 5);
        assert!(!args.iter().any(|a| a == "/d" || a == "/t" || a.contains("h0x-ade")), "{args:?}");
        assert_eq!(args[0], "delete");
        assert_eq!(args[1], reg_add_args(Path::new(r"C:\x.exe"))[1], "the same key as add");
        assert_eq!(args[3], reg_add_args(Path::new(r"C:\x.exe"))[3], "the same value name as add");
    }

    /// Every `&` starts one of the five XML entities; no `<` or `>` sits inside a `<string>`.
    fn well_formed_strings(plist: &str) {
        for (i, _) in plist.match_indices('&') {
            let rest = &plist[i..];
            assert!(["&amp;", "&lt;", "&gt;", "&quot;", "&apos;"].iter().any(|e| rest.starts_with(e)), "bare & at {i}: {plist}");
        }
        assert_eq!(plist.matches("<string>").count(), plist.matches("</string>").count(), "{plist}");
        for part in plist.split("<string>").skip(1) {
            let body = part.split("</string>").next().unwrap();
            assert!(!body.contains('<') && !body.contains('>'), "{body:?} in {plist}");
        }
    }

    #[test]
    fn the_launch_agent_is_valid_xml_for_any_path_and_escapes_amp_and_lt() {
        for (app, want) in [
            ("/Applications/HyperOx.app", "/Applications/HyperOx.app"),
            ("/Applications/A & B.app", "/Applications/A &amp; B.app"),
            ("/Applications/a<b>c.app", "/Applications/a&lt;b&gt;c.app"),
            ("/Applications/&lt;.app", "/Applications/&amp;lt;.app"),
            ("/Users/O'Brien/Apps/\"Hyper\".app", "/Users/O'Brien/Apps/\"Hyper\".app"),
            ("/Users/jos\u{e9}/\u{5f20}\u{4e09}/HyperOx.app", "/Users/jos\u{e9}/\u{5f20}\u{4e09}/HyperOx.app"),
            ("/Volumes/100% Disk/HyperOx.app", "/Volumes/100% Disk/HyperOx.app"),
        ] {
            let plist = launch_agent_plist(Path::new(app));
            assert!(plist.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"), "{plist}");
            assert!(plist.trim_end().ends_with("</plist>"), "{plist}");
            assert!(
                plist.contains(&format!("<array><string>/usr/bin/open</string><string>-a</string><string>{want}</string></array>")),
                "{app}: {plist}"
            );
            assert_eq!(plist.matches("<key>").count(), 3, "{plist}");
            well_formed_strings(&plist);
        }
    }

    #[test]
    fn the_desktop_entry_quotes_exec_and_doubles_percent() {
        let line = |exe: &str| desktop_entry(Path::new(exe)).lines().find(|l| l.starts_with("Exec=")).unwrap().to_owned();
        assert_eq!(line("/opt/hyperox/h0x-ade"), "Exec=\"/opt/hyperox/h0x-ade\"");
        assert_eq!(line("/opt/Hyper Ox/h0x-ade"), "Exec=\"/opt/Hyper Ox/h0x-ade\"");
        assert_eq!(line("/home/jos\u{e9}/\u{5f20}\u{4e09}/h0x-ade"), "Exec=\"/home/jos\u{e9}/\u{5f20}\u{4e09}/h0x-ade\"");
        // A field code is %<letter>: a literal % in the path is written %%.
        assert_eq!(line("/opt/100%/h0x-ade"), "Exec=\"/opt/100%%/h0x-ade\"");
        assert_eq!(line("/opt/x%f/%%u/h0x-ade"), "Exec=\"/opt/x%%f/%%%%u/h0x-ade\"");
        // Quote-level backslash, doubled by the file's string escaping.
        assert_eq!(line("/o/a$b"), r#"Exec="/o/a\\$b""#);
        assert_eq!(line("/o/a`b"), r#"Exec="/o/a\\`b""#);
        assert_eq!(line(r#"/o/a"b"#), r#"Exec="/o/a\\"b""#);
        assert_eq!(line(r"/o/a\b"), r#"Exec="/o/a\\\\b""#);
        let entry = desktop_entry(Path::new("/opt/100%/h0x-ade"));
        assert_eq!(entry.lines().filter(|l| l.starts_with("Exec=")).count(), 1, "{entry}");
        assert!(entry.starts_with("[Desktop Entry]\n") && entry.contains("\nType=Application\n") && entry.ends_with('\n'), "{entry}");
    }

    #[test]
    fn the_paths_sit_under_the_given_root() {
        let root = Path::new("C:\\Users\\Jos\u{e9} X\\scratch");
        assert!(launch_agent_path(root).starts_with(root) && launch_agent_path(root).ends_with("tech.pavii.h0xade.plist"));
        assert!(autostart_path(root).starts_with(root) && autostart_path(root).ends_with("hyperox.desktop"));
        let unix = Path::new("/home/a b/.config");
        assert_eq!(autostart_path(unix), PathBuf::from("/home/a b/.config/autostart/hyperox.desktop"));
    }

    #[test]
    fn the_autostart_writer_makes_deep_missing_folders_and_overwrites() {
        let root = scratch("deep-linux");
        let config = root.join("a b").join("\u{5f20}").join(".config");
        assert!(!config.exists());
        write_autostart(&config, Path::new("/opt/one/h0x-ade"), true).unwrap();
        let file = autostart_path(&config);
        assert!(std::fs::read_to_string(&file).unwrap().contains("Exec=\"/opt/one/h0x-ade\""));
        // A stale or foreign file is replaced whole, not appended to.
        std::fs::write(&file, "garbage that is much longer than the entry ".repeat(20)).unwrap();
        write_autostart(&config, Path::new("/opt/two/h0x-ade"), true).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), desktop_entry(Path::new("/opt/two/h0x-ade")));
        // Turning off leaves the folder and its other entries alone.
        let other = config.join("autostart/other.desktop");
        std::fs::write(&other, "x").unwrap();
        write_autostart(&config, Path::new("/opt/two/h0x-ade"), false).unwrap();
        assert!(!file.exists() && other.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_launch_agent_writer_makes_missing_folders_and_overwrites() {
        let root = scratch("deep-mac");
        let home = root.join("Jos\u{e9} Home");
        let exe = Path::new("/Applications/A & B.app/Contents/MacOS/HyperOx");
        write_launch_agent(&home, exe, true).unwrap();
        let file = launch_agent_path(&home);
        std::fs::write(&file, "<old/>".repeat(100)).unwrap();
        write_launch_agent(&home, exe, true).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), launch_agent_plist(Path::new("/Applications/A & B.app")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn turning_off_what_is_absent_is_fine_even_with_no_folders() {
        let root = scratch("absent");
        assert!(!root.exists());
        write_autostart(&root.join("none"), Path::new("/x/h0x-ade"), false).unwrap();
        write_launch_agent(&root.join("none"), Path::new("/x/h0x-ade"), false).unwrap();
        assert!(!root.exists(), "turning off creates nothing");
    }

    #[test]
    fn a_folder_where_the_file_goes_is_an_error_both_ways() {
        let config = scratch("dir-on-file");
        std::fs::create_dir_all(autostart_path(&config)).unwrap();
        assert!(write_autostart(&config, Path::new("/x/h0x-ade"), false).is_err());
        assert!(write_autostart(&config, Path::new("/x/h0x-ade"), true).is_err());
        let _ = std::fs::remove_dir_all(&config);
    }
}

//! A launch from a shell (`h0x-ade` typed in a terminal) opens HyperOx in the
//! folder the shell is in, and says so in the terminal.

use std::path::PathBuf;
use std::sync::OnceLock;

static FOLDER: OnceLock<Option<String>> = OnceLock::new();

/// Records the launch folder once, before the window opens. Not from a shell:
/// no folder.
pub fn init(from_shell: bool) {
    let _ = FOLDER.set(pick(from_shell, std::env::current_dir()));
}

/// The folder this launch was started in; None for a launch from a shortcut,
/// Dock or Explorer, or when the shell's folder is unusable.
pub fn folder() -> Option<String> {
    FOLDER.get().cloned().flatten()
}

/// True when this process was started from a shell.
pub fn from_shell() -> bool {
    #[cfg(windows)]
    {
        crate::attach_console()
    }
    #[cfg(not(windows))]
    {
        use std::io::IsTerminal;
        std::io::stdin().is_terminal() || std::io::stdout().is_terminal() || std::io::stderr().is_terminal()
    }
}

/// The shell's folder as given (no canonicalize), if it is an existing
/// directory with a UTF-8 path.
fn pick(from_shell: bool, cwd: std::io::Result<PathBuf>) -> Option<String> {
    if !from_shell {
        return None;
    }
    let cwd = cwd.ok()?;
    if !cwd.is_dir() {
        return None;
    }
    cwd.to_str().map(str::to_string)
}

pub fn started_line(folder: Option<&str>) -> String {
    match folder {
        Some(folder) => format!("HyperOx started in {folder}."),
        None => "HyperOx started.".to_string(),
    }
}

pub fn running_line(folder: Option<&str>) -> String {
    match folder {
        Some(folder) => format!("HyperOx is already running. Opened {folder}."),
        None => "HyperOx is already running.".to_string(),
    }
}

/// Tells the person in the shell, in the terminal and as an OS notification.
/// The notification's thread is returned for a process that is about to exit.
pub fn announce(line: &str, folder: Option<&str>) -> Option<std::thread::JoinHandle<()>> {
    use std::io::Write;
    println!("{line}");
    let _ = std::io::stdout().flush();
    crate::os_notice::show("HyperOx has been launched", folder.unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::{pick, running_line, started_line};
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("h0x-ade-launch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_launch_not_from_a_shell_has_no_folder_even_in_a_real_one() {
        let dir = temp("not-shell");
        assert_eq!(pick(false, Ok(dir.clone())), None);
        assert_eq!(pick(false, Err(std::io::Error::other("no cwd"))), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unusable_shell_folder_gives_no_folder() {
        // The shell's folder can't be read at all.
        assert_eq!(pick(true, Err(std::io::Error::from(std::io::ErrorKind::NotFound))), None);
        assert_eq!(pick(true, Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))), None);
        // An empty path is no folder.
        assert_eq!(pick(true, Ok(PathBuf::new())), None);
        let dir = temp("unusable");
        // A file, not a folder.
        let file = dir.join("file.txt");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(pick(true, Ok(file)), None);
        // Never there, and deleted after the shell went into it.
        assert_eq!(pick(true, Ok(dir.join("missing"))), None);
        let gone = dir.join("gone");
        std::fs::create_dir(&gone).unwrap();
        std::fs::remove_dir(&gone).unwrap();
        assert_eq!(pick(true, Ok(gone)), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_shell_folder_is_kept_exactly_as_given() {
        let dir = temp("exact");
        let given = dir.to_str().unwrap().to_owned();
        assert_eq!(pick(true, Ok(dir.clone())).as_deref(), Some(given.as_str()));
        // Not canonicalized: a `..` stays, and Windows gets no verbatim prefix.
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let dotted = dir.join("sub").join("..");
        assert_eq!(pick(true, Ok(dotted.clone())).as_deref(), dotted.to_str());
        assert!(!pick(true, Ok(dir.clone())).unwrap().starts_with(r"\\?\"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_with_spaces_unicode_and_shell_characters_is_kept_literally() {
        let dir = temp("odd");
        for name in ["with space", "caf\u{e9} \u{65e5}\u{672c}", "a & b", "it's \"quoted\"", "semi;colon $HOME %PATH%"] {
            // Double quotes are not allowed in Windows file names.
            if cfg!(windows) && name.contains('"') {
                continue;
            }
            let folder = dir.join(name);
            std::fs::create_dir_all(&folder).unwrap();
            let picked = pick(true, Ok(folder.clone())).unwrap();
            assert_eq!(picked, folder.to_str().unwrap());
            assert!(picked.ends_with(name), "{picked}");
            assert_eq!(started_line(Some(&picked)), format!("HyperOx started in {picked}."));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_whose_path_is_not_utf8_gives_no_folder() {
        use std::os::unix::ffi::OsStrExt;
        let dir = temp("non-utf8");
        let folder = dir.join(std::ffi::OsStr::from_bytes(b"bad-\xff-name"));
        // macOS refuses such names; the path then isn't a folder either way.
        let _ = std::fs::create_dir(&folder);
        assert_eq!(pick(true, Ok(folder)), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_terminal_lines_name_the_folder_or_leave_it_out() {
        assert_eq!(started_line(Some("/home/me/proj")), "HyperOx started in /home/me/proj.");
        assert_eq!(started_line(Some(r"D:\work\x")), r"HyperOx started in D:\work\x.");
        assert_eq!(started_line(None), "HyperOx started.");
        assert_eq!(running_line(Some(r"\\server\share\x")), r"HyperOx is already running. Opened \\server\share\x.");
        assert_eq!(running_line(Some("/")), "HyperOx is already running. Opened /.");
        assert_eq!(running_line(None), "HyperOx is already running.");
    }
}

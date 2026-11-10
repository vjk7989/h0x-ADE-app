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

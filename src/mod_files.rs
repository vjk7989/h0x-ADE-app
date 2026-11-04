//! Asynchronous text files for mods. Writes reject a stale editor snapshot.
//! Mods reach only files inside the person's home folder (ADR-048).
use gpui_shell::{HostError, HostValue};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const LIMIT: u64 = 1024 * 1024;
static WRITES: Mutex<()> = Mutex::new(());

pub fn read(path: &Path) -> Result<Option<String>, String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut text = String::new();
    file.take(LIMIT + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() as u64 > LIMIT {
        return Err("Text file exceeds 1 MiB".into());
    }
    Ok(Some(text))
}

fn write(path: &Path, text: &str, expected: Option<&str>) -> Result<(), String> {
    let _guard = WRITES.lock().map_err(|e| e.to_string())?;
    if text.len() as u64 > LIMIT {
        return Err("Text file exceeds 1 MiB".into());
    }
    if read(path)?.as_deref() != expected {
        return Err(
            "The file changed outside HyperOx. Reload before saving; your draft is still here."
                .into(),
        );
    }
    let parent = path.parent().ok_or("File needs a parent directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    if expected.is_none() {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        return f.write_all(text.as_bytes()).map_err(|e| e.to_string());
    }
    // Resolve symlinks so replacing the file does not replace the link itself.
    let target = crate::persist::resolve_target(path).map_err(|e| e.to_string())?;
    let temp = target.with_file_name(format!(
        ".h0x-ade-write-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        f.set_permissions(
            std::fs::metadata(&target)
                .map_err(|e| e.to_string())?
                .permissions(),
        )
        .map_err(|e| e.to_string())?;
        f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        if read(&target)?.as_deref() != expected {
            return Err("The file changed while saving. Reload before saving.".into());
        }
        std::fs::rename(&temp, &target).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(temp);
    result
}

/// Refuses a path outside the person's home folder, with `..` and symlinks
/// resolved (a path that doesn't exist yet: by its nearest existing folder).
pub fn inside_home(path: &Path) -> Result<(), String> {
    inside(path, &crate::mods::home())
}

fn inside(path: &Path, home: &Path) -> Result<(), String> {
    let outside = || format!("{} is outside your home folder", path.display());
    let home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let target = real(path).ok_or_else(outside)?;
    if within(&target.to_string_lossy(), &home.to_string_lossy(), cfg!(windows)) {
        Ok(())
    } else {
        Err(outside())
    }
}

/// The real path: symlinks and `..` resolved. Names that don't exist yet are
/// kept as written; `..` after them, or no existing folder at all, is None.
fn real(path: &Path) -> Option<PathBuf> {
    let path = crate::persist::resolve_target(path).ok()?;
    let mut tail = Vec::new();
    let mut base = path.as_path();
    loop {
        match base.canonicalize() {
            Ok(real) => return Some(tail.into_iter().rev().fold(real, |p, name| p.join(name))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tail.push(base.file_name()?.to_owned());
                base = base.parent()?;
            }
            Err(_) => return None,
        }
    }
}

/// Whether the real path `target` is `home` or inside it. On Windows in any
/// case, with `\\?\` and `\\?\UNC\` read as the plain drive and UNC paths.
fn within(target: &str, home: &str, windows: bool) -> bool {
    let norm = |s: &str| {
        if !windows {
            return s.trim_end_matches('/').to_owned();
        }
        let s = s.replace('/', r"\");
        let s = match s.strip_prefix(r"\\?\UNC\") {
            Some(rest) => format!(r"\\{rest}"),
            None => s.strip_prefix(r"\\?\").unwrap_or(s.as_str()).to_owned(),
        };
        s.trim_end_matches('\\').to_lowercase()
    };
    let (target, home) = (norm(target), norm(home));
    let sep = if windows { '\\' } else { '/' };
    target == home || target.strip_prefix(&home).is_some_and(|rest| rest.starts_with(sep))
}

/// Reads off the UI thread: the text, or null when absent.
pub fn read_with(
    read: impl FnOnce() -> Result<Option<String>, String> + Send + 'static,
) -> impl std::future::Future<Output = Result<HostValue, HostError>> + Send {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(read());
    });
    async move {
        let value = rx
            .recv()
            .await
            .map_err(|e| HostError::new(e.to_string()))?
            .map_err(HostError::new)?;
        Ok(value.map(HostValue::from).unwrap_or(HostValue::Null))
    }
}

pub fn write_text(
    path: String,
    text: String,
    expected: Option<String>,
) -> impl std::future::Future<Output = Result<HostValue, HostError>> + Send {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let path = Path::new(&path);
        let _ = tx.send_blocking(inside_home(path).and_then(|_| write(path, &text, expected.as_deref())));
    });
    async move {
        rx.recv()
            .await
            .map_err(|e| HostError::new(e.to_string()))?
            .map_err(HostError::new)?;
        Ok(HostValue::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creates_updates_and_rejects_stale_text() {
        let root = std::path::PathBuf::from("/var/tmp").join(format!(
            "h0x-ade-files-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join(".brain/todo.md");
        assert_eq!(read(&path).unwrap(), None);
        write(&path, "- [ ] Task\n", None).unwrap();
        assert!(write(&path, "overwrite", None).is_err());
        write(&path, "- [x] Task\n", Some("- [ ] Task\n")).unwrap();
        assert!(write(&path, "stale", Some("- [ ] Task\n")).is_err());
        assert_eq!(read(&path).unwrap().unwrap(), "- [x] Task\n");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mods_reach_only_the_home_folder() {
        let root = std::env::temp_dir().join(format!(
            "h0x-ade-home-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = root.join("home");
        std::fs::create_dir_all(home.join("project")).unwrap();
        std::fs::write(home.join("project/a.txt"), "a").unwrap();
        std::fs::write(root.join("secret.txt"), "s").unwrap();
        // Inside, existing or not yet written (its folders made on write).
        assert_eq!(inside(&home.join("project/a.txt"), &home), Ok(()));
        assert_eq!(inside(&home.join("project/new/deep.md"), &home), Ok(()));
        assert_eq!(inside(&home.join("project/../project/a.txt"), &home), Ok(()));
        // The home folder itself.
        assert_eq!(inside(&home, &home), Ok(()));
        // `..` out of it, existing or not.
        let escape = home.join("../secret.txt");
        assert_eq!(inside(&escape, &home), Err(format!("{} is outside your home folder", escape.display())));
        assert!(inside(&home.join("../elsewhere.txt"), &home).is_err());
        assert!(inside(&home.join("missing/../../secret.txt"), &home).is_err());
        assert!(inside(&root.join("secret.txt"), &home).is_err());
        // A sibling whose name starts like the home folder's.
        std::fs::create_dir_all(root.join("home2")).unwrap();
        assert!(inside(&root.join("home2/x.txt"), &home).is_err());
        // Empty and relative paths that don't exist resolve nowhere.
        assert!(inside(Path::new(""), &home).is_err());
        assert!(inside(Path::new("no-such-file-h0x.txt"), &home).is_err());
        #[cfg(unix)]
        {
            // A symlink inside home pointing out, to a folder or a missing file.
            std::os::unix::fs::symlink(&root, home.join("out")).unwrap();
            assert!(inside(&home.join("out/secret.txt"), &home).is_err());
            assert!(inside(&home.join("out/new.txt"), &home).is_err());
            std::os::unix::fs::symlink(root.join("gone.txt"), home.join("dangling")).unwrap();
            assert!(inside(&home.join("dangling"), &home).is_err());
            // And one pointing within home is fine.
            std::os::unix::fs::symlink(home.join("project"), home.join("in")).unwrap();
            assert_eq!(inside(&home.join("in/a.txt"), &home), Ok(()));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_paths_compare_without_case_or_verbatim_prefixes() {
        let home = r"\\?\C:\Users\Ana";
        assert!(within(r"\\?\C:\Users\Ana", home, true));
        assert!(within(r"C:\Users\Ana\", home, true));
        assert!(within(r"\\?\c:\users\ana\Projects\a.txt", home, true));
        assert!(within(r"C:/Users/Ana/notes.md", home, true));
        // Another drive, a sibling with the same start, UNC shares.
        assert!(!within(r"\\?\D:\Users\Ana\a.txt", home, true));
        assert!(!within(r"\\?\C:\Users\Anabel\a.txt", home, true));
        assert!(!within(r"\\?\C:\Users", home, true));
        assert!(!within(r"\\?\UNC\server\share\a.txt", home, true));
        assert!(!within(r"\\server\share\a.txt", home, true));
        // A home on a share: both spellings are the same place.
        assert!(within(r"\\?\UNC\server\share\ana\a.txt", r"\\server\share\ana", true));
        assert!(!within(r"\\?\UNC\server\other\a.txt", r"\\server\share\ana", true));
        // Unix: exact case, component boundaries.
        assert!(within("/home/ana", "/home/ana", false));
        assert!(within("/home/ana/x", "/home/ana/", false));
        assert!(!within("/home/Ana/x", "/home/ana", false));
        assert!(!within("/home/anabel", "/home/ana", false));
        assert!(!within("/etc/passwd", "/home/ana", false));
    }
}

//! Asynchronous text files for mods. Writes reject a stale editor snapshot.
//! Mods reach only files in their allowed folders (ADR-048, `allowed_roots`).
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

/// Where mods may read, write and watch (ADR-048, amended): the person's home
/// folder, HyperOx's app home, and the folders of the spaces and terminals open
/// on this computer. Computed per call: spaces come and go.
pub fn allowed_roots() -> Vec<String> {
    let mut roots = vec![crate::mods::home(), crate::mods::app_dir()];
    roots.extend(crate::mods::local_space_folders());
    roots.extend(crate::mods::local_pane_folders());
    real_roots(roots)
}

/// The roots as real paths. A filesystem root (`/`, `G:\`, a bare share)
/// would open a whole drive, so it is left out.
fn real_roots(roots: impl IntoIterator<Item = PathBuf>) -> Vec<String> {
    roots
        .into_iter()
        .filter_map(|root| {
            let real = real(&root)?.to_string_lossy().into_owned();
            if is_fs_root(&real, cfg!(windows)) {
                tracing::warn!("{}: a filesystem root is never a folder mods may use", root.display());
                return None;
            }
            Some(real)
        })
        .collect()
}

/// Refuses a path outside every root, with `..` and symlinks resolved (a path
/// that doesn't exist yet: by its nearest existing folder).
pub fn check(path: &Path, roots: &[String]) -> Result<(), String> {
    let outside = || format!("{} is outside your home folder and your spaces", path.display());
    let target = real(path).ok_or_else(outside)?;
    if allowed(&target.to_string_lossy(), roots, cfg!(windows)) {
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

/// Whether the real path `target` is one of `roots` or inside one. Filesystem
/// roots never count.
fn allowed(target: &str, roots: &[String], windows: bool) -> bool {
    roots.iter().any(|root| !is_fs_root(root, windows) && within(target, root, windows))
}

/// `/`, a drive (`C:\`) or a bare share (`\\server\share`).
fn is_fs_root(path: &str, windows: bool) -> bool {
    let p = norm(path, windows);
    if !windows {
        return p.is_empty();
    }
    p.is_empty() || (p.len() == 2 && p.ends_with(':')) || p.strip_prefix(r"\\").is_some_and(|share| share.matches('\\').count() <= 1)
}

/// Whether `target` is `root` or inside it.
fn within(target: &str, root: &str, windows: bool) -> bool {
    let (target, root) = (norm(target, windows), norm(root, windows));
    let sep = if windows { '\\' } else { '/' };
    target == root || target.strip_prefix(&root).is_some_and(|rest| rest.starts_with(sep))
}

/// Comparable form, without a trailing separator. Windows: any case, with
/// `\\?\` and `\\?\UNC\` read as the plain drive and UNC paths.
fn norm(s: &str, windows: bool) -> String {
    if !windows {
        return s.trim_end_matches('/').to_owned();
    }
    let s = s.replace('/', r"\");
    let s = match s.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => s.strip_prefix(r"\\?\").unwrap_or(s.as_str()).to_owned(),
    };
    s.trim_end_matches('\\').to_lowercase()
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
    let roots = allowed_roots();
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let path = Path::new(&path);
        let _ = tx.send_blocking(check(path, &roots).and_then(|_| write(path, &text, expected.as_deref())));
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
    fn mods_reach_only_their_allowed_folders() {
        let root = std::env::temp_dir().join(format!(
            "h0x-ade-roots-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = root.join("home");
        let space = root.join("work/repo");
        std::fs::create_dir_all(home.join("project")).unwrap();
        std::fs::create_dir_all(&space).unwrap();
        std::fs::write(home.join("project/a.txt"), "a").unwrap();
        std::fs::write(space.join("b.txt"), "b").unwrap();
        std::fs::write(root.join("secret.txt"), "s").unwrap();
        std::fs::write(root.join("work/beside.txt"), "x").unwrap();
        let roots = real_roots([home.clone(), space.clone()]);
        assert_eq!(roots.len(), 2);
        // Inside the home folder, existing or not yet written (its folders made on write).
        assert_eq!(check(&home.join("project/a.txt"), &roots), Ok(()));
        assert_eq!(check(&home.join("project/new/deep.md"), &roots), Ok(()));
        assert_eq!(check(&home.join("project/../project/a.txt"), &roots), Ok(()));
        assert_eq!(check(&home, &roots), Ok(()));
        // Inside a space's folder, and the folder itself.
        assert_eq!(check(&space.join("b.txt"), &roots), Ok(()));
        assert_eq!(check(&space.join("src/new.rs"), &roots), Ok(()));
        assert_eq!(check(&space, &roots), Ok(()));
        // `..` out of the home folder or out of a space, existing or not.
        let escape = home.join("../secret.txt");
        assert_eq!(check(&escape, &roots), Err(format!("{} is outside your home folder and your spaces", escape.display())));
        assert!(check(&space.join("../beside.txt"), &roots).is_err());
        assert!(check(&space.join("../../secret.txt"), &roots).is_err());
        assert!(check(&home.join("../elsewhere.txt"), &roots).is_err());
        assert!(check(&home.join("missing/../../secret.txt"), &roots).is_err());
        // Outside every root.
        assert!(check(&root.join("secret.txt"), &roots).is_err());
        assert!(check(&root.join("work"), &roots).is_err());
        // A sibling whose name starts like a root's.
        std::fs::create_dir_all(root.join("home2")).unwrap();
        assert!(check(&root.join("home2/x.txt"), &roots).is_err());
        // Empty and relative paths that don't exist resolve nowhere; no roots, nothing.
        assert!(check(Path::new(""), &roots).is_err());
        assert!(check(Path::new("no-such-file-h0x.txt"), &roots).is_err());
        assert!(check(&home.join("project/a.txt"), &[]).is_err());
        // A root that doesn't exist yet still counts where it will be.
        let later = real_roots([root.join("later")]);
        assert_eq!(check(&root.join("later/x.txt"), &later), Ok(()));
        // The filesystem root is never a root.
        let top = std::path::Path::new(&root).ancestors().last().unwrap().to_path_buf();
        assert!(real_roots([top]).is_empty());
        #[cfg(unix)]
        {
            // A symlink inside home pointing out, to a folder or a missing file.
            std::os::unix::fs::symlink(&root, home.join("out")).unwrap();
            assert!(check(&home.join("out/secret.txt"), &roots).is_err());
            assert!(check(&home.join("out/new.txt"), &roots).is_err());
            std::os::unix::fs::symlink(root.join("gone.txt"), home.join("dangling")).unwrap();
            assert!(check(&home.join("dangling"), &roots).is_err());
            // One pointing into a space is fine.
            std::os::unix::fs::symlink(&space, home.join("in")).unwrap();
            assert_eq!(check(&home.join("in/b.txt"), &roots), Ok(()));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// HyperOx's app home is allowed even when it is not in the home folder
    /// (an isolated `H0X_ADE_HOME`).
    #[test]
    fn the_app_home_is_an_allowed_root() {
        let app = real(&crate::mods::app_dir()).unwrap().to_string_lossy().into_owned();
        let roots = allowed_roots();
        assert!(roots.contains(&app), "{app} not in {roots:?}");
        let inside = format!("{app}{}settings.json", std::path::MAIN_SEPARATOR);
        assert!(allowed(&inside, &roots, cfg!(windows)));
        assert!(allowed(&inside, &[app.clone()], cfg!(windows)));
    }

    #[test]
    fn filesystem_roots_never_open_a_drive() {
        // A space at `G:\` (or `/`) doesn't make the drive reachable...
        for root in [r"G:\", r"\\?\G:\", "g:", r"\\?\UNC\server\share\", r"\\server\share"] {
            assert!(is_fs_root(root, true), "{root}");
            assert!(!allowed(r"\\?\G:\pavii-ade\a.txt", &[root.to_owned()], true), "{root}");
            assert!(!allowed(r"\\server\share\a.txt", &[root.to_owned()], true), "{root}");
        }
        assert!(is_fs_root("/", false));
        assert!(!allowed("/etc/passwd", &["/".to_owned()], false));
        // ...while the other roots still work beside it.
        let roots = [r"G:\".to_owned(), r"\\?\G:\pavii-ade".to_owned()];
        assert!(allowed(r"\\?\g:\PAVII-ADE\src\a.rs", &roots, true));
        assert!(!allowed(r"\\?\G:\other\a.rs", &roots, true));
        // Folders on a drive or share are not roots of it.
        for folder in [r"G:\pavii-ade", r"\\?\UNC\server\share\ana", r"\\server\share\ana"] {
            assert!(!is_fs_root(folder, true), "{folder}");
        }
        assert!(!is_fs_root("/home/ana", false));
        assert!(allowed("/home/ana/x", &["/".to_owned(), "/home/ana".to_owned()], false));
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

    /// A folder on another drive is a root like any other: whatever the case,
    /// slashes or `\\?\` prefix of the path asked about, and nothing beside it.
    #[test]
    fn a_folder_on_another_drive_is_a_root() {
        for (root, inside, outside) in [
            (r"\\?\G:\proj", r"g:/PROJ/notes/a.txt", r"G:\proj2\a.txt"),
            (r"\\?\G:\test-claude-makes-app", r"G:\test-claude-makes-app/solana.txt", r"G:\other\solana.txt"),
            (r"D:\x", r"\\?\d:\X\a.txt", r"D:\xy\a.txt"),
        ] {
            let roots = [root.to_owned()];
            assert!(!is_fs_root(root, true), "{root}");
            assert!(allowed(inside, &roots, true), "{inside}");
            assert!(allowed(root, &roots, true), "{root}");
            assert!(!allowed(outside, &roots, true), "{outside}");
            assert!(!allowed(r"C:\Windows\win.ini", &roots, true));
        }
        // The drive itself is still no root.
        assert!(!allowed(r"G:\proj\a.txt", &[r"G:\".to_owned()], true));
    }

    /// The Files panel lists the focused terminal's live folder, so that folder
    /// is a root too (local terminals only), and the file in it reads as text
    /// whatever its extension.
    #[test]
    fn a_local_terminals_folder_is_a_root_and_any_extension_reads_as_text() {
        let dir = std::env::temp_dir().join(format!("h0x-ade-pane-{}", std::process::id()));
        let far = std::env::temp_dir().join(format!("h0x-ade-far-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&far).unwrap();
        for name in ["a.txt", "b.log", "README"] {
            std::fs::write(dir.join(name), "plain\n").unwrap();
        }
        std::fs::write(far.join("c.txt"), "x").unwrap();
        crate::mods::publish_panes(serde_json::json!([
            {"pane": "p1", "terminal": "t1", "cwd": dir},
            {"pane": "p2", "terminal": "vps/t2", "cwd": far},
            {"pane": "p3", "terminal": null, "cwd": far},
            {"pane": "p4", "terminal": "t4", "cwd": "relative/dir"},
        ]));
        assert_eq!(crate::mods::local_pane_folders(), vec![dir.clone()]);
        let roots = allowed_roots();
        for name in ["a.txt", "b.log", "README"] {
            assert_eq!(check(&dir.join(name), &roots), Ok(()), "{name}");
            assert_eq!(read(&dir.join(name)).unwrap().as_deref(), Some("plain\n"), "{name}");
        }
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(far).unwrap();
    }

    /// A terminal at `/` or a drive, or with no usable folder, opens nothing.
    #[test]
    fn a_terminal_at_a_filesystem_root_or_without_a_folder_adds_no_root() {
        let top = std::env::temp_dir().ancestors().last().unwrap().to_path_buf();
        crate::mods::publish_panes(serde_json::json!([
            {"pane": "p1", "terminal": "t1", "cwd": top},
            {"pane": "p2", "terminal": "t2", "cwd": "/"},
            {"pane": "p3", "terminal": "t3", "cwd": r"C:\"},
            {"pane": "p4", "terminal": "t4", "cwd": r"G:\"},
            {"pane": "p5", "terminal": "t5", "cwd": r"C:relative"},
            {"pane": "p6", "terminal": "t6", "cwd": ""},
            {"pane": "p7", "terminal": "t7", "cwd": null},
            {"pane": "p8", "terminal": "t8"},
            {"pane": "p9", "terminal": "t9", "cwd": 42},
            {"pane": "p10", "cwd": top},
        ]));
        let roots = real_roots(crate::mods::local_pane_folders());
        assert!(roots.is_empty(), "{roots:?}");
        // No panes at all, or no list: nothing.
        crate::mods::publish_panes(serde_json::json!([]));
        assert!(crate::mods::local_pane_folders().is_empty());
        crate::mods::publish_panes(serde_json::Value::Null);
        assert!(crate::mods::local_pane_folders().is_empty());
    }

    /// Every local terminal's folder is a root while its pane is open: written
    /// with `..`, not created yet, or joined with `/` the way the Files panel
    /// does on Windows too. Closing the pane takes its folder away.
    #[test]
    fn terminal_roots_follow_the_panes_snapshot() {
        let base = std::env::temp_dir().join(format!(
            "h0x-ade-pane-roots-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (one, two) = (base.join("one"), base.join("two"));
        std::fs::create_dir_all(one.join("sub")).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        std::fs::write(one.join("a.txt"), "a").unwrap();
        std::fs::write(two.join("b.txt"), "b").unwrap();
        std::fs::write(base.join("beside.txt"), "x").unwrap();
        let one_pane = serde_json::json!({"pane": "p1", "terminal": "t1", "cwd": one.join("sub").join("..")});
        crate::mods::publish_panes(serde_json::json!([
            one_pane,
            {"pane": "p2", "terminal": "t2", "cwd": two},
            {"pane": "p3", "terminal": "t3", "cwd": base.join("gone")},
        ]));
        let roots = real_roots(crate::mods::local_pane_folders());
        assert_eq!(roots.len(), 3, "{roots:?}");
        assert_eq!(check(&one.join("a.txt"), &roots), Ok(()));
        assert_eq!(check(&two.join("b.txt"), &roots), Ok(()));
        // A folder that no longer exists still counts where it was.
        assert_eq!(check(&base.join("gone").join("new.txt"), &roots), Ok(()));
        // `root/path`, as the Files panel joins them, with mixed separators on Windows.
        assert_eq!(check(Path::new(&format!("{}/a.txt", one.display())), &roots), Ok(()));
        assert_eq!(check(Path::new(&format!("{}/sub/../a.txt", one.display())), &roots), Ok(()));
        // Beside and above the terminals' folders, nothing.
        assert!(check(&base.join("beside.txt"), &roots).is_err());
        assert!(check(&base, &roots).is_err());
        assert!(check(&one.join("..").join("beside.txt"), &roots).is_err());
        assert!(check(Path::new(&format!("{}/../beside.txt", two.display())), &roots).is_err());
        // The second terminal closes: its folder is no root any more.
        crate::mods::publish_panes(serde_json::json!([one_pane]));
        let roots = real_roots(crate::mods::local_pane_folders());
        assert_eq!(roots.len(), 1, "{roots:?}");
        assert_eq!(check(&one.join("a.txt"), &roots), Ok(()));
        assert!(check(&two.join("b.txt"), &roots).is_err());
        crate::mods::publish_panes(serde_json::json!([]));
        assert!(real_roots(crate::mods::local_pane_folders()).is_empty());
        std::fs::remove_dir_all(base).unwrap();
    }

    /// A terminal whose folder is a link: the link's target is the root, and a
    /// link inside it that points out reaches nothing.
    #[cfg(unix)]
    #[test]
    fn a_link_inside_a_terminals_folder_does_not_reach_outside_it() {
        let base = std::env::temp_dir().join(format!(
            "h0x-ade-pane-link-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (pane, outside) = (base.join("pane"), base.join("outside"));
        std::fs::create_dir_all(&pane).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "s").unwrap();
        std::os::unix::fs::symlink(&outside, pane.join("out")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), pane.join("secret.txt")).unwrap();
        std::os::unix::fs::symlink(&pane, base.join("alias")).unwrap();
        crate::mods::publish_panes(serde_json::json!([{"pane": "p1", "terminal": "t1", "cwd": base.join("alias")}]));
        let roots = real_roots(crate::mods::local_pane_folders());
        assert_eq!(roots, vec![pane.canonicalize().unwrap().to_string_lossy().into_owned()]);
        assert_eq!(check(&base.join("alias").join("new.txt"), &roots), Ok(()));
        assert!(check(&pane.join("out").join("secret.txt"), &roots).is_err());
        assert!(check(&pane.join("out").join("new.txt"), &roots).is_err());
        assert!(check(&pane.join("secret.txt"), &roots).is_err());
        crate::mods::publish_panes(serde_json::json!([]));
        std::fs::remove_dir_all(base).unwrap();
    }

    /// A UNC folder is a terminal folder on Windows only (elsewhere it is a
    /// relative name), and as a root it opens that folder, never the share.
    /// Pure: no network lookup.
    #[test]
    fn a_unc_terminal_folder_never_opens_its_share() {
        crate::mods::publish_panes(serde_json::json!([{"pane": "p1", "terminal": "t1", "cwd": r"\\server\share\x"}]));
        let expected = if cfg!(windows) { vec![PathBuf::from(r"\\server\share\x")] } else { Vec::new() };
        assert_eq!(crate::mods::local_pane_folders(), expected);
        crate::mods::publish_panes(serde_json::json!([]));
        let roots = [r"\\?\UNC\server\share\x".to_owned()];
        assert!(allowed(r"\\server\share\x\a.txt", &roots, true));
        assert!(allowed(r"\\?\UNC\SERVER\share\X/sub/b.txt", &roots, true));
        assert!(allowed(r"//server/share/x/a.txt", &roots, true));
        assert!(!allowed(r"\\server\share\y.txt", &roots, true));
        assert!(!allowed(r"\\server\share", &roots, true));
        assert!(!allowed(r"\\server\share\xy\a.txt", &roots, true));
        assert!(!allowed(r"\\other\share\x\a.txt", &roots, true));
    }
}

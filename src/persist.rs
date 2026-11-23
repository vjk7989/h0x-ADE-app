//! Atomic replacement for app-owned files.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Follow file symlinks, including relative links and links to missing files.
pub(crate) fn resolve_target(path: &Path) -> std::io::Result<PathBuf> {
    let mut target = path.to_path_buf();
    for _ in 0..40 {
        match std::fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.is_symlink() => {
                let link = std::fs::read_link(&target)?;
                target = target.parent().unwrap_or(Path::new("")).join(link);
            }
            Ok(_) => return Ok(target),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(target),
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("Too many symlinks"))
}

pub fn write_atomic(path: &Path, body: impl AsRef<[u8]>) -> std::io::Result<()> {
    let target = resolve_target(path)?;
    let path = target.as_path();
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".tmp.{}.{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let tmp = std::path::PathBuf::from(name);
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        // The replacement keeps the file's permissions: settings.json may be
        // private (0600), and a fresh file would take the umask's (0644).
        match std::fs::metadata(path) {
            Ok(existing) => file.set_permissions(existing.permissions())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        file.write_all(body.as_ref())?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn atomic_write_preserves_symlink_chains_and_dangling_targets() {
        let root = std::env::temp_dir().join(format!(
            "h0x-ade-persist-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let links = root.join("links");
        std::fs::create_dir_all(&links).unwrap();
        let target = root.join("settings.json");
        let intermediate = links.join("intermediate");
        let link = root.join("settings-link.json");
        symlink("../settings.json", &intermediate).unwrap();
        symlink(&intermediate, &link).unwrap();

        for body in ["created", "updated"] {
            write_atomic(&link, body).unwrap();
            assert_eq!(std::fs::read_to_string(&target).unwrap(), body);
            assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
            assert!(std::fs::symlink_metadata(&intermediate).unwrap().is_symlink());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn atomic_write_keeps_the_files_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("h0x-ade-persist-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("settings.json");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        write_atomic(&file, "created").unwrap();
        let fresh = mode(&file);
        for kept in [0o600, 0o640, 0o644] {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(kept)).unwrap();
            write_atomic(&file, "updated").unwrap();
            assert_eq!(mode(&file), kept, "{kept:o} after a rewrite");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "updated");
        }
        // Through a link, the target's permissions are the ones kept.
        let link = root.join("link.json");
        symlink(&file, &link).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_atomic(&link, "linked").unwrap();
        assert_eq!(mode(&file), 0o600);
        // A new file takes the process's default, as before.
        let new = root.join("new.json");
        write_atomic(&new, "new").unwrap();
        assert_eq!(mode(&new), fresh);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn atomic_write_rejects_symlink_cycles() {
        let link = std::env::temp_dir().join(format!("h0x-ade-persist-cycle-{}", std::process::id()));
        symlink(&link, &link).unwrap();
        assert!(write_atomic(&link, "ignored").is_err());
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        std::fs::remove_file(link).unwrap();
    }

    fn mode_scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("h0x-ade-persist-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn leftovers(root: &Path) -> Vec<String> {
        std::fs::read_dir(root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.contains(".tmp.")).collect()
    }

    #[test]
    fn atomic_write_keeps_the_extreme_modes_and_a_read_only_file_is_still_rewritten() {
        use std::os::unix::fs::PermissionsExt;
        let root = mode_scratch("mode-edges");
        let file = root.join("settings.json");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        write_atomic(&file, "created").unwrap();
        // Read-only (as before: the directory decides, so the rewrite goes through) and the two ends.
        for kept in [0o444, 0o400, 0o000, 0o777] {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(kept)).unwrap();
            write_atomic(&file, format!("body {kept:o}")).unwrap();
            assert_eq!(mode(&file), kept, "{kept:o} after a rewrite");
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("body {kept:o}"));
        }
        // An empty body is a rewrite too.
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
        write_atomic(&file, "").unwrap();
        assert_eq!((mode(&file), std::fs::read_to_string(&file).unwrap()), (0o640, String::new()));
        assert!(leftovers(&root).is_empty(), "{:?}", leftovers(&root));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_dangling_link_creates_its_target_with_the_default_mode() {
        use std::os::unix::fs::PermissionsExt;
        let root = mode_scratch("mode-dangling");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let fresh = root.join("fresh.json");
        write_atomic(&fresh, "x").unwrap();
        let link = root.join("link.json");
        symlink("missing.json", &link).unwrap();
        write_atomic(&link, "made").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(root.join("missing.json")).unwrap(), "made");
        assert_eq!(mode(&root.join("missing.json")), mode(&fresh));
        // A link to a private file: the target keeps 0600, the link stays a link.
        std::fs::set_permissions(root.join("missing.json"), std::fs::Permissions::from_mode(0o600)).unwrap();
        write_atomic(&link, "again").unwrap();
        assert_eq!(mode(&root.join("missing.json")), 0o600);
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_replacement_leaves_no_temporary_file() {
        let root = mode_scratch("mode-dir");
        let folder = root.join("settings.json");
        std::fs::create_dir_all(&folder).unwrap();
        // The folder's permissions copy onto the temporary file, then the rename fails.
        assert!(write_atomic(&folder, "body").is_err());
        assert!(folder.is_dir());
        assert!(leftovers(&root).is_empty(), "{:?}", leftovers(&root));
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// Windows has no mode bits: the read-only attribute is the permission that is kept.
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    fn leftovers(root: &Path) -> Vec<String> {
        std::fs::read_dir(root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.contains(".tmp.")).collect()
    }

    #[test]
    fn a_read_only_file_still_refuses_a_rewrite_and_no_temporary_file_is_left() {
        let root = std::env::temp_dir().join(format!("h0x-ade persist ro {}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("settings.json");
        write_atomic(&file, "old").unwrap();
        assert!(!std::fs::metadata(&file).unwrap().permissions().readonly(), "a new file is writable");
        write_atomic(&file, "updated").unwrap();
        assert!(!std::fs::metadata(&file).unwrap().permissions().readonly(), "a writable file stays writable");
        let mut read_only = std::fs::metadata(&file).unwrap().permissions();
        read_only.set_readonly(true);
        std::fs::set_permissions(&file, read_only).unwrap();
        // As before this change: replacing a read-only file is denied.
        let error = write_atomic(&file, "new").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "updated");
        // The temporary file took the read-only attribute and is still removed.
        assert!(leftovers(&root).is_empty(), "{:?}", leftovers(&root));
        let mut writable = std::fs::metadata(&file).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        writable.set_readonly(false);
        std::fs::set_permissions(&file, writable).unwrap();
        // A backslash path works the same way.
        let backslashed = PathBuf::from(file.to_string_lossy().replace('/', "\\"));
        write_atomic(&backslashed, "last").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "last");
        std::fs::remove_dir_all(root).unwrap();
    }
}

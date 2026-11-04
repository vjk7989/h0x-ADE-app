//! The DMG has no installer to put the app's CLI on PATH.

use std::io;
use std::path::Path;

fn h0x_bundle(exe: &Path) -> bool {
    exe.is_absolute() && exe.ends_with("HyperOx.app/Contents/MacOS/HyperOx")
}

fn installed_bundle(exe: &Path) -> bool {
    h0x_bundle(exe)
        && !exe.starts_with("/Volumes")
        && !exe.components().any(|c| c.as_os_str() == "AppTranslocation" || c.as_os_str() == "target")
}

pub(super) fn ensure(home: &Path, exe: &Path) -> io::Result<()> {
    if !installed_bundle(exe) {
        return Ok(());
    }
    let link = home.join(".local/bin/h0x-ade");
    match std::fs::symlink_metadata(&link) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let target = std::fs::read_link(&link)?;
            if target == exe {
                return Ok(());
            }
            let resolved = if target.is_absolute() { target } else { link.parent().unwrap().join(target) };
            if !h0x_bundle(&resolved) {
                return Ok(());
            }
            std::fs::remove_file(&link)?;
        }
        Ok(_) => return Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::fs::create_dir_all(link.parent().unwrap())?;
    std::os::unix::fs::symlink(exe, link)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn repairs_only_our_cli_link() {
        let home = std::env::temp_dir().join(format!("h0x-ade-cli-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let exe = home.join("Applications/HyperOx.app/Contents/MacOS/HyperOx");
        let link = home.join(".local/bin/h0x-ade");
        ensure(&home, &exe).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), exe);
        // Running it again leaves the same link alone.
        ensure(&home, &exe).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), exe);

        for old in [
            home.join("old/HyperOx.app/Contents/MacOS/HyperOx"),
            "../../Applications/HyperOx.app/Contents/MacOS/HyperOx".into(),
            "/Volumes/HyperOx/HyperOx.app/Contents/MacOS/HyperOx".into(),
            "/private/var/folders/AppTranslocation/old/d/HyperOx.app/Contents/MacOS/HyperOx".into(),
        ] {
            std::fs::remove_file(&link).unwrap();
            symlink(old, &link).unwrap();
            ensure(&home, &exe).unwrap();
            assert_eq!(std::fs::read_link(&link).unwrap(), exe);
        }
        let old = home.join("previous/HyperOx.app/Contents/MacOS/HyperOx");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&old, "previous executable").unwrap();
        std::fs::remove_file(&link).unwrap();
        symlink(&old, &link).unwrap();
        ensure(&home, &exe).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), exe);
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "previous executable");

        std::fs::remove_file(&link).unwrap();
        std::fs::write(&link, "personal CLI").unwrap();
        ensure(&home, &exe).unwrap();
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "personal CLI");
        std::fs::remove_file(&link).unwrap();
        let foreign = home.join("other-app");
        symlink(&foreign, &link).unwrap();
        ensure(&home, &exe).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), foreign);
        std::fs::remove_file(&link).unwrap();

        for skipped in [
            "/Volumes/HyperOx/HyperOx.app/Contents/MacOS/HyperOx",
            "/private/var/folders/AppTranslocation/random/d/HyperOx.app/Contents/MacOS/HyperOx",
            "/work/target/debug/HyperOx.app/Contents/MacOS/HyperOx",
            "/work/target/debug/h0x-ade",
            "/Applications/Other.app/Contents/MacOS/HyperOx",
        ] {
            ensure(&home, Path::new(skipped)).unwrap();
            assert!(link.symlink_metadata().is_err(), "{skipped}");
        }
        std::fs::remove_dir_all(home).unwrap();
    }
}

//! Account secrets (refresh tokens) in the OS secret store, never in a plain file
//! (same design as the core's bridge secrets, ADR-064). Windows: a DPAPI blob in
//! `<app home>/secrets/<name>.dpapi` (current user only). macOS: the login keychain
//! through `security -i`, the secret on stdin. Linux: `secret-tool` (libsecret), the
//! secret on stdin. On macOS and Linux, when the store is unusable (no tool, no keyring,
//! a hung or locked keychain) the secret goes to an owner-only (0600) file in the same folder.
//! Unit tests never touch the real store: on unix they use the owner-only file.

use std::io;
use std::path::{Path, PathBuf};

fn secrets_dir(dir: &Path) -> PathBuf {
    dir.join("secrets")
}

fn check(name: &str) -> io::Result<()> {
    let ok = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    if ok { Ok(()) } else { Err(io::Error::other("a secret needs a plain name")) }
}

fn read_file(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn remove_file(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// The secret `name` under the app home `dir`; `None` when there is none.
pub fn get(dir: &Path, name: &str) -> io::Result<Option<String>> {
    check(name)?;
    let found = imp::get(dir, name);
    #[cfg(unix)]
    {
        if let Ok(Some(value)) = found {
            return Ok(Some(value));
        }
        // A store that is unusable is not "no secret": the owner-only file may hold it.
        return match read_file(&secrets_dir(dir).join(name))? {
            Some(bytes) => String::from_utf8(bytes).map(Some).map_err(|_| io::Error::other("the stored secret is damaged")),
            None => found,
        };
    }
    #[cfg(windows)]
    found
}

/// Stores `value` as `name`, replacing any earlier one.
pub fn put(dir: &Path, name: &str, value: &str) -> io::Result<()> {
    check(name)?;
    std::fs::create_dir_all(secrets_dir(dir))?;
    #[cfg(unix)]
    {
        if imp::put(dir, name, value).is_ok() {
            return remove_file(&secrets_dir(dir).join(name));
        }
        return put_file(dir, name, value);
    }
    #[cfg(windows)]
    imp::put(dir, name, value)
}

/// Forgets `name`, in the store and in the owner-only file; fine when there was none.
pub fn del(dir: &Path, name: &str) -> io::Result<()> {
    check(name)?;
    let stored = imp::del(dir, name);
    #[cfg(unix)]
    {
        remove_file(&secrets_dir(dir).join(name))?;
    }
    stored
}

/// The owner-only file used when the OS store is unusable.
#[cfg(unix)]
fn put_file(dir: &Path, name: &str, value: &str) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| tracing::warn!("the OS secret store is unusable; account secrets are kept in an owner-only file"));
    let path = secrets_dir(dir).join(name);
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(value.as_bytes())
}

/// How long a `security` / `secret-tool` call may take before it counts as unusable.
#[cfg(all(unix, not(test)))]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Runs `program` with `input` on stdin; its stdout when it exits 0, `Ok(None)` for exit
/// code `not_found`. Errors never carry the output. A tool that outlives [`TIMEOUT`] is killed.
#[cfg(all(unix, not(test)))]
fn run(program: &str, args: &[&str], input: &[u8], not_found: i32) -> io::Result<Option<Vec<u8>>> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    let mut child =
        Command::new(program).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let _ = stdin.write_all(input); // a tool that exits early is judged by its status
    drop(stdin);
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let deadline = std::time::Instant::now() + TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other(format!("{program} did not answer")));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let stdout = reader.join().unwrap_or_default();
    match status.code() {
        Some(0) => Ok(Some(stdout)),
        Some(code) if code == not_found => Ok(None),
        code => Err(io::Error::other(format!("{program} failed ({code:?})"))),
    }
}

#[cfg(all(unix, not(test)))]
fn text(out: Vec<u8>) -> String {
    String::from_utf8_lossy(&out).trim().to_string()
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    fn blob(dir: &Path, name: &str) -> PathBuf {
        secrets_dir(dir).join(format!("{name}.dpapi"))
    }

    /// Runs one DPAPI call over `input`; copies the result and frees the system's buffer.
    fn crypt(input: &[u8], protect: bool) -> io::Result<Vec<u8>> {
        let len = u32::try_from(input.len()).map_err(|_| io::Error::other("secret too large"))?;
        let data = CRYPT_INTEGER_BLOB { cbData: len, pbData: input.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: `data` points at `input` for the call; `out` is filled by the
        // system and released with LocalFree below; the other arguments are optional.
        let ok = unsafe {
            if protect {
                CryptProtectData(
                    &data,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            } else {
                CryptUnprotectData(
                    &data,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            }
        };
        if ok == 0 {
            return Err(io::Error::other(if protect {
                "the secret could not be protected for this user"
            } else {
                "the stored secret could not be unprotected (damaged, or saved by another user)"
            }));
        }
        // SAFETY: on success `out` is `cbData` bytes owned by the system.
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        // SAFETY: allocated by DPAPI for us; freed once.
        unsafe { LocalFree(out.pbData as _) };
        Ok(bytes)
    }

    pub fn get(dir: &Path, name: &str) -> io::Result<Option<String>> {
        match read_file(&blob(dir, name))? {
            Some(sealed) => String::from_utf8(crypt(&sealed, false)?)
                .map(Some)
                .map_err(|_| io::Error::other("the stored secret is damaged")),
            None => Ok(None),
        }
    }

    pub fn put(dir: &Path, name: &str, value: &str) -> io::Result<()> {
        crate::persist::write_atomic(&blob(dir, name), crypt(value.as_bytes(), true)?)
    }

    pub fn del(dir: &Path, name: &str) -> io::Result<()> {
        remove_file(&blob(dir, name))
    }
}

/// Unit tests: no OS store, so every secret takes the owner-only file path.
#[cfg(all(unix, test))]
mod imp {
    use super::*;

    pub fn get(_: &Path, _: &str) -> io::Result<Option<String>> {
        Ok(None)
    }

    pub fn put(_: &Path, _: &str, _: &str) -> io::Result<()> {
        Err(io::Error::other("no store in tests"))
    }

    pub fn del(_: &Path, _: &str) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(all(target_os = "macos", not(test)))]
mod imp {
    use super::*;

    const SECURITY: &str = "/usr/bin/security";
    const SERVICE: &str = "h0x-ade";
    /// `security` exit code for "item not found".
    const NOT_FOUND: i32 = 44;

    /// A double-quoted argument for `security -i`.
    fn quote(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }

    /// The user's login keychain, named on every call so `security` never asks which
    /// one to use. Without the file `security` would wait on a dialog, so that is "unusable".
    fn keychain() -> io::Result<String> {
        let home = std::env::var_os("HOME").ok_or_else(|| io::Error::other("no HOME"))?;
        let path = Path::new(&home).join("Library/Keychains/login.keychain-db");
        if !path.is_file() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no login keychain"));
        }
        Ok(path.to_string_lossy().into_owned())
    }

    pub fn get(_: &Path, name: &str) -> io::Result<Option<String>> {
        let args = ["find-generic-password", "-s", SERVICE, "-a", name, "-w", &keychain()?];
        match run(SECURITY, &args, b"", NOT_FOUND) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None), // no keychain: nothing was stored
            found => Ok(found?.map(text)),
        }
    }

    pub fn put(_: &Path, name: &str, value: &str) -> io::Result<()> {
        // The command, with the secret in it, goes over stdin, never argv.
        let cmd = format!(
            "add-generic-password -U -s {} -a {} -w {} {}\n",
            quote(SERVICE),
            quote(name),
            quote(value),
            quote(&keychain()?)
        );
        run(SECURITY, &["-i"], cmd.as_bytes(), i32::MAX).map(|_| ())
    }

    pub fn del(_: &Path, name: &str) -> io::Result<()> {
        let args = ["delete-generic-password", "-s", SERVICE, "-a", name, &keychain()?];
        match run(SECURITY, &args, b"", NOT_FOUND) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            done => done.map(|_| ()),
        }
    }
}

#[cfg(all(unix, not(target_os = "macos"), not(test)))]
mod imp {
    use super::*;
    /// `secret-tool lookup` and `clear` exit 1 for no match.
    const NOT_FOUND: i32 = 1;

    pub fn get(_: &Path, name: &str) -> io::Result<Option<String>> {
        let args = ["lookup", "service", "h0x-ade", "key", name];
        match run("secret-tool", &args, b"", NOT_FOUND) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None), // no secret-tool: nothing was stored
            found => Ok(found?.map(text)),
        }
    }

    pub fn put(_: &Path, name: &str, value: &str) -> io::Result<()> {
        let args = ["store", "--label=h0x-ade", "service", "h0x-ade", "key", name];
        run("secret-tool", &args, value.as_bytes(), i32::MAX).map(|_| ())
    }

    pub fn del(_: &Path, name: &str) -> io::Result<()> {
        let args = ["clear", "service", "h0x-ade", "key", name];
        match run("secret-tool", &args, b"", NOT_FOUND) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()), // no tool: the file removal is what counts
            done => done.map(|_| ()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("h0x-secret-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_secret_round_trips_is_replaced_and_goes() {
        let dir = scratch("round");
        assert_eq!(get(&dir, "account.u1").unwrap(), None);
        put(&dir, "account.u1", "tok-1").unwrap();
        assert_eq!(get(&dir, "account.u1").unwrap().as_deref(), Some("tok-1"));
        put(&dir, "account.u1", "tok-2").unwrap();
        assert_eq!(get(&dir, "account.u1").unwrap().as_deref(), Some("tok-2"));
        del(&dir, "account.u1").unwrap();
        assert_eq!(get(&dir, "account.u1").unwrap(), None);
        del(&dir, "account.u1").unwrap(); // nothing there is fine
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_that_could_leave_the_secrets_folder_are_refused() {
        let dir = scratch("names");
        for name in ["", "../x", "a/b", "a\\b", "a b", "x\0y"] {
            assert!(put(&dir, name, "v").is_err(), "{name:?}");
            assert!(get(&dir, name).is_err(), "{name:?}");
            assert!(del(&dir, name).is_err(), "{name:?}");
        }
        assert!(!dir.exists());
    }

    #[test]
    fn the_stored_secret_is_not_in_plain_text_on_windows_and_owner_only_on_unix() {
        let dir = scratch("form");
        put(&dir, "account.u2", "plain-token-value").unwrap();
        let entries: Vec<_> = std::fs::read_dir(secrets_dir(&dir)).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(entries.len(), 1);
        #[cfg(windows)]
        {
            assert!(entries[0].to_string_lossy().ends_with("account.u2.dpapi"));
            let bytes = std::fs::read(&entries[0]).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("plain-token-value"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&entries[0]).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

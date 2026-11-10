//! `h0x-ade do <command> [--arg value ... | json]`, `h0x-ade ls`, `h0x-ade commands`
//! and `h0x-ade status`: drive a running window from a shell (agents, tests)
//! over `$H0X_ADE_HOME/h0x-ade.sock`, one JSON line each way.

use crate::mods::{self, Request};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn socket() -> PathBuf {
    mods::app_dir().join("h0x-ade.sock")
}

/// Serves the socket for this window. A live socket means another window owns this home.
pub fn listen(tx: async_channel::Sender<Request>) {
    let path = socket();
    if UnixStream::connect(&path).is_ok() {
        tracing::warn!("another HyperOx is serving {}; `h0x-ade do` goes there", path.display());
        return;
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => return tracing::warn!("cannot serve {}: {e}", path.display()),
    };
    // Only this user may drive the window.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
            let _ = std::fs::remove_file(&path);
            return tracing::warn!("cannot restrict {}: {e}", path.display());
        }
    }
    // Only this user (and SYSTEM) may drive the window.
    #[cfg(windows)]
    if let Err(e) = restrict_to_owner(&path) {
        let _ = std::fs::remove_file(&path);
        return tracing::warn!("cannot restrict {}: {e}", path.display());
    }
    let _ = std::thread::Builder::new().name("h0x-ade-control".into()).spawn(move || {
        let open = std::sync::Arc::new(AtomicUsize::new(0));
        for stream in listener.incoming().flatten() {
            let Some(slot) = Slot::take(&open) else { continue };
            let tx = tx.clone();
            std::thread::spawn(move || {
                let _slot = slot;
                serve(stream, tx);
            });
        }
    });
}

/// One of the `MAX_CONNECTIONS` places; freed when dropped, even by a panic.
struct Slot(std::sync::Arc<AtomicUsize>);

impl Slot {
    fn take(open: &std::sync::Arc<AtomicUsize>) -> Option<Slot> {
        if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            open.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Slot(open.clone()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The SID of the user running this process, from `whoami /user`: names (and
/// domains) are looked up by icacls and can differ or be spoofed by the environment.
#[cfg(windows)]
fn current_user_sid() -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("whoami").args(["/user", "/fo", "csv", "/nh"]).creation_flags(0x0800_0000).output().map_err(|e| e.to_string())?;
    sid_from_whoami(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| "no user SID".to_owned())
}

/// `"DOMAIN\user","S-1-5-21-…"` to the SID.
#[cfg(windows)]
fn sid_from_whoami(csv: &str) -> Option<String> {
    let sid = csv.trim().rsplit(',').next()?.trim_matches('"');
    // S-1-<digits>(-<digits>)+
    let parts = sid.strip_prefix("S-1-")?.split('-');
    let all_digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let parts: Vec<&str> = parts.collect();
    (parts.len() >= 2 && parts.into_iter().all(all_digits)).then(|| sid.to_owned())
}

/// The ACL of the socket file: this user and SYSTEM only, nothing inherited.
#[cfg(windows)]
fn restrict_to_owner(path: &std::path::Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let user = current_user_sid()?;
    let icacls = |build: &dyn Fn(&mut std::process::Command)| {
        let mut command = std::process::Command::new("icacls");
        command.arg(path).creation_flags(0x0800_0000);
        build(&mut command);
        let out = command.output().map_err(|e| e.to_string())?;
        if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stdout).trim().to_owned()) }
    };
    // Start from no explicit entries, then drop the inherited ones and grant the two.
    icacls(&|c| {
        c.arg("/reset");
    })?;
    icacls(&|c| {
        c.args(["/inheritance:r", "/grant:r"]).arg(format!("*{user}:F")).arg("*S-1-5-18:F").args(["/remove:g", "*S-1-5-32-544"]);
    })
}

/// A request is one JSON line; anything longer is not one.
const MAX_LINE: u64 = 64 * 1024;
const MAX_CONNECTIONS: usize = 16;

/// The request line, or why it is refused.
fn read_request(reader: impl BufRead) -> Result<String, String> {
    let mut line = String::new();
    reader.take(MAX_LINE + 1).read_line(&mut line).map_err(|e| format!("bad request: {e}"))?;
    if line.len() as u64 > MAX_LINE {
        return Err(format!("bad request: longer than {MAX_LINE} bytes"));
    }
    Ok(line)
}

/// `read_request` from a socket, which must deliver the whole line within `limit`:
/// a client sending a byte now and then does not keep its place.
fn read_request_within(stream: &UnixStream, limit: Duration) -> Result<String, String> {
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Ok(late) = stream.try_clone() {
        let done = done.clone();
        std::thread::spawn(move || {
            std::thread::sleep(limit);
            if !done.load(Ordering::SeqCst) {
                let _ = late.shutdown(std::net::Shutdown::Read);
            }
        });
    }
    let line = read_request(BufReader::new(stream));
    done.store(true, Ordering::SeqCst);
    line
}

fn serve(stream: UnixStream, tx: async_channel::Sender<Request>) {
    let line = match read_request_within(&stream, Duration::from_secs(10)) {
        Ok(line) => line,
        Err(e) => {
            let _ = (&stream).write_all(format!("{}\n", json!({ "error": e })).as_bytes());
            return;
        }
    };
    let reply = match serde_json::from_str::<Value>(&line) {
        Ok(cmd) => {
            let timeout = Duration::from_secs(if cmd.get("shot").is_some() { 30 } else { 5 });
            let (reply_tx, reply_rx) = std::sync::mpsc::channel();
            let _ = tx.send_blocking(Request::Control { cmd, reply: reply_tx });
            reply_rx.recv_timeout(timeout).unwrap_or_else(|_| json!({ "error": "the window did not answer" }))
        }
        Err(e) => json!({ "error": format!("bad request: {e}") }),
    };
    let _ = (&stream).write_all(format!("{reply}\n").as_bytes());
}

/// One HyperOx per home: a second launch brings the running window to the
/// front and quits. True when one is running (even one too busy to answer:
/// a second window would ring every alert twice).
pub fn raise_running(folder: Option<&str>) -> bool {
    raise_running_at(&socket(), folder)
}

fn raise_running_at(path: &std::path::Path, folder: Option<&str>) -> bool {
    let Ok(mut stream) = UnixStream::connect(path) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let mut ask = json!({ "raise": true });
    if let Some(folder) = folder {
        ask["folder"] = json!(folder);
    }
    if stream.write_all(format!("{ask}\n").as_bytes()).is_ok() {
        let _ = BufReader::new(&stream).read_line(&mut String::new());
    }
    true
}

/// The client side; returns the process exit code.
pub fn client(args: &[String]) -> i32 {
    if args == ["commands"] {
        println!("{}", serde_json::to_string_pretty(&crate::commands::listing()).unwrap());
        return 0;
    }
    let cmd = match args {
        [verb, rest @ ..] if verb == "shot" && rest.len() <= 1 => {
            let path = PathBuf::from(rest.first().map(String::as_str).unwrap_or("h0x-shot.png"));
            let path = match std::path::absolute(path) {
                Ok(path) => path,
                Err(e) => return fail(&format!("cannot resolve screenshot path: {e}")),
            };
            if !is_png(&path) {
                return fail("screenshot path must end in .png");
            }
            json!({ "shot": path })
        },
        [verb] if verb == "status" => json!({ "status": true }),
        [verb] if verb == "ls" => json!({ "ls": true }),
        [verb, message] if verb == "feedback" => json!({ "do": "app.feedback", "args": { "message": message, "from": "agent" } }),
        [verb, command, rest @ ..] if verb == "do" => match parse_args(rest) {
            Ok(a) => json!({ "do": command, "args": a }),
            Err(e) => return fail(&e),
        },
        _ => return fail("usage: h0x-ade ls | h0x-ade commands | h0x-ade status | h0x-ade shot [file.png] | h0x-ade do <command> [--arg value ... | json]"),
    };
    let mut stream = match UnixStream::connect(socket()) {
        Ok(s) => s,
        Err(_) if args.first().is_some_and(|v| v == "feedback") => {
            let body = crate::feedback::draft(&args[1], "", 0, "agent", "other");
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
            eprintln!("HyperOx must be open to review and send this feedback. Nothing was sent; core version and layout will reflect the open window.");
            return 1;
        }
        Err(e) => return fail(&format!("no HyperOx window at {}: {e}", socket().display())),
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(35)));
    let mut line = String::new();
    if stream.write_all(format!("{cmd}\n").as_bytes()).is_err() || BufReader::new(&stream).read_line(&mut line).is_err() {
        return fail("the window hung up");
    }
    let reply: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
    if args.first().is_some_and(|v| v == "shot") {
        if let Some(error) = reply.get("error") {
            return fail(error.as_str().unwrap_or("screenshot failed"));
        }
        let Some(path) = reply.get("path").and_then(Value::as_str) else {
            return fail("the window returned no screenshot path");
        };
        println!("{path}");
    } else {
        println!("{}", serde_json::to_string_pretty(&reply).unwrap_or_default());
    }
    if reply.get("error").is_some() { 1 } else { 0 }
}

/// `--name api --argv '["pi"]'` (values that parse as JSON are JSON), or one JSON object.
fn parse_args(rest: &[String]) -> Result<Value, String> {
    match rest {
        [] => Ok(Value::Null),
        [raw] if raw.trim_start().starts_with('{') => serde_json::from_str(raw).map_err(|e| format!("args are not JSON: {e}")),
        _ => {
            let mut out = serde_json::Map::new();
            let mut it = rest.iter();
            while let Some(flag) = it.next() {
                let key = flag.strip_prefix("--").ok_or_else(|| format!("expected --<arg>, got {flag:?}"))?;
                let value = it.next().ok_or_else(|| format!("--{key} needs a value"))?;
                let v = serde_json::from_str::<Value>(value).ok().filter(|v| !v.is_string()).unwrap_or_else(|| Value::String(value.clone()));
                out.insert(key.to_owned(), v);
            }
            Ok(Value::Object(out))
        }
    }
}

/// `shot` writes only `.png` files (any case); the window checks again, as
/// anything this user runs can write to the socket. A `:` in the file name
/// (an NTFS stream) and network paths (`\\server`, `//server`) are refused.
pub fn is_png(path: &std::path::Path) -> bool {
    let text = path.to_string_lossy();
    !text.starts_with(r"\\")
        && !text.starts_with("//")
        && !text.rsplit(['/', '\\']).next().unwrap_or_default().contains(':')
        && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
}

/// Encode before opening the destination: GPU errors must not truncate a good shot.
pub fn save_shot(window: &gpui::Window, path: &std::path::Path) -> anyhow::Result<()> {
    let image = window.render_to_image()?;
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, image.width(), image.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        encoder.write_header()?.write_image_data(image.as_raw())?;
    }
    // A sibling temporary file keeps publication on the same filesystem.
    let temporary = path.with_file_name(format!(".h0x-shot-{}.png", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let result = (|| -> anyhow::Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn fail(message: &str) -> i32 {
    eprintln!("h0x-ade: {message}");
    2
}

#[cfg(test)]
mod shot_tests {
    use super::is_png;
    use std::path::Path;

    #[test]
    fn a_shot_path_must_end_in_png() {
        assert!(!is_png(Path::new("")), "empty");
        assert!(!is_png(Path::new("a")), "no extension");
        assert!(!is_png(Path::new("a.png.exe")), "png is not the last extension");
        assert!(!is_png(Path::new(".png")), "a dot file named png has no extension");
        assert!(!is_png(Path::new("a.pngx")));
        assert!(is_png(Path::new("a.png")));
        assert!(is_png(Path::new("a.PNG")), "any case");
        assert!(is_png(Path::new("/tmp/shots/a.Png")), "unix dirs");
        assert!(is_png(Path::new(r"C:\Users\Me\My Shots\a.png")), "windows dirs");
        assert!(!is_png(Path::new("shots.png/a")), "a .png directory does not count");
    }

    #[test]
    fn a_shot_path_is_judged_by_its_last_extension_only() {
        assert!(is_png(Path::new(".PNG.png")), "a dot file with a second extension");
        assert!(is_png(Path::new("a.exe.PNG")));
        assert!(is_png(Path::new("C:/Users/Me/shots/a.png")), "windows forward slashes");
        assert!(!is_png(Path::new("a.png.")), "a trailing period is an empty extension");
        assert!(!is_png(Path::new("a.png ")), "a trailing space is part of the extension");
        assert!(!is_png(Path::new("a.png:evil")), "an NTFS stream name is part of the extension");
        assert!(!is_png(Path::new("a.p\u{0}ng")));
        assert!(!is_png(Path::new("a.\u{ff50}ng")), "a fullwidth letter is not png");
    }

    #[test]
    fn a_shot_path_is_never_a_stream_or_a_network_path() {
        assert!(!is_png(Path::new("a.exe:x.png")), "an NTFS stream of a.exe");
        assert!(!is_png(Path::new(r"C:\a\b.exe:x.png")), "a stream inside a folder");
        assert!(!is_png(Path::new(r"\\server\share\a.png")), "UNC");
        assert!(!is_png(Path::new("//server/share/a.png")), "UNC with forward slashes");
        assert!(!is_png(Path::new(r"\\?\UNC\server\share\a.png")), "verbatim UNC");
        assert!(is_png(Path::new(r"C:\a\b.png")), "a drive letter colon is not in the file name");
        assert!(is_png(Path::new("C:/a/b.png")));
        assert!(is_png(Path::new("/a/b.png")), "one leading slash is local");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_request_line_is_refused() {
        let ok = "{\"status\":true}\n";
        assert_eq!(read_request(ok.as_bytes()).unwrap(), ok);
        let exact = format!("{}\n", "a".repeat(MAX_LINE as usize - 1));
        assert_eq!(read_request(exact.as_bytes()).unwrap().len() as u64, MAX_LINE);
        let big = "a".repeat(MAX_LINE as usize + 10);
        assert!(read_request(big.as_bytes()).unwrap_err().contains("longer"));
        assert!(read_request(format!("{big}\n").as_bytes()).is_err());
        assert_eq!(read_request(&b""[..]).unwrap(), "");
    }

    #[test]
    fn the_line_limit_holds_however_the_bytes_arrive() {
        let max = MAX_LINE as usize;
        // Small buffers: the line reaches read_request in many pieces.
        for capacity in [1, 7, 4096, 65_535, 65_536, 65_537] {
            let read = |bytes: &[u8]| read_request(BufReader::with_capacity(capacity, bytes));
            let exact = format!("{}\n", "a".repeat(max - 1));
            assert_eq!(read(exact.as_bytes()).unwrap(), exact, "{capacity}");
            let over = format!("{}\n", "a".repeat(max));
            assert!(read(over.as_bytes()).unwrap_err().contains("longer"), "{capacity}");
            // No newline at all: exactly the limit is a request, one byte more is not.
            assert_eq!(read("a".repeat(max).as_bytes()).unwrap().len(), max, "{capacity}");
            assert!(read("a".repeat(max + 1).as_bytes()).is_err(), "{capacity}");
            // A two-byte character straddling the limit is refused, not cut in half.
            let straddle = format!("{}\u{e9}\n", "a".repeat(max - 1));
            assert!(read(straddle.as_bytes()).is_err(), "{capacity}");
            // Only the first line is the request.
            assert_eq!(read(b"{}\n{\"quit\":true}\n").unwrap(), "{}\n", "{capacity}");
        }
        // Not UTF-8: refused with the reason.
        assert!(read_request(&b"{\"a\":\"\xff\xfe\"}\n"[..]).unwrap_err().starts_with("bad request"));
        assert_eq!(read_request(&b"\n"[..]).unwrap(), "\n");
    }

    #[test]
    fn the_socket_answers_an_oversized_line_with_an_error_and_forwards_one_at_the_limit() {
        let max = MAX_LINE as usize;
        // `{"pad":"…"}\n` of exactly `len` bytes.
        let line = |len: usize| format!("{{\"pad\":\"{}\"}}\n", "a".repeat(len - 11));
        for (len, forwarded) in [(max, true), (max + 1, false), (max * 4, false)] {
            let (server, client) = UnixStream::pair().unwrap();
            let (tx, rx) = async_channel::unbounded();
            let served = std::thread::spawn(move || serve(server, tx));
            let request = line(len);
            assert_eq!(request.len(), len);
            let writer = std::thread::spawn(move || {
                // In pieces, as a slow client sends it.
                for piece in request.as_bytes().chunks(1000) {
                    if (&client).write_all(piece).is_err() {
                        break;
                    }
                }
                client
            });
            if forwarded {
                let Ok(Request::Control { cmd, reply }) = rx.recv_blocking() else { panic!("{len}: nothing forwarded") };
                assert_eq!(cmd["pad"].as_str().unwrap().len(), len - 11);
                reply.send(json!({ "ok": true })).unwrap();
            }
            served.join().unwrap();
            let client = writer.join().unwrap();
            let mut answer = String::new();
            BufReader::new(&client).read_line(&mut answer).unwrap();
            let answer: Value = serde_json::from_str(&answer).unwrap();
            if forwarded {
                assert_eq!(answer, json!({ "ok": true }));
            } else {
                assert!(answer["error"].as_str().unwrap().contains("longer"), "{len}: {answer}");
                assert!(rx.try_recv().is_err(), "{len}: an oversized line reached the window");
            }
        }
    }

    #[test]
    fn a_silent_client_is_dropped_with_an_error_and_nothing_reaches_the_window() {
        let (server, client) = UnixStream::pair().unwrap();
        let (tx, rx) = async_channel::unbounded();
        let started = std::time::Instant::now();
        // Half a request, then silence (the connection stays open).
        (&client).write_all(b"{\"status\"").unwrap();
        let served = std::thread::spawn(move || serve(server, tx));
        served.join().unwrap();
        let waited = started.elapsed();
        assert!(waited >= Duration::from_secs(9) && waited < Duration::from_secs(30), "{waited:?}");
        let mut answer = String::new();
        BufReader::new(&client).read_line(&mut answer).unwrap();
        assert!(serde_json::from_str::<Value>(&answer).unwrap()["error"].as_str().unwrap().starts_with("bad request"), "{answer}");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_slow_client_loses_its_place_after_the_limit_for_the_whole_line() {
        let dir = std::env::temp_dir().join(format!("h0x-ade-slow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("h0x-ade.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let mut client = UnixStream::connect(&path).unwrap();
        let (server, _) = listener.accept().unwrap();
        // One byte every 100 ms never ends the line; each read alone is well inside the limit.
        let drip = std::thread::spawn(move || {
            for _ in 0..40 {
                if client.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let started = std::time::Instant::now();
        let result = read_request_within(&server, Duration::from_millis(500));
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
        assert!(!result.is_ok_and(|line| line.ends_with('\n')));
        drip.join().unwrap();
        // A prompt client is served.
        let mut client = UnixStream::connect(&path).unwrap();
        let (server, _) = listener.accept().unwrap();
        client.write_all(b"{}\n").unwrap();
        assert_eq!(read_request_within(&server, Duration::from_millis(500)).unwrap(), "{}\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_connection_place_is_freed_even_when_the_handler_panics() {
        let open = std::sync::Arc::new(AtomicUsize::new(0));
        let slots: Vec<Slot> = (0..MAX_CONNECTIONS).map(|_| Slot::take(&open).unwrap()).collect();
        assert!(Slot::take(&open).is_none());
        assert_eq!(open.load(Ordering::SeqCst), MAX_CONNECTIONS);
        drop(slots);
        assert_eq!(open.load(Ordering::SeqCst), 0);
        let slot = Slot::take(&open).unwrap();
        let panicked = std::thread::spawn(move || {
            let _slot = slot;
            panic!("handler failed");
        })
        .join();
        assert!(panicked.is_err());
        assert_eq!(open.load(Ordering::SeqCst), 0);
        assert!(Slot::take(&open).is_some());
    }

    #[test]
    fn a_second_launch_raises_the_running_window() {
        let dir = std::env::temp_dir().join(format!("h0x-ade-raise-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("h0x-ade.sock");
        // Nothing running: this launch opens its own window.
        assert!(!raise_running_at(&path, None));
        // One running: it gets asked to come to the front.
        let listener = UnixListener::bind(&path).unwrap();
        let asked = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            (&stream).write_all(b"{\"ok\":true}\n").unwrap();
            serde_json::from_str::<Value>(&line).unwrap()
        });
        assert!(raise_running_at(&path, None));
        assert_eq!(asked.join().unwrap(), json!({ "raise": true }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn the_socket_file_is_restricted_to_its_owner_and_a_failed_restriction_is_an_error() {
        // A home with spaces, shell characters and non-ASCII letters, as a person may have.
        let dir = std::env::temp_dir().join(format!("h0x ade & (\u{e9}\u{65e5}) {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("h0x-ade.sock");
        std::fs::write(&path, b"").unwrap();
        restrict_to_owner(&path).unwrap();
        let out = std::process::Command::new("icacls").arg(&path).output().unwrap();
        let acl = String::from_utf8_lossy(&out.stdout).to_lowercase();
        let user = std::env::var("USERNAME").unwrap().to_lowercase();
        let sid = current_user_sid().unwrap();
        assert!(sid.starts_with("S-1-5-"), "{sid}");
        assert_eq!(sid_from_whoami("\"PC\\me\",\"S-1-5-21-1-2-3-1001\"\r\n").as_deref(), Some("S-1-5-21-1-2-3-1001"));
        assert_eq!(sid_from_whoami("ERROR: nothing"), None);
        assert_eq!(sid_from_whoami(""), None);
        assert!(acl.contains(&format!("{user}:(f)")), "{acl}");
        assert!(acl.contains("system:(f)"), "{acl}");
        for other in ["everyone", "users:", "authenticated users", "administrators", "(i)"] {
            assert!(!acl.contains(other), "{other} still has access: {acl}");
        }
        // Nothing to restrict: an error, so listen() serves nothing.
        assert!(restrict_to_owner(&dir.join("missing.sock")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sid_comes_from_odd_whoami_output_and_anything_that_is_not_a_sid_is_refused() {
        for (out, sid) in [
            // With the header row, unquoted, LF only, padded, a localized or odd account name.
            ("\"User Name\",\"SID\"\r\n\"PC\\me\",\"S-1-5-21-1-2-3-1001\"\r\n", "S-1-5-21-1-2-3-1001"),
            ("PC\\me,S-1-5-21-7-1001", "S-1-5-21-7-1001"),
            ("\"PC\\me\",\"S-1-5-21-7-1001\"\n", "S-1-5-21-7-1001"),
            ("  \"PC\\me\",\"S-1-5-21-7-1001\"  \r\n\r\n", "S-1-5-21-7-1001"),
            ("\"BÜRO\\José Müller\",\"S-1-5-21-4-1001\"\r\n", "S-1-5-21-4-1001"),
            ("\"PC\\a,\"\"b\"\"\",\"S-1-5-21-9-1001\"\r\n", "S-1-5-21-9-1001"),
            ("\"NT AUTHORITY\\SYSTEM\",\"S-1-5-18\"", "S-1-5-18"),
        ] {
            assert_eq!(sid_from_whoami(out).as_deref(), Some(sid), "{out:?}");
        }
        // Fails closed: an extra column, a lower-case or star-prefixed SID, anything icacls would read as more
        // than a SID, or no SID at all.
        for out in [
            "\"PC\\me\",\"S-1-5-21-1-1001\",\"extra\"",
            "\"PC\\me\",\"s-1-5-21-1-1001\"",
            "\"PC\\me\",\"*S-1-5-21-1-1001\"",
            "\"PC\\me\",\"S-1-5-21-1-1001:F\"",
            "\"PC\\me\",\"S-1-5-21-1-1001 /grant Everyone\"",
            "\"PC\\me\",\"S-1-5-21-1-1001\t\"x",
            "\"PC\\me\",\"\"",
            "\"PC\\me\"",
            "\"\"",
            ",",
            "\r\n",
            "INFO: Der Benutzer konnte nicht gefunden werden.",
        ] {
            assert_eq!(sid_from_whoami(out), None, "{out:?}");
        }
    }
}

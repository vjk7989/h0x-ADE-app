//! `h0x-ade do <command> [--arg value ... | json]`, `h0x-ade ls`, `h0x-ade commands`
//! and `h0x-ade status`: drive a running window from a shell (agents, tests)
//! over `$H0X_ADE_HOME/h0x-ade.sock`, one JSON line each way.

use crate::mods::{self, Request};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};
use std::path::PathBuf;
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
    let _ = std::thread::Builder::new().name("h0x-ade-control".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || serve(stream, tx));
        }
    });
}

fn serve(stream: UnixStream, tx: async_channel::Sender<Request>) {
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
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
pub fn raise_running() -> bool {
    raise_running_at(&socket())
}

fn raise_running_at(path: &std::path::Path) -> bool {
    let Ok(mut stream) = UnixStream::connect(path) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    if stream.write_all(format!("{}\n", json!({ "raise": true })).as_bytes()).is_ok() {
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
    fn a_second_launch_raises_the_running_window() {
        let dir = std::env::temp_dir().join(format!("h0x-ade-raise-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("h0x-ade.sock");
        // Nothing running: this launch opens its own window.
        assert!(!raise_running_at(&path));
        // One running: it gets asked to come to the front.
        let listener = UnixListener::bind(&path).unwrap();
        let asked = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            (&stream).write_all(b"{\"ok\":true}\n").unwrap();
            serde_json::from_str::<Value>(&line).unwrap()
        });
        assert!(raise_running_at(&path));
        assert_eq!(asked.join().unwrap(), json!({ "raise": true }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

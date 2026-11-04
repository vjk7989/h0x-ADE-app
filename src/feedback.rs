//! Explicit, person-reviewed feedback. No usage-data consent or install id.
//! Nothing is posted anywhere: sending opens a new issue on the app's GitHub
//! repository in the browser, prefilled with the reviewed draft. Screenshots
//! are what the person pastes or picks, each shown before sending; they stay
//! in the form's folder so the person can drag them into the issue.
use serde_json::{Value, json};
use std::path::PathBuf;

pub const MAX_IMAGES: usize = 3;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

const ISSUES_URL: &str = "https://github.com/vjk7989/h0x-ADE-app/issues/new";

pub fn draft(message: &str, email: &str, images: usize, from: &str, layout: &str) -> Value {
    json!({
        "v": 1,
        "from": if from == "agent" { "agent" } else { "person" },
        "message": message.trim(),
        "email": if email.trim().is_empty() { None } else { Some(email.trim()) },
        "app": {
            "version": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "channel": "stable",
        },
        "core": { "version": crate::update::core().version.as_deref().map(crate::telemetry::version) },
        "layout": if crate::window::shipped_layouts().contains(&layout) { layout } else { "other" },
        "images": images,
    })
}

/// Where the form keeps the screenshots it shows (its data folder: `image()` reads from the app home).
fn shots_dir() -> PathBuf { crate::mods::app_dir().join("data").join("frame-feedback") }

fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { Some("png") }
    else if bytes.starts_with(&[0xff, 0xd8, 0xff]) { Some("jpg") }
    else { None }
}

/// Keeps a pasted or picked image for the form: PNG or JPEG as is, any other
/// image as PNG. Returns its path from the app home.
pub fn attach(bytes: &[u8]) -> Result<String, String> {
    let (bytes, ext) = match sniff(bytes) {
        Some(ext) => (bytes.to_vec(), ext),
        None => {
            let img = image::load_from_memory(bytes).map_err(|_| "That isn't an image HyperOx can read.".to_owned())?;
            let mut png = std::io::Cursor::new(Vec::new());
            img.write_to(&mut png, image::ImageFormat::Png).map_err(|_| "Couldn't convert that image.".to_owned())?;
            (png.into_inner(), "png")
        }
    };
    if bytes.len() > MAX_IMAGE_BYTES { return Err("That image is over 5 MB.".into()); }
    let dir = shots_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    let name = format!("shot-{stamp}.{ext}");
    std::fs::write(dir.join(&name), bytes).map_err(|e| e.to_string())?;
    Ok(format!("data/frame-feedback/{name}"))
}

/// Picker and external drops share the same conversion and size limits.
pub fn attach_files(paths: &[PathBuf]) -> Vec<Value> {
    paths.iter().take(MAX_IMAGES).map(|p| {
        match std::fs::read(p) {
            Ok(bytes) => match attach(&bytes) {
                Ok(path) => json!({ "path": path }),
                Err(error) => json!({ "error": error }),
            },
            Err(_) => json!({ "error": "Couldn't read that file." }),
        }
    }).collect()
}

/// Drops the form's screenshots (a new form starts empty; a sent one is done with them).
pub fn clear_shots() {
    let Ok(entries) = std::fs::read_dir(shots_dir()) else { return };
    for e in entries.flatten() {
        if e.file_name().to_string_lossy().starts_with("shot-") { let _ = std::fs::remove_file(e.path()); }
    }
}

/// A path `attach` returned, checked again before it leaves the machine.
fn shot_file(rel: &str) -> Result<PathBuf, String> {
    let name = rel.strip_prefix("data/frame-feedback/").filter(|n| n.starts_with("shot-") && !n.contains(['/', '\\']))
        .ok_or("Unknown screenshot.")?;
    let path = shots_dir().join(name);
    let bytes = std::fs::read(&path).map_err(|_| "A screenshot is gone. Remove it and add it again.".to_owned())?;
    if bytes.len() > MAX_IMAGE_BYTES || sniff(&bytes).is_none() { return Err("A screenshot isn't a PNG or JPEG under 5 MB.".into()); }
    Ok(path)
}

fn validate(body: &Value) -> Result<(), String> {
    let message = body["message"].as_str().unwrap_or_default();
    if message.is_empty() { return Err("Write a message first.".into()); }
    if message.encode_utf16().count() > 4000 { return Err("Keep the message to 4000 characters.".into()); }
    if let Some(email) = body["email"].as_str() {
        let valid = email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.split_once('.').is_some_and(|(a, b)| !a.is_empty() && !b.is_empty())
                && !domain.contains('@')
        });
        if email.len() > 254 || !valid || email.chars().any(char::is_whitespace) {
            return Err("Enter a plain email address, or leave it blank.".into());
        }
    }
    Ok(())
}

/// Opens a new GitHub issue in the browser, prefilled with the reviewed draft.
/// Screenshots are checked but never uploaded: they stay in the form's folder
/// for the person to drag into the issue.
pub async fn send(body: Value, shots: Vec<String>) -> Result<(), String> {
    validate(&body)?;
    if shots.len() > MAX_IMAGES || body["images"].as_u64() != Some(shots.len() as u64) {
        return Err("Up to 3 screenshots.".into());
    }
    shots.iter().map(|s| shot_file(s)).collect::<Result<Vec<_>, _>>()?;
    crate::links::open(&crate::links::Target::Url(issue_url(&body)));
    Ok(())
}

/// `ISSUES_URL?title=…&body=…`: the message's first line as the title, the
/// whole draft in the body.
fn issue_url(body: &Value) -> String {
    let message = body["message"].as_str().unwrap_or_default();
    let title: String = message.lines().next().unwrap_or_default().chars().take(80).collect();
    let field = |k: &str| body[k].as_str().unwrap_or_default().to_owned();
    let email = field("email");
    let text = format!(
        "{message}\n\n---\n- from: {}\n- email: {}\n- app: {} ({} {})\n- core: {}\n- layout: {}\n- screenshots: {} (drag them into this issue)\n",
        field("from"),
        if email.is_empty() { "-" } else { email.as_str() },
        body["app"]["version"].as_str().unwrap_or_default(),
        body["app"]["os"].as_str().unwrap_or_default(),
        body["app"]["arch"].as_str().unwrap_or_default(),
        body["core"]["version"].as_str().unwrap_or("-"),
        field("layout"),
        body["images"].as_u64().unwrap_or(0),
    );
    format!("{ISSUES_URL}?title={}&body={}", encode(&title), encode(&text))
}

/// Percent-encodes everything but unreserved characters (RFC 3986).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn payload_is_text_only_and_custom_layouts_are_hidden() {
        let body = draft("  broken  ", "  ", 2, "agent", "private-layout");
        assert_eq!(body["message"], "broken");
        assert_eq!(body["email"], Value::Null);
        assert_eq!(body["from"], "agent");
        assert_eq!(body["layout"], "other");
        assert_eq!(body["images"], 2);
        assert_eq!(body.as_object().unwrap().len(), 8);
        assert!(body.get("install").is_none());
        assert_eq!(draft("ok", "a@b.c", 0, "person", "main")["layout"], "main");
    }
    #[test]
    fn invalid_message_and_email_are_explained() {
        for (message, email) in [("", ""), (&"x".repeat(4001), ""), ("ok", "bad"), ("ok", "a b@c.d")] {
            assert!(validate(&draft(message, email, 0, "person", "main")).is_err());
        }
        assert!(validate(&draft("ok", "a@b.c", 0, "person", "main")).is_ok());
    }
    #[test]
    fn the_issue_link_targets_the_repo_and_carries_the_draft() {
        let url = issue_url(&draft("Tabs vanish\nafter a resize & reload", "me@x.y", 1, "person", "main"));
        assert!(url.starts_with("https://github.com/vjk7989/h0x-ADE-app/issues/new?title=Tabs%20vanish&body="), "{url}");
        assert_eq!(url.matches('&').count(), 1, "everything else is encoded: {url}");
        assert!(url.contains("resize%20%26%20reload"), "{url}");
        assert!(url.contains("email%3A%20me%40x.y"), "{url}");
        assert!(url.contains("screenshots%3A%201"), "{url}");
        let blank = issue_url(&draft("x", "", 0, "person", "main"));
        assert!(blank.contains("email%3A%20-"), "{blank}");
        assert_eq!(encode(""), "");
        assert_eq!(encode("a b/\u{e9}"), "a%20b%2F%C3%A9");
    }

    #[test]
    fn only_png_and_jpeg_leave_as_they_are() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("jpg"));
        assert_eq!(sniff(b"GIF89a"), None);
        assert!(shot_file("data/frame-feedback/../../settings.json").is_err());
        assert!(shot_file("/etc/passwd").is_err());
    }
}

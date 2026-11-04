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
/// GitHub rejects a longer new-issue link.
const MAX_URL: usize = 8000;
const TRUNCATED: &str = "\n\n(truncated — paste the rest into the issue)";

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
        "core": { "version": crate::update::core().version.as_deref().map(version) },
        "layout": if crate::window::shipped_layouts().contains(&layout) { layout } else { "other" },
        "images": images,
    })
}

/// A release version (`0.2.0`, `0.2.0-rc.1`), else `other`: never a free string.
fn version(v: &str) -> String {
    let v = v.trim().trim_start_matches("h0x-core ").trim_start_matches('v');
    let number = |s: &str| !s.is_empty() && s.len() <= 4 && s.bytes().all(|b| b.is_ascii_digit());
    let (core, pre) = v.split_once('-').map_or((v, None), |(c, p)| (c, Some(p)));
    let parts: Vec<&str> = core.split('.').collect();
    let pre_ok = pre.is_none_or(|p| {
        let (label, n) = p.split_once('.').map_or((p, None), |(l, n)| (l, Some(n)));
        matches!(label, "alpha" | "beta" | "rc" | "preview") && n.is_none_or(number)
    });
    if parts.len() == 3 && parts.iter().all(|p| number(p)) && pre_ok { v.to_owned() } else { "other".into() }
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
/// whole draft in the body, cut (with a note) to keep the link under `MAX_URL`.
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
    let url = format!("{ISSUES_URL}?title={}&body=", encode(&title));
    let budget = MAX_URL - 1 - url.len();
    let body = encode(&text);
    if body.len() <= budget {
        return url + &body;
    }
    let room = budget - encode(TRUNCATED).len();
    let mut used = 0;
    let end = text
        .char_indices()
        .find_map(|(i, c)| {
            used += encode(c.encode_utf8(&mut [0; 4])).len();
            (used > room).then_some(i)
        })
        .unwrap_or(text.len());
    url + &encode(&text[..end]) + &encode(TRUNCATED)
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
    fn versions_are_releases_or_other() {
        assert_eq!(version("0.2.0"), "0.2.0");
        assert_eq!(version("h0x-core 0.1.5"), "0.1.5");
        assert_eq!(version("0.2.0-rc.1"), "0.2.0-rc.1");
        assert_eq!(version("0.2.0-my/secret path"), "other");
        assert_eq!(version("h0x-core 0.1.5-acmeclient"), "other");
        assert_eq!(version("0.1.5-rc.x"), "other");
        assert_eq!(version("dev"), "other");
    }

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
    fn a_long_draft_is_cut_on_a_character_to_keep_the_link_short() {
        for c in ["\u{e9}", "\u{1F600}"] {
            let url = issue_url(&draft(&c.repeat(4000), "", 0, "person", "main"));
            assert!(url.len() < MAX_URL, "{}", url.len());
            assert!(url.is_ascii());
            let body = url.split_once("&body=").unwrap().1;
            let kept = body.strip_suffix(&encode(TRUNCATED)).expect("ends with the truncation note");
            let one = encode(c);
            assert!(!kept.is_empty(), "{kept}");
            assert_eq!(kept, one.repeat(kept.len() / one.len()), "cut between characters, never inside one");
        }
        let short = issue_url(&draft("short", "", 0, "person", "main"));
        assert!(!short.contains("truncated"), "{short}");
    }
    #[test]
    fn the_issue_title_is_the_first_line_cut_at_80_characters() {
        let title = |url: &str| url.split_once("?title=").unwrap().1.split_once("&body=").unwrap().0.to_owned();
        // An empty draft still makes a well-formed link, with an empty title.
        let empty = issue_url(&draft("", "", 0, "person", "main"));
        assert!(empty.starts_with("https://github.com/vjk7989/h0x-ADE-app/issues/new?title=&body="), "{empty}");
        // 80 is kept whole; 200 is cut to 80 in the title but whole in the body.
        assert_eq!(title(&issue_url(&draft(&"a".repeat(80), "", 0, "person", "main"))), "a".repeat(80));
        let long = issue_url(&draft(&format!("{}\nsecond line", "b".repeat(200)), "", 0, "person", "main"));
        assert_eq!(title(&long), "b".repeat(80));
        assert!(long.contains(&format!("&body={}%0Asecond%20line", "b".repeat(200))), "{long}");
        // Non-ASCII is cut by character, never inside one, and fully encoded.
        let accents = issue_url(&draft(&"\u{e9}".repeat(200), "", 0, "person", "main"));
        assert_eq!(title(&accents), "%C3%A9".repeat(80));
        let emoji = issue_url(&draft("\u{1F600} \u{65e5}\u{672c}", "", 0, "person", "main"));
        assert_eq!(title(&emoji), "%F0%9F%98%80%20%E6%97%A5%E6%9C%AC");
        assert!(emoji.is_ascii(), "{emoji}");
        // A Windows line ending ends the first line too.
        assert_eq!(title(&issue_url(&draft("crash\r\nat start", "", 0, "person", "main"))), "crash");
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

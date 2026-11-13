//! HyperOx accounts (ADR-087). Sign-in happens in the user's browser on pavii.tech, which
//! form-POSTs the Firebase refresh token back to a one-shot listener on 127.0.0.1. The token
//! is proven by exchanging it at securetoken.googleapis.com, then kept in the OS secret store
//! (`secret.rs`); the non-secret profile is `<app home>/accounts.json`.
//! Several accounts can be signed in on one computer; one is current. A session that could not
//! be re-verified for 30 days stops counting (offline grace) but is kept until the user signs in.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The desktop API key of Firebase project pavii-ai3 ("HyperOx desktop"): public like any
/// browser key, limited to the Token Service and Identity Toolkit APIs and with no website
/// restriction. The website's own key only accepts requests from pavii.tech pages, so a
/// desktop app without a web referrer is refused with it.
const API_KEY: &str = "AIzaSyBoU_fZ2-5aNVDgxjLqSJq_GBUL4BfvG6o";
const PROJECT_ID: &str = "pavii-ai3";
const LOGIN_URL: &str = "https://www.pavii.tech/login/";
const GRACE_SECS: u64 = 30 * 24 * 60 * 60;
const SIGN_IN_WAIT: Duration = Duration::from_secs(5 * 60);
/// Largest callback body accepted.
const MAX_BODY: usize = 16 * 1024;
const MAX_HEAD: usize = 8 * 1024;
/// Most a local client may take to send its request, and how many may be served at once.
const CONN_DEADLINE: Duration = Duration::from_secs(5);
const MAX_CONNS: usize = 32;
/// securetoken answers that mean the session is over for good.
const REJECTED: [&str; 4] = ["TOKEN_EXPIRED", "USER_DISABLED", "USER_NOT_FOUND", "INVALID_REFRESH_TOKEN"];

/// A signed-in account (no secrets).
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub uid: String,
    /// "google.com" | "github.com" | "password"
    pub provider: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub picture: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    uid: String,
    provider: String,
    name: Option<String>,
    email: Option<String>,
    picture: Option<String>,
    /// Unix seconds of the last successful token check; missing means 0 (needs sign-in).
    #[serde(default)]
    last_verified: u64,
}

#[derive(Debug, Default, PartialEq, Serialize)]
struct Store {
    current: Option<String>,
    accounts: Vec<Entry>,
}

impl Entry {
    fn account(&self) -> Account {
        Account {
            uid: self.uid.clone(),
            provider: self.provider.clone(),
            name: self.name.clone(),
            email: self.email.clone(),
            picture: self.picture.clone(),
        }
    }
}

// ---- public API (the app home is `mods::app_dir()`) ----

/// The active signed-in account; `None` when none, or when its 30-day offline grace is over.
pub fn current() -> Option<Account> {
    current_in(&crate::mods::app_dir(), now())
}

/// Every account signed in on this computer.
pub fn accounts() -> Vec<Account> {
    load(&crate::mods::app_dir()).accounts.iter().map(Entry::account).collect()
}

pub fn signed_in() -> bool {
    current().is_some()
}

pub fn switch(uid: &str) -> Result<(), String> {
    switch_in(&crate::mods::app_dir(), uid)
}

/// Forgets the session of `uid` (the local profile data stays).
pub fn sign_out(uid: &str) -> Result<(), String> {
    sign_out_in(&crate::mods::app_dir(), uid)
}

/// Signs `uid` out and deletes `<app home>/profiles/<uid>/`.
pub fn remove(uid: &str) -> Result<(), String> {
    remove_in(&crate::mods::app_dir(), uid)
}

/// Blocks (up to 5 minutes) until the browser sign-in finishes. `open` gets the URL to show the
/// user. Errors: "cancelled", "timeout", "network: <detail>", "rejected: <reason>", or other text.
pub fn sign_in(open: impl FnOnce(&str)) -> Result<Account, String> {
    sign_in_in(&crate::mods::app_dir(), open, &securetoken, SIGN_IN_WAIT)
}

/// Makes a pending [`sign_in`] stop at once with `Err("cancelled")`; nothing is stored.
pub fn cancel_sign_in() {
    CANCELS.fetch_add(1, Ordering::SeqCst);
}

/// True until this computer has signed in or skipped once (the marker `<app home>/signin-done`).
/// The sign-in screen never comes back after that: signing out or removing accounts keeps the marker.
pub fn screen_needed() -> bool {
    screen_needed_in(&crate::mods::app_dir())
}

/// Records that the person chose to skip the sign-in screen.
pub fn skip_screen() -> Result<(), String> {
    write_marker(&crate::mods::app_dir())
}

/// Once at start (before `refresh_all`): a home that already has a saved account (0.1.27)
/// gets the marker, so the screen does not appear after a later sign-out or lapsed grace.
pub fn heal_marker() {
    heal_marker_in(&crate::mods::app_dir());
}

/// Re-checks every session's token. A network failure keeps the session (grace); a definitive
/// rejection removes it. Safe to call from a background thread.
pub fn refresh_all() {
    refresh_all_in(&crate::mods::app_dir(), &securetoken, now());
}

// ---- store ----

static LOCK: Mutex<()> = Mutex::new(());
static CANCELS: AtomicU64 = AtomicU64::new(0);

fn locked<T>(f: impl FnOnce() -> T) -> T {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}


fn valid_uid(uid: &str) -> bool {
    !uid.is_empty() && uid.len() <= 128 && uid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn secret_name(uid: &str) -> String {
    format!("account.{uid}")
}

/// The saved accounts. One bad entry (no uid or provider, or a uid unsafe for paths) is dropped
/// on its own; a file that is not JSON at all is empty. `current` must name a kept account.
fn load(dir: &Path) -> Store {
    let json: Option<serde_json::Value> =
        std::fs::read(dir.join("accounts.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let Some(json) = json else { return Store::default() };
    let accounts: Vec<Entry> = json["accounts"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|e| serde_json::from_value::<Entry>(e.clone()).ok())
                .filter(|e| valid_uid(&e.uid))
                .collect()
        })
        .unwrap_or_default();
    let current = json["current"].as_str().filter(|uid| accounts.iter().any(|e| e.uid == *uid)).map(str::to_string);
    Store { current, accounts }
}

/// A last check in the future (the clock was wrong) counts as now, so grace runs out 30 days
/// after the correction. True when anything changed.
fn clamp_future(store: &mut Store, now: u64) -> bool {
    let mut changed = false;
    for e in store.accounts.iter_mut().filter(|e| e.last_verified > now) {
        e.last_verified = now;
        changed = true;
    }
    changed
}

fn save(dir: &Path, store: &Store) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("accounts: {e}"))?;
    let json = serde_json::to_vec_pretty(store).map_err(|e| format!("accounts: {e}"))?;
    crate::persist::write_atomic(&dir.join("accounts.json"), json).map_err(|e| format!("accounts: {e}"))
}

fn within_grace(last_verified: u64, now: u64) -> bool {
    now.saturating_sub(last_verified) <= GRACE_SECS
}

fn current_in(dir: &Path, now: u64) -> Option<Account> {
    locked(|| {
        let mut store = load(dir);
        if clamp_future(&mut store, now) {
            let _ = save(dir, &store);
        }
        let uid = store.current?;
        store.accounts.iter().find(|e| e.uid == uid && within_grace(e.last_verified, now)).map(Entry::account)
    })
}

const MARKER: &str = "signin-done";

fn screen_needed_in(dir: &Path) -> bool {
    !dir.join(MARKER).exists()
}

fn write_marker(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("signin marker: {e}"))?;
    crate::persist::write_atomic(&dir.join(MARKER), "1").map_err(|e| format!("signin marker: {e}"))
}

fn heal_marker_in(dir: &Path) {
    if screen_needed_in(dir) && !locked(|| load(dir)).accounts.is_empty() {
        let _ = write_marker(dir);
    }
}

fn switch_in(dir: &Path, uid: &str) -> Result<(), String> {
    locked(|| {
        let mut store = load(dir);
        if !store.accounts.iter().any(|e| e.uid == uid) {
            return Err("unknown account".into());
        }
        store.current = Some(uid.to_string());
        save(dir, &store)
    })
}

fn sign_out_in(dir: &Path, uid: &str) -> Result<(), String> {
    if !valid_uid(uid) {
        return Err("invalid account id".into());
    }
    locked(|| {
        let mut store = load(dir);
        let before = store.accounts.len();
        store.accounts.retain(|e| e.uid != uid);
        if store.current.as_deref() == Some(uid) {
            store.current = store.accounts.first().map(|e| e.uid.clone());
        }
        if store.accounts.len() != before {
            save(dir, &store)?;
        }
        crate::secret::del(dir, &secret_name(uid)).map_err(|e| format!("secret: {e}"))
    })
}

fn remove_in(dir: &Path, uid: &str) -> Result<(), String> {
    sign_out_in(dir, uid)?;
    match std::fs::remove_dir_all(dir.join("profiles").join(uid)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("profile: {e}")),
        _ => Ok(()),
    }
}

/// Keeps a verified session and makes it current.
fn store_session(dir: &Path, claims: &Claims, refresh_token: &str, now: u64) -> Result<Account, String> {
    locked(|| {
        crate::secret::put(dir, &secret_name(&claims.uid), refresh_token).map_err(|e| format!("secret: {e}"))?;
        let mut store = load(dir);
        let entry = Entry {
            uid: claims.uid.clone(),
            provider: claims.provider.clone(),
            name: claims.name.clone(),
            email: claims.email.clone(),
            picture: claims.picture.clone(),
            last_verified: now,
        };
        let account = entry.account();
        match store.accounts.iter_mut().find(|e| e.uid == entry.uid) {
            Some(slot) => *slot = entry,
            None => store.accounts.push(entry),
        }
        store.current = Some(account.uid.clone());
        save(dir, &store)?;
        let _ = write_marker(dir); // a marker failure never fails the sign-in
        Ok(account)
    })
}

// ---- tokens ----

struct Exchanged {
    id_token: String,
    refresh_token: String,
    user_id: String,
    /// From `accounts:lookup`, only when the id token has no email claim.
    email: Option<String>,
}

enum Failure {
    /// The server says this session is over.
    Rejected(String),
    /// Anything else (offline, timeout, 5xx): try again later.
    Network(String),
}

impl Failure {
    fn message(self) -> String {
        match self {
            Failure::Rejected(r) => format!("rejected: {r}"),
            Failure::Network(d) => format!("network: {d}"),
        }
    }
}

type Exchange<'a> = &'a dyn Fn(&str) -> Result<Exchanged, Failure>;

/// Reads securetoken's `body\n<http code>` answer.
fn parse_exchange(out: &str) -> Result<Exchanged, Failure> {
    let net = |d: &str| Failure::Network(d.to_string());
    let (body, code) = out.rsplit_once('\n').ok_or_else(|| net("no answer"))?;
    let code: u16 = code.trim().parse().map_err(|_| net("no answer"))?;
    let json: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    if (200..300).contains(&code) {
        let field = |k: &str| json[k].as_str().filter(|s| !s.is_empty()).map(str::to_string);
        return match (field("id_token"), field("refresh_token"), field("user_id")) {
            (Some(id_token), Some(refresh_token), Some(user_id)) => Ok(Exchanged { id_token, refresh_token, user_id, email: None }),
            _ => Err(net("incomplete answer")),
        };
    }
    let message = json["error"]["message"].as_str().unwrap_or("");
    match REJECTED.iter().find(|r| message.starts_with(**r)) {
        Some(reason) if code == 400 => Err(Failure::Rejected((*reason).to_string())),
        _ => Err(Failure::Network(format!("HTTP {code}"))),
    }
}

fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Exchanges a refresh token at securetoken.googleapis.com (curl, https only, body on stdin).
fn securetoken(refresh_token: &str) -> Result<Exchanged, Failure> {
    let net = |e: String| Failure::Network(e);
    let url = format!("https://securetoken.googleapis.com/v1/token?key={API_KEY}");
    let body = format!("grant_type=refresh_token&refresh_token={}", form_encode(refresh_token));
    let mut curl = crate::spawn::command(crate::update::system("curl"));
    curl.args([
        "-sS",
        "--max-time",
        "10",
        "-H",
        "Content-Type: application/x-www-form-urlencoded",
        "--data-binary",
        "@-",
        "-w",
        "\n%{http_code}",
    ])
    .args(crate::update::CURL_HTTPS)
    .arg(&url);
    let mut child = curl
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| net(format!("curl: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(body.as_bytes()).map_err(|e| net(format!("curl: {e}")))?;
    }
    let out = child.wait_with_output().map_err(|e| net(format!("curl: {e}")))?;
    if !out.status.success() {
        return Err(net(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    let mut got = parse_exchange(&String::from_utf8_lossy(&out.stdout))?;
    // The project keeps one account per identity provider, so federated tokens carry no email.
    if let Ok(c) = decode_claims(&got.id_token)
        && c.email.is_none()
    {
        got.email = lookup_email(&got.id_token, &c.provider);
    }
    Ok(got)
}

/// The account email from Identity Toolkit `accounts:lookup`; any failure is just `None`.
/// The id token goes on stdin, never into argv or the log.
fn lookup_email(id_token: &str, provider: &str) -> Option<String> {
    let url = format!("https://identitytoolkit.googleapis.com/v1/accounts:lookup?key={API_KEY}");
    let mut curl = crate::spawn::command(crate::update::system("curl"));
    curl.args(["-sS", "--max-time", "5", "-H", "Content-Type: application/json", "--data-binary", "@-"])
        .args(crate::update::CURL_HTTPS)
        .arg(&url);
    let mut child = curl.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let body = serde_json::json!({ "idToken": id_token }).to_string();
    child.stdin.take()?.write_all(body.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_lookup(&String::from_utf8_lossy(&out.stdout), provider)
}

/// The email in an `accounts:lookup` answer: the entry of the sign-in provider, else the user's
/// own, else the first non-empty provider entry.
fn parse_lookup(body: &str, provider: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let user = json["users"].get(0)?;
    let text = |v: &serde_json::Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let infos = user["providerUserInfo"].as_array().map(Vec::as_slice).unwrap_or_default();
    infos
        .iter()
        .find(|i| i["providerId"].as_str() == Some(provider))
        .and_then(|i| text(&i["email"]))
        .or_else(|| text(&user["email"]))
        .or_else(|| infos.iter().find_map(|i| text(&i["email"])))
}

#[derive(Debug, PartialEq)]
struct Claims {
    uid: String,
    provider: String,
    name: Option<String>,
    email: Option<String>,
    picture: Option<String>,
}

/// The profile in a Firebase id token. The signature is not checked: the token came straight
/// from securetoken over TLS; this only reads it and requires it to be for our project.
fn decode_claims(id_token: &str) -> Result<Claims, String> {
    let mut parts = id_token.split('.');
    let (Some(_), Some(payload), Some(_), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err("not a token".into());
    };
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).map_err(|_| "token payload is not base64")?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "token payload is not JSON")?;
    let text = |v: &serde_json::Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_string);
    if json["aud"].as_str() != Some(PROJECT_ID) {
        return Err("token is for another project".into());
    }
    let uid = text(&json["user_id"]).or_else(|| text(&json["sub"])).ok_or("token has no user id")?;
    let provider = text(&json["firebase"]["sign_in_provider"]).ok_or("token has no sign-in provider")?;
    Ok(Claims { uid, provider, name: text(&json["name"]), email: text(&json["email"]), picture: text(&json["picture"]) })
}

/// Checks the refresh token with securetoken; the uid must be the one in the id token.
fn verify(refresh_token: &str, exchange: Exchange) -> Result<(Claims, String), String> {
    let got = exchange(refresh_token).map_err(Failure::message)?;
    let mut claims = decode_claims(&got.id_token).map_err(|e| format!("rejected: {e}"))?;
    claims.email = claims.email.or(got.email);
    if claims.uid != got.user_id || !valid_uid(&claims.uid) {
        return Err("rejected: account mismatch".into());
    }
    Ok((claims, got.refresh_token))
}

fn refresh_all_in(dir: &Path, exchange: Exchange, now: u64) {
    let uids: Vec<String> = locked(|| load(dir).accounts.into_iter().map(|e| e.uid).collect());
    for uid in uids {
        let token = match crate::secret::get(dir, &secret_name(&uid)) {
            Ok(Some(token)) => token,
            Ok(None) => {
                tracing::warn!("account session has no stored token; removing it");
                let _ = sign_out_in(dir, &uid);
                continue;
            }
            Err(e) => {
                tracing::warn!("account token unreadable, keeping the session: {e}");
                continue;
            }
        };
        match exchange(&token) {
            Err(Failure::Rejected(reason)) => {
                tracing::info!("account session ended: {reason}");
                let _ = sign_out_in(dir, &uid);
            }
            Err(Failure::Network(detail)) => tracing::info!("account not re-checked (kept): {detail}"),
            Ok(got) => {
                let claims = decode_claims(&got.id_token).ok().filter(|c| c.uid == uid && got.user_id == uid);
                let looked_up = got.email.clone();
                let Some(claims) = claims else {
                    tracing::warn!("account re-check gave an unusable token (kept)");
                    continue;
                };
                locked(|| {
                    let mut store = load(dir);
                    if let Some(slot) = store.accounts.iter_mut().find(|e| e.uid == uid) {
                        let sealed = crate::secret::put(dir, &secret_name(&uid), &got.refresh_token);
                        if let Err(e) = sealed {
                            tracing::warn!("account token not saved: {e}");
                        }
                        *slot = Entry {
                            uid: uid.clone(),
                            provider: claims.provider,
                            name: claims.name,
                            email: claims.email.or(looked_up).or_else(|| slot.email.clone()),
                            picture: claims.picture,
                            last_verified: now,
                        };
                        let _ = save(dir, &store);
                    }
                });
            }
        }
    }
}

// ---- browser callback ----

/// A fresh 43-character URL-safe state: 32 bytes from OS-seeded hashers, the clock and the pid.
fn new_state() -> String {
    use sha2::{Digest, Sha256};
    use std::hash::{BuildHasher, Hasher};
    let random = || std::collections::hash_map::RandomState::new().build_hasher().finish();
    let mut hasher = Sha256::new();
    for _ in 0..8 {
        hasher.update(random().to_le_bytes());
    }
    hasher.update(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos()).to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

#[derive(Debug, PartialEq)]
enum Form {
    Session { refresh_token: String },
    Cancelled,
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() && hex(b[i + 1]).is_some() && hex(b[i + 2]).is_some() => {
                out.push((hex(b[i + 1]).unwrap() * 16 + hex(b[i + 2]).unwrap()) as u8);
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The callback form; `Err` is why it is not for us (the listener keeps waiting).
fn parse_form(body: &str, state: &str) -> Result<Form, &'static str> {
    let (mut got_state, mut refresh, mut error) = (None, None, None);
    for pair in body.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match percent_decode(k).as_str() {
            "state" => got_state = Some(percent_decode(v)),
            "refresh_token" => refresh = Some(percent_decode(v)),
            "error" => error = Some(percent_decode(v)),
            _ => {}
        }
    }
    if got_state.as_deref() != Some(state) {
        return Err("wrong state");
    }
    match (refresh.filter(|t| !t.is_empty()), error.as_deref()) {
        (Some(refresh_token), None) => Ok(Form::Session { refresh_token }),
        (None, Some("cancelled")) => Ok(Form::Cancelled),
        _ => Err("incomplete"),
    }
}

enum Req {
    Request { method: String, path: String, body: String },
    TooBig,
    Bad,
}

/// One read into `buf`; false when the connection ended, failed, or `stop` says to give up.
/// The stream has a short read timeout, so `stop` is looked at often even if the client is silent.
fn read_more(stream: &mut TcpStream, buf: &mut Vec<u8>, stop: &dyn Fn() -> bool) -> bool {
    let mut chunk = [0u8; 2048];
    loop {
        if stop() {
            return false;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return false,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                return true;
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
}

fn read_request(stream: &mut TcpStream, stop: &dyn Fn() -> bool) -> Req {
    let mut buf = Vec::new();
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > MAX_HEAD || !read_more(stream, &mut buf, stop) {
            return Req::Bad;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let (Some(method), Some(path)) = (first.next(), first.next()) else { return Req::Bad };
    let length = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .map_or(Some(0), |(_, v)| v.trim().parse::<usize>().ok());
    let Some(length) = length else { return Req::Bad };
    if length > MAX_BODY {
        return Req::TooBig;
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        if !read_more(stream, &mut body, stop) {
            return Req::Bad;
        }
    }
    body.truncate(length);
    Req::Request { method: method.to_string(), path: path.to_string(), body: String::from_utf8_lossy(&body).into_owned() }
}

fn respond(mut stream: TcpStream, status: &str, message: &str) {
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>HyperOx</title>\
         <body style=\"font-family:sans-serif;text-align:center;margin-top:20vh\"><p>{message}</p>"
    );
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    let _ = stream.write_all(reply.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
}

/// Waits for the one valid callback. The browser's connection is returned open so the page can
/// say how the sign-in ended.
fn wait_for_callback(listener: &TcpListener, state: &str, wait: Duration, cancels: u64) -> Result<(TcpStream, Form), String> {
    listener.set_nonblocking(true).map_err(|e| format!("listener: {e}"))?;
    let deadline = Instant::now() + wait;
    let (tx, rx) = mpsc::channel();
    let active = Arc::new(AtomicUsize::new(0));
    loop {
        if CANCELS.load(Ordering::SeqCst) != cancels {
            return Err("cancelled".into());
        }
        if Instant::now() >= deadline {
            return Err("timeout".into());
        }
        let mut idle = Duration::from_millis(50);
        match listener.accept() {
            Ok((stream, _)) => {
                idle = Duration::ZERO;
                if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNS {
                    active.fetch_sub(1, Ordering::SeqCst);
                } else {
                    // Each client is read on its own thread, so a slow one never holds up the
                    // real callback, a cancel or the timeout.
                    let (tx, active, state) = (tx.clone(), active.clone(), state.to_string());
                    std::thread::spawn(move || {
                        serve(stream, &state, cancels, &tx);
                        active.fetch_sub(1, Ordering::SeqCst);
                    });
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(format!("listener: {e}")),
        }
        match rx.recv_timeout(idle) {
            Ok((stream, Form::Cancelled)) => {
                respond(stream, "200 OK", "Sign-in cancelled.");
                return Err("cancelled".into());
            }
            Ok(found) => return Ok(found),
            Err(_) => {}
        }
    }
}

/// Reads one client (at most [`CONN_DEADLINE`]); answers anything that is not the callback and
/// hands a valid one to the waiting sign-in with its connection still open.
fn serve(mut stream: TcpStream, state: &str, cancels: u64, tx: &mpsc::Sender<(TcpStream, Form)>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let give_up = Instant::now() + CONN_DEADLINE;
    let stop = || CANCELS.load(Ordering::SeqCst) != cancels || Instant::now() >= give_up;
    match read_request(&mut stream, &stop) {
        Req::TooBig => respond(stream, "413 Payload Too Large", "Too large."),
        Req::Bad => respond(stream, "400 Bad Request", "Bad request."),
        Req::Request { method, path, body } => {
            if method != "POST" || path != "/callback" {
                return respond(stream, "404 Not Found", "Not found.");
            }
            match parse_form(&body, state) {
                Err(_) => respond(stream, "400 Bad Request", "Bad request."),
                Ok(form) => {
                    let _ = tx.send((stream, form));
                }
            }
        }
    }
}

fn sign_in_in(dir: &Path, open: impl FnOnce(&str), exchange: Exchange, wait: Duration) -> Result<Account, String> {
    // One sign-in at a time: starting one cancels any pending one.
    let cancels = CANCELS.fetch_add(1, Ordering::SeqCst) + 1;
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("listener: {e}"))?;
    let port = listener.local_addr().map_err(|e| format!("listener: {e}"))?.port();
    if port < 1024 {
        return Err("listener: unusable port".into());
    }
    let state = new_state();
    open(&format!("{LOGIN_URL}?client=hyperox-desktop&port={port}&state={state}"));
    let (stream, form) = wait_for_callback(&listener, &state, wait, cancels)?;
    let Form::Session { refresh_token } = form else { return Err("cancelled".into()) };
    let done = verify(&refresh_token, exchange).and_then(|(claims, refresh)| {
        if CANCELS.load(Ordering::SeqCst) != cancels {
            return Err("cancelled".into());
        }
        store_session(dir, &claims, &refresh, now())
    });
    match &done {
        Ok(_) => respond(stream, "200 OK", "You're signed in to HyperOx \u{2014} you can close this tab."),
        Err(_) => respond(stream, "200 OK", "Sign-in failed \u{2014} go back to HyperOx and try again."),
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) static SIGN_IN_TESTS: Mutex<()> = Mutex::new(());

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("h0x-auth-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn jwt(payload: &serde_json::Value) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(payload.to_string()))
    }

    fn good_payload(uid: &str) -> serde_json::Value {
        serde_json::json!({
            "aud": PROJECT_ID, "user_id": uid, "sub": uid, "name": "Ada", "email": "ada@example.test",
            "picture": "https://example.test/a.png", "firebase": {"sign_in_provider": "github.com"}
        })
    }

    fn exchange_ok(uid: &'static str) -> impl Fn(&str) -> Result<Exchanged, Failure> {
        move |rt| {
            Ok(Exchanged { id_token: jwt(&good_payload(uid)), refresh_token: format!("{rt}-new"), user_id: uid.into(), email: None })
        }
    }

    fn entry(uid: &str, last_verified: u64) -> Entry {
        Entry { uid: uid.into(), provider: "google.com".into(), name: None, email: None, picture: None, last_verified }
    }

    #[test]
    fn state_is_43_url_safe_chars_and_fresh() {
        let (a, b) = (new_state(), new_state());
        for s in [&a, &b] {
            assert_eq!(s.len(), 43);
            assert!(s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'), "{s}");
            assert_eq!(URL_SAFE_NO_PAD.decode(s).unwrap().len(), 32);
        }
        assert_ne!(a, b);
    }

    #[test]
    fn form_fields_cancel_and_wrong_state() {
        let s = "abc";
        assert_eq!(
            parse_form("state=abc&id_token=x.y.z&refresh_token=r%2Fk+1&provider=google.com", s),
            Ok(Form::Session { refresh_token: "r/k 1".into() })
        );
        assert_eq!(parse_form("state=abc&error=cancelled", s), Ok(Form::Cancelled));
        assert_eq!(parse_form("state=nope&refresh_token=r", s), Err("wrong state"));
        assert_eq!(parse_form("refresh_token=r", s), Err("wrong state"));
        assert_eq!(parse_form("state=abc", s), Err("incomplete"));
        assert_eq!(parse_form("state=abc&refresh_token=", s), Err("incomplete"));
        assert_eq!(parse_form("state=abc&error=other", s), Err("incomplete"));
        assert_eq!(parse_form("", s), Err("wrong state"));
        // A bad escape is kept as written.
        assert_eq!(parse_form("state=abc&refresh_token=a%zz%4", s), Ok(Form::Session { refresh_token: "a%zz%4".into() }));
    }

    #[test]
    fn jwt_payload_good_bad_and_missing_fields() {
        let c = decode_claims(&jwt(&good_payload("u1"))).unwrap();
        assert_eq!(
            c,
            Claims {
                uid: "u1".into(),
                provider: "github.com".into(),
                name: Some("Ada".into()),
                email: Some("ada@example.test".into()),
                picture: Some("https://example.test/a.png".into())
            }
        );
        // Only a uid (via sub) and a provider are needed.
        let min = serde_json::json!({"aud": PROJECT_ID, "sub": "u2", "firebase": {"sign_in_provider": "password"}});
        let c = decode_claims(&jwt(&min)).unwrap();
        assert_eq!((c.uid.as_str(), c.name, c.email, c.picture), ("u2", None, None, None));
        for bad in ["", "abc", "a.b", "a.b.c.d", "a.!!!.c", &format!("a.{}.c", URL_SAFE_NO_PAD.encode("not json"))] {
            assert!(decode_claims(bad).is_err(), "{bad}");
        }
        let mut no_uid = good_payload("u1");
        no_uid.as_object_mut().unwrap().retain(|k, _| k != "user_id" && k != "sub");
        assert!(decode_claims(&jwt(&no_uid)).is_err());
        let no_provider = serde_json::json!({"aud": PROJECT_ID, "sub": "u1"});
        assert!(decode_claims(&jwt(&no_provider)).is_err());
        let mut other_project = good_payload("u1");
        other_project["aud"] = "someone-else".into();
        assert!(decode_claims(&jwt(&other_project)).is_err());
    }

    #[test]
    fn securetoken_answers_are_classified() {
        let ok = format!(
            "{{\"id_token\":\"i\",\"refresh_token\":\"r\",\"user_id\":\"u\"}}\n200"
        );
        assert!(matches!(parse_exchange(&ok), Ok(e) if e.user_id == "u" && e.refresh_token == "r"));
        for reason in REJECTED {
            let body = format!("{{\"error\":{{\"code\":400,\"message\":\"{reason}\"}}}}\n400");
            assert!(matches!(parse_exchange(&body), Err(Failure::Rejected(r)) if r == reason), "{reason}");
        }
        let other = ["{\"error\":{\"message\":\"QUOTA\"}}\n400", "{}\n500", "oops\n503", "{\"id_token\":\"i\"}\n200", "", "x\nnope"];
        for body in other {
            assert!(matches!(parse_exchange(body), Err(Failure::Network(_))), "{body}");
        }
        // TOKEN_EXPIRED on a non-400 is not definitive.
        assert!(matches!(parse_exchange("{\"error\":{\"message\":\"TOKEN_EXPIRED\"}}\n500"), Err(Failure::Network(_))));
    }

    #[test]
    fn grace_window_boundaries() {
        let day = 24 * 60 * 60;
        assert!(within_grace(1000, 1000));
        assert!(within_grace(1000, 1000 + 30 * day));
        assert!(!within_grace(1000, 1000 + 30 * day + 1));
        assert!(within_grace(5000, 1000), "a clock that went back keeps the session");
        let dir = scratch("grace");
        save(&dir, &Store { current: Some("u1".into()), accounts: vec![entry("u1", 1000)] }).unwrap();
        assert!(current_in(&dir, 1000 + 30 * day).is_some());
        assert!(current_in(&dir, 1000 + 30 * day + 1).is_none());
        assert_eq!(load(&dir).accounts.len(), 1, "an expired session is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accounts_json_round_trips_and_a_damaged_file_is_empty() {
        let dir = scratch("json");
        assert_eq!(load(&dir), Store::default());
        let store = Store {
            current: Some("u1".into()),
            accounts: vec![Entry { name: Some("Ada".into()), ..entry("u1", 7) }, entry("u2", 9)],
        };
        save(&dir, &store).unwrap();
        assert_eq!(load(&dir), store);
        std::fs::write(dir.join("accounts.json"), "{ nope").unwrap();
        assert_eq!(load(&dir), Store::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn switch_sign_out_and_remove_touch_only_their_account() {
        let dir = scratch("remove");
        for (uid, t) in [("u1", "t1"), ("u2", "t2")] {
            let claims = decode_claims(&jwt(&good_payload(uid))).unwrap();
            store_session(&dir, &claims, t, 100).unwrap();
            std::fs::create_dir_all(dir.join("profiles").join(uid)).unwrap();
            std::fs::write(dir.join("profiles").join(uid).join("data"), "x").unwrap();
        }
        assert_eq!(load(&dir).current.as_deref(), Some("u2"));
        switch_in(&dir, "u1").unwrap();
        assert_eq!(load(&dir).current.as_deref(), Some("u1"));
        assert!(switch_in(&dir, "nobody").is_err());
        // Sign out keeps the profile and the other account.
        sign_out_in(&dir, "u1").unwrap();
        assert!(dir.join("profiles/u1/data").exists());
        assert_eq!(crate::secret::get(&dir, "account.u1").unwrap(), None);
        assert_eq!(crate::secret::get(&dir, "account.u2").unwrap().as_deref(), Some("t2"));
        assert_eq!(load(&dir).current.as_deref(), Some("u2"), "the other account becomes current");
        sign_out_in(&dir, "u1").unwrap(); // again: fine
        // Remove deletes that profile only, even after sign-out.
        remove_in(&dir, "u1").unwrap();
        assert!(!dir.join("profiles/u1").exists());
        assert!(dir.join("profiles/u2/data").exists());
        remove_in(&dir, "u2").unwrap();
        assert!(!dir.join("profiles/u2").exists());
        assert_eq!(load(&dir), Store::default());
        for bad in ["", "..", "../u2", "a/b", "a\\b"] {
            assert!(remove_in(&dir, bad).is_err(), "{bad:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sign_in_screen_is_needed_only_until_a_skip_or_a_sign_in() {
        let dir = scratch("marker-skip");
        assert!(screen_needed_in(&dir));
        write_marker(&dir).unwrap();
        assert!(!screen_needed_in(&dir));
        write_marker(&dir).unwrap(); // twice is fine
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn signing_out_or_removing_the_last_account_keeps_the_screen_away() {
        let dir = scratch("marker-out");
        let claims = decode_claims(&jwt(&good_payload("u1"))).unwrap();
        store_session(&dir, &claims, "t1", 100).unwrap();
        assert!(!screen_needed_in(&dir));
        sign_out_in(&dir, "u1").unwrap();
        assert!(!screen_needed_in(&dir));
        remove_in(&dir, "u1").unwrap();
        assert!(!screen_needed_in(&dir));
        assert_eq!(load(&dir), Store::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_upgrader_with_any_saved_account_gets_the_marker_even_with_lapsed_grace_or_no_token() {
        let dir = scratch("marker-heal");
        heal_marker_in(&dir); // fresh home: nothing to heal
        assert!(screen_needed_in(&dir));
        // Lapsed grace, no current account, no secret stored.
        save(&dir, &Store { current: None, accounts: vec![entry("old", 1)] }).unwrap();
        assert!(screen_needed_in(&dir));
        heal_marker_in(&dir);
        assert!(!screen_needed_in(&dir));
        assert!(dir.join(MARKER).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_marker_that_cannot_be_written_is_an_error_and_the_screen_stays_needed() {
        let dir = scratch("marker-file");
        std::fs::write(&dir, "i am a file").unwrap();
        assert!(write_marker(&dir).is_err());
        assert!(screen_needed_in(&dir));
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn the_marker_works_in_a_path_with_spaces_and_non_ascii_characters() {
        let dir = scratch("marker Ana López ü");
        write_marker(&dir).unwrap();
        assert!(!screen_needed_in(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refresh_keeps_on_network_failure_removes_on_rejection_and_renews_on_success() {
        let dir = scratch("refresh");
        let claims = decode_claims(&jwt(&good_payload("u1"))).unwrap();
        store_session(&dir, &claims, "t1", 100).unwrap();
        refresh_all_in(&dir, &|_| Err(Failure::Network("offline".into())), 5000);
        assert_eq!(load(&dir).accounts[0].last_verified, 100);
        assert_eq!(crate::secret::get(&dir, "account.u1").unwrap().as_deref(), Some("t1"));
        refresh_all_in(&dir, &exchange_ok("u1"), 5000);
        assert_eq!(load(&dir).accounts[0].last_verified, 5000);
        assert_eq!(crate::secret::get(&dir, "account.u1").unwrap().as_deref(), Some("t1-new"));
        // A token for a different user is not accepted as a re-check.
        refresh_all_in(&dir, &exchange_ok("u9"), 9000);
        assert_eq!(load(&dir).accounts[0].last_verified, 5000);
        refresh_all_in(&dir, &|_| Err(Failure::Rejected("USER_DISABLED".into())), 9000);
        assert!(load(&dir).accounts.is_empty() && load(&dir).current.is_none());
        assert_eq!(crate::secret::get(&dir, "account.u1").unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A browser stand-in: POSTs `body` to the callback and returns the whole reply.
    fn post(port: u16, path: &str, body: &str) -> String {
        post_declaring(port, path, body, body.len())
    }

    /// Like `post`, but the Content-Length header says `declared`.
    fn post_declaring(port: u16, path: &str, body: &str, declared: usize) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let req = format!("POST {path} HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {declared}\r\n\r\n{body}");
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    fn url_part(url: &str, key: &str) -> String {
        let q = url.split_once('?').unwrap().1;
        q.split('&').find_map(|p| p.strip_prefix(&format!("{key}="))).unwrap().to_string()
    }

    /// Runs `sign_in_in` with a fake browser thread that runs `browser(port, state)`.
    fn run_sign_in(
        dir: &Path,
        exchange: Exchange,
        wait: Duration,
        browser: impl FnOnce(u16, String) + Send + 'static,
    ) -> (Result<Account, String>, String) {
        let seen = std::cell::RefCell::new((String::new(), None));
        let result = sign_in_in(
            dir,
            |url| {
                seen.borrow_mut().0 = url.to_string();
                let (port, state) = (url_part(url, "port").parse().unwrap(), url_part(url, "state"));
                seen.borrow_mut().1 = Some(std::thread::spawn(move || browser(port, state)));
            },
            exchange,
            wait,
        );
        let (url, handle) = seen.into_inner();
        if let Some(h) = handle {
            h.join().unwrap();
        }
        (result, url)
    }

    #[test]
    fn sign_in_end_to_end_ignores_strangers_and_stores_the_account() {
        let _g = SIGN_IN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch("e2e");
        let (result, url) = run_sign_in(&dir, &exchange_ok("u1"), Duration::from_secs(20), |port, state| {
            assert!(post(port, "/callback", "state=wrong&refresh_token=evil").contains("400"));
            assert!(post(port, "/other", "x=1").contains("404"));
            assert!(post_declaring(port, "/callback", "", MAX_BODY + 1).contains("413"));
            let ok = post(port, "/callback", &format!("state={state}&id_token=a.b.c&refresh_token=rt0&provider=github.com"));
            assert!(ok.contains("200") && ok.contains("signed in to HyperOx"), "{ok}");
        });
        assert!(url.starts_with("https://www.pavii.tech/login/?client=hyperox-desktop&port="), "{url}");
        assert_eq!(url_part(&url, "state").len(), 43);
        let account = result.unwrap();
        assert_eq!((account.uid.as_str(), account.provider.as_str(), account.name.as_deref()), ("u1", "github.com", Some("Ada")));
        assert_eq!(current_in(&dir, now()), Some(account));
        // The securetoken-issued token is the one kept, never the posted one.
        assert_eq!(crate::secret::get(&dir, "account.u1").unwrap().as_deref(), Some("rt0-new"));
        assert!(!std::fs::read_to_string(dir.join("accounts.json")).unwrap().contains("rt0"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sign_in_errors_are_stable_and_store_nothing() {
        let _g = SIGN_IN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch("errors");
        let wait = Duration::from_secs(20);
        let session = |port, state: String| {
            post(port, "/callback", &format!("state={state}&refresh_token=rt"));
        };
        let (r, _) = run_sign_in(&dir, &|_| Err(Failure::Network("offline".into())), wait, session);
        assert_eq!(r.unwrap_err(), "network: offline");
        let (r, _) = run_sign_in(&dir, &|_| Err(Failure::Rejected("USER_DISABLED".into())), wait, session);
        assert_eq!(r.unwrap_err(), "rejected: USER_DISABLED");
        let (r, _) = run_sign_in(&dir, &exchange_ok("u1"), wait, |port, state| {
            let page = post(port, "/callback", &format!("state={state}&error=cancelled"));
            assert!(page.contains("Sign-in cancelled"));
        });
        assert_eq!(r.unwrap_err(), "cancelled");
        let (r, _) = run_sign_in(&dir, &exchange_ok("u1"), Duration::from_millis(200), |_, _| {});
        assert_eq!(r.unwrap_err(), "timeout");
        // A uid that differs from securetoken's user_id is refused.
        let mismatch = |rt: &str| {
            Ok::<_, Failure>(Exchanged { id_token: jwt(&good_payload("u1")), refresh_token: rt.into(), user_id: "u2".into(), email: None })
        };
        let (r, _) = run_sign_in(&dir, &mismatch, wait, session);
        assert_eq!(r.unwrap_err(), "rejected: account mismatch");
        assert_eq!(load(&dir), Store::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_sign_in_stops_a_pending_sign_in_even_if_the_browser_posts_later() {
        let _g = SIGN_IN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch("cancel");
        let result = sign_in_in(&dir, |_| cancel_sign_in(), &exchange_ok("u1"), Duration::from_secs(20));
        assert_eq!(result.unwrap_err(), "cancelled");
        assert_eq!(load(&dir), Store::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn form_values_are_decoded_once_with_plus_as_a_space_and_2b_as_a_plus() {
        let s = "st-1_x";
        let session = |t: &str| -> Result<Form, &'static str> { Ok(Form::Session { refresh_token: t.into() }) };
        assert_eq!(parse_form("state=st-1_x&refresh_token=a%2Bb%2bc+d", s), session("a+b+c d"));
        assert_eq!(parse_form("state=st-1_x&refresh_token=%252B", s), session("%2B"), "decoded once only");
        assert_eq!(parse_form("state=st-1_x&refresh_token=%C3%A9", s), session("\u{e9}"));
        assert_eq!(parse_form("state=st-1_x&refresh_token=r%2", s), session("r%2"), "a cut-off escape is kept");
        // Encoded keys and an encoded state still match.
        assert_eq!(parse_form("%73tate=st%2D1%5Fx&refresh%5Ftoken=r", s), session("r"));
        // An empty token beside a cancel is a cancel; a token beside an error is neither.
        assert_eq!(parse_form("state=st-1_x&refresh_token=&error=cancelled", s), Ok(Form::Cancelled));
        assert_eq!(parse_form("state=st-1_x&refresh_token=rt&error=cancelled", s), Err("incomplete"));
        // A repeated field: the last one counts.
        assert_eq!(parse_form("state=st-1_x&state=nope&refresh_token=r", s), Err("wrong state"));
        assert_eq!(parse_form("state=nope&state=st-1_x&refresh_token=r", s), session("r"));
    }

    #[test]
    fn verify_refuses_another_project_another_user_and_a_uid_unsafe_for_paths() {
        fn answer(payload: serde_json::Value, user_id: String) -> impl Fn(&str) -> Result<Exchanged, Failure> {
            let id_token = jwt(&payload);
            move |rt: &str| Ok(Exchanged { id_token: id_token.clone(), refresh_token: rt.to_string(), user_id: user_id.clone(), email: None })
        }
        let mut other_project = good_payload("u1");
        other_project["aud"] = "pavii-ai2".into();
        let mut no_aud = good_payload("u1");
        no_aud.as_object_mut().unwrap().remove("aud");
        let mut aud_list = good_payload("u1");
        aud_list["aud"] = serde_json::json!([PROJECT_ID]);
        for payload in [other_project, no_aud, aud_list] {
            let err = verify("rt", &answer(payload.clone(), "u1".into())).err().unwrap();
            assert!(err.starts_with("rejected: "), "{payload}: {err}");
        }
        // securetoken says 200 but for a different user than the token.
        assert_eq!(verify("rt", &answer(good_payload("u1"), "u2".into())).err().unwrap(), "rejected: account mismatch");
        // Both agree on a uid that could leave a folder: still refused.
        for uid in ["../x", "..\\x", "a/b", "C:\\x", "u 1", ""] {
            let err = verify("rt", &answer(good_payload(uid), uid.into())).err().unwrap();
            assert!(err.starts_with("rejected: "), "{uid:?}: {err}");
        }
        // 128 characters is the longest uid.
        let longest = "a".repeat(128);
        let (claims, token) = verify("rt", &answer(good_payload(&longest), longest.clone())).unwrap();
        assert_eq!((claims.uid.len(), token.as_str()), (128, "rt"));
        let too_long = "a".repeat(129);
        assert_eq!(verify("rt", &answer(good_payload(&too_long), too_long.clone())).err().unwrap(), "rejected: account mismatch");
        // A 200 answer with an empty refresh token is not a session.
        let empty = "{\"id_token\":\"i\",\"refresh_token\":\"\",\"user_id\":\"u\"}\n200";
        assert!(matches!(parse_exchange(empty), Err(Failure::Network(_))));
    }

    #[test]
    fn accounts_json_ignores_unknown_fields_and_an_entry_without_last_verified_needs_sign_in() {
        let dir = scratch("json-shape");
        std::fs::create_dir_all(&dir).unwrap();
        let want = Store { current: Some("u1".into()), accounts: vec![entry("u1", 7)] };
        let extra = r#"{"current":"u1","future":1,"accounts":[{"uid":"u1","provider":"google.com","name":null,"email":null,"picture":null,"last_verified":7,"plan":"pro"}]}"#;
        std::fs::write(dir.join("accounts.json"), extra).unwrap();
        assert_eq!(load(&dir), want, "unknown fields are ignored");
        let minimal = r#"{"current":"u1","accounts":[{"uid":"u1","provider":"google.com","last_verified":7}]}"#;
        std::fs::write(dir.join("accounts.json"), minimal).unwrap();
        assert_eq!(load(&dir), want, "missing name, email and picture are none");
        let no_time = r#"{"current":"u1","accounts":[{"uid":"u1","provider":"google.com","last_verified":7},{"uid":"u2","provider":"github.com"}]}"#;
        std::fs::write(dir.join("accounts.json"), no_time).unwrap();
        // Spec change (AUTH-A1 fix pass): the entry stays, with last_verified 0, and so do the others.
        let loaded = load(&dir);
        let u2 = Entry { provider: "github.com".into(), ..entry("u2", 0) };
        assert_eq!(loaded.accounts, vec![entry("u1", 7), u2]);
        assert_eq!(current_in(&dir, 7).map(|a| a.uid), Some("u1".into()));
        switch_in(&dir, "u2").unwrap();
        assert_eq!(current_in(&dir, GRACE_SECS + 1), None, "last_verified 0 is long out of grace");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_drops_bad_entries_one_by_one_and_current_must_name_a_kept_account() {
        let dir = scratch("json-bad");
        std::fs::create_dir_all(&dir).unwrap();
        let text = r#"{"current":"../x","accounts":[
            {"uid":"../x","provider":"p","last_verified":1},
            {"uid":"","provider":"p","last_verified":1},
            {"provider":"p","last_verified":1},
            {"uid":"u1","last_verified":1},
            {"uid":"u2","provider":"p","last_verified":"soon"},
            {"uid":"ok_1-A","provider":"p","last_verified":5}]}"#;
        std::fs::write(dir.join("accounts.json"), text).unwrap();
        let loaded = load(&dir);
        assert_eq!(loaded.accounts.iter().map(|e| e.uid.as_str()).collect::<Vec<_>>(), ["ok_1-A"]);
        assert_eq!(loaded.current, None);
        let long = "a".repeat(129);
        let text = format!(r#"{{"current":"{long}","accounts":[{{"uid":"{long}","provider":"p"}}]}}"#);
        std::fs::write(dir.join("accounts.json"), text).unwrap();
        assert_eq!(load(&dir), Store::default());
        for not_json in ["", "[]", "null", "{\"accounts\":3}"] {
            std::fs::write(dir.join("accounts.json"), not_json).unwrap();
            assert_eq!(load(&dir), Store::default(), "{not_json:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_last_check_in_the_future_counts_as_now_so_grace_ends_30_days_after_the_correction() {
        let dir = scratch("future");
        save(&dir, &Store { current: Some("u1".into()), accounts: vec![entry("u1", 1_000_000_000_000)] }).unwrap();
        assert!(current_in(&dir, 5000).is_some());
        assert_eq!(load(&dir).accounts[0].last_verified, 5000, "the clamp is saved");
        assert!(current_in(&dir, 5000 + GRACE_SECS).is_some());
        assert!(current_in(&dir, 5001 + GRACE_SECS).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_uid_unsafe_for_paths_in_accounts_json_is_never_used_as_a_path() {
        let dir = scratch("traversal");
        std::fs::create_dir_all(dir.join("x")).unwrap();
        std::fs::write(dir.join("x").join("keep"), "x").unwrap();
        std::fs::create_dir_all(dir.join("profiles")).unwrap();
        let bad = ["../x", "..\\x"];
        save(&dir, &Store { current: Some(bad[0].into()), accounts: bad.iter().map(|u| entry(u, 100)).collect() }).unwrap();
        for uid in bad {
            assert!(remove_in(&dir, uid).is_err(), "{uid:?}");
            assert!(sign_out_in(&dir, uid).is_err(), "{uid:?}");
        }
        let asked = std::cell::Cell::new(0);
        refresh_all_in(
            &dir,
            &|_| {
                asked.set(asked.get() + 1);
                Err(Failure::Network("offline".into()))
            },
            200,
        );
        assert_eq!(asked.get(), 0, "no token is looked up for such a uid");
        assert!(dir.join("x").join("keep").exists(), "nothing outside profiles/ is touched");
        assert!(!dir.join("secrets").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_an_account_without_a_profile_folder_and_signing_out_another_keep_the_rest() {
        let dir = scratch("no-profile");
        for uid in ["u1", "u2", "u3"] {
            store_session(&dir, &decode_claims(&jwt(&good_payload(uid))).unwrap(), uid, 100).unwrap();
        }
        assert_eq!(load(&dir).current.as_deref(), Some("u3"));
        // Signing out an account that is not current leaves the current one.
        sign_out_in(&dir, "u1").unwrap();
        assert_eq!(load(&dir).current.as_deref(), Some("u3"));
        // No profiles folder at all: remove still works and the next account becomes current.
        assert!(!dir.join("profiles").exists());
        remove_in(&dir, "u3").unwrap();
        assert_eq!(load(&dir).current.as_deref(), Some("u2"));
        // A well-formed uid that was never signed in: nothing to do.
        remove_in(&dir, "never-signed-in").unwrap();
        assert_eq!(load(&dir).accounts.iter().map(|e| e.uid.as_str()).collect::<Vec<_>>(), ["u2"]);
        assert_eq!(crate::secret::get(&dir, "account.u2").unwrap().as_deref(), Some("u2"));
        assert_eq!(crate::secret::get(&dir, "account.u3").unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_grace_window_survives_extreme_clocks_and_a_refresh_with_a_clock_set_back() {
        assert!(within_grace(0, GRACE_SECS));
        assert!(!within_grace(0, GRACE_SECS + 1));
        assert!(!within_grace(0, u64::MAX));
        assert!(within_grace(u64::MAX, 0), "no overflow when the last check is in the future");
        let dir = scratch("clock-back");
        store_session(&dir, &decode_claims(&jwt(&good_payload("u1"))).unwrap(), "t1", 10_000).unwrap();
        refresh_all_in(&dir, &exchange_ok("u1"), 500);
        assert_eq!(load(&dir).accounts[0].last_verified, 500, "the check time is the clock's, even if earlier");
        assert!(current_in(&dir, 500 + GRACE_SECS).is_some());
        assert!(current_in(&dir, 501 + GRACE_SECS).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_secret_store_failure_during_sign_in_stores_no_account() {
        let _g = SIGN_IN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch("no-secret");
        std::fs::create_dir_all(&dir).unwrap();
        // `secrets` is a file, so the store cannot be written.
        std::fs::write(dir.join("secrets"), "not a folder").unwrap();
        let (result, _) = run_sign_in(&dir, &exchange_ok("u1"), Duration::from_secs(20), |port, state| {
            let page = post(port, "/callback", &format!("state={state}&refresh_token=rt0"));
            assert!(page.contains("Sign-in failed") && !page.contains("signed in to HyperOx"), "{page}");
        });
        let err = result.unwrap_err();
        assert!(err.starts_with("secret: "), "{err}");
        assert!(!dir.join("accounts.json").exists(), "no half-written account");
        assert_eq!(current_in(&dir, now()), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_guessed_or_altered_state_from_another_local_process_is_refused_and_nothing_is_stored() {
        let _g = SIGN_IN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch("guess");
        let never = |_: &str| -> Result<Exchanged, Failure> { panic!("a refused callback is never exchanged") };
        let (result, _) = run_sign_in(&dir, &never, Duration::from_secs(20), |port, state| {
            let guessed = new_state();
            assert_eq!(guessed.len(), state.len());
            assert_ne!(guessed, state);
            let mut bodies = vec![
                format!("state={guessed}&refresh_token=evil"),
                format!("state={}&refresh_token=evil", &state[..42]),
                format!("state={state}x&refresh_token=evil"),
                format!("state=%20{state}&refresh_token=evil"),
                format!("state={state}&state={guessed}&refresh_token=evil"),
                format!("state={guessed}&error=cancelled"),
            ];
            let upper = state.to_ascii_uppercase();
            if upper != state {
                bodies.push(format!("state={upper}&refresh_token=evil"));
            }
            for body in bodies {
                let page = post(port, "/callback", &body);
                assert!(page.contains("400") && !page.contains("signed in") && !page.contains("cancelled"), "{body}: {page}");
            }
            // The state in a GET query counts for nothing.
            let mut get = TcpStream::connect(("127.0.0.1", port)).unwrap();
            get.write_all(format!("GET /callback?state={state}&refresh_token=evil HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).unwrap();
            let mut page = String::new();
            let _ = get.read_to_string(&mut page);
            assert!(page.contains("404"), "{page}");
            // The real page ends it.
            post(port, "/callback", &format!("state={state}&error=cancelled"));
        });
        assert_eq!(result.unwrap_err(), "cancelled");
        assert_eq!(load(&dir), Store::default());
        assert!(!dir.join("secrets").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_huge_header_a_bad_length_or_a_silent_client_is_let_go_and_the_real_callback_still_lands() {
        let _g = SIGN_IN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch("slow");
        let (result, _) = run_sign_in(&dir, &exchange_ok("u1"), Duration::from_secs(60), |port, state| {
            let answered_soon = |mut s: TcpStream| {
                s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let start = Instant::now();
                let mut reply = String::new();
                let _ = s.read_to_string(&mut reply);
                assert!(start.elapsed() < Duration::from_secs(9), "the listener held the connection: {reply}");
                reply
            };
            // A header that never ends is cut off past MAX_HEAD.
            let mut big = TcpStream::connect(("127.0.0.1", port)).unwrap();
            let _ = big.write_all(format!("POST /callback HTTP/1.1\r\nX-Pad: {}", "a".repeat(MAX_HEAD * 2)).as_bytes());
            let reply = answered_soon(big);
            assert!(reply.is_empty() || reply.contains("400"), "{reply}");
            // A length that is not a number.
            let mut bad = TcpStream::connect(("127.0.0.1", port)).unwrap();
            bad.write_all(b"POST /callback HTTP/1.1\r\nContent-Length: lots\r\n\r\n").unwrap();
            assert!(answered_soon(bad).contains("400"));
            // A client that connects and says nothing is dropped after the read timeout.
            let silent = TcpStream::connect(("127.0.0.1", port)).unwrap();
            assert!(answered_soon(silent).contains("400"));
            // A body shorter than its length, then silence: dropped too.
            let mut short = TcpStream::connect(("127.0.0.1", port)).unwrap();
            short.write_all(b"POST /callback HTTP/1.1\r\nContent-Length: 100\r\n\r\nstate=").unwrap();
            assert!(answered_soon(short).contains("400"));
            let ok = post(port, "/callback", &format!("state={state}&refresh_token=rt0"));
            assert!(ok.contains("signed in to HyperOx"), "{ok}");
        });
        assert_eq!(result.unwrap().uid, "u1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_sign_in_cancels_the_first_and_nothing_is_stored_for_it() {
        // Spec change (AUTH-A1 fix pass): one sign-in at a time; the newer one wins.
        let _g = SIGN_IN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let (a, b) = (scratch("twice-a"), scratch("twice-b"));
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let (first, second) = std::thread::scope(|s| {
            let tx1 = tx.clone();
            let a = &a;
            let one = s.spawn(move || {
                let exchange = exchange_ok("u1");
                sign_in_in(a, |url| tx1.send(url.to_string()).unwrap(), &exchange, Duration::from_secs(20))
            });
            let first_url = rx.recv_timeout(Duration::from_secs(10)).unwrap();
            let b = &b;
            let two = s.spawn(move || {
                let exchange = exchange_ok("u2");
                sign_in_in(b, |url| tx.send(url.to_string()).unwrap(), &exchange, Duration::from_secs(20))
            });
            let second_url = rx.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_ne!(url_part(&first_url, "port"), url_part(&second_url, "port"));
            assert_ne!(url_part(&first_url, "state"), url_part(&second_url, "state"));
            // The first is over at once; the second still takes its callback.
            let first = one.join().unwrap();
            let (port, state) = (url_part(&second_url, "port").parse().unwrap(), url_part(&second_url, "state"));
            let ok = post(port, "/callback", &format!("state={state}&refresh_token=rt"));
            assert!(ok.contains("signed in to HyperOx"), "{ok}");
            (first, two.join().unwrap())
        });
        assert_eq!(first, Err("cancelled".to_string()));
        assert_eq!(second.unwrap().uid, "u2");
        assert_eq!(load(&a), Store::default());
        assert_eq!(load(&b).accounts.len(), 1);
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn a_slow_client_never_blocks_the_real_callback() {
        let _g = SIGN_IN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch("slowloris");
        let (result, _) = run_sign_in(&dir, &exchange_ok("u1"), Duration::from_secs(60), |port, state| {
            // Several clients that start a request and then stall (one drips a byte at a time).
            let mut stalled: Vec<TcpStream> = (0..4).map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap()).collect();
            stalled[0].write_all(b"POST /callback HTTP/1.1\r\nContent-Length: 100\r\n\r\nstate=").unwrap();
            stalled[1].write_all(b"POST /call").unwrap();
            let start = Instant::now();
            let ok = post(port, "/callback", &format!("state={state}&refresh_token=rt0"));
            assert!(ok.contains("signed in to HyperOx"), "{ok}");
            assert!(start.elapsed() < Duration::from_secs(3), "the real callback waited {:?}", start.elapsed());
            stalled.clear();
        });
        assert_eq!(result.unwrap().uid, "u1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_and_timeout_work_while_a_slow_client_holds_a_connection() {
        let _g = SIGN_IN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch("slow-cancel");
        let start = Instant::now();
        let (result, _) = run_sign_in(&dir, &exchange_ok("u1"), Duration::from_secs(60), |port, _| {
            let mut slow = TcpStream::connect(("127.0.0.1", port)).unwrap();
            slow.write_all(b"POST /callback HTTP/1.1\r\nContent-Length: 100\r\n\r\nstate=").unwrap();
            std::thread::sleep(Duration::from_millis(300));
            cancel_sign_in();
        });
        assert_eq!(result.unwrap_err(), "cancelled");
        assert!(start.elapsed() < Duration::from_secs(3), "{:?}", start.elapsed());
        let start = Instant::now();
        let (result, _) = run_sign_in(&dir, &exchange_ok("u1"), Duration::from_millis(400), |port, _| {
            let mut slow = TcpStream::connect(("127.0.0.1", port)).unwrap();
            slow.write_all(b"POST /callback HTTP/1.1\r\nContent-Length: 100\r\n\r\nstate=").unwrap();
            std::thread::sleep(Duration::from_millis(600));
        });
        assert_eq!(result.unwrap_err(), "timeout");
        assert!(start.elapsed() < Duration::from_secs(3), "{:?}", start.elapsed());
        assert_eq!(load(&dir), Store::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    fn jwt(email: Option<&str>) -> String {
        let mut p = serde_json::json!({"aud": PROJECT_ID, "user_id": "u1", "firebase": {"sign_in_provider": "google.com"}});
        if let Some(e) = email {
            p["email"] = e.into();
        }
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(p.to_string()))
    }

    #[test]
    fn lookup_reads_the_email_from_provider_info_or_the_user() {
        let only_info = r#"{"users":[{"localId":"u1","providerUserInfo":[{"providerId":"google.com","email":"a@x.test"}]}]}"#;
        assert_eq!(parse_lookup(only_info, "google.com").as_deref(), Some("a@x.test"));
        let top = r#"{"users":[{"email":"top@x.test","providerUserInfo":[{"providerId":"google.com","email":"a@x.test"}]}]}"#;
        assert_eq!(parse_lookup(top, "github.com").as_deref(), Some("top@x.test"));
        let multi = r#"{"users":[{"providerUserInfo":[{"providerId":"github.com","email":"g@x.test"},{"providerId":"google.com","email":"a@x.test"}]}]}"#;
        assert_eq!(parse_lookup(multi, "google.com").as_deref(), Some("a@x.test"));
        assert_eq!(parse_lookup(multi, "password").as_deref(), Some("g@x.test"));
    }

    #[test]
    fn lookup_without_an_email_or_with_bad_answers_is_none() {
        for body in [
            r#"{"users":[{"localId":"u1","providerUserInfo":[{"providerId":"google.com"}]}]}"#,
            r#"{"users":[]}"#,
            r#"{}"#,
            r#"{"users":[{"email":"","providerUserInfo":[{"providerId":"google.com","email":""}]}]}"#,
            r#"{"error":{"message":"INVALID_ID_TOKEN"}}"#,
            "not json",
            "",
        ] {
            assert_eq!(parse_lookup(body, "google.com"), None, "{body}");
        }
    }

    fn exchange(email: Option<&'static str>, token_email: Option<&'static str>) -> impl Fn(&str) -> Result<Exchanged, Failure> {
        move |rt| {
            Ok(Exchanged {
                id_token: jwt(token_email),
                refresh_token: rt.into(),
                user_id: "u1".into(),
                email: email.map(str::to_string),
            })
        }
    }

    #[test]
    fn refresh_fills_the_email_from_the_lookup_and_keeps_the_stored_one_when_it_fails() {
        let dir = std::env::temp_dir().join(format!("h0x-auth-lookup-refresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let claims = decode_claims(&jwt(None)).unwrap();
        store_session(&dir, &claims, "t1", 100).unwrap();
        assert_eq!(load(&dir).accounts[0].email, None);
        refresh_all_in(&dir, &exchange(Some("found@x.test"), None), 200);
        assert_eq!(load(&dir).accounts[0].email.as_deref(), Some("found@x.test"));
        // The lookup failed this time: the stored email stays.
        refresh_all_in(&dir, &exchange(None, None), 300);
        assert_eq!(load(&dir).accounts[0].email.as_deref(), Some("found@x.test"));
        // A claim in the token wins over the lookup.
        refresh_all_in(&dir, &exchange(Some("found@x.test"), Some("claim@x.test")), 400);
        assert_eq!(load(&dir).accounts[0].email.as_deref(), Some("claim@x.test"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_takes_the_lookup_email_only_when_the_token_has_none() {
        let (claims, _) = verify("rt", &exchange(Some("found@x.test"), None)).unwrap();
        assert_eq!(claims.email.as_deref(), Some("found@x.test"));
        let (claims, _) = verify("rt", &exchange(Some("found@x.test"), Some("claim@x.test"))).unwrap();
        assert_eq!(claims.email.as_deref(), Some("claim@x.test"));
        let (claims, _) = verify("rt", &exchange(None, None)).unwrap();
        assert_eq!(claims.email, None);
    }
}

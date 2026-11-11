//! Shares this machine's saved spaces with the core (`h0x-core bridge spaces`),
//! so the Telegram bot's /projects can list them (ADR-085). Local spaces only.
use crate::model::Model;
use std::time::Duration;

pub const DEBOUNCE: Duration = Duration::from_secs(2);

/// What the core was last given, and what is waiting to be sent.
#[derive(Default)]
pub struct State {
    sent: Option<String>,
    wanted: Option<String>,
    /// Bumped by every change; only the newest timer sends.
    epoch: u64,
    /// A core older than 0.1.15 has no `bridge spaces`: stop trying this session.
    unsupported: bool,
}

pub enum Outcome {
    Sent,
    Unsupported,
    Failed,
}

impl State {
    /// A new payload to send after the debounce: the epoch to wait on.
    pub fn changed(&mut self, payload: String) -> Option<u64> {
        if self.unsupported || self.wanted.as_ref().or(self.sent.as_ref()) == Some(&payload) { return None; }
        self.wanted = Some(payload);
        self.epoch += 1;
        Some(self.epoch)
    }

    /// The payload to send now, if `epoch` is still the newest.
    pub fn due(&self, epoch: u64) -> Option<String> {
        if self.unsupported || epoch != self.epoch { return None; }
        self.wanted.clone().filter(|w| self.sent.as_ref() != Some(w))
    }

    pub fn finished(&mut self, payload: String, outcome: Outcome) {
        match outcome {
            Outcome::Sent => self.sent = Some(payload),
            Outcome::Unsupported => self.unsupported = true,
            // Forget it so the next change (or start) tries again.
            Outcome::Failed => if self.wanted.as_ref() == Some(&payload) { self.wanted = None },
        }
    }
}

/// The local spaces with a folder, as the core's `[{"name","folder"}]`, sorted.
pub fn spaces_payload(model: &Model) -> String {
    let mut list: Vec<(&str, &str)> =
        model.spaces.iter().filter(|s| s.machine.is_none() && !s.folder.is_empty()).map(|s| (s.name.as_str(), s.folder.as_str())).collect();
    list.sort();
    serde_json::Value::Array(list.into_iter().map(|(name, folder)| serde_json::json!({"name": name, "folder": folder})).collect()).to_string()
}

/// Blocking: write the temp file (owner-only), run the core, remove the file.
pub fn send(payload: &str) -> Outcome {
    let file = crate::mods::app_dir().join("spaces-share.tmp");
    let _ = std::fs::remove_file(&file);
    let outcome = (|| {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        if let Err(e) = options.open(&file).and_then(|mut f| f.write_all(payload.as_bytes())) {
            tracing::debug!("spaces share: cannot write {}: {e}", file.display());
            return Outcome::Failed;
        }
        match crate::spawn::command(crate::core_bin::path()).args(["bridge", "spaces", "--json", "--file"]).arg(&file).output() {
            Ok(o) if o.status.success() => Outcome::Sent,
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                tracing::debug!("spaces share: core said {}", err.trim());
                if err.contains("unrecognized subcommand") || err.contains("unknown subcommand") { Outcome::Unsupported } else { Outcome::Failed }
            }
            Err(e) => {
                tracing::debug!("spaces share: cannot run the core: {e}");
                Outcome::Failed
            }
        }
    })();
    let _ = std::fs::remove_file(&file);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Space;

    fn space(name: &str, folder: &str, machine: Option<&str>) -> Space {
        Space { id: name.into(), name: name.into(), folder: folder.into(), machine: machine.map(str::to_owned), tabs: vec![], active: None }
    }

    #[test]
    fn only_local_spaces_with_a_folder_are_shared_sorted() {
        let mut m = Model::default();
        assert_eq!(spaces_payload(&m), "[]");
        m.spaces = vec![space("b", "/work/b", None), space("remote", "/srv", Some("box")), space("none", "", None), space("a", "/work/a", None)];
        assert_eq!(spaces_payload(&m), r#"[{"name":"a","folder":"/work/a"},{"name":"b","folder":"/work/b"}]"#);
    }

    #[test]
    fn bursts_coalesce_and_only_real_changes_send() {
        let mut s = State::default();
        let first = s.changed("[1]".into()).unwrap();
        let second = s.changed("[2]".into()).unwrap();
        assert!(s.due(first).is_none(), "an older timer does not send");
        assert_eq!(s.due(second).as_deref(), Some("[2]"));
        s.finished("[2]".into(), Outcome::Sent);
        assert!(s.changed("[2]".into()).is_none(), "unchanged since the last send");
        let third = s.changed("[3]".into()).unwrap();
        s.finished("[3]".into(), Outcome::Failed);
        assert!(s.due(third).is_none());
        assert!(s.changed("[3]".into()).is_some(), "a failed send is retried on the next change");
    }

    #[test]
    fn an_old_core_stops_the_attempts() {
        let mut s = State::default();
        let e = s.changed("[]".into()).unwrap();
        s.finished(s.due(e).unwrap(), Outcome::Unsupported);
        assert!(s.changed("[1]".into()).is_none());
    }
}

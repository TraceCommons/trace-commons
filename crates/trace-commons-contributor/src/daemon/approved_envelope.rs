//! The redacted envelope a contributor was actually shown, kept on disk so
//! the upload can send exactly those bytes.
//!
//! An earlier design pinned a *digest* of the previewed envelope on the
//! queue entry and re-derived it immediately before the upload, refusing if
//! the two disagreed. That guard was correct and it broke the feature under
//! the configuration the pilot actually runs. With `pii_filter =
//! "near-ai"` the redaction step is an LLM-backed service, and an LLM does
//! not return identical spans for identical text: preview pins D1, the
//! upload rebuilds and gets D2, the entry is refused and re-offered with
//! the pin cleared, the next preview pins D3 which will not reproduce
//! either. Nothing unsafe ships -- it fails closed -- but the primary
//! consent path never completes. Every test used the deterministic local
//! redactor, so nothing caught it.
//!
//! So the divergence class is eliminated rather than detected: the envelope
//! a preview built is written here, and the upload sends precisely those
//! bytes instead of building a second envelope and comparing. "What you saw
//! is what was sent" stops being an equality check and becomes literally
//! true.
//!
//! One bounded exception. The uploader stamps the contributor's verdict onto
//! `outcome.task_success` after loading and digest-checking these bytes, so
//! the envelope sent differs from the envelope stored by exactly that field.
//! The verdict is collected at approval time, after the preview was rendered,
//! and it is an output of the approval rather than an input that existed when
//! the preview was built. The digest pin therefore describes the previewed
//! bytes; it is not a claim about the final wire bytes. See
//! `envelope::apply_verdict`.
//!
//! **These files are redacted trace content at rest.** That is the same
//! deliberate, bounded exemption `preview` already carries (see the
//! `preview` module doc and the IPC contract's "The preview exemption"):
//! post-redaction content only, only for an entry the contributor asked
//! about, and never onward into a log line, an audit entry, a history
//! record, notification text, a receipt, or an IPC response. The bounds
//! this file adds are:
//!
//! * **Same protection as the device key.** The files live in the 0700
//!   state directory, are written 0600 through the same atomic temp-then
//!   -rename path as every other daemon file, and are removed by
//!   `ConfigStore::wipe` on logout.
//! * **Bounded.** One file per previewed-and-approved entry, each at most
//!   `MAX_ENVELOPE_BYTES`, and live entries are capped by
//!   `max_queue_entries`. An approved-but-unsent backlog is the only thing
//!   that can accumulate.
//! * **Deleted as soon as it is not needed.** `sweep` removes every stored
//!   envelope whose entry has reached a terminal state or has lost its pin
//!   -- to a revoked approval or an undone one -- and runs after every
//!   upload pass and every watcher tick.

use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use uuid::Uuid;

use crate::config::{ConfigStore, DAEMON_APPROVED_ENVELOPE_PREFIX};
use crate::envelope::MAX_ENVELOPE_BYTES;
use trace_commons_protocol::trace_contribution::TraceContributionEnvelope;

/// Stored envelopes are named `daemon-approved-envelope-{entry_id}.json`.
/// The variable part is a `Uuid` rendered by `Display`, so it can never
/// contain a path separator and the name can never escape the state
/// directory. The prefix is shared with `ConfigStore::wipe`, which sweeps
/// these on logout.
const FILE_PREFIX: &str = DAEMON_APPROVED_ENVELOPE_PREFIX;
const FILE_SUFFIX: &str = ".json";

pub fn file_name(entry_id: Uuid) -> String {
    format!("{FILE_PREFIX}{entry_id}{FILE_SUFFIX}")
}

/// Whether `name` is one of this module's files, and for which entry.
/// Returns `None` for anything else in the state directory, so a sweep can
/// never remove a file it does not own.
pub fn entry_id_of(name: &str) -> Option<Uuid> {
    name.strip_prefix(FILE_PREFIX)?
        .strip_suffix(FILE_SUFFIX)?
        .parse()
        .ok()
}

/// Persist the redacted envelope a preview just built for `entry_id`.
///
/// Refuses anything over `MAX_ENVELOPE_BYTES`: such an envelope would be
/// refused for size at upload anyway, and the cap is what bounds this
/// directory.
pub fn save(
    store: &ConfigStore,
    entry_id: Uuid,
    envelope: &TraceContributionEnvelope,
) -> Result<()> {
    let body = serde_json::to_vec(envelope).context("serializing approved envelope")?;
    if body.len() > MAX_ENVELOPE_BYTES {
        bail!("approved-envelope-too-large");
    }
    store.write_daemon_file(&file_name(entry_id), &body)
}

/// Read back the envelope stored for `entry_id`, or `None` when there is
/// none. An unreadable or unparseable file is an `Err`, never a silent
/// `None`: the caller must be able to tell "never stored" from "stored and
/// now unusable", because only the second one means an approval has to be
/// revoked.
pub fn load(store: &ConfigStore, entry_id: Uuid) -> Result<Option<TraceContributionEnvelope>> {
    let Some(body) = store.read_daemon_file(&file_name(entry_id))? else {
        return Ok(None);
    };
    if body.len() > MAX_ENVELOPE_BYTES {
        bail!("approved-envelope-too-large");
    }
    let envelope = serde_json::from_slice(&body).context("parsing stored approved envelope")?;
    Ok(Some(envelope))
}

pub fn remove(store: &ConfigStore, entry_id: Uuid) -> Result<()> {
    store.remove_daemon_file(&file_name(entry_id))
}

/// Delete every stored envelope not in `keep`.
///
/// `keep` is the set of entries that are still live *and* still pinned to a
/// preview (`Queue::pinned_entry_ids`). An entry that reached a terminal
/// state, or whose approval was revoked or undone -- both of which clear
/// the pin -- keeps no trace content on disk. Best-effort per file: one undeletable file does
/// not stop the rest.
pub fn sweep(store: &ConfigStore, keep: &HashSet<Uuid>) -> Result<()> {
    let entries = match std::fs::read_dir(store.dir()) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).context("reading contributor state dir"),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(id) = entry_id_of(&name) else {
            continue;
        };
        if !keep.contains(&id) {
            let _ = store.remove_daemon_file(&name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;

    /// A real redacted envelope, built by the same pipeline preview uses.
    async fn envelope() -> TraceContributionEnvelope {
        let (_d, store) = temp_store();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "http://issuer.invalid".into(),
            ingest_url: "http://ingest.invalid".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id,
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        let project = root.join("-Users-testuser-code-myproj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("44444444-4444-4444-4444-444444444444.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"},\
             \"cwd\":\"/Users/testuser/code/myproj\",\
             \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
             \"sessionId\":\"44444444-4444-4444-4444-444444444444\",\"uuid\":\"a1\"}\n",
        )
        .unwrap();
        let src = crate::source::claude_code::ClaudeCodeSource::new(root);
        let session_ref = crate::source::TraceSource::discover(&src)
            .unwrap()
            .remove(0);
        let transcript = crate::source::TraceSource::load(&src, &session_ref).unwrap();
        let raw = crate::envelope::build_raw_contribution(&transcript, &cfg, chrono::Utc::now());
        let redactor =
            crate::envelope::build_redactor_with(&cfg, transcript.cwd.as_deref(), None).unwrap();
        crate::envelope::redact_to_envelope(&redactor, raw)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn an_envelope_round_trips_byte_for_byte() {
        let (_d, store) = temp_store();
        let id = Uuid::new_v4();
        let e = envelope().await;
        save(&store, id, &e).unwrap();
        assert_eq!(load(&store, id).unwrap().unwrap(), e);
    }

    #[test]
    fn a_missing_envelope_reads_back_as_none_not_an_error() {
        let (_d, store) = temp_store();
        assert!(load(&store, Uuid::new_v4()).unwrap().is_none());
    }

    #[test]
    fn an_unparseable_envelope_is_an_error_not_a_silent_none() {
        // The caller has to be able to tell "never stored" from "stored and
        // now unusable": only the second one revokes an approval.
        let (_d, store) = temp_store();
        let id = Uuid::new_v4();
        store
            .write_daemon_file(&file_name(id), b"not json")
            .unwrap();
        assert!(load(&store, id).is_err());
    }

    #[tokio::test]
    async fn sweep_removes_only_envelopes_no_live_entry_still_needs() {
        let (_d, store) = temp_store();
        let keep_id = Uuid::new_v4();
        let drop_id = Uuid::new_v4();
        let e = envelope().await;
        save(&store, keep_id, &e).unwrap();
        save(&store, drop_id, &e).unwrap();
        sweep(&store, &HashSet::from([keep_id])).unwrap();
        assert!(load(&store, keep_id).unwrap().is_some());
        assert!(load(&store, drop_id).unwrap().is_none());
    }

    #[test]
    fn sweep_never_touches_a_file_it_does_not_own() {
        let (_d, store) = temp_store();
        store
            .write_daemon_file(crate::config::DAEMON_QUEUE_FILE, b"{}\n")
            .unwrap();
        sweep(&store, &HashSet::new()).unwrap();
        assert!(
            store
                .read_daemon_file(crate::config::DAEMON_QUEUE_FILE)
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn a_stored_envelope_is_removed_on_logout() {
        // It is redacted trace content at rest; a logout must not leave one
        // contributor's transcript on disk for whoever enrolls next.
        let (_d, store) = temp_store();
        let id = Uuid::new_v4();
        save(&store, id, &envelope().await).unwrap();
        store.wipe().unwrap();
        assert!(load(&store, id).unwrap().is_none());
    }
}

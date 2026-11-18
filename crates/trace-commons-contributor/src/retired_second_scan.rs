//! The retired client-side NEAR AI second scan.
//!
//! Until 2026-11 a contributor could set `pii_filter = "near-ai"` (with a
//! NEAR AI credential under `near_ai` in `daemon-settings.json`, or the
//! `TRACE_PRIVACY_FILTER_BACKEND=near-ai` environment switch) and the client
//! sent each trace's message text to a NEAR AI classifier before upload.
//! The owner retired it: an old setting is ignored, traces upload with
//! local scrubbing only, and the daemon says once that it ignored it. The
//! server-side privacy filter is separate and unaffected.
//!
//! Nothing here reads a value. Detection only asks whether a retired key
//! is present, and the one log line is a fixed label: no value, path, key
//! or credential reaches it.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::{CONFIG_FILE, ConfigStore, DAEMON_SETTINGS_FILE};

/// The fixed label of the one log line.
pub const LABEL_SETTING_IGNORED: &str = "second-scan-setting-ignored";

/// The environment switch that attached the same scan, and the one value of
/// it that is retired. Other backends (`sidecar`, `self-hosted`) are not
/// NEAR AI and stay as they are.
pub const ENV_BACKEND: &str = "TRACE_PRIVACY_FILTER_BACKEND";
const ENV_RETIRED_VALUE: &str = "near-ai";

/// Whether a raw `contributor.json` still names the retired filter. Any
/// value counts, not only `"near-ai"`: the whole key is retired.
pub fn config_names_it(raw: &serde_json::Value) -> bool {
    raw.get("pii_filter").is_some_and(|v| !v.is_null())
}

/// Whether a raw `daemon-settings.json` still holds the retired credential.
pub fn settings_names_it(raw: &serde_json::Value) -> bool {
    raw.get("near_ai").is_some_and(|v| !v.is_null())
}

/// Whether the environment, read through `get`, names the retired backend.
/// Mirrors the protocol crate's own reading of the variable (trimmed, then
/// exact), without building the adapter the way
/// `privacy_filter_backend_from_env` does.
pub fn env_names_it_with(get: impl Fn(&str) -> Option<String>) -> bool {
    get(ENV_BACKEND).is_some_and(|v| v.trim() == ENV_RETIRED_VALUE)
}

/// [`env_names_it_with`] over the process environment.
pub fn env_names_it() -> bool {
    env_names_it_with(|k| std::env::var(k).ok())
}

/// Whether either state file this machine holds still names the retired
/// scan. A file that is missing or does not parse names nothing: reading it
/// is the loaders' job, and they report their own failures.
pub fn configured(store: &ConfigStore) -> bool {
    let raw = |bytes: Option<Vec<u8>>| {
        bytes.and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    };
    let config = raw(std::fs::read(store.dir().join(CONFIG_FILE)).ok());
    let settings = raw(store.read_daemon_file(DAEMON_SETTINGS_FILE).ok().flatten());
    config.as_ref().is_some_and(config_names_it) || settings.as_ref().is_some_and(settings_names_it)
}

/// Drop the retired plaintext `near_ai` credential from
/// `daemon-settings.json`, so it does not sit on disk until some unrelated
/// save. Only that one key is removed from the raw document, written back
/// through the store's atomic daemon-file write: every other key, known or
/// not, is kept as it was, and the credential value is never read into a
/// log or an error. Returns whether the file was rewritten. A missing or
/// unparseable file, or one without the key, is left alone.
pub fn drop_retired_credential(store: &ConfigStore) -> bool {
    let Ok(Some(bytes)) = store.read_daemon_file(DAEMON_SETTINGS_FILE) else {
        return false;
    };
    let Ok(mut raw) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    if !settings_names_it(&raw) {
        return false;
    }
    let Some(object) = raw.as_object_mut() else {
        return false;
    };
    object.remove("near_ai");
    let Ok(body) = serde_json::to_vec_pretty(&raw) else {
        return false;
    };
    store.write_daemon_file(DAEMON_SETTINGS_FILE, &body).is_ok()
}

/// Emits the log line at most once over its lifetime.
pub struct OnceLog {
    fired: AtomicBool,
}

impl OnceLog {
    pub const fn new() -> Self {
        Self {
            fired: AtomicBool::new(false),
        }
    }

    /// Log the fixed label unless this instance already has. Returns whether
    /// this call logged.
    pub fn log(&self) -> bool {
        if self.fired.swap(true, Ordering::SeqCst) {
            return false;
        }
        tracing::warn!(
            label = LABEL_SETTING_IGNORED,
            "retired second-scan setting ignored; local scrubbing only"
        );
        // The daemon's installed subscriber keeps a warning's callsite and
        // drops its fields, so the label is also written as a label line,
        // where one is installed. Nothing else: no value, path or key.
        crate::daemon::diagnostics_log::record_labels(
            tracing::Level::WARN,
            LABEL_SETTING_IGNORED,
            &[],
        );
        true
    }
}

impl Default for OnceLog {
    fn default() -> Self {
        Self::new()
    }
}

static PROCESS_LOG: OnceLog = OnceLog::new();

/// Note that a retired setting was ignored: once per process.
pub fn note_ignored() {
    PROCESS_LOG.log();
}

/// Log once if anything this machine holds names the retired scan. Called
/// at daemon start and by the CLI's `submit`.
pub fn log_if_configured(store: &ConfigStore) {
    if configured(store) || env_names_it() {
        note_ignored();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn any_pii_filter_value_names_the_retired_scan() {
        assert!(config_names_it(&json!({ "pii_filter": "near-ai" })));
        assert!(config_names_it(&json!({ "pii_filter": "bogus" })));
        assert!(!config_names_it(&json!({ "pii_filter": null })));
        assert!(!config_names_it(&json!({ "tenant_id": "t" })));
    }

    #[test]
    fn a_near_ai_credential_names_the_retired_scan_and_its_namesakes_do_not() {
        assert!(settings_names_it(&json!({ "near_ai": { "api_key": "k" } })));
        assert!(!settings_names_it(&json!({ "near_ai": null })));
        // The inference and sign-in credentials share the prefix only.
        assert!(!settings_names_it(&json!({
            "near_ai_inference": { "api_key": "k" },
            "near_ai_session": {},
        })));
    }

    #[test]
    fn only_the_near_ai_backend_is_retired_from_the_environment() {
        let env = |v: &'static str| move |k: &str| (k == ENV_BACKEND).then(|| v.to_string());
        assert!(env_names_it_with(env("near-ai")));
        assert!(env_names_it_with(env("  near-ai\n")));
        assert!(!env_names_it_with(env("sidecar")));
        assert!(!env_names_it_with(env("self-hosted")));
        assert!(!env_names_it_with(env("")));
        assert!(!env_names_it_with(|_| None));
    }

    #[test]
    fn the_log_fires_once_with_the_fixed_label() {
        assert_eq!(LABEL_SETTING_IGNORED, "second-scan-setting-ignored");
        let log = OnceLog::new();
        assert!(log.log(), "the first call logs");
        assert!(!log.log(), "a second call does not");
        assert!(!log.log());
    }

    #[test]
    fn an_old_config_or_settings_file_is_detected_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().join("state")).unwrap();
        assert!(!configured(&store), "a fresh machine names nothing");

        let config_path = store.dir().join(CONFIG_FILE);
        std::fs::write(&config_path, br#"{"pii_filter":"near-ai"}"#).unwrap();
        assert!(configured(&store));
        std::fs::write(&config_path, br#"{"pii_filter":null}"#).unwrap();
        assert!(!configured(&store));

        store
            .write_daemon_file(DAEMON_SETTINGS_FILE, br#"{"near_ai":{"api_key":"k"}}"#)
            .unwrap();
        assert!(configured(&store));
    }

    #[test]
    fn the_retired_credential_is_dropped_from_disk_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().join("state")).unwrap();
        assert!(!drop_retired_credential(&store), "no file, nothing to do");

        store
            .write_daemon_file(
                DAEMON_SETTINGS_FILE,
                br#"{"near_ai":{"api_key":"k"},"canary_interval_secs":7,"near_ai_inference":null}"#,
            )
            .unwrap();
        assert!(drop_retired_credential(&store));
        let body = store
            .read_daemon_file(DAEMON_SETTINGS_FILE)
            .unwrap()
            .unwrap();
        let raw: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(raw.get("near_ai").is_none(), "the credential is gone");
        assert!(!String::from_utf8_lossy(&body).contains("\"k\""));
        assert_eq!(raw["canary_interval_secs"], 7, "other keys are kept");
        assert!(!configured(&store));
        assert!(!drop_retired_credential(&store), "second call is a no-op");
    }
}

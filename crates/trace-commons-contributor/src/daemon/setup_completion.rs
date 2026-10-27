//! Explicit CLI setup completion, bound to the current enrollment. Completing
//! setup records UI state; it never arms a folder or approves an upload.

use super::ipc::{
    DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, EVENT_STATUS_CHANGED, Request, Response,
};
use crate::config::{ConfigStore, ContributorConfig, DAEMON_SETUP_COMPLETION_FILE};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Read;

const MAX_RECORD_BYTES: u64 = 4096;
const VERSION: &str = "trace_commons.setup.v1";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    version: u8,
    scope: String,
}

pub(crate) fn scope(cfg: &ContributorConfig) -> String {
    let identity = (
        "trace-commons/setup-completion/v1",
        &cfg.tenant_id,
        &cfg.ingest_url,
        &cfg.issuer_url,
        &cfg.instance_id,
        &cfg.user_subject,
        &cfg.device_key_id,
    );
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&identity).expect("identity serializes"))
    )
}

fn read_completion(store: &ConfigStore) -> Result<Option<Completion>, &'static str> {
    let path = store.daemon_path(DAEMON_SETUP_COMPLETION_FILE);
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(m) if m.is_file() && !m.file_type().is_symlink() && m.len() <= MAX_RECORD_BYTES => {}
        _ => return Err("setup-state-unreadable"),
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| "setup-state-unreadable")?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("setup-state-unreadable");
    }
    let record: Completion =
        serde_json::from_slice(&bytes).map_err(|_| "setup-state-unreadable")?;
    if record.version != 1
        || record.scope.len() != 64
        || !record.scope.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("setup-state-unreadable");
    }
    Ok(Some(record))
}

fn readiness(
    cfg: Option<&ContributorConfig>,
    logged_in: bool,
    roots_declared: bool,
) -> Option<&'static str> {
    if !logged_in
        || cfg.is_none_or(|cfg| {
            cfg.tenant_id.trim().is_empty() || cfg.device_key_id.trim().is_empty()
        })
    {
        Some("setup-sign-in-required")
    } else if !roots_declared {
        Some("setup-roots-required")
    } else if crate::config::consent_hold(cfg).is_some() {
        Some("setup-scopes-required")
    } else {
        None
    }
}

/// Uses the same configuration snapshot as the status response's tenant ID.
/// A malformed marker never opens onboarding or changes any consent choice.
pub(crate) fn is_complete(
    store: &ConfigStore,
    cfg: Option<&ContributorConfig>,
    logged_in: bool,
    roots_declared: bool,
) -> bool {
    readiness(cfg, logged_in, roots_declared).is_none()
        && cfg.is_some_and(|cfg| {
            read_completion(store)
                .ok()
                .flatten()
                .is_some_and(|record| record.scope == scope(cfg))
        })
}

fn view(shared: &DaemonShared) -> Value {
    let cfg = shared.store.load_config();
    let roots = super::settings::roots_declared(&shared.settings.lock().expect("settings lock"));
    let cfg = match cfg {
        Ok(cfg) => cfg,
        Err(_) => {
            return json!({"schema_version":VERSION,"complete":false,"can_complete":false,"blocked_reason":"setup-state-unreadable","scope":null});
        }
    };
    let mut blocked = readiness(cfg.as_ref(), shared.logged_in(), roots);
    let record = read_completion(&shared.store);
    if record.is_err() {
        blocked = Some("setup-state-unreadable");
    }
    let current_scope = cfg.as_ref().map(scope);
    let complete = blocked.is_none()
        && record
            .ok()
            .flatten()
            .is_some_and(|r| Some(r.scope) == current_scope);
    json!({"schema_version":VERSION,"complete":complete,"can_complete":blocked.is_none(),
        "blocked_reason":blocked,"scope":current_scope,
        "consent_scopes":cfg.as_ref().map(|c| &c.consent_scopes),
        "consent_scopes_chosen":cfg.as_ref().and_then(|c| c.consent_scopes_chosen),
        "consent_options":super::enroll::consent_options()})
}

pub(super) fn handle_status(shared: &DaemonShared, req: &Request) -> Response {
    if req.params.as_object().is_none_or(|p| !p.is_empty()) {
        return Response::err(req.id, ERR_BAD_PARAMS, "setup-params-invalid");
    }
    Response::ok(req.id, view(shared))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompleteRequest {
    expected_scope: String,
    #[serde(default)]
    scopes: Option<Vec<String>>,
}

pub(super) fn handle_complete(shared: &DaemonShared, req: &Request) -> Response {
    match complete(shared, &req.params) {
        Ok(value) => Response::ok(req.id, value),
        Err((bad_params, label)) => Response::err(
            req.id,
            if bad_params {
                ERR_BAD_PARAMS
            } else {
                ERR_UNAVAILABLE
            },
            label,
        ),
    }
}

fn complete(shared: &DaemonShared, params: &Value) -> Result<Value, (bool, &'static str)> {
    let request: CompleteRequest =
        serde_json::from_value(params.clone()).map_err(|_| (true, "setup-params-invalid"))?;
    let scopes = request
        .scopes
        .as_ref()
        .map(|names| {
            if names.is_empty() {
                return Err((true, "scopes-invalid"));
            }
            crate::consent::validate_scopes(names).map_err(|_| (true, "scopes-invalid"))
        })
        .transpose()?;
    let Some(mut cfg) = shared
        .store
        .load_config()
        .map_err(|_| (false, "setup-state-unreadable"))?
    else {
        return Err((false, "setup-sign-in-required"));
    };
    if scope(&cfg) != request.expected_scope {
        return Err((false, "setup-account-changed"));
    }
    let roots = super::settings::roots_declared(&shared.settings.lock().expect("settings lock"));
    if let Some(reason) = readiness(Some(&cfg), shared.logged_in(), roots) {
        // An explicit choice can resolve missing scopes, but no mutation may
        // precede valid enrollment identity and declared source roots.
        if reason != "setup-scopes-required" || scopes.is_none() {
            return Err((false, reason));
        }
    }
    // Reject corrupt state before an optional data-use write.
    read_completion(&shared.store).map_err(|label| (false, label))?;
    let scopes_saved = scopes.is_some();
    if let Some(scopes) = scopes {
        cfg = super::enroll::apply_setup_scopes(shared, &cfg, &scopes)
            .map_err(|label| (false, label))?;
    }
    if let Some(reason) = readiness(Some(&cfg), true, roots) {
        return Err((false, reason));
    }
    let current_scope = scope(&cfg);
    let mut already_complete = false;
    let result = super::commons_credentials::with_current_config(&shared.store, &cfg, || {
        // The credential commit lock excludes enrollment switches and other
        // completion writers until this atomic marker is durably published.
        already_complete = read_completion(&shared.store)
            .map_err(anyhow::Error::msg)?
            .is_some_and(|r| r.scope == current_scope);
        if !already_complete {
            let bytes = serde_json::to_vec(&Completion {
                version: 1,
                scope: current_scope.clone(),
            })?;
            shared
                .store
                .write_daemon_file(DAEMON_SETUP_COMPLETION_FILE, &bytes)?;
            super::cloud_credential_lifecycle::sync_directory(&shared.store)?;
        }
        Ok(())
    });
    if result.is_err() {
        return Err((
            false,
            if scopes_saved {
                "setup-completion-failed-scopes-saved"
            } else {
                "setup-completion-failed"
            },
        ));
    }
    if !already_complete || scopes_saved {
        shared.publish(EVENT_STATUS_CHANGED, json!({}));
    }
    let mut result = view(shared);
    // A concurrent account switch after the commit must not claim completion
    // for the replacement enrollment.
    if result["scope"].as_str() != Some(&current_scope) {
        return Err((false, "setup-account-changed"));
    }
    if result["complete"] != true {
        return Err((false, "setup-not-ready"));
    }
    result["already_complete"] = json!(already_complete);
    result["scopes_saved"] = json!(scopes_saved);
    Ok(result)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        config::tests_support::temp_store,
        daemon::{ipc::handle_request, settings::SourceDeclaration},
        identity::DeviceIdentity,
    };

    pub(crate) fn enrolled(chosen: bool) -> (tempfile::TempDir, DaemonShared) {
        let (dir, store) = temp_store();
        let key = DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg: ContributorConfig = serde_json::from_value(json!({
            "schema_version":crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,"issuer_url":"https://issuer.example.ai","ingest_url":"https://ingest.example.ai",
            "audience":"trace-commons-upload","tenant_id":"tenant-test","instance_id":"instance-test","user_subject":"subject-test",
            "device_key_id":key.device_key_id,"consent_scopes":["debugging_evaluation"],"consent_scopes_chosen":chosen
        })).unwrap();
        store.save_config(&cfg).unwrap();
        let shared = DaemonShared::load(store).unwrap();
        let mut settings = shared.settings.lock().unwrap();
        settings.claude_source = Some(SourceDeclaration::Off);
        settings.codex_source = Some(SourceDeclaration::Off);
        drop(settings);
        (dir, shared)
    }
    fn req(method: &str, params: Value) -> Request {
        Request {
            id: 1,
            method: method.into(),
            params,
        }
    }
    fn finish(shared: &DaemonShared, scopes: Option<Value>) -> Response {
        let mut params = json!({"expected_scope":view(shared)["scope"]});
        if let Some(scopes) = scopes {
            params["scopes"] = scopes;
        }
        handle_request(shared, &req("complete_setup", params))
    }

    #[test]
    fn completion_survives_restart_and_changes_no_sharing_rules() {
        let (_dir, shared) = enrolled(true);
        shared.settings.lock().unwrap().save(&shared.store).unwrap();
        let settings = serde_json::to_value(&*shared.settings.lock().unwrap()).unwrap();
        let policy = std::fs::read(
            shared
                .store
                .daemon_path(crate::config::DAEMON_PROJECTS_FILE),
        )
        .ok();
        let mut events = shared.events.subscribe();
        let result = finish(&shared, None);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(shared.status_value()["setup_complete"], true);
        assert_eq!(
            serde_json::to_value(&*shared.settings.lock().unwrap()).unwrap(),
            settings
        );
        assert_eq!(
            std::fs::read(
                shared
                    .store
                    .daemon_path(crate::config::DAEMON_PROJECTS_FILE)
            )
            .ok(),
            policy
        );
        assert_eq!(events.try_recv().unwrap().event, EVENT_STATUS_CHANGED);
        let bytes = std::fs::read(shared.store.daemon_path(DAEMON_SETUP_COMPLETION_FILE)).unwrap();
        let again = finish(&shared, None).result.unwrap();
        assert_eq!(again["already_complete"], true);
        assert_eq!(
            std::fs::read(shared.store.daemon_path(DAEMON_SETUP_COMPLETION_FILE)).unwrap(),
            bytes
        );
        let store = shared.store.clone();
        drop(shared);
        let restarted = DaemonShared::load(store).unwrap();
        assert_eq!(restarted.status_value()["setup_complete"], true);
    }

    #[test]
    fn agent_can_choose_data_uses_and_complete_without_ui() {
        let (_dir, shared) = enrolled(false);
        assert_eq!(view(&shared)["blocked_reason"], "setup-scopes-required");
        assert!(finish(&shared, None).error.is_some());
        let result = finish(&shared, Some(json!(["model_training"])));
        assert!(result.error.is_none(), "{:?}", result.error);
        let cfg = shared.store.load_config().unwrap().unwrap();
        assert_eq!(cfg.consent_scopes_chosen, Some(true));
        assert!(cfg.consent_scopes.contains(&"model_training".into()));
        assert_eq!(shared.status_value()["setup_complete"], true);
    }

    #[test]
    fn invalid_scopes_and_stale_identity_write_nothing() {
        let (_dir, shared) = enrolled(false);
        let before = std::fs::read(shared.store.dir().join("contributor.json")).unwrap();
        assert!(
            finish(&shared, Some(json!(["SECRET-invalid"])))
                .error
                .is_some()
        );
        let bad = handle_request(
            &shared,
            &req(
                "complete_setup",
                json!({"expected_scope":"stale","scopes":["model_training"]}),
            ),
        );
        assert!(bad.error.is_some());
        assert_eq!(
            std::fs::read(shared.store.dir().join("contributor.json")).unwrap(),
            before
        );
        assert!(
            !shared
                .store
                .daemon_path(DAEMON_SETUP_COMPLETION_FILE)
                .exists()
        );
    }

    #[test]
    fn marker_is_bound_to_identity_and_consent_readiness() {
        let (_dir, shared) = enrolled(true);
        assert!(finish(&shared, None).error.is_none());
        let original = shared.store.load_config().unwrap().unwrap();
        for field in ["tenant_id", "device_key_id", "ingest_url"] {
            let mut v = serde_json::to_value(&original).unwrap();
            v[field] = json!(if field == "ingest_url" {
                "https://other.example.ai"
            } else {
                "different"
            });
            shared
                .store
                .save_config(&serde_json::from_value(v).unwrap())
                .unwrap();
            assert_eq!(shared.status_value()["setup_complete"], false, "{field}");
        }
        let mut cfg = original;
        cfg.consent_scopes_chosen = Some(false);
        shared.store.save_config(&cfg).unwrap();
        assert_eq!(shared.status_value()["setup_complete"], false);
    }

    #[test]
    fn missing_roots_key_or_corrupt_record_do_not_complete() {
        let (_dir, shared) = enrolled(true);
        shared.settings.lock().unwrap().claude_source = None;
        assert_eq!(view(&shared)["blocked_reason"], "setup-roots-required");
        assert!(finish(&shared, None).error.is_some());
        shared.settings.lock().unwrap().claude_source = Some(SourceDeclaration::Off);
        std::fs::write(
            shared.store.daemon_path(DAEMON_SETUP_COMPLETION_FILE),
            b"{}",
        )
        .unwrap();
        assert_eq!(view(&shared)["blocked_reason"], "setup-state-unreadable");
        assert!(
            finish(&shared, Some(json!(["model_training"])))
                .error
                .is_some()
        );
        assert!(
            !shared
                .store
                .load_config()
                .unwrap()
                .unwrap()
                .consent_scopes
                .contains(&"model_training".into())
        );
        let (other, empty) = temp_store();
        let empty = DaemonShared::load(empty).unwrap();
        assert_eq!(view(&empty)["blocked_reason"], "setup-sign-in-required");
        assert!(!other.path().join(DAEMON_SETUP_COMPLETION_FILE).exists());
    }

    #[test]
    fn enrollment_cleanup_and_wipe_remove_completion() {
        let (_dir, shared) = enrolled(true);
        assert!(finish(&shared, None).error.is_none());
        assert!(crate::config::ENROLLMENT_FILES.contains(&DAEMON_SETUP_COMPLETION_FILE));
        shared.store.wipe().unwrap();
        assert!(
            !shared
                .store
                .daemon_path(DAEMON_SETUP_COMPLETION_FILE)
                .exists()
        );
    }
    #[test]
    fn setup_inspection_is_read_only_and_scoped_to_the_state_directory() {
        let (_dir, shared) = enrolled(true);
        let before = std::fs::read_dir(shared.store.dir())
            .unwrap()
            .map(|p| p.unwrap().file_name())
            .collect::<std::collections::BTreeSet<_>>();
        let response = handle_request(&shared, &req("get_setup_status", json!({})));
        assert!(response.error.is_none());
        let value = response.result.unwrap();
        assert_eq!(value["complete"], false);
        assert_eq!(value["can_complete"], true);
        assert_eq!(
            value["consent_options"]["scopes"].as_array().unwrap().len(),
            crate::consent::VALID_SCOPES.len()
        );
        assert_eq!(
            std::fs::read_dir(shared.store.dir())
                .unwrap()
                .map(|p| p.unwrap().file_name())
                .collect::<std::collections::BTreeSet<_>>(),
            before
        );
        assert!(finish(&shared, None).error.is_none());
        let (_other, other) = enrolled(true);
        assert_eq!(view(&other)["complete"], false);
        let mut cfg = shared.store.load_config().unwrap().unwrap();
        cfg.consent_scopes = vec!["debugging_evaluation".into(), "benchmark_only".into()];
        shared.store.save_config(&cfg).unwrap();
        assert_eq!(
            view(&shared)["complete"],
            true,
            "a normal preference/consent edit retains completion"
        );
    }

    #[test]
    fn checked_config_callback_never_writes_a_replacement_enrollment() {
        let (_dir, shared) = enrolled(true);
        let old = shared.store.load_config().unwrap().unwrap();
        let mut replacement = old.clone();
        replacement.tenant_id = "replacement-tenant".into();
        shared.store.save_config(&replacement).unwrap();
        let called = std::sync::atomic::AtomicBool::new(false);
        let result =
            super::super::commons_credentials::with_current_config(&shared.store, &old, || {
                called.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            });
        assert!(result.is_err());
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(
            shared.store.load_config().unwrap().unwrap().tenant_id,
            "replacement-tenant"
        );
        assert!(
            super::super::enroll::apply_setup_scopes(&shared, &old, &["model_training".into()])
                .is_err()
        );
        assert_eq!(
            shared.store.load_config().unwrap().unwrap().tenant_id,
            "replacement-tenant"
        );
    }

    #[test]
    fn blank_enrollment_identity_cannot_complete_or_save_scopes() {
        for field in ["tenant_id", "device_key_id"] {
            for blank in ["", " \t"] {
                let (_dir, shared) = enrolled(false);
                let mut cfg =
                    serde_json::to_value(shared.store.load_config().unwrap().unwrap()).unwrap();
                cfg[field] = json!(blank);
                shared
                    .store
                    .save_config(&serde_json::from_value(cfg).unwrap())
                    .unwrap();
                let before = std::fs::read(shared.store.dir().join("contributor.json")).unwrap();
                assert_eq!(view(&shared)["blocked_reason"], "setup-sign-in-required");
                assert_eq!(view(&shared)["can_complete"], false);
                assert!(
                    finish(&shared, Some(json!(["model_training"])))
                        .error
                        .is_some()
                );
                assert_eq!(shared.status_value()["setup_complete"], false);
                assert_eq!(
                    std::fs::read(shared.store.dir().join("contributor.json")).unwrap(),
                    before
                );
                assert!(
                    !shared
                        .store
                        .daemon_path(DAEMON_SETUP_COMPLETION_FILE)
                        .exists()
                );
            }
        }
    }

    #[test]
    fn missing_device_key_and_malformed_requests_do_not_skip_setup() {
        let (_dir, shared) = enrolled(true);
        for params in [
            json!({}),
            json!({"expected_scope":view(&shared)["scope"],"unknown":true}),
            json!({"expected_scope":view(&shared)["scope"],"scopes":[]}),
        ] {
            assert!(
                handle_request(&shared, &req("complete_setup", params))
                    .error
                    .is_some()
            );
        }
        super::super::commons_credentials::clear(
            &shared.store,
            &[super::super::commons_credentials::Kind::Device],
        )
        .unwrap();
        assert_eq!(view(&shared)["blocked_reason"], "setup-sign-in-required");
        assert!(finish(&shared, None).error.is_some());
        assert_eq!(shared.status_value()["setup_complete"], false);
    }
}

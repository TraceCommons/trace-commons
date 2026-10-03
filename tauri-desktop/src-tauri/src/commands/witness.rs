use tauri::State;
use trace_commons_contributor::config::{ConfigStore, WitnessOrigin};
use trace_commons_contributor::disclosure::RouteDisclosure;

use crate::ipc::{call_daemon, shared_state};
use crate::state::{AppState, state_directory};

fn witness_status_value(store: &ConfigStore) -> Result<serde_json::Value, String> {
    let config = store
        .load_config()
        .map_err(|_| "witness-config-unreadable".to_owned())?;
    let Some(config) = config else {
        return Ok(serde_json::json!({
            "state": "not_enrolled",
            "state_code": -1,
            "state_line": trace_commons_contributor::witness_copy::witness_state_line(
                trace_commons_contributor::witness::status::WitnessTrustState::NotEnrolled,
            ),
            "refusal": null,
            "url": null,
            "signing_address": null,
            "pinned_measurement_count": 0,
            "pinned_measurements": [],
        }));
    };
    let status = trace_commons_contributor::witness::status::witness_status(&config);
    Ok(serde_json::json!({
        "state": status.state,
        "state_code": status.state.abi_code(),
        "state_line": trace_commons_contributor::witness_copy::witness_state_line(status.state),
        "refusal": status.state.refusal_label(),
        "url": status.url,
        "signing_address": status.signing_address,
        "pinned_measurement_count": status.pinned_measurement_count,
        "pinned_measurement_line": status.pinned_measurement_line(),
        "pinned_measurements": status.pinned_measurements,
    }))
}

#[tauri::command]
pub(crate) async fn witness_status(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let store = ConfigStore::open(state_directory(&state)?)
        .map_err(|_| "witness-config-unreadable".to_owned())?;
    witness_status_value(&store)
}

#[tauri::command]
pub(crate) async fn configure_witness(
    state: State<'_, AppState>,
    url: String,
    signing_address: String,
    measurements: Vec<String>,
) -> Result<serde_json::Value, String> {
    let store = ConfigStore::open(state_directory(&state)?)
        .map_err(|_| "witness-config-unreadable".to_owned())?;
    configure_witness_in(&store, url, signing_address, measurements)
}

/// `configure_witness` against a store, recording the witness as entered in
/// Settings (K11: the disclosure screen says where a witness came from).
///
/// The validation itself -- the URL's shape, the signing address, the pin
/// list -- is `WitnessSettings::configure`, the one implementation the C
/// ABI's `tc_witness_configure` also calls; this is a thin pass of the
/// store's current witness (for `admission_evidence`) and this call's inputs.
fn configure_witness_in(
    store: &ConfigStore,
    url: String,
    signing_address: String,
    measurements: Vec<String>,
) -> Result<serde_json::Value, String> {
    let mut config = store
        .load_config()
        .map_err(|_| "witness-config-unreadable".to_owned())?
        .ok_or_else(|| "witness-not-enrolled".to_owned())?;
    let admission_evidence = config
        .witness
        .as_ref()
        .is_some_and(|w| w.admission_evidence);
    let settings = trace_commons_contributor::config::WitnessSettings::configure(
        admission_evidence,
        &url,
        &signing_address,
        measurements,
    )
    .map_err(str::to_owned)?;
    config.set_witness(settings, WitnessOrigin::Settings);
    store
        .save_config(&config)
        .map_err(|_| "witness-config-write-failed".to_owned())?;
    witness_status_value(store)
}

#[tauri::command]
pub(crate) async fn clear_witness(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let store = ConfigStore::open(state_directory(&state)?)
        .map_err(|_| "witness-config-unreadable".to_owned())?;
    clear_witness_in(&store)
}

fn clear_witness_in(store: &ConfigStore) -> Result<serde_json::Value, String> {
    let mut config = store
        .load_config()
        .map_err(|_| "witness-config-unreadable".to_owned())?
        .ok_or_else(|| "witness-not-enrolled".to_owned())?;
    config.clear_witness();
    store
        .save_config(&config)
        .map_err(|_| "witness-config-write-failed".to_owned())?;
    witness_status_value(store)
}

/// K11: what leaves this machine, to whom, and what this client checked --
/// the daemon's `route_disclosure` facts, with the contributor core's words
/// for them. The shell renders both and chooses neither.
///
/// Asked of the daemon rather than read from the config here, because the
/// daemon is the process that sends: the environment's privacy filter is its
/// environment, and the attested-bodies switch is its setting.
#[tauri::command]
pub(crate) async fn route_disclosure(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let facts = call_daemon(
        shared_state(&state)?,
        "route_disclosure",
        serde_json::json!({}),
    )
    .await?;
    route_disclosure_value(facts)
}

/// Refuses a shape this build cannot read, rather than rendering it as some
/// other route: an unknown origin or route is a newer daemon's answer, and
/// the nearest one this build knows would claim something it does not.
fn route_disclosure_value(facts: serde_json::Value) -> Result<serde_json::Value, String> {
    let parsed: RouteDisclosure =
        serde_json::from_value(facts).map_err(|_| "route-disclosure-unreadable".to_owned())?;
    let copy = trace_commons_contributor::consent_copy::route_disclosure_copy(&parsed);
    Ok(serde_json::json!({ "facts": parsed, "copy": copy }))
}

/// What a disclosure surface says when [`route_disclosure`] fails: the
/// section's title and the unreadable lines, from the contributor core, so
/// the shell writes none of them.
#[tauri::command]
pub(crate) fn route_disclosure_unreadable_copy() -> serde_json::Value {
    serde_json::to_value(trace_commons_contributor::consent_copy::disclosure_unreadable_copy())
        .unwrap_or(serde_json::Value::Null)
}

/// The held certificate's claims for one pending entry, as the daemon's
/// `certificate_detail` returns them, with the core's labels. The daemon
/// refuses entries without a held certificate; that refusal is passed on.
#[tauri::command]
pub(crate) async fn certificate_detail(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<serde_json::Value, String> {
    let entry_id = entry_id.trim();
    if entry_id.is_empty() {
        return Err("certificate-entry-required".to_owned());
    }
    let detail = call_daemon(
        shared_state(&state)?,
        "certificate_detail",
        serde_json::json!({ "entry_id": entry_id }),
    )
    .await?;
    Ok(certificate_detail_value(detail))
}

fn certificate_detail_value(detail: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "detail": detail,
        "copy": trace_commons_contributor::consent_copy::certificate_detail_copy(),
    })
}

#[cfg(test)]
mod tests {
    use trace_commons_contributor::config::{ConfigStore, WitnessOrigin, WitnessOriginView};

    use super::{
        certificate_detail_value, clear_witness_in, configure_witness_in,
        route_disclosure_unreadable_copy, route_disclosure_value,
    };

    fn temp_store() -> (std::path::PathBuf, ConfigStore) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("tc-tauri-witness-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ConfigStore::open(dir.clone()).unwrap();
        let cfg: trace_commons_contributor::config::ContributorConfig =
            serde_json::from_value(serde_json::json!({
                "schema_version": trace_commons_contributor::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
                "issuer_url": "https://issuer.invalid",
                "ingest_url": "https://ingest.invalid",
                "audience": "aud",
                "tenant_id": "tenant-1",
                "instance_id": "instance-1",
                "user_subject": "alice",
                "device_key_id": "sha256:aa",
                "consent_scopes": ["debugging_evaluation"],
                "pii_filter": null,
                "allowed_hosts": null,
            }))
            .unwrap();
        store.save_config(&cfg).unwrap();
        (dir, store)
    }

    fn pin() -> String {
        format!("mrtd={}", "ab".repeat(48))
    }

    /// K11: a witness typed into Settings is recorded as such, so the
    /// disclosure screen can say where it came from.
    #[test]
    fn a_witness_configured_here_is_recorded_as_entered_in_settings() {
        let (dir, store) = temp_store();
        configure_witness_in(
            &store,
            "https://witness.invalid".into(),
            "0xab".into(),
            vec![pin()],
        )
        .expect("configured");
        let cfg = store.load_config().unwrap().unwrap();
        assert_eq!(
            cfg.witness_origin_view(),
            Some(WitnessOriginView::Recorded(WitnessOrigin::Settings))
        );
        clear_witness_in(&store).expect("cleared");
        let cfg = store.load_config().unwrap().unwrap();
        assert!(cfg.witness.is_none() && cfg.witness_origin.is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    /// The panel's words are the contributor core's, chosen from the
    /// daemon's facts; the shell passes the facts back canonicalised.
    #[test]
    fn the_route_disclosure_is_the_daemons_facts_and_the_cores_words() {
        let facts = serde_json::json!({
            "route": "local",
            "witness": null,
            "local_filter": "near_ai",
            "receipts": {"endpoint_configured": false, "check_attestation": false},
            "attested_bodies": false,
        });
        let value = route_disclosure_value(facts.clone()).expect("readable");
        assert_eq!(value["facts"], facts);
        let parsed: trace_commons_contributor::disclosure::RouteDisclosure =
            serde_json::from_value(facts).unwrap();
        let expected = serde_json::to_value(
            trace_commons_contributor::consent_copy::route_disclosure_copy(&parsed),
        )
        .unwrap();
        assert_eq!(value["copy"], expected);
    }

    /// A shape this build cannot read is refused, never rendered as some
    /// other route.
    #[test]
    fn an_unreadable_disclosure_is_an_error_not_a_guess() {
        for facts in [
            serde_json::json!({"route": "somewhere_new"}),
            serde_json::json!("witness"),
            serde_json::json!({
                "route": "witness",
                "witness": {"state": "pinned", "url": "u", "signing_address": "s",
                            "pinned_measurements": [], "origin": "an_operator"},
                "local_filter": null,
                "receipts": {"endpoint_configured": false, "check_attestation": false},
                "attested_bodies": false,
            }),
        ] {
            assert_eq!(
                route_disclosure_value(facts),
                Err("route-disclosure-unreadable".to_owned())
            );
        }
    }

    #[test]
    fn the_unreadable_copy_is_the_cores() {
        assert_eq!(
            route_disclosure_unreadable_copy(),
            serde_json::to_value(
                trace_commons_contributor::consent_copy::disclosure_unreadable_copy()
            )
            .unwrap()
        );
    }

    #[test]
    fn certificate_detail_carries_the_cores_labels() {
        let detail =
            serde_json::json!({"state": "held", "witness_measurement": "m", "signer": "0xab"});
        let value = certificate_detail_value(detail.clone());
        assert_eq!(value["detail"], detail);
        assert_eq!(
            value["copy"],
            serde_json::to_value(
                trace_commons_contributor::consent_copy::certificate_detail_copy()
            )
            .unwrap()
        );
    }
}

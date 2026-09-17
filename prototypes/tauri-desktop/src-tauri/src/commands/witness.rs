use tauri::State;
use trace_commons_contributor::config::ConfigStore;

use crate::state::{AppState, state_directory};

fn witness_status_value(store: &ConfigStore) -> Result<serde_json::Value, String> {
    let config = store
        .load_config()
        .map_err(|_| "witness-config-unreadable".to_owned())?;
    let Some(config) = config else {
        return Ok(serde_json::json!({
            "state": "not_enrolled",
            "state_code": -1,
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
    let mut config = store
        .load_config()
        .map_err(|_| "witness-config-unreadable".to_owned())?
        .ok_or_else(|| "witness-not-enrolled".to_owned())?;
    let url = url.trim();
    let url_has_scheme = url.starts_with("https://") || url.starts_with("http://");
    let url_host = url.split(['/', '?', '#']).nth(2).unwrap_or("");
    if !url_has_scheme || url_host.is_empty() || url_host.chars().any(char::is_whitespace) {
        return Err("witness-url-invalid".to_owned());
    }
    let signing_address = signing_address.trim();
    if signing_address.is_empty() {
        return Err("witness-signing-address-invalid".to_owned());
    }
    let measurements: Vec<String> = measurements
        .into_iter()
        .map(|entry| entry.trim().to_owned())
        .filter(|entry| !entry.is_empty())
        .collect();
    if measurements.is_empty() {
        return Err("witness-pin-required".to_owned());
    }
    let settings = trace_commons_contributor::config::WitnessSettings {
        admission_evidence: config
            .witness
            .as_ref()
            .is_some_and(|w| w.admission_evidence),
        url: url.to_owned(),
        signing_address: signing_address.to_owned(),
        expected_measurements: measurements,
    };
    match settings.trust() {
        Ok(trust) if trust.is_pinned() => {}
        _ => return Err("witness-pin-malformed".to_owned()),
    }
    config.witness = Some(settings);
    store
        .save_config(&config)
        .map_err(|_| "witness-config-write-failed".to_owned())?;
    witness_status_value(&store)
}

#[tauri::command]
pub(crate) async fn clear_witness(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let store = ConfigStore::open(state_directory(&state)?)
        .map_err(|_| "witness-config-unreadable".to_owned())?;
    let mut config = store
        .load_config()
        .map_err(|_| "witness-config-unreadable".to_owned())?
        .ok_or_else(|| "witness-not-enrolled".to_owned())?;
    config.witness = None;
    store
        .save_config(&config)
        .map_err(|_| "witness-config-write-failed".to_owned())?;
    witness_status_value(&store)
}

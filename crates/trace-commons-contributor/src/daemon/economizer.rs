//! The economizer on the socket, for macOS Insights' Economizer tab:
//!
//! - `economizer_status`: whether it is on, and what it learned per folder
//!   and tool ([`crate::economizer::summary`]);
//! - `economizer_set_enabled` `{on}`: the same switch as
//!   `near-ai economizer on|off`;
//! - `economizer_restore` `{tool, key}`: give back every tool left out in
//!   one folder, as `near-ai economizer restore` does.
//!
//! The economizer's store cannot list its own folders (it is keyed by a
//! digest of each path), so the folders come from the managed session rows
//! and the remembered folder settings. Synchronous, no network: each method
//! reads or writes only `<config dir>/economizer`, so all three are on the
//! developer dry-run allowlist.
//!
//! # What crosses the socket
//!
//! A folder's last path component (what managed session rows already
//! show), the store's digest key, built-in tool names, counts, and the
//! program names and launcher prefixes the economizer tells the agent.
//! Never a path.

use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use crate::economizer::Harness;
use crate::economizer::store::Store;
use crate::economizer::summary;
use crate::managed::ToolId;

/// The economizer's state could not be written.
pub const ERR_ECONOMIZER_WRITE: &str = "economizer-write-failed";

fn store(shared: &DaemonShared) -> Store {
    Store::new(shared.store.dir())
}

/// Every folder a managed session ran in or has remembered settings for,
/// with its tool. A registry or settings read that fails adds nothing.
fn known_folders(shared: &DaemonShared) -> Vec<(Harness, PathBuf)> {
    let mut folders: Vec<(Harness, PathBuf)> = super::managed::session_folders(shared)
        .into_iter()
        .map(|(tool, cwd)| (Harness::from(tool), cwd))
        .collect();
    if let Ok(settings) = shared.settings.lock() {
        folders.extend(
            settings
                .managed_folder_settings
                .iter()
                .map(|entry| (Harness::from(entry.tool), entry.cwd.clone())),
        );
    }
    folders
}

pub fn handle_status(shared: &DaemonShared, req: &Request) -> Response {
    let summary = summary::summarize(&store(shared), known_folders(shared));
    match serde_json::to_value(summary) {
        Ok(value) => Response::ok(req.id, value),
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "economizer-unavailable"),
    }
}

pub fn handle_set_enabled(shared: &DaemonShared, req: &Request) -> Response {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Params {
        on: bool,
    }
    let Ok(params) = serde_json::from_value::<Params>(req.params.clone()) else {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_BAD_PARAMS);
    };
    let store = store(shared);
    if store.set_enabled(params.on).is_err() {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_ECONOMIZER_WRITE);
    }
    Response::ok(req.id, json!({ "enabled": store.enabled() }))
}

pub fn handle_restore(shared: &DaemonShared, req: &Request) -> Response {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Params {
        tool: ToolId,
        key: String,
    }
    let Ok(params) = serde_json::from_value::<Params>(req.params.clone()) else {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_BAD_PARAMS);
    };
    match summary::restore(&store(shared), Harness::from(params.tool), &params.key) {
        Ok(given) => Response::ok(req.id, json!({ "restored": given })),
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, ERR_ECONOMIZER_WRITE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economizer::store::{RepoState, repo_key};

    fn call(shared: &DaemonShared, method: &str, params: serde_json::Value) -> Response {
        super::super::ipc::handle_request(
            shared,
            &Request {
                id: 1,
                method: method.into(),
                params,
            },
        )
    }

    #[test]
    fn the_methods_are_advertised_and_local() {
        for method in [
            "economizer_status",
            "economizer_set_enabled",
            "economizer_restore",
        ] {
            assert!(super::super::ipc::METHODS.contains(&method), "{method}");
            assert!(
                super::super::ipc::DEV_DRY_RUN_LOCAL_METHODS.contains(&method),
                "{method}"
            );
        }
    }

    #[test]
    fn status_reads_remembered_folders_and_the_switch_and_restore_write_the_store() {
        let (_dir, config) = crate::config::tests_support::temp_store();
        let folder = tempfile::tempdir().unwrap();
        let project = folder.path().join("my-app");
        std::fs::create_dir_all(&project).unwrap();
        let project = std::fs::canonicalize(&project).unwrap();
        let economizer = Store::new(config.dir());
        let key = repo_key(&project, Harness::Claude);
        let mut state = RepoState::default();
        for _ in 0..5 {
            state.tools.fold_session(
                Harness::Claude,
                3,
                &["Bash".to_owned()].into_iter().collect(),
            );
            state.sessions += 1;
        }
        economizer.save_repo(&key, &state).unwrap();

        let shared = DaemonShared::load(config).unwrap();
        shared
            .settings
            .lock()
            .unwrap()
            .managed_folder_settings
            .push(crate::managed::folder::FolderSettings {
                tool: ToolId::Claude,
                cwd: project.clone(),
                model: None,
                background_model: None,
                collect_traces: false,
                cli_options: String::new(),
            });

        let status = call(&shared, "economizer_status", json!({}));
        let value = serde_json::to_value(&status).unwrap();
        let result = &value["result"];
        assert_eq!(result["enabled"], false, "{value}");
        assert_eq!(result["folders"][0]["folder"], "my-app");
        assert_eq!(result["folders"][0]["harness"], "claude");
        assert_eq!(result["folders"][0]["key"], key.as_str());
        assert_eq!(result["folders"][0]["sessions"], 5);
        assert!(
            result["folders"][0]["left_out"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool == "Agent")
        );
        assert!(
            !value
                .to_string()
                .contains(&project.to_string_lossy().into_owned())
        );

        let on = call(&shared, "economizer_set_enabled", json!({"on": true}));
        assert_eq!(
            serde_json::to_value(&on).unwrap()["result"]["enabled"],
            true
        );
        assert!(economizer.enabled());
        let bad = call(&shared, "economizer_set_enabled", json!({"on": "yes"}));
        assert_eq!(
            serde_json::to_value(&bad).unwrap()["error"]["code"],
            ERR_BAD_PARAMS
        );

        let restored = call(
            &shared,
            "economizer_restore",
            json!({"tool": "claude", "key": key}),
        );
        let restored = serde_json::to_value(&restored).unwrap();
        assert!(
            restored["result"]["restored"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool == "Agent"),
            "{restored}"
        );
        let after = serde_json::to_value(call(&shared, "economizer_status", json!({}))).unwrap();
        assert_eq!(after["result"]["enabled"], true);
        assert!(
            after["result"]["folders"][0]["left_out"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

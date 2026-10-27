//! Agent-facing configuration always addresses the running app daemon.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use serde::Deserialize;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

use crate::config::ConfigStore;
use crate::daemon::{client, ipc};

const MAX_PATCH_BYTES: u64 = 1024 * 1024;

#[derive(Args)]
#[command(
    after_help = "Trace estimates: near-ai --json daemon pending\nFolder rules: near-ai --json daemon projects\nChange a folder: near-ai daemon project PATH --mode ignore|notify|auto\nSetup readiness: near-ai --json config setup\nFinish setup: near-ai --json config complete --scopes debugging_evaluation,model_training\nAutomatic mode includes existing traces unless --from-now is supplied."
)]
pub(super) struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Inspect setup readiness and available data-use choices from the running app
    Setup,
    /// Finish configured setup so the app skips first-run onboarding
    Complete {
        /// Explicit data-use choice made by you or your agent; omitted preserves the saved choice
        #[arg(long)]
        scopes: Option<String>,
    },
    /// Discover writable settings, types, ranges, and effects from the running app
    Schema,
    /// Inspect the running app's redacted settings (includes read-only fields)
    Show,
    /// Validate a partial JSON settings object and show changes without applying
    Preview {
        /// JSON object file, at most 1 MiB; duplicate keys are refused
        #[arg(long)]
        file: PathBuf,
    },
    /// Apply a partial JSON settings object through the running app's daemon
    Apply {
        /// JSON object file, at most 1 MiB; use preview first to inspect changes
        #[arg(long)]
        file: PathBuf,
    },
}

pub(super) fn run(store: &ConfigStore, args: &ConfigArgs) -> Result<()> {
    let value = execute(store, &args.command)?;
    // Configuration commands describe structured data, so their human rendering is
    // readable JSON too. The global error renderer owns --json failures.
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn execute(store: &ConfigStore, command: &ConfigCommand) -> Result<Value> {
    if let ConfigCommand::Complete { scopes } = command {
        let scopes = scopes
            .as_ref()
            .map(|csv| {
                let names = csv
                    .split(',')
                    .map(|s| s.trim().to_owned())
                    .collect::<Vec<_>>();
                crate::consent::validate_scopes(&names)
                    .map_err(|_| anyhow::anyhow!("scopes-invalid"))
            })
            .transpose()?;
        let setup = call(store, "get_setup_status", serde_json::json!({}))?;
        let scope = setup["scope"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("setup-sign-in-required"))?;
        let mut params = serde_json::json!({"expected_scope":scope});
        if let Some(scopes) = scopes {
            params["scopes"] = serde_json::json!(scopes);
        }
        return call(store, "complete_setup", params);
    }
    let (method, params) = match command {
        ConfigCommand::Setup => ("get_setup_status", serde_json::json!({})),
        ConfigCommand::Complete { .. } => unreachable!("handled above"),
        ConfigCommand::Schema => ("get_config_schema", serde_json::json!({})),
        ConfigCommand::Show => ("get_settings", serde_json::json!({})),
        ConfigCommand::Preview { file } => ("preview_settings", read_patch(file)?),
        ConfigCommand::Apply { file } => ("set_settings", read_patch(file)?),
    };
    call(store, method, params)
}

fn call(store: &ConfigStore, method: &str, params: Value) -> Result<Value> {
    // In particular, do not call commands::daemon_call: its offline fallback
    // creates private state and can write it while the app is unreachable.
    let response = client::try_call(store, method, &params)
        .map_err(|_| anyhow::anyhow!("config-daemon-request-failed"))?
        .ok_or_else(|| {
            anyhow::anyhow!("daemon-unreachable: start the app and check --config-dir")
        })?;
    response_value(response)
}

fn response_value(response: ipc::Response) -> Result<Value> {
    if let Some(error) = response.error {
        if error.code == ipc::ERR_UNKNOWN_METHOD {
            bail!("config-daemon-unsupported: update the app and retry");
        }
        // IPC errors are fixed labels, never submitted values or provider bodies.
        bail!("{}: {}", error.code, error.message);
    }
    response
        .result
        .ok_or_else(|| anyhow::anyhow!("config-daemon-response-invalid"))
}

fn read_patch(path: &Path) -> Result<Value> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_PATCH_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| anyhow::anyhow!("config-patch-unreadable"))?;
    decode_patch(&bytes)
}

fn decode_patch(bytes: &[u8]) -> Result<Value> {
    if bytes.len() as u64 > MAX_PATCH_BYTES {
        bail!("config-patch-too-large: maximum is 1048576 bytes");
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = UniqueValue::deserialize(&mut decoder)
        .map_err(|_| anyhow::anyhow!("config-patch-invalid-json-or-duplicate-key"))?
        .0;
    decoder
        .end()
        .map_err(|_| anyhow::anyhow!("config-patch-invalid-json-or-duplicate-key"))?;
    let Some(object) = value.as_object() else {
        bail!("settings-must-be-object");
    };
    if object.is_empty() {
        bail!("no-known-setting-supplied");
    }
    Ok(value)
}

/// serde_json's default object reader keeps the last duplicate key. A patch
/// should never silently choose between two instructions, including nested ones.
struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON without duplicate object keys")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
                let mut names = BTreeSet::new();
                let mut result = Map::new();
                while let Some(key) = input.next_key::<String>()? {
                    if !names.insert(key.clone()) {
                        return Err(de::Error::custom("duplicate key"));
                    }
                    result.insert(key, input.next_value::<UniqueValue>()?.0);
                }
                Ok(UniqueValue(Value::Object(result)))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
                let mut result = Vec::new();
                while let Some(value) = input.next_element::<UniqueValue>()? {
                    result.push(value.0);
                }
                Ok(UniqueValue(Value::Array(result)))
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Number::from_f64(value)
                    .map(|number| UniqueValue(Value::Number(number)))
                    .ok_or_else(|| de::Error::custom("invalid number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value.into())))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_keeps_structured_values_and_integer_precision() {
        let patch = br#"{"notify":{"enabled":false},"count":18446744073709551615,"optional":null,"list":[true,-3,1.5]}"#;
        assert_eq!(
            decode_patch(patch).unwrap(),
            serde_json::from_slice::<Value>(patch).unwrap()
        );
    }

    #[test]
    fn ambiguous_or_invalid_patches_are_refused_without_echoing_values() {
        for patch in [
            r#"{"private":"SECRET","private":"other"}"#,
            r#"{"notify":{"private":"SECRET","private":"other"}}"#,
            r#"{"items":[{"private":"SECRET","private":"other"}]}"#,
            r#"{"private":"SECRET","\u0070rivate":"other"}"#,
            r#"{"private":"SECRET",}"#,
            r#"{"private":"SECRET"} {}"#,
            r#"["SECRET"]"#,
            "{}",
        ] {
            let error = decode_patch(patch.as_bytes()).unwrap_err().to_string();
            assert!(!error.contains("SECRET"), "{error}");
        }
        assert!(
            decode_patch(&vec![b' '; MAX_PATCH_BYTES as usize + 1])
                .unwrap_err()
                .to_string()
                .starts_with("config-patch-too-large")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cli_can_choose_scopes_finish_setup_and_read_it_back() {
        let (dir, shared) = crate::daemon::setup_completion::tests::enrolled(false);
        shared.settings.lock().unwrap().save(&shared.store).unwrap();
        let store = shared.store.clone();
        drop(shared);
        let embedded = crate::daemon::start_embedded(store).await.unwrap();
        let inspect = ConfigStore::resolve_existing(Some(dir.path().into())).unwrap();
        assert_eq!(
            execute(&inspect, &ConfigCommand::Setup).unwrap()["blocked_reason"],
            "setup-scopes-required"
        );
        assert!(execute(&inspect, &ConfigCommand::Complete { scopes: None }).is_err());
        let result = execute(
            &inspect,
            &ConfigCommand::Complete {
                scopes: Some("model_training,benchmark_only".into()),
            },
        )
        .unwrap();
        assert_eq!(result["complete"], true);
        assert_eq!(result["scopes_saved"], true);
        assert_eq!(
            execute(&inspect, &ConfigCommand::Setup).unwrap()["complete"],
            true
        );
        assert!(
            execute(
                &inspect,
                &ConfigCommand::Complete {
                    scopes: Some("SECRET-unknown".into())
                }
            )
            .unwrap_err()
            .to_string()
                == "scopes-invalid"
        );
        embedded.close();
    }

    #[test]
    fn no_daemon_never_falls_back_to_saved_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().into()).unwrap();
        let file = dir.path().join("patch.json");
        std::fs::write(&file, br#"{"local_notifications":false}"#).unwrap();
        for command in [
            ConfigCommand::Schema,
            ConfigCommand::Show,
            ConfigCommand::Setup,
            ConfigCommand::Complete { scopes: None },
            ConfigCommand::Preview { file: file.clone() },
            ConfigCommand::Apply { file },
        ] {
            assert!(
                execute(&store, &command)
                    .unwrap_err()
                    .to_string()
                    .starts_with("daemon-unreachable")
            );
        }
        assert!(
            !dir.path()
                .join(crate::config::DAEMON_SETTINGS_FILE)
                .exists()
        );
    }

    #[test]
    fn unreachable_config_commands_leave_missing_directory_absent_and_exit_nonzero() {
        use clap::Parser;
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing-app-state");
        let cli = super::super::Cli::try_parse_from([
            "near-ai",
            "--json",
            "--config-dir",
            missing.to_str().unwrap(),
            "config",
            "show",
        ])
        .unwrap();
        assert_eq!(
            super::super::async_main(cli, true),
            std::process::ExitCode::FAILURE
        );
        assert!(!missing.exists());
    }

    #[cfg(unix)]
    #[test]
    fn inspecting_configuration_does_not_change_directory_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
        let store = ConfigStore::resolve_existing(Some(dir.path().into())).unwrap();
        assert_eq!(store.dir(), dir.path());
        assert_eq!(
            std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }

    #[test]
    fn config_subcommands_accept_global_options_and_require_patch_files() {
        use super::super::{Cli, Command};
        use clap::Parser;
        for action in ["schema", "show", "setup", "complete"] {
            let cli = Cli::try_parse_from([
                "near-ai",
                "config",
                action,
                "--json",
                "--config-dir",
                "/tmp/fixture",
            ])
            .unwrap();
            assert!(cli.json);
            assert!(matches!(cli.command, Command::Config(_)));
        }
        let cli = Cli::try_parse_from([
            "near-ai",
            "--json",
            "config",
            "complete",
            "--scopes",
            "model_training",
        ])
        .unwrap();
        assert!(matches!(cli.command, Command::Config(_)));
        assert!(
            response_value(ipc::Response::err(
                1,
                ipc::ERR_UNKNOWN_METHOD,
                "unknown_method"
            ))
            .unwrap_err()
            .to_string()
            .starts_with("config-daemon-unsupported")
        );
        for action in ["preview", "apply"] {
            assert!(Cli::try_parse_from(["near-ai", "config", action]).is_err());
            let cli = Cli::try_parse_from([
                "near-ai",
                "--json",
                "config",
                action,
                "--file",
                "patch.json",
            ])
            .unwrap();
            assert!(matches!(cli.command, Command::Config(_)));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn managed_folder_settings_config_preview_apply_and_show_share_the_launch_store() {
        let (dir, store) = crate::config::tests_support::temp_store();
        let embedded = crate::daemon::start_embedded(store).await.unwrap();
        let inspect = ConfigStore::resolve_existing(Some(dir.path().into())).unwrap();
        let file = dir.path().join("folder-patch.json");
        let patch = serde_json::json!({"managed_folder_settings":[{"tool":"codex","cwd":dir.path(),"model":"provider/model","collect_traces":true,"cli_options":"--sandbox workspace-write"}]});
        std::fs::write(&file, serde_json::to_vec(&patch).unwrap()).unwrap();
        let before = execute(&inspect, &ConfigCommand::Show).unwrap();
        assert_eq!(
            execute(&inspect, &ConfigCommand::Preview { file: file.clone() }).unwrap()["applied"],
            false
        );
        assert_eq!(execute(&inspect, &ConfigCommand::Show).unwrap(), before);
        execute(&inspect, &ConfigCommand::Apply { file }).unwrap();
        let shown = execute(&inspect, &ConfigCommand::Show).unwrap();
        assert_eq!(
            shown["managed_folder_settings"][0]["model"],
            "provider/model"
        );
        assert!(
            shown["managed_folder_settings"][0]
                .get("account_id")
                .is_none()
        );
        let response = client::try_call(
            &inspect,
            "managed_folder_settings_get",
            &serde_json::json!({"tool":"codex","cwd":dir.path()}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(response.result.unwrap()["collect_traces"], true);
        embedded.close();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn managed_cli_options_config_commands_update_the_ui_and_launch_store() {
        let (dir, store) = crate::config::tests_support::temp_store();
        let embedded = crate::daemon::start_embedded(store).await.unwrap();
        let inspect = ConfigStore::resolve_existing(Some(dir.path().into())).unwrap();
        let file = dir.path().join("options-patch.json");
        std::fs::write(&file, br#"{"managed_cli_options":{"codex":"-c model_reasoning_effort=high","claude":"--model sonnet"}}"#).unwrap();
        let before = execute(&inspect, &ConfigCommand::Show).unwrap();
        let preview = execute(&inspect, &ConfigCommand::Preview { file: file.clone() }).unwrap();
        assert_eq!(preview["applied"], false);
        assert_eq!(execute(&inspect, &ConfigCommand::Show).unwrap(), before);
        execute(&inspect, &ConfigCommand::Apply { file }).unwrap();
        let shown = execute(&inspect, &ConfigCommand::Show).unwrap();
        assert_eq!(
            shown["managed_cli_options"]["codex"],
            "-c model_reasoning_effort=high"
        );
        let response = client::try_call(&inspect, "managed_snapshot", &serde_json::json!({}))
            .unwrap()
            .unwrap();
        assert_eq!(
            response.result.unwrap()["cli_options"]["claude"],
            "--model sonnet"
        );
        assert_eq!(
            crate::managed::options::arguments(&inspect, crate::managed::ToolId::Codex).unwrap(),
            ["-c", "model_reasoning_effort=high"]
        );
        embedded.close();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn preview_and_apply_reach_the_live_daemon_without_losing_other_settings() {
        let (dir, store) = crate::config::tests_support::temp_store();
        let mut settings = crate::daemon::settings::DaemonSettings::load(&store).unwrap();
        settings.local_notifications = true;
        settings.quiescence_secs = 123;
        settings.save(&store).unwrap();
        let embedded = crate::daemon::start_embedded(store).await.unwrap();
        let settings_path = dir.path().join(crate::config::DAEMON_SETTINGS_FILE);
        let original = std::fs::read(&settings_path).unwrap();
        let patch = dir.path().join("patch.json");
        std::fs::write(&patch, br#"{"local_notifications":false}"#).unwrap();
        let path = dir.path().to_path_buf();
        let result = tokio::task::spawn_blocking(move || {
            let cli_store = ConfigStore::open(path).unwrap();
            let schema = execute(&cli_store, &ConfigCommand::Schema).unwrap();
            assert_eq!(schema["schema_version"], "trace_commons.config_schema.v1");
            assert!(
                schema["patch_schema"]["properties"]
                    .get("local_notifications")
                    .is_some()
            );
            let preview = execute(
                &cli_store,
                &ConfigCommand::Preview {
                    file: patch.clone(),
                },
            )
            .unwrap();
            assert_eq!(preview["applied"], false);
            assert_eq!(std::fs::read(&settings_path).unwrap(), original);
            assert_eq!(
                execute(&cli_store, &ConfigCommand::Show).unwrap()["local_notifications"],
                true
            );
            let applied = execute(&cli_store, &ConfigCommand::Apply { file: patch }).unwrap();
            assert_eq!(applied["local_notifications"], false);
            let shown = execute(&cli_store, &ConfigCommand::Show).unwrap();
            assert_eq!(shown["local_notifications"], false);
            assert_eq!(shown["quiescence_secs"], 123);
            let persisted = crate::daemon::settings::DaemonSettings::load(&cli_store).unwrap();
            assert!(!persisted.local_notifications);
            assert_eq!(persisted.quiescence_secs, 123);
        })
        .await;
        embedded.close();
        result.unwrap();
    }

    #[test]
    fn response_preserves_runtime_result_and_explains_old_daemons() {
        let result =
            serde_json::json!({"private_inference":true,"private_inference_state":"needs_sign_in"});
        assert_eq!(
            response_value(ipc::Response::ok(1, result.clone())).unwrap(),
            result
        );
        assert!(
            response_value(ipc::Response::err(
                1,
                ipc::ERR_UNKNOWN_METHOD,
                "unknown-method"
            ))
            .unwrap_err()
            .to_string()
            .starts_with("config-daemon-unsupported")
        );
    }
}

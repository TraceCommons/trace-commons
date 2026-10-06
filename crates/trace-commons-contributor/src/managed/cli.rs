//! The CLI and desktop launch helper use the same daemon-owned session service.

use super::{
    AccountView, ConnectionKind, Generation, ManagedError, Selection, ToolId,
    sessions::{LaunchRequest, PreparedLaunch},
};
use crate::{config::ConfigStore, daemon::client};
use clap::{Args, Subcommand};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Args)]
pub struct LaunchArgs {
    #[arg(value_enum)]
    pub tool: ToolId,
    #[arg(long)]
    pub account: Option<String>,
    #[arg(long, value_enum)]
    pub connection: Option<ConnectionKind>,
    #[arg(long)]
    pub cwd: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum AccountCommand {
    /// Show saved accounts; keys and native credentials are never printed.
    List,
    /// Save a named account, then sign in using the native tool.
    Add {
        #[arg(long, value_enum)]
        tool: ToolId,
        #[arg(long, value_enum, default_value = "subscription")]
        connection: ConnectionKind,
        #[arg(long)]
        label: String,
        /// Read an API key from stdin, never from a command-line argument.
        #[arg(long)]
        key_stdin: bool,
    },
    Rename {
        account: String,
        #[arg(long, value_enum)]
        tool: ToolId,
        #[arg(long)]
        label: String,
    },
    Reconnect {
        account: String,
        #[arg(long, value_enum)]
        tool: ToolId,
    },
    Remove {
        account: String,
        #[arg(long, value_enum)]
        tool: ToolId,
    },
    Select {
        account: String,
        #[arg(long, value_enum)]
        tool: ToolId,
    },
}

#[derive(Subcommand)]
pub enum SessionCommand {
    List,
}

#[derive(Deserialize)]
pub struct Snapshot {
    pub accounts: Vec<AccountView>,
    pub defaults: Vec<Selection>,
    pub generations: std::collections::BTreeMap<ToolId, Generation>,
}

pub fn call<T: DeserializeOwned>(
    store: &ConfigStore,
    method: &str,
    params: Value,
) -> anyhow::Result<T> {
    let response =
        client::try_call(store, method, &params)?.ok_or(ManagedError::DaemonUnavailable)?;
    if let Some(error) = response.error {
        anyhow::bail!("{}", error.code);
    }
    Ok(serde_json::from_value(
        response.result.ok_or(ManagedError::InvalidRequest)?,
    )?)
}

pub fn resolve(
    snapshot: &Snapshot,
    tool: ToolId,
    name: Option<&str>,
    connection: Option<ConnectionKind>,
) -> Result<AccountView, ManagedError> {
    let candidates: Vec<_> = snapshot
        .accounts
        .iter()
        .filter(|a| a.tool == tool && connection.is_none_or(|c| a.connection == c))
        .collect();
    if let Some(name) = name {
        let matches: Vec<_> = candidates
            .into_iter()
            .filter(|a| a.id.to_string() == name || a.label == name)
            .collect();
        return match matches.as_slice() {
            [a] => Ok((*a).clone()),
            [] => Err(ManagedError::NotFound),
            _ => Err(ManagedError::AmbiguousAccount),
        };
    }
    if let Some(selection) = snapshot
        .defaults
        .iter()
        .find(|s| s.tool == tool && connection.is_none_or(|c| s.connection == c))
    {
        return snapshot
            .accounts
            .iter()
            .find(|a| a.id == selection.account_id)
            .cloned()
            .ok_or(ManagedError::NotFound);
    }
    Err(ManagedError::NotFound)
}

pub async fn launch(store: &ConfigStore, args: LaunchArgs) -> anyhow::Result<i32> {
    use std::io::IsTerminal;
    let snapshot: Snapshot = call(store, "managed_snapshot", json!({}))?;
    let account = resolve(
        &snapshot,
        args.tool,
        args.account.as_deref(),
        args.connection,
    )?;
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("managed launch requires an interactive terminal");
    }
    let cwd = std::fs::canonicalize(args.cwd.unwrap_or(std::env::current_dir()?))?;
    let expected_generation = snapshot
        .generations
        .get(&args.tool)
        .copied()
        .unwrap_or_default();
    let request = LaunchRequest {
        purpose: super::sessions::LaunchPurpose::Coding,
        request_id: Uuid::new_v4(),
        tool: args.tool,
        connection: account.connection,
        account_id: account.id,
        cwd,
        expected_generation,
        save_default: false,
    };
    let prepared: PreparedLaunch = call(store, "managed_launch_prepare", json!(request))?;
    super::supervisor::redeem_and_run(store, prepared).await
}

fn print_value(value: Value, _json_output: bool) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

pub async fn accounts(
    store: &ConfigStore,
    command: AccountCommand,
    json_output: bool,
) -> anyhow::Result<()> {
    if matches!(command, AccountCommand::List) {
        let snapshot: Value = call(store, "managed_snapshot", json!({}))?;
        return print_value(snapshot["accounts"].clone(), json_output);
    }
    if let AccountCommand::Add {
        tool,
        connection,
        label,
        key_stdin,
    } = command
    {
        if connection == ConnectionKind::Subscription && key_stdin {
            anyhow::bail!("subscription sign-in does not accept API keys");
        }
        if connection != ConnectionKind::Subscription && !key_stdin {
            anyhow::bail!("use --key-stdin to supply an API key through stdin");
        }
        let account: AccountView = call(
            store,
            "managed_account_add",
            json!({"tool":tool,"connection":connection,"label":label}),
        )?;
        if connection != ConnectionKind::Subscription {
            use std::io::Read;
            let mut key = String::new();
            std::io::stdin().take(8194).read_to_string(&mut key)?;
            let key = key.trim_end_matches(['\r', '\n']);
            let result: Value = call(
                store,
                "managed_account_set_key",
                json!({"account_id":account.id,"key":key}),
            )?;
            return print_value(result, json_output);
        }
        return login(store, &account, json_output).await;
    }
    let snapshot: Snapshot = call(store, "managed_snapshot", json!({}))?;
    let (name, tool) = match &command {
        AccountCommand::Rename { account, tool, .. }
        | AccountCommand::Reconnect { account, tool }
        | AccountCommand::Remove { account, tool }
        | AccountCommand::Select { account, tool } => (account, *tool),
        _ => return Err(ManagedError::InvalidRequest.into()),
    };
    let account = resolve(&snapshot, tool, Some(name), None)?;
    let result: Value = match command {
        AccountCommand::Rename { label, .. } => call(
            store,
            "managed_account_rename",
            json!({"account_id":account.id,"label":label}),
        )?,
        AccountCommand::Remove { .. } => call(
            store,
            "managed_account_remove",
            json!({"account_id":account.id}),
        )?,
        AccountCommand::Reconnect { .. } => return login(store, &account, json_output).await,
        AccountCommand::Select { .. } => {
            let generation = snapshot.generations.get(&tool).copied().unwrap_or_default();
            call(
                store,
                "managed_select",
                json!({"selection":{"tool":tool,"connection":account.connection,"account_id":account.id,"generation":generation},"expected_generation":generation}),
            )?
        }
        _ => return Err(ManagedError::InvalidRequest.into()),
    };
    print_value(result, json_output)
}

async fn login(
    store: &ConfigStore,
    account: &AccountView,
    json_output: bool,
) -> anyhow::Result<()> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("native sign-in requires an interactive terminal");
    }
    let prepared: PreparedLaunch = call(
        store,
        "managed_account_reconnect",
        json!({"account_id":account.id}),
    )?;
    let code = super::supervisor::redeem_and_run(store, prepared).await?;
    let result: Value = call(
        store,
        "managed_account_verify",
        json!({"account_id":account.id}),
    )?;
    if code != 0 {
        anyhow::bail!("native sign-in did not complete; reconnect this saved account");
    }

    print_value(result, json_output)
}

pub fn sessions(
    store: &ConfigStore,
    _command: SessionCommand,
    json_output: bool,
) -> anyhow::Result<()> {
    let snapshot: Value = call(store, "managed_snapshot", json!({}))?;
    print_value(snapshot["sessions"].clone(), json_output)
}

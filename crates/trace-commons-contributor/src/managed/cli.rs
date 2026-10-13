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
    /// Exact saved account name or ID; uses that account's backend.
    #[arg(long)]
    pub account: Option<String>,
    /// Explicit backend filter. With --account, must match that account's backend.
    #[arg(long, value_enum)]
    pub connection: Option<ConnectionKind>,
    /// Project folder (defaults to the current directory).
    #[arg(long)]
    pub cwd: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum AccountCommand {
    /// Show every saved account, backend, readiness, default, and launch command.
    ///
    /// Use an exact name or the displayed ID with launch --account. Example commands
    /// use IDs so duplicate names remain unambiguous. NEAR AI DEFAULT marks the
    /// account used when launch omits --account and --connection.
    ///
    /// --json emits the account array with a launch_default boolean on each row.
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
    /// Choose the NEAR AI default for future launches of a tool.
    ///
    /// Accepts an exact saved name or ID. Only NEAR AI accounts can be selected;
    /// use launch --account for subscription or direct API-key accounts.
    /// This does not change any running session. Run near-ai accounts list to see all choices.
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
    super::accounts::resolve_launch_account(
        &snapshot.accounts,
        &snapshot.defaults,
        tool,
        name,
        connection,
    )
}

pub async fn launch(store: &ConfigStore, args: LaunchArgs) -> anyhow::Result<i32> {
    use std::io::IsTerminal;
    let snapshot: Snapshot = call(store, "managed_snapshot", json!({}))?;
    let account = resolve(
        &snapshot,
        args.tool,
        args.account.as_deref(),
        args.connection,
    )
    .map_err(|error| {
        launch_selection_error(
            error,
            args.tool,
            args.account.is_none() && args.connection.is_none_or(|c| c == ConnectionKind::NearAi),
        )
    })?;
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
        let snapshot: Snapshot = call(store, "managed_snapshot", json!({}))?;
        println!("{}", format_accounts(&snapshot, json_output)?);
        return Ok(());
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
            if account.connection != ConnectionKind::NearAi {
                anyhow::bail!(
                    "Only NEAR AI accounts can be the launch default. Use --account for another backend. Run near-ai accounts list to see all choices."
                );
            }
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

fn tool_name(tool: ToolId) -> &'static str {
    match tool {
        ToolId::Claude => "claude",
        ToolId::Codex => "codex",
    }
}

fn launch_selection_error(error: ManagedError, tool: ToolId, implicit_near: bool) -> anyhow::Error {
    let tool = tool_name(tool);
    match error {
        ManagedError::NotFound if implicit_near => anyhow::anyhow!(
            "No NEAR AI account is saved for {tool}. Add one in the app, or pipe your key to: near-ai accounts add --tool {tool} --connection near-ai --label 'NEAR AI' --key-stdin. Run near-ai accounts list to see saved choices; use --account to choose another backend."
        ),
        ManagedError::AmbiguousAccount => anyhow::anyhow!(
            "Choose an account with --account <name-or-id>. Run near-ai accounts list to see all choices. To save a NEAR AI default: near-ai accounts select <name-or-id> --tool {tool}."
        ),
        ManagedError::NotFound => anyhow::anyhow!(
            "No matching account for {tool}. Run near-ai accounts list to see all choices, then pass --account <name-or-id>."
        ),
        _ => error.into(),
    }
}

/// Local account metadata only. Example commands use IDs, avoiding ambiguous
/// labels and preventing shell metacharacters in a label from becoming code.
pub fn format_accounts(snapshot: &Snapshot, json_output: bool) -> anyhow::Result<String> {
    let is_default = |account: &AccountView| {
        resolve(snapshot, account.tool, None, None).is_ok_and(|a| a.id == account.id)
    };
    if json_output {
        let rows: Vec<_> = snapshot
            .accounts
            .iter()
            .map(|account| {
                let mut row = json!(account);
                row["launch_default"] = json!(is_default(account));
                row
            })
            .collect();
        return Ok(serde_json::to_string_pretty(&rows)?);
    }
    let mut out = String::from("ACCOUNT | TOOL | BACKEND | READINESS | NEAR AI DEFAULT\n");
    if snapshot.accounts.is_empty() {
        out.push_str("No saved accounts. Add one in the app or use near-ai accounts add --help.\n");
    }
    let copy = super::copy::copy();
    for account in &snapshot.accounts {
        let connection = serde_json::to_value(account.connection)?;
        let auth = serde_json::to_value(account.auth_state)?;
        // Legacy metadata may predate the label validation. Do not emit terminal controls.
        let label: String = account.label.chars().filter(|c| !c.is_control()).collect();
        out.push_str(&format!(
            "{} | {} | {} | {} | {}\n  near-ai launch {} --account {}\n",
            label,
            tool_name(account.tool),
            copy[connection.as_str().unwrap_or_default()]
                .as_str()
                .unwrap_or("Unknown"),
            copy[format!("auth_{}", auth.as_str().unwrap_or_default())]
                .as_str()
                .unwrap_or("Unknown"),
            if is_default(account) { "yes" } else { "—" },
            tool_name(account.tool),
            account.id
        ));
    }
    out.push_str(
        "\nWithout --account, launches use NEAR AI. Names or IDs can select another account.\n",
    );
    out.push_str(
        "Set a NEAR AI default: near-ai accounts select <name-or-id> --tool <claude|codex>\n",
    );
    Ok(out)
}

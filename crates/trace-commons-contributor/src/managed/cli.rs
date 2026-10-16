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
    /// Explicit model ID for a managed OpenCode session.
    #[arg(long)]
    pub model: Option<String>,
    /// Collect this OpenCode session on exit for the usual review pipeline.
    #[arg(long)]
    pub collect_traces: bool,
}

#[derive(Subcommand)]
pub enum AccountCommand {
    /// Show every saved account, backend, readiness, default, and launch command.
    ///
    /// Use an exact name or the displayed ID with launch --account. Example commands
    /// use IDs so duplicate names remain unambiguous. NEAR AI DEFAULT marks the
    /// account used when launch omits --account and --connection. NEAR AI
    /// sign-in rows are listed while you are signed in.
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
    /// Accepts an exact saved name or ID. Only saved NEAR AI keys can be selected;
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
    /// Derived from the NEAR AI sign-in, never saved; empty without one.
    #[serde(default)]
    pub sign_in_accounts: Vec<AccountView>,
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
        &snapshot.sign_in_accounts,
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
            args.account.is_some(),
            args.connection,
            snapshot
                .sign_in_accounts
                .iter()
                .any(|a| a.tool == args.tool),
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
        model: args.model,
        collect_traces: args.collect_traces,
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
    let mut snapshot: Snapshot = call(store, "managed_snapshot", json!({}))?;
    // Saved accounts only: a sign-in entry cannot be renamed, removed,
    // reconnected or saved as a default.
    snapshot.sign_in_accounts.clear();
    let (name, tool) = match &command {
        AccountCommand::Rename { account, tool, .. }
        | AccountCommand::Reconnect { account, tool }
        | AccountCommand::Remove { account, tool }
        | AccountCommand::Select { account, tool } => (account, *tool),
        _ => return Err(ManagedError::InvalidRequest.into()),
    };
    // Only a NEAR AI key can be the launch default, so select looks among
    // those alone: a same-named subscription or API-key account is no
    // ambiguity. Sign-in entries were cleared above; they are never saved.
    let filter = matches!(command, AccountCommand::Select { .. }).then_some(ConnectionKind::NearAi);
    let account = resolve(&snapshot, tool, Some(name), filter).map_err(|error| match error {
        ManagedError::NotFound if filter.is_some() => anyhow::anyhow!(
            "No saved NEAR AI key for {} has that name or ID. Only NEAR AI keys can be the launch default. Run near-ai accounts list to see all choices.",
            tool_name(tool)
        ),
        ManagedError::AmbiguousAccount => anyhow::anyhow!(
            "More than one {} account has that name. Pass its ID from near-ai accounts list instead.",
            tool_name(tool)
        ),
        error => error.into(),
    })?;
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

fn tool_name(tool: ToolId) -> &'static str {
    match tool {
        ToolId::Claude => "claude",
        ToolId::Codex => "codex",
        ToolId::Opencode => "opencode",
    }
}

/// `signed_in`: whether the tool has a NEAR AI sign-in entry.
fn launch_selection_error(
    error: ManagedError,
    tool: ToolId,
    account_given: bool,
    connection: Option<ConnectionKind>,
    signed_in: bool,
) -> anyhow::Error {
    let tool = tool_name(tool);
    match (error, account_given, connection) {
        // A name matched several accounts: only an ID tells them apart.
        (ManagedError::AmbiguousAccount, true, _) => anyhow::anyhow!(
            "More than one {tool} account has that name. Pass its ID from near-ai accounts list to --account."
        ),
        (ManagedError::AmbiguousAccount, false, None) => anyhow::anyhow!(
            "More than one NEAR AI key is saved for {tool}. Choose one with --account <name-or-id>, or save a default: near-ai accounts select <name-or-id> --tool {tool}. Run near-ai accounts list to see all choices."
        ),
        (ManagedError::AmbiguousAccount, false, Some(_)) => anyhow::anyhow!(
            "More than one {tool} account uses that backend. Pass --account with an ID from near-ai accounts list."
        ),
        // Signed in, the tool's sign-in is the implicit launch; only a
        // launch that names the key backend can miss it.
        (ManagedError::NotFound, false, Some(ConnectionKind::NearAi)) if signed_in => {
            anyhow::anyhow!(
                "No NEAR AI key is saved for {tool}. Leave out --connection to use your NEAR AI sign-in."
            )
        }
        // OpenCode has no sign-in: a saved key is its only NEAR AI account.
        (ManagedError::NotFound, false, None | Some(ConnectionKind::NearAi))
            if tool == "opencode" =>
        {
            anyhow::anyhow!(
                "No NEAR AI key is saved for opencode. Add one in the app, or pipe your key to: near-ai accounts add --tool opencode --connection near-ai --label 'NEAR AI' --key-stdin. Use --account to choose another account."
            )
        }
        (ManagedError::NotFound, false, None | Some(ConnectionKind::NearAi)) => anyhow::anyhow!(
            "No NEAR AI account is saved for {tool}. Sign in to NEAR AI in the app, or pipe your key to: near-ai accounts add --tool {tool} --connection near-ai --label 'NEAR AI' --key-stdin. Run near-ai accounts list to see saved choices; use --account to choose another backend."
        ),
        (ManagedError::NotFound, _, _) => anyhow::anyhow!(
            "No matching account for {tool}. Run near-ai accounts list to see all choices, then pass --account <name-or-id>."
        ),
        (error, _, _) => error.into(),
    }
}

/// Local account metadata only. Example commands use IDs, avoiding ambiguous
/// labels and preventing shell metacharacters in a label from becoming code.
/// The NEAR AI sign-in entries are listed after the saved accounts.
pub fn format_accounts(snapshot: &Snapshot, json_output: bool) -> anyhow::Result<String> {
    let is_default = |account: &AccountView| {
        resolve(snapshot, account.tool, None, None).is_ok_and(|a| a.id == account.id)
    };
    let all = || snapshot.accounts.iter().chain(&snapshot.sign_in_accounts);
    if json_output {
        let rows: Vec<_> = all()
            .map(|account| {
                let mut row = json!(account);
                row["launch_default"] = json!(is_default(account));
                row
            })
            .collect();
        return Ok(serde_json::to_string_pretty(&rows)?);
    }
    let mut out = String::from("ACCOUNT | TOOL | BACKEND | READINESS | NEAR AI DEFAULT\n");
    if all().next().is_none() {
        out.push_str("No saved accounts. Add one in the app or use near-ai accounts add --help.\n");
    }
    let copy = super::copy::copy();
    for account in all() {
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
        "\nWithout --account, launches use NEAR AI: the NEAR AI default or sole NEAR AI key, then the NEAR AI sign-in.\n",
    );
    out.push_str(
        "Set a NEAR AI default: near-ai accounts select <name-or-id> --tool <claude|codex|opencode>\n",
    );
    Ok(out)
}

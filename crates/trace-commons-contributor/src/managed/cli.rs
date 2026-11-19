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
    /// A saved account to run on, by label or ID. Without it, the session
    /// runs on your NEAR AI sign-in, and asks you to sign in if needed.
    #[arg(long)]
    pub account: Option<String>,
    /// Run on a saved account of this kind (its default, else a choice).
    /// `near-ai` is your NEAR AI sign-in, or a saved NEAR AI key.
    #[arg(long, value_enum)]
    pub connection: Option<ConnectionKind>,
    #[arg(long)]
    pub cwd: Option<PathBuf>,
    /// Model ID. Required for OpenCode on a saved key. For Claude Code or
    /// Codex on NEAR AI, picks the model the session runs on instead of the
    /// tool's default; it must be a NEAR AI chat model.
    #[arg(long)]
    pub model: Option<String>,
    /// Collect this managed session on exit for the usual review pipeline.
    #[arg(long)]
    pub collect_traces: bool,
    /// Disable remembered trace collection for this launch.
    #[arg(long, conflicts_with = "collect_traces")]
    pub no_collect_traces: bool,
    /// Use the selected account's default model instead of its folder choice.
    #[arg(long, conflicts_with = "model")]
    pub default_model: bool,
    /// Override this folder's saved CLI options; an empty string clears them.
    #[arg(long, allow_hyphen_values = true)]
    pub cli_options: Option<String>,
    /// Input file or directory copied into `/in` inside a Red Team sandbox (repeatable).
    #[arg(long = "input")]
    pub inputs: Vec<PathBuf>,
    /// Network allowlist entry for a Red Team sandbox (repeatable).
    #[arg(long = "allow-host")]
    pub allow_hosts: Vec<String>,
    /// Allow standard package registries inside a Red Team sandbox.
    #[arg(long)]
    pub packages: bool,
    /// Guest vCPU count for a Red Team sandbox.
    #[arg(long)]
    pub cpus: Option<u32>,
    /// Guest memory size for a Red Team sandbox (e.g. `4096` or `4G`).
    #[arg(long)]
    pub memory: Option<String>,
    /// Initial prompt text (UTF-8, at most 64 KiB) for any managed tool.
    #[arg(long, conflicts_with = "prompt_file")]
    pub prompt: Option<String>,
    /// Read the initial prompt from a UTF-8 file (at most 64 KiB).
    #[arg(long, conflicts_with = "prompt")]
    pub prompt_file: Option<PathBuf>,
    /// Open the prepared session in a new terminal window and print its session ID.
    #[arg(long)]
    pub new_terminal: bool,
}

/// Maximum UTF-8 byte length of `--prompt` / `--prompt-file`.
pub const MAX_INITIAL_PROMPT_BYTES: usize = 64 * 1024;

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
    /// Continue an ended session's conversation in this terminal, as a new
    /// session with the same account, folder and options.
    Resume {
        /// The session's id, as `sessions list` shows it.
        session_id: Uuid,
    },
}

#[derive(Deserialize)]
pub struct Snapshot {
    pub accounts: Vec<AccountView>,
    /// Derived from the NEAR AI sign-in, never saved; empty without one.
    #[serde(default)]
    pub sign_in_accounts: Vec<AccountView>,
    pub defaults: Vec<Selection>,
    pub generations: std::collections::BTreeMap<ToolId, Generation>,
    #[serde(default)]
    pub tools: Vec<Value>,
}

pub fn call<T: DeserializeOwned>(
    store: &ConfigStore,
    method: &str,
    params: Value,
) -> anyhow::Result<T> {
    let response =
        client::try_call(store, method, &params)?.ok_or(ManagedError::DaemonUnavailable)?;
    if let Some(error) = response.error {
        return Err(render_refusal(&error.code, &error.message));
    }
    Ok(serde_json::from_value(
        response.result.ok_or(ManagedError::InvalidRequest)?,
    )?)
}

/// The error for a daemon refusal labelled `code`: its label, except for a
/// refusal whose sentence tells the person what to change.
#[cfg(test)]
fn refusal(code: &str) -> anyhow::Error {
    render_refusal(code, code)
}

pub fn render_refusal(code: &str, message: &str) -> anyhow::Error {
    if code == super::DAEMON_UPDATE_REQUIRED_LABEL {
        return ManagedError::DaemonUpdateRequired.into();
    }
    if code == super::PROJECT_CHOOSES_MODEL_LABEL {
        return ManagedError::ProjectChoosesModel.into();
    }
    if code == super::PROJECT_SETS_AUTO_MODE_SERVER_LABEL {
        return ManagedError::ProjectSetsAutoModeServer.into();
    }
    if code == super::RED_TEAM_UNAVAILABLE_LABEL {
        return ManagedError::RedTeamUnavailable.into();
    }
    if code == super::RED_TEAM_NEEDS_NEAR_AI_LABEL {
        return ManagedError::RedTeamNeedsNearAi.into();
    }
    if code == super::SANDBOX_INPUT_REFUSED_LABEL || code == super::SANDBOX_HOST_REFUSED_LABEL {
        return anyhow::anyhow!("{message}");
    }
    anyhow::anyhow!("{code}")
}

/// Parse `--memory` (`N`, `NM`, `NG`, at least 64 MiB).
pub fn parse_memory_mib(raw: &str) -> anyhow::Result<u32> {
    let (digits, scale) = match raw.as_bytes().last() {
        Some(b'G' | b'g') => (&raw[..raw.len() - 1], 1024u32),
        Some(b'M' | b'm') => (&raw[..raw.len() - 1], 1u32),
        _ => (raw, 1u32),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        anyhow::bail!("--memory must be a whole number of MiB or end in M or G: {raw}");
    }
    let mib = digits
        .parse::<u32>()
        .ok()
        .and_then(|n| n.checked_mul(scale))
        .ok_or_else(|| anyhow::anyhow!("--memory value out of range: {raw}"))?;
    if mib < 64 {
        anyhow::bail!("--memory must be at least 64 MiB: {raw}");
    }
    Ok(mib)
}

/// Read and validate `--prompt` or `--prompt-file` (UTF-8, at most 64 KiB).
pub fn resolve_initial_prompt(
    prompt: Option<String>,
    prompt_file: Option<&std::path::Path>,
) -> anyhow::Result<Option<String>> {
    if prompt.is_some() && prompt_file.is_some() {
        anyhow::bail!("--prompt and --prompt-file cannot be used together");
    }
    if let Some(text) = prompt {
        if text.len() > MAX_INITIAL_PROMPT_BYTES {
            anyhow::bail!(
                "--prompt exceeds the {} KiB limit",
                MAX_INITIAL_PROMPT_BYTES / 1024
            );
        }
        return Ok(Some(text));
    }
    if let Some(path) = prompt_file {
        let bytes = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("cannot read --prompt-file {}: {e}", path.display()))?;
        if bytes.len() > MAX_INITIAL_PROMPT_BYTES {
            anyhow::bail!(
                "--prompt-file {} exceeds the {} KiB limit",
                path.display(),
                MAX_INITIAL_PROMPT_BYTES / 1024
            );
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| anyhow::anyhow!("--prompt-file {} must be valid UTF-8", path.display()))?;
        return Ok(Some(text));
    }
    Ok(None)
}

/// Validate CLI launch options before contacting the daemon.
pub fn validate_launch_args(
    args: &LaunchArgs,
) -> anyhow::Result<(Option<super::sandbox::SandboxSettings>, Option<String>)> {
    let has_red_team_opts = !args.inputs.is_empty()
        || !args.allow_hosts.is_empty()
        || args.packages
        || args.cpus.is_some()
        || args.memory.is_some();
    if args.tool != ToolId::RedTeam && has_red_team_opts {
        anyhow::bail!("these options apply only to red-team");
    }
    let initial_prompt = resolve_initial_prompt(args.prompt.clone(), args.prompt_file.as_deref())?;
    let sandbox = if args.tool == ToolId::RedTeam && has_red_team_opts {
        let memory_mib = args.memory.as_deref().map(parse_memory_mib).transpose()?;
        Some(super::sandbox::SandboxSettings {
            inputs: args.inputs.clone(),
            allow_hosts: args.allow_hosts.clone(),
            packages: args.packages,
            cpus: args.cpus,
            memory_mib,
        })
    } else {
        None
    };
    Ok((sandbox, initial_prompt))
}

/// Whether `--connection asked` admits an account on `connection`. The
/// NEAR AI sign-in is not offered as a `--connection` value of its own;
/// `near-ai` names it as well as a saved NEAR AI key.
fn admits(asked: Option<ConnectionKind>, connection: ConnectionKind) -> bool {
    asked.is_none_or(|asked| {
        asked == connection
            || (asked == ConnectionKind::NearAi && connection == ConnectionKind::NearAiSignIn)
    })
}

/// What to say when a launch names no account and one must be chosen: the
/// `--account` value for each that fits, by label, or by ID where labels
/// repeat.
fn choose_account(snapshot: &Snapshot, tool: ToolId, connection: Option<ConnectionKind>) -> String {
    let fits: Vec<_> = snapshot
        .accounts
        .iter()
        .chain(&snapshot.sign_in_accounts)
        .filter(|a| a.tool == tool && admits(connection, a.connection))
        .collect();
    let mut text = String::from("choose the account to run this tool on:");
    for account in &fits {
        let repeated = fits.iter().filter(|a| a.label == account.label).count() > 1;
        let name = if repeated {
            account.id.0.to_string()
        } else {
            account.label.clone()
        };
        let kind = match account.connection {
            ConnectionKind::NearAiSignIn => "NEAR AI sign-in",
            ConnectionKind::NearAi => "NEAR AI key",
            ConnectionKind::Subscription => "subscription",
            ConnectionKind::ApiKey => "API key",
        };
        text.push_str(&format!("\n  --account {name:?}  ({kind})"));
    }
    text
}

/// The account a launch runs on. Named (`--account`): that account. Not
/// named: the NEAR AI sign-in, as the app's Start button uses it, or
/// `NearAiSignInRequired` when there is none. A `--connection` other than
/// `near-ai` asks for a saved account instead: the tool's default for that
/// connection, else a choice (`AmbiguousAccount`). A saved account is
/// never used unless named or the default, so removing a default asks for
/// a new choice rather than moving to another saved account.
pub fn resolve(
    snapshot: &Snapshot,
    tool: ToolId,
    name: Option<&str>,
    connection: Option<ConnectionKind>,
) -> Result<AccountView, ManagedError> {
    let candidates: Vec<_> = snapshot
        .accounts
        .iter()
        .chain(&snapshot.sign_in_accounts)
        .filter(|a| a.tool == tool && admits(connection, a.connection))
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
    let near_ai = matches!(connection, None | Some(ConnectionKind::NearAi));
    if near_ai {
        if let Some(sign_in) = candidates
            .iter()
            .find(|a| a.connection == ConnectionKind::NearAiSignIn)
        {
            return Ok((*sign_in).clone());
        }
        if connection.is_none() || candidates.is_empty() {
            return Err(ManagedError::NearAiSignInRequired);
        }
    }
    if let Some(selection) = snapshot
        .defaults
        .iter()
        .find(|s| s.tool == tool && admits(connection, s.connection))
    {
        return snapshot
            .accounts
            .iter()
            .find(|a| a.id == selection.account_id)
            .cloned()
            .ok_or(ManagedError::NotFound);
    }
    match candidates.as_slice() {
        [] => Err(ManagedError::NotFound),
        _ => Err(ManagedError::AmbiguousAccount),
    }
}

/// Asked when a launch names no account and there is no NEAR AI sign-in.
const SIGN_IN_QUESTION: &str = "Sessions run on your NEAR AI account. Sign in now? [Y/n] ";
/// Without a terminal to ask at, or when the person declines.
const SIGN_IN_NEEDED: &str = "Sign in to NEAR AI to start a session, or choose a saved account with --account. See `trace-commons-contributor accounts list`.";

/// How long `launch` waits, after signing in, for the daemon to offer the
/// sign-in, and how often it looks.
const SIGN_IN_PICKUP: std::time::Duration = std::time::Duration::from_secs(10);
const SIGN_IN_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// Ask at the terminal whether to sign in to NEAR AI. Empty means yes.
fn agrees(answer: &str) -> bool {
    matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    )
}

pub async fn launch(
    store: &ConfigStore,
    args: LaunchArgs,
    json_output: bool,
) -> anyhow::Result<i32> {
    use std::io::IsTerminal;
    launch_with_tty(store, args, json_output, std::io::stdin().is_terminal()).await
}

pub async fn launch_with_tty(
    store: &ConfigStore,
    args: LaunchArgs,
    json_output: bool,
    interactive: bool,
) -> anyhow::Result<i32> {
    let (sandbox, initial_prompt) = validate_launch_args(&args)?;
    if !args.new_terminal && !interactive {
        anyhow::bail!(
            "managed launch requires an interactive terminal (pass --new-terminal to open one)"
        );
    }
    let mut snapshot: Snapshot = call(store, "managed_snapshot", json!({}))?;
    if args.tool == ToolId::RedTeam && !snapshot.tools.iter().any(|t| t["tool"] == "red_team") {
        return Err(ManagedError::RedTeamUnavailable.into());
    }
    let resolved = |snapshot: &Snapshot| {
        resolve(
            snapshot,
            args.tool,
            args.account.as_deref(),
            args.connection,
        )
    };
    let account = match resolved(&snapshot) {
        Err(ManagedError::NearAiSignInRequired) => {
            if !interactive {
                anyhow::bail!(SIGN_IN_NEEDED);
            }
            eprint!("{SIGN_IN_QUESTION}");
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;
            if !agrees(&answer) {
                anyhow::bail!(SIGN_IN_NEEDED);
            }
            crate::near_ai_login::run_for_launch(store)?;
            // The daemon takes up a new sign-in on its next pass, not at
            // once: wait a little for the sign-in to be offered.
            let mut waited = std::time::Duration::ZERO;
            loop {
                snapshot = call(store, "managed_snapshot", json!({}))?;
                match resolved(&snapshot) {
                    Err(ManagedError::NearAiSignInRequired) if waited < SIGN_IN_PICKUP => {
                        std::thread::sleep(SIGN_IN_POLL);
                        waited += SIGN_IN_POLL;
                    }
                    other => break other?,
                }
            }
        }
        Err(ManagedError::AmbiguousAccount) if args.account.is_none() => {
            anyhow::bail!("{}", choose_account(&snapshot, args.tool, args.connection))
        }
        other => other?,
    };
    let cwd = std::fs::canonicalize(args.cwd.unwrap_or(std::env::current_dir()?))?;
    let expected_generation = snapshot
        .generations
        .get(&args.tool)
        .copied()
        .unwrap_or_default();
    let remembered: Option<super::folder::FolderSnapshot> = {
        let response = client::try_call(
            store,
            "managed_folder_settings_get",
            &json!({"tool":args.tool,"cwd":cwd}),
        )?
        .ok_or(ManagedError::DaemonUnavailable)?;
        match response.error {
            Some(error) if error.code == crate::daemon::ipc::ERR_UNKNOWN_METHOD => None,
            Some(error) => anyhow::bail!("{}", error.code),
            None => Some(serde_json::from_value(
                response.result.ok_or(ManagedError::InvalidRequest)?,
            )?),
        }
    };
    let model = effective_model(
        args.tool,
        account.connection,
        args.model,
        args.default_model,
        remembered.as_ref().and_then(|value| value.model.clone()),
    );
    let collect_traces = !args.no_collect_traces
        && (args.collect_traces
            || remembered
                .as_ref()
                .is_some_and(|value| value.collect_traces));
    let request = LaunchRequest {
        cli_options: args.cli_options,
        model,
        collect_traces,
        purpose: super::sessions::LaunchPurpose::Coding,
        request_id: Uuid::new_v4(),
        tool: args.tool,
        connection: account.connection,
        account_id: account.id,
        cwd,
        expected_generation,
        save_default: false,
        keep_saved_model: false,
        resume_of: None,
        background_model: None,
        sandbox,
        initial_prompt,
    };
    let prepared: PreparedLaunch = call(store, "managed_launch_prepare", json!(request))?;
    if args.new_terminal {
        let ticket = prepared.ticket.ok_or(ManagedError::LaunchUnknown)?;
        let _opened: Value = call(
            store,
            "managed_terminal_launch",
            json!({"session_id": prepared.session_id, "ticket": ticket}),
        )?;
        if json_output {
            println!("{}", json!({"session_id": prepared.session_id}));
        } else {
            println!("{}", prepared.session_id);
        }
        return Ok(0);
    }
    super::supervisor::redeem_and_run(store, prepared).await
}

fn effective_model(
    tool: ToolId,
    connection: ConnectionKind,
    explicit: Option<String>,
    default_model: bool,
    remembered: Option<String>,
) -> Option<String> {
    if explicit.is_some() {
        return explicit;
    }
    if default_model {
        return None;
    }
    if tool == ToolId::Opencode
        || matches!(
            connection,
            ConnectionKind::NearAi | ConnectionKind::NearAiSignIn
        )
    {
        remembered
    } else {
        None
    }
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

/// `sessions list`. `sessions resume` runs a tool, so it is [`resume`].
pub fn list_sessions(store: &ConfigStore, json_output: bool) -> anyhow::Result<()> {
    let snapshot: Value = call(store, "managed_snapshot", json!({}))?;
    print_value(snapshot["sessions"].clone(), json_output)
}

/// Continue the conversation of the ended session `session_id` in this
/// terminal: the daemon prepares a new session that resumes it, with the
/// checks Start again makes, and this process runs the tool as `launch`
/// does. Returns the tool's exit code.
pub async fn resume(store: &ConfigStore, session_id: Uuid) -> anyhow::Result<i32> {
    let prepared: PreparedLaunch = call(
        store,
        "managed_session_resume",
        json!({"session_id":session_id,"request_id":Uuid::new_v4()}),
    )?;
    super::supervisor::redeem_and_run(store, prepared).await
}

#[cfg(test)]
mod tests {
    /// A refusal the person can act on reads as its sentence in the
    /// terminal, not as its label (owner, 2026-11-01); any other refusal
    /// keeps its label.
    #[test]
    fn a_project_auto_mode_server_refusal_prints_why() {
        assert_eq!(
            refusal(crate::managed::PROJECT_SETS_AUTO_MODE_SERVER_LABEL).to_string(),
            "Remove CLAUDE_CODE_AUTO_MODE_SERVER from this project's settings."
        );
    }

    #[test]
    fn a_project_model_refusal_prints_why() {
        assert_eq!(
            refusal(crate::managed::PROJECT_CHOOSES_MODEL_LABEL).to_string(),
            crate::managed::ManagedError::ProjectChoosesModel.to_string()
        );
        assert_eq!(
            refusal("managed-model-unavailable").to_string(),
            "managed-model-unavailable"
        );
    }

    #[test]
    fn an_old_daemon_refusal_names_the_app_update_action() {
        assert_eq!(
            refusal("managed-daemon-update-required").to_string(),
            format!(
                "Update and reopen {} before launching Codex.",
                crate::brand::APP_NAME
            )
        );
    }

    use super::*;
    use crate::managed::{AccountId, AuthState, signin};

    fn saved(tool: ToolId, connection: ConnectionKind, label: &str) -> AccountView {
        AccountView {
            id: AccountId(Uuid::new_v4()),
            tool,
            connection,
            label: label.into(),
            auth_state: AuthState::Ready,
            verified_at: None,
        }
    }

    fn snapshot(accounts: Vec<AccountView>, signed_in: bool) -> Snapshot {
        Snapshot {
            accounts,
            sign_in_accounts: if signed_in {
                signin::entries(true)
            } else {
                Vec::new()
            },
            defaults: Vec::new(),
            generations: Default::default(),
            tools: Vec::new(),
        }
    }

    /// With no account named, every tool runs on the NEAR AI sign-in, even
    /// beside a saved account or default for that tool.
    #[test]
    fn an_unnamed_launch_runs_on_the_near_ai_sign_in() {
        let personal = saved(ToolId::Claude, ConnectionKind::Subscription, "Personal");
        let mut snap = snapshot(vec![personal.clone()], true);
        snap.defaults.push(Selection {
            tool: ToolId::Claude,
            connection: ConnectionKind::Subscription,
            account_id: personal.id,
            generation: Generation(1),
        });
        for tool in [ToolId::Claude, ToolId::Codex, ToolId::Opencode] {
            for connection in [None, Some(ConnectionKind::NearAi)] {
                assert_eq!(
                    resolve(&snap, tool, None, connection).map(|a| a.id),
                    Ok(signin::entry(tool).unwrap().id),
                    "{tool:?} {connection:?}"
                );
            }
        }
        // Another account is still there to name, or to reach by its
        // connection's default.
        assert_eq!(
            resolve(&snap, ToolId::Claude, Some("Personal"), None).map(|a| a.id),
            Ok(personal.id)
        );
        assert_eq!(
            resolve(
                &snap,
                ToolId::Claude,
                None,
                Some(ConnectionKind::Subscription)
            )
            .map(|a| a.id),
            Ok(personal.id)
        );
    }

    /// Without a NEAR AI sign-in, an unnamed launch asks for one, whatever
    /// is saved; `launch` turns this into the sign-in question.
    #[test]
    fn without_a_sign_in_an_unnamed_launch_needs_one() {
        let personal = saved(ToolId::Claude, ConnectionKind::Subscription, "Personal");
        for accounts in [vec![], vec![personal]] {
            let snap = snapshot(accounts, false);
            for connection in [None, Some(ConnectionKind::NearAi)] {
                assert_eq!(
                    resolve(&snap, ToolId::Claude, None, connection).map(|a| a.id),
                    Err(ManagedError::NearAiSignInRequired),
                    "{connection:?}"
                );
            }
        }
        // A saved NEAR AI key is a NEAR AI connection too: named by
        // `--connection near-ai`, it is a choice, not a sign-in.
        let key = saved(ToolId::Codex, ConnectionKind::NearAi, "Key");
        let snap = snapshot(vec![key], false);
        assert_eq!(
            resolve(&snap, ToolId::Codex, None, Some(ConnectionKind::NearAi)).map(|a| a.id),
            Err(ManagedError::AmbiguousAccount)
        );
        assert_eq!(
            resolve(&snap, ToolId::Codex, None, None).map(|a| a.id),
            Err(ManagedError::NearAiSignInRequired)
        );
    }

    /// A connection other than NEAR AI, with no default, is a choice: a
    /// saved account runs only when named or the default.
    #[test]
    fn a_saved_account_is_not_used_without_being_chosen() {
        let personal = saved(ToolId::Claude, ConnectionKind::Subscription, "Personal");
        let snap = snapshot(vec![personal], true);
        let asked = Some(ConnectionKind::Subscription);
        assert_eq!(
            resolve(&snap, ToolId::Claude, None, asked).map(|a| a.id),
            Err(ManagedError::AmbiguousAccount)
        );
        let text = choose_account(&snap, ToolId::Claude, asked);
        assert!(
            text.contains("--account \"Personal\"  (subscription)"),
            "{text}"
        );
        assert!(!text.contains("NEAR AI sign-in"), "{text}");
        // Nothing saved for the connection asked for: not found.
        assert_eq!(
            resolve(&snap, ToolId::Codex, None, asked).map(|a| a.id),
            Err(ManagedError::NotFound)
        );
    }

    /// The choice names each account by label, or by ID where labels repeat.
    #[test]
    fn a_choice_lists_the_accounts_by_label() {
        let a = saved(ToolId::Codex, ConnectionKind::ApiKey, "Work");
        let b = saved(ToolId::Codex, ConnectionKind::ApiKey, "Work");
        let snap = snapshot(vec![a.clone(), b], true);
        let text = choose_account(&snap, ToolId::Codex, Some(ConnectionKind::ApiKey));
        assert!(text.contains(&a.id.0.to_string()), "{text}");
        assert!(text.contains("(API key)"), "{text}");
    }

    /// Return or "y" signs in; anything else does not.
    #[test]
    fn only_yes_or_return_agrees_to_sign_in() {
        for yes in ["", "\n", "y\n", "Y", " yes \n"] {
            assert!(agrees(yes), "{yes:?}");
        }
        for no in ["n\n", "no", "later", "q"] {
            assert!(!agrees(no), "{no:?}");
        }
    }
    #[test]
    fn remembered_models_follow_the_current_connection_and_explicit_choices() {
        let remembered = Some("provider/model".to_string());
        for tool in [ToolId::Claude, ToolId::Codex, ToolId::Opencode] {
            assert_eq!(
                effective_model(
                    tool,
                    ConnectionKind::NearAiSignIn,
                    None,
                    false,
                    remembered.clone()
                ),
                remembered
            );
            assert_eq!(
                effective_model(
                    tool,
                    ConnectionKind::NearAiSignIn,
                    None,
                    true,
                    remembered.clone()
                ),
                None
            );
            assert_eq!(
                effective_model(
                    tool,
                    ConnectionKind::NearAiSignIn,
                    Some("provider/explicit".into()),
                    false,
                    remembered.clone()
                )
                .as_deref(),
                Some("provider/explicit")
            );
        }
        assert_eq!(
            effective_model(
                ToolId::Claude,
                ConnectionKind::Subscription,
                None,
                false,
                remembered.clone()
            ),
            None
        );
        assert_eq!(
            effective_model(
                ToolId::Codex,
                ConnectionKind::ApiKey,
                None,
                false,
                remembered
            ),
            None
        );
    }
}

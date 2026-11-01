//! Own the native child and report its lifetime to the shared UI registry.

use super::{
    AccountView, AuthState, ConnectionKind, ManagedError,
    cli::call,
    profiles::{NativeAction, NativeProfiles, find_native},
    secrets::SecretStore,
    sessions::{LaunchContext, LifecycleReport, PreparedLaunch, SessionState},
};
use crate::config::ConfigStore;
use serde_json::{Value, json};

/// A floor for the tool's soft open-file limit. Terminal.app starts shells
/// at launchd's 256, which Claude Code outgrows at startup ("low max file
/// descriptors"). It is a floor, not a setting: a limit already at or above
/// it, or unlimited, is left alone, and only the soft limit moves.
#[cfg(unix)]
const OPEN_FILES_FLOOR: u32 = 10240;

/// Run `command` under `/bin/sh`, which lifts a soft open-file limit below
/// [`OPEN_FILES_FLOOR`] and then execs the tool, so the child inherits the
/// raised limit however the helper was started (the app's button, `near-ai
/// launch`, or a sign-in). The shell becomes the tool, so the pid, signals
/// and exit code are the tool's own. A refusal (a lower hard limit) leaves
/// the limit as it was. Program, arguments, environment changes and working
/// directory carry over; the spawn sets nothing else on the command.
#[cfg(unix)]
fn with_open_files_floor(command: std::process::Command) -> std::process::Command {
    let mut wrapped = std::process::Command::new("/bin/sh");
    wrapped
        .arg("-c")
        .arg(format!(
            "[ \"$(ulimit -Sn)\" = unlimited ] || [ \"$(ulimit -Sn)\" -ge {OPEN_FILES_FLOOR} ] || ulimit -Sn {OPEN_FILES_FLOOR} 2>/dev/null; exec \"$0\" \"$@\""
        ))
        .arg(command.get_program())
        .args(command.get_args());
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => wrapped.env(key, value),
            None => wrapped.env_remove(key),
        };
    }
    if let Some(dir) = command.get_current_dir() {
        wrapped.current_dir(dir);
    }
    wrapped
}

/// Markers a Claude Code session sets in the environment of the processes it
/// starts, and the IDE attachment it inherits from an IDE terminal. A
/// managed session is a new top-level session, never a child of whatever
/// started the helper, the daemon or the terminal, so none of them may reach
/// the tool: with `CLAUDE_CODE_CHILD_SESSION` inherited, Claude Code treats
/// the session as a subagent and turns transcript saving off, which also
/// leaves trace collection with nothing to read.
///
/// Named one by one rather than by the `CLAUDE_CODE_` prefix: that prefix
/// also carries the person's own settings (`CLAUDE_CODE_MAX_OUTPUT_TOKENS`,
/// `CLAUDE_CODE_DISABLE_*`), which a managed session keeps.
const PARENT_SESSION_ENV: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_BRIDGE_SESSION_ID",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_INVOKED_SKILLS",
    "CLAUDE_CODE_SSE_PORT",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
];

/// Drop every [`PARENT_SESSION_ENV`] marker from the tool's environment.
fn clear_parent_session(command: &mut std::process::Command) {
    for key in PARENT_SESSION_ENV {
        command.env_remove(key);
    }
}

pub async fn redeem_and_run(store: &ConfigStore, prepared: PreparedLaunch) -> anyhow::Result<i32> {
    let mut signals = TerminalSignals::new()?;
    let context: LaunchContext = call(
        store,
        "managed_launch_redeem",
        json!({"session_id":prepared.session_id,"ticket":prepared.ticket.ok_or(ManagedError::LaunchUnknown)?}),
    )?;
    let mut spawned = false;
    let result = run_native(
        store,
        &context,
        &mut spawned,
        &mut signals,
        context.session.purpose == super::sessions::LaunchPurpose::Login,
    )
    .await;
    if result.is_err() && !spawned {
        let report = LifecycleReport {
            model: None,
            collection: context
                .session
                .collect_traces
                .then_some(super::sessions::CollectionState::Failed),
            session_id: context.session.id,
            proof: context.proof.clone(),
            state: SessionState::Failed,
            process_id: None,
            process_identity: None,
            exit_code: None,
        };
        let _ = super::sessions::persist_exit(store, &report);
        let response: anyhow::Result<Value> = call(store, "managed_session_report", json!(report));
        if response.is_ok() {
            super::sessions::clear_exit(store, context.session.id);
        }
    }
    result
}

async fn run_native(
    store: &ConfigStore,
    context: &LaunchContext,
    spawned: &mut bool,
    signals: &mut TerminalSignals,
    login: bool,
) -> anyhow::Result<i32> {
    let row = &context.session;
    let account = AccountView {
        id: row.account_id,
        tool: row.tool,
        connection: row.connection,
        label: row.account_label.clone(),
        auth_state: AuthState::SignInRequired,
        verified_at: None,
    };
    let profiles = NativeProfiles::new(store);
    let native = find_native(row.tool)?;
    profiles.verify_version(&account, &native)?;
    if row.connection == ConnectionKind::Subscription
        && !login
        && profiles.inspect(&account, &native)? != AuthState::Ready
    {
        return Err(ManagedError::SignInRequired.into());
    }
    let resume = if login {
        None
    } else {
        resume_for(store, row.tool, context.resume_of)?
    };
    let mut command = profiles.command(
        &account,
        &native,
        if login {
            NativeAction::Login
        } else {
            NativeAction::Run
        },
        &row.cwd,
    )?;
    command.args(leading_args(row.tool, resume));
    if !login && row.tool != super::ToolId::Opencode {
        command.args(&context.cli_options);
    }
    if !login && row.tool == super::ToolId::Claude {
        command.args(claude_session_args(row.id, resume));
    }
    let mut route = None;
    // Removed when this function returns, however it returns.
    let mut codex_catalog: Option<super::codex::CatalogFile> = None;
    let opencode = if row.tool == super::ToolId::Opencode {
        // Every connection below runs on this session, seeded only when new.
        let session = opencode_session(store, &account, &native, row, resume)?;
        // A saved key names its model; the sign-in's proxy names a default.
        let model = || row.model.as_deref().ok_or(ManagedError::InvalidRequest);
        match row.connection {
            ConnectionKind::Subscription => {
                return Err(ManagedError::InvalidRequest.into());
            }
            ConnectionKind::NearAiSignIn => {
                command = opencode_sign_in_command(&session, context)?;
            }
            ConnectionKind::ApiKey => {
                let model = model()?;
                command = session.command(
                    model,
                    "https://api.openai.com/v1",
                    SecretStore::new(store).get(account.id)?.expose(),
                )?;
            }
            ConnectionKind::NearAi => {
                let model = model()?;
                let proxy = super::route::ManagedRoute::start(
                    store,
                    row.id,
                    SecretStore::new(store).get(account.id)?,
                    row.tool,
                    Some(model),
                )
                .await?;
                command = opencode_route_command(
                    &session,
                    proxy.port(),
                    proxy.default_model(),
                    proxy.models(),
                )?;
                route = Some(proxy);
            }
        }
        Some(session)
    } else {
        None
    };
    if opencode.is_some() && !login {
        command.args(&context.cli_options);
    }
    if opencode.is_none() {
        match row.connection {
            ConnectionKind::Subscription => {}
            ConnectionKind::ApiKey => {
                let key = SecretStore::new(store).get(account.id)?;
                match row.tool {
                    super::ToolId::Opencode => return Err(ManagedError::InvalidRequest.into()),
                    super::ToolId::Claude => {
                        command.env("ANTHROPIC_API_KEY", key.expose());
                    }
                    super::ToolId::Codex => {
                        command.env("NEAR_AI_SESSION_API_KEY", key.expose()).args([
                            "-c",
                            "model_provider=\"near_ai_openai\"",
                            "-c",
                            "model_providers.near_ai_openai.name=\"OpenAI API\"",
                            "-c",
                            "model_providers.near_ai_openai.base_url=\"https://api.openai.com/v1\"",
                            "-c",
                            "model_providers.near_ai_openai.env_key=\"NEAR_AI_SESSION_API_KEY\"",
                            "-c",
                            "model_providers.near_ai_openai.wire_api=\"responses\"",
                        ]);
                    }
                }
            }
            ConnectionKind::NearAi => {
                let proxy = super::route::ManagedRoute::start(
                    store,
                    row.id,
                    SecretStore::new(store).get(account.id)?,
                    row.tool,
                    row.model.as_deref(),
                )
                .await?;
                codex_catalog = codex_catalog_file(
                    store,
                    &profiles,
                    &account,
                    &native,
                    row,
                    proxy.default_model(),
                    proxy.models(),
                )?;
                near_ai_route(
                    &mut command,
                    row.tool,
                    proxy.port(),
                    proxy.default_model(),
                    proxy.models(),
                    codex_catalog.as_ref().map(|file| file.path()),
                )?;
                if !login {
                    status_line(&mut command, row.tool, proxy.home());
                }
                route = Some(proxy);
            }
            // The daemon holds the sign-in and hosts this session's proxy. This
            // process learns its port, default and models, never the
            // credential; without them, no run.
            ConnectionKind::NearAiSignIn => {
                codex_catalog = sign_in_route_command(
                    &mut command,
                    store,
                    &profiles,
                    &account,
                    &native,
                    context,
                    login,
                )?;
            }
        }
    }
    // Signals were registered before ticket redemption. A terminal closed
    // during native status checks must never lead to a late child spawn.
    #[cfg(unix)]
    tokio::select! {
        biased;
        _ = signals.hangup.recv() => return Err(ManagedError::LaunchUnknown.into()),
        _ = signals.terminate.recv() => return Err(ManagedError::LaunchUnknown.into()),
        _ = signals.interrupt.recv() => return Err(ManagedError::LaunchUnknown.into()),
        _ = tokio::task::yield_now() => {},
    }
    #[cfg(windows)]
    tokio::select! {
        biased;
        _ = signals.interrupt.recv() => return Err(ManagedError::LaunchUnknown.into()),
        _ = signals.break_signal.recv() => return Err(ManagedError::LaunchUnknown.into()),
        _ = tokio::task::yield_now() => {},
    }
    // After every other argument, so a hook the person passed is seen.
    let codex_binding = if row.tool == super::ToolId::Codex && !login && resume.is_none() {
        codex_binding(
            store,
            row,
            login,
            resume,
            profiles.native_version(&account, &native).ok().flatten(),
            &mut command,
        )
    } else {
        None
    };
    // Staged under this row's id. A resumed row (collecting only because its
    // root did) stages the root's continued conversation file.
    let native_collection = if row.collect_traces && !login && opencode.is_none() {
        let collection = super::native_collection::NativeCollection::snapshot(
            store,
            row.tool,
            row.account_id,
            row.id,
            codex_binding,
        )?;
        Some(match resume {
            Some(resume) => collection.continuing(resume.root),
            None => collection,
        })
    } else {
        None
    };
    clear_parent_session(&mut command);
    #[cfg(unix)]
    let command = with_open_files_floor(command);
    let mut child = tokio::process::Command::from(command)
        .kill_on_drop(false)
        .spawn()?;
    *spawned = true;
    let pid = child.id().ok_or(ManagedError::LaunchUnknown)?;
    let process = super::collection_plan::ProcessIdentity::of(pid);
    let identity = process
        .label()
        .unwrap_or_else(|| format!("{}:{}", pid, uuid::Uuid::new_v4()));
    // Written before the first report, so a launcher that dies from here on
    // leaves the daemon what it needs to collect once the tool is gone. A
    // plan that cannot be written leaves collection to this launcher alone.
    let plan_written = row.collect_traces
        && !login
        && super::collection_plan::write(
            store,
            &super::collection_plan::CollectionPlan {
                schema_version: 1,
                session_id: row.id,
                tool: row.tool,
                account_id: row.account_id,
                resume_root: resume.map(|resume| resume.root),
                cwd: row.cwd.clone(),
                collect_traces: true,
                created_at: chrono::Utc::now(),
                native: native_collection.as_ref().map(|c| c.plan()),
                opencode: opencode.as_ref().map(|s| s.plan()),
                process,
            },
        )
        .is_ok();
    // A proxy this launcher started tells the daemon the default model,
    // so the row can show it.
    let default_model = route
        .as_ref()
        .map(|r: &super::route::ManagedRoute| r.default_model().to_owned());
    let report = |state, exit_code| LifecycleReport {
        model: default_model.clone(),
        collection: None,
        session_id: row.id,
        proof: context.proof.clone(),
        state,
        process_id: Some(pid),
        process_identity: Some(identity.clone()),
        exit_code,
    };
    let answer: anyhow::Result<Value> = call(
        store,
        "managed_session_report",
        json!(report(SessionState::Running, None)),
    );
    let status = wait_for_tool(&mut child, pid, &identity, signals, answer, || {
        call(
            store,
            "managed_session_report",
            json!(report(SessionState::Running, None)),
        )
    })
    .await?;
    let code = exit_code(status);
    let mut receipt = report(SessionState::Exited, Some(code));
    // The terminal may be gone by now (a closed window is how a hangup
    // arrives), so nothing below may fail on a write to it.
    let claim = claim_collection(store, row.id, plan_written);
    if row.collect_traces {
        receipt.collection = match claim {
            Claim::Collect => {
                let collected = match opencode.as_ref() {
                    Some(session) => session.collect_as(store, row.id),
                    None => native_collection
                        .as_ref()
                        .ok_or(ManagedError::InvalidRequest)?
                        .collect(store),
                };
                Some(match collected {
                    Ok(()) => super::sessions::CollectionState::Collected,
                    Err(_) => {
                        best_effort_stderr(
                            "The session exited, but its trace could not be collected. The local session is preserved.",
                        );
                        super::sessions::CollectionState::Failed
                    }
                })
            }
            // The daemon collected, or is collecting, this row.
            Claim::Taken => None,
            Claim::Unavailable => Some(super::sessions::CollectionState::Failed),
        };
    }
    let persisted = super::sessions::persist_exit(store, &receipt);
    let final_report: anyhow::Result<Value> = call(store, "managed_session_report", json!(receipt));
    if final_report.is_ok() {
        super::sessions::clear_exit(store, row.id);
    }
    if finishes_plan(plan_written, claim) {
        super::collection_plan::finish(store, row.id);
    }
    if final_report.is_err() && persisted.is_err() {
        best_effort_stderr(&format!(
            "Native session exited; {} status is unknown until the daemon reconnects.",
            crate::brand::APP_NAME
        ));
    }
    if let Some(route) = route {
        route.shutdown().await;
    }
    drop(codex_catalog);
    if login {
        let _: anyhow::Result<Value> = call(
            store,
            "managed_account_verify",
            json!({"account_id":row.account_id}),
        );
    }
    Ok(code)
}

/// The conversation a launch continues ([`LaunchContext::resume_of`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Resume {
    /// The root row, whose id is Claude Code's and OpenCode's conversation.
    root: uuid::Uuid,
    /// Codex's own thread id, which the root's binding recorded. Codex only.
    codex_thread: Option<uuid::Uuid>,
}

/// The conversation `resume_of` names for `tool`, or none for a fresh one.
/// Codex reads the thread id its root's binding recorded and never prepares
/// that binding again; without one there is nothing to resume.
fn resume_for(
    store: &ConfigStore,
    tool: super::ToolId,
    resume_of: Option<uuid::Uuid>,
) -> Result<Option<Resume>, ManagedError> {
    let Some(root) = resume_of else {
        return Ok(None);
    };
    let codex_thread = if tool == super::ToolId::Codex {
        Some(
            super::session_binding::read(store, root)
                .ok_or(ManagedError::NotFound)?
                .session_id,
        )
    } else {
        None
    };
    Ok(Some(Resume { root, codex_thread }))
}

/// Arguments straight after the tool's executable, before any option:
/// Codex continues a conversation through `resume <thread id>`, which takes
/// the same options and `-c` overrides as a fresh launch.
fn leading_args(tool: super::ToolId, resume: Option<Resume>) -> Vec<String> {
    match (tool, resume) {
        (
            super::ToolId::Codex,
            Some(Resume {
                codex_thread: Some(thread),
                ..
            }),
        ) => vec!["resume".to_owned(), thread.to_string()],
        _ => vec![],
    }
}

/// Claude Code's conversation, after the options: the row's own id for a
/// fresh one, or `--resume <root>` to continue the root's (and no
/// `--session-id`, which would start another).
fn claude_session_args(id: uuid::Uuid, resume: Option<Resume>) -> [String; 2] {
    match resume {
        Some(resume) => ["--resume".to_owned(), resume.root.to_string()],
        None => ["--session-id".to_owned(), id.to_string()],
    }
}

/// OpenCode's session for this launch. A resumed one is the root row's:
/// the same per-session profile and `ses_<root>` id, and never seeded
/// again. OpenCode 2.1.4 answers a re-import of an existing id with
/// "Session already exists" and changes nothing; v1 is unverified. Skipping
/// import depends on neither. A new one is the row's own, seeded through
/// import. Collection stages under the row's id either way.
fn opencode_session(
    store: &ConfigStore,
    account: &AccountView,
    native: &std::path::Path,
    row: &super::sessions::SessionView,
    resume: Option<Resume>,
) -> Result<super::opencode::OpenCodeSession, ManagedError> {
    let id = resume.map_or(row.id, |resume| resume.root);
    let session = super::opencode::OpenCodeSession::new(store, account, native, id, &row.cwd)?;
    if resume.is_none() {
        session.prepare()?;
    }
    Ok(session)
}

/// Prepare and install this Codex coding launch's SessionStart binding,
/// which records Codex's own thread id so the conversation can be resumed,
/// whether or not the launch collects. Trace collection reads the same
/// records. None for a sign-in, a resumed conversation (its root's binding
/// holds the id), a Codex before 0.164, or a refusal (the person passed
/// their own CLI hooks): the launch goes on, and that row cannot be
/// resumed. Unavailable tracking never blocks a session or loosens hook
/// trust.
fn codex_binding(
    store: &ConfigStore,
    row: &super::sessions::SessionView,
    login: bool,
    resume: Option<Resume>,
    version: Option<(u32, u32, u32)>,
    command: &mut std::process::Command,
) -> Option<super::session_binding::CodexBinding> {
    if row.tool != super::ToolId::Codex
        || login
        || resume.is_some()
        || !version.is_some_and(|version| version.0 == 0 && version >= (0, 164, 0))
    {
        return None;
    }
    let binding = super::session_binding::prepare(store, row.id).ok()?;
    binding.configure(command).ok()?;
    Some(binding)
}

/// The daemon-hosted proxy a sign-in session runs on: its loopback port,
/// default model and NEAR AI's chat models. Without all three, no run.
///
/// The models are the ones the tool is offered ([`super::models::offered`]):
/// a daemon from before a model was hidden from a tool can still list it,
/// and name it as the default. Such a default is replaced by the tool's
/// default among the offered models; the proxy serves any listed model. A
/// default the daemon did not list at all is passed on unchanged.
fn sign_in_route(
    context: &LaunchContext,
) -> Result<(u16, String, Vec<super::models::ChatModel>), ManagedError> {
    let tool = context.session.tool;
    let port = context.route_port.ok_or(ManagedError::LaunchUnknown)?;
    let named = context
        .route_model
        .as_deref()
        .ok_or(ManagedError::LaunchUnknown)?;
    let listed = context
        .route_models
        .clone()
        .ok_or(ManagedError::LaunchUnknown)?;
    let hidden = listed.iter().any(|m| m.id == named);
    let models = super::models::offered(tool, listed);
    let hidden = hidden && !models.iter().any(|m| m.id == named);
    // Only a listed default this tool is not offered is replaced; one the
    // list does not hold at all is passed on, and refused there.
    let default = if hidden {
        super::models::default_for(tool, &models)
            .ok_or(ManagedError::ModelUnavailable)?
            .id
            .clone()
    } else {
        named.to_owned()
    };
    Ok((port, default, models))
}

/// Claude Code or Codex on the NEAR AI sign-in: the daemon holds the
/// sign-in and hosts this session's proxy, and says its port, default and
/// models ([`sign_in_route`]). Points `command` at it, writes Codex's
/// catalog, and, unless this is a sign-in, adds the status line for the
/// daemon's route, whose home is where every route's is
/// ([`super::route::home_for`]). Returns the catalog file, which lives
/// as long as the session.
fn sign_in_route_command(
    command: &mut std::process::Command,
    store: &ConfigStore,
    profiles: &NativeProfiles,
    account: &AccountView,
    native: &std::path::Path,
    context: &LaunchContext,
    login: bool,
) -> Result<Option<super::codex::CatalogFile>, ManagedError> {
    let row = &context.session;
    let (port, default, models) = sign_in_route(context)?;
    let catalog = codex_catalog_file(store, profiles, account, native, row, &default, &models)?;
    near_ai_route(
        command,
        row.tool,
        port,
        &default,
        &models,
        catalog.as_ref().map(|file| file.path()),
    )?;
    if !login {
        status_line(command, row.tool, &super::route::home_for(store, row.id));
    }
    Ok(catalog)
}

/// For Codex, write the session's model catalog: every NEAR AI chat model,
/// with Codex's own settings for the ones it knows. None for other tools.
fn codex_catalog_file(
    store: &ConfigStore,
    profiles: &NativeProfiles,
    account: &AccountView,
    native: &std::path::Path,
    row: &super::sessions::SessionView,
    default: &str,
    models: &[super::models::ChatModel],
) -> Result<Option<super::codex::CatalogFile>, ManagedError> {
    if row.tool != super::ToolId::Codex {
        return Ok(None);
    }
    let mut read = profiles.command(account, native, NativeAction::Run, &row.cwd)?;
    read.args(["debug", "models", "--bundled"]);
    let catalog = super::codex::catalog(default, models, super::codex::bundled(read).as_ref());
    super::codex::CatalogFile::write(store, row.id, &catalog).map(Some)
}

/// OpenCode on the NEAR AI sign-in: the daemon hosts this session's proxy
/// and says its loopback port, default model and models. This process
/// never sees the credential; without all three, no run.
fn opencode_sign_in_command(
    session: &super::opencode::OpenCodeSession,
    context: &LaunchContext,
) -> Result<std::process::Command, ManagedError> {
    let (port, default, models) = sign_in_route(context)?;
    opencode_route_command(session, port, &default, &models)
}

/// OpenCode on a session's local NEAR AI proxy, the launcher's own (a
/// saved key) or the daemon's (the sign-in): it starts on `default` and
/// offers every model in `models`, with a placeholder key.
fn opencode_route_command(
    session: &super::opencode::OpenCodeSession,
    port: u16,
    default: &str,
    models: &[super::models::ChatModel],
) -> Result<std::process::Command, ManagedError> {
    session.command_with_models(
        default,
        models,
        &format!("http://127.0.0.1:{port}/openai/v1"),
        "managed-local-session",
    )
}

/// Point the tool at a session's local NEAR AI proxy. The tool gets the
/// loopback URL and a placeholder token, never a credential, starts on
/// `default`, and can name only models in `models`: the proxy serves a
/// listed model as named and gives an unlisted id a substitute. Codex
/// reads its list from `codex_catalog` ([`super::codex`]).
fn near_ai_route(
    command: &mut std::process::Command,
    tool: super::ToolId,
    port: u16,
    default: &str,
    models: &[super::models::ChatModel],
    codex_catalog: Option<&std::path::Path>,
) -> Result<(), ManagedError> {
    let base = format!("http://127.0.0.1:{port}");
    match tool {
        super::ToolId::Opencode => return Err(ManagedError::InvalidRequest),
        super::ToolId::Claude => {
            command
                .env("ANTHROPIC_BASE_URL", format!("{base}/anthropic"))
                .env("ANTHROPIC_AUTH_TOKEN", "managed-local-session");
            for (name, value) in super::models::claude_env(default, models) {
                match value {
                    Some(value) => command.env(name, value),
                    None => command.env_remove(name),
                };
            }
        }
        super::ToolId::Codex => {
            command
                .env("NEAR_AI_SESSION_API_KEY", "managed-local-session")
                .args([
                    "-c",
                    "model_provider=\"near_ai_managed\"",
                    "-c",
                    "model_providers.near_ai_managed.name=\"NEAR AI\"",
                    "-c",
                ])
                .arg(format!(
                    "model_providers.near_ai_managed.base_url=\"{base}/openai/v1\""
                ))
                .args([
                    "-c",
                    "model_providers.near_ai_managed.env_key=\"NEAR_AI_SESSION_API_KEY\"",
                    "-c",
                    "model_providers.near_ai_managed.wire_api=\"responses\"",
                    // The upstream compatibility lane is Chat Completions,
                    // which cannot execute provider-hosted web search.
                    "-c",
                    "web_search=\"disabled\"",
                ]);
            let catalog = codex_catalog
                .and_then(|path| path.to_str())
                .ok_or(ManagedError::InvalidRequest)?;
            // TOML strings, quoted as the config loader reads them.
            let quoted = |value: &str| toml::Value::String(value.into()).to_string();
            command
                .arg("-c")
                .arg(format!("model_catalog_json={}", quoted(catalog)))
                .arg("-c")
                .arg(format!("model={}", quoted(default)));
        }
    }
    Ok(())
}

/// For Claude Code, show the route's status line: `--settings` naming
/// this executable's `statusline` for the route's IronWire home
/// ([`super::statusline`]). Settings from the command line take precedence
/// over the profile's and a project's, and nothing is written to disk.
/// When this executable's path cannot be found or written, the session runs
/// without a status line. Other tools have no status-line slot.
fn status_line(command: &mut std::process::Command, tool: super::ToolId, home: &std::path::Path) {
    if tool != super::ToolId::Claude {
        return;
    }
    match super::statusline::settings(home) {
        Some(settings) => {
            command.arg("--settings").arg(settings);
        }
        None => tracing::warn!(
            harness = "claude",
            "managed session runs without a status line"
        ),
    }
}

/// Whether the daemon's answer to a running report asks for a stop. Only
/// an explicit `true` does; anything else leaves the tool running.
fn stop_asked(answer: &anyhow::Result<Value>) -> bool {
    matches!(answer, Ok(value) if value.get("stop") == Some(&Value::Bool(true)))
}

#[cfg(unix)]
fn forward_signal(pid: u32, identity: &str, signal: sysinfo::Signal) {
    let mut system = sysinfo::System::new();
    system.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
        true,
    );
    if let Some(process) = system.process(sysinfo::Pid::from_u32(pid)) {
        if format!("{pid}:{}", process.start_time()) == identity {
            let _ = process.kill_with(signal);
        }
    }
}

/// Write `line` to stderr if it is still there. `eprintln!` panics when
/// it is not, which after a terminal closed would end the launcher before
/// its final report.
fn best_effort_stderr(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{line}");
}

/// The tool's exit code; one killed by a signal exits 128 + that signal,
/// as a shell reports it.
fn exit_code(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
    }
    #[cfg(not(unix))]
    {
        status.code().unwrap_or(1)
    }
}

/// Whether this launcher collects its row after the tool exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Claim {
    /// It does: it took the claim, or wrote no plan the daemon could act on.
    Collect,
    /// The daemon claimed the row first, so it collects.
    Taken,
    /// The claim could not be read or taken; nobody collects.
    Unavailable,
}

/// Whether this launcher removes its row's plan and claim once its tool
/// exited: whenever it collected under its own claim, which then has a
/// final result, even when neither its report nor its exit receipt could
/// be written. A plan left behind with this launcher's claim would read to
/// the daemon as a holder that died mid-collection, and be recorded as
/// failed though the trace was staged; without one, the row's collection
/// stays unknown until this launcher's result reaches the daemon.
fn finishes_plan(plan_written: bool, claim: Claim) -> bool {
    plan_written && claim == Claim::Collect
}

fn claim_collection(store: &ConfigStore, id: uuid::Uuid, plan_written: bool) -> Claim {
    if !plan_written {
        return Claim::Collect;
    }
    match super::collection_plan::claim(
        store,
        id,
        &super::collection_plan::ProcessIdentity::current(),
    ) {
        Ok(true) => Claim::Collect,
        Ok(false) => Claim::Taken,
        Err(_) => Claim::Unavailable,
    }
}

/// Wait for the tool to exit, reporting it running every 5 seconds, and
/// pass on what would end it. `answer` is the daemon's reply to the first
/// running report; `heartbeat` sends the next one.
///
/// Unix: a hangup (terminal window closed) or a terminate (logout, `kill`)
/// is forwarded to the tool, which gets 5 seconds before it is killed;
/// either way this keeps waiting, so the caller still collects and reports.
/// An interrupt is not forwarded: Ctrl-C at the terminal already reaches
/// the tool, which shares this process's foreground process group, and a
/// second copy would read as a double Ctrl-C. The tool is deliberately not
/// given its own process group: it reads the terminal, and a background
/// group would be stopped by SIGTTIN. The kernel delivers a terminal's
/// hangup to both; the forwarded copy is checked against the tool's start
/// time, so it never reaches a reused pid.
///
/// Windows is unchanged: Ctrl-C and Ctrl-Break reach the tool through the
/// console and are only absorbed here, and a stop the daemon asks for kills
/// it. Closing a console window is not handled.
pub async fn wait_for_tool(
    child: &mut tokio::process::Child,
    pid: u32,
    identity: &str,
    signals: &mut TerminalSignals,
    mut answer: anyhow::Result<Value>,
    mut heartbeat: impl FnMut() -> anyhow::Result<Value>,
) -> std::io::Result<std::process::ExitStatus> {
    #[cfg(not(any(unix, windows)))]
    let _ = (pid, identity, &signals);
    #[cfg(windows)]
    let _ = (pid, identity);
    #[cfg(unix)]
    let mut shutdown_at: Option<std::time::Instant> = None;
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(5));
    // The first tick is immediate; the caller already sent the first report.
    ticks.tick().await;
    loop {
        // The daemon asked for a stop: this process owns the child, so it
        // signals it exactly as a terminal's own SIGTERM would, and the
        // escalation below kills it if it has not exited in time.
        #[cfg(unix)]
        if stop_asked(&answer) && shutdown_at.is_none() {
            forward_signal(pid, identity, sysinfo::Signal::Term);
            shutdown_at = Some(std::time::Instant::now());
        }
        #[cfg(windows)]
        if stop_asked(&answer) {
            let _ = child.start_kill();
        }
        answer = Ok(Value::Null);
        #[cfg(unix)]
        tokio::select! {
            status = child.wait() => return status,
            _ = signals.interrupt.recv() => {},
            _ = signals.hangup.recv() => { forward_signal(pid, identity, sysinfo::Signal::Hangup); shutdown_at.get_or_insert_with(std::time::Instant::now); },
            _ = signals.terminate.recv() => { forward_signal(pid, identity, sysinfo::Signal::Term); shutdown_at.get_or_insert_with(std::time::Instant::now); },
            _ = ticks.tick() => {
                if shutdown_at.is_some_and(|at| at.elapsed() >= std::time::Duration::from_secs(5)) { let _ = child.start_kill(); }
                answer = heartbeat();
            },
        }
        #[cfg(windows)]
        tokio::select! {
            status = child.wait() => return status,
            _ = signals.interrupt.recv() => {},
            _ = signals.break_signal.recv() => {},
            _ = ticks.tick() => { answer = heartbeat(); },
        }
        #[cfg(not(any(unix, windows)))]
        tokio::select! {
            status = child.wait() => return status,
            _ = ticks.tick() => { let _ = heartbeat(); },
        }
    }
}

/// The launcher's handlers for what a terminal sends, registered before a
/// launch ticket is redeemed so a closed terminal never kills the launcher
/// and leaves its tool unreported.
pub struct TerminalSignals {
    #[cfg(windows)]
    interrupt: tokio::signal::windows::CtrlC,
    #[cfg(windows)]
    break_signal: tokio::signal::windows::CtrlBreak,
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    hangup: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
}
impl TerminalSignals {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            #[cfg(windows)]
            interrupt: tokio::signal::windows::ctrl_c()?,
            #[cfg(windows)]
            break_signal: tokio::signal::windows::ctrl_break()?,
            #[cfg(unix)]
            interrupt: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
            #[cfg(unix)]
            hangup: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?,
            #[cfg(unix)]
            terminate: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
        })
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::{PARENT_SESSION_ENV, clear_parent_session, with_open_files_floor};
    use super::{near_ai_route, sign_in_route_command, status_line, stop_asked};
    #[cfg(unix)]
    use super::{opencode_route_command, opencode_sign_in_command};
    use crate::config::ConfigStore;
    #[cfg(unix)]
    use crate::managed::{ConnectionKind, opencode::OpenCodeSession};
    use crate::managed::{
        ManagedError, ToolId, models::ChatModel, sessions::LaunchContext, signin,
    };
    use serde_json::json;

    /// The launcher removes its plan whenever it collected under its own
    /// claim, whether or not its result reached the daemon; never a plan
    /// the daemon claimed, or one whose claim could not be taken.
    #[test]
    fn the_launcher_finishes_only_a_plan_it_collected_under_its_own_claim() {
        use super::{Claim, finishes_plan};
        assert!(finishes_plan(true, Claim::Collect));
        assert!(!finishes_plan(true, Claim::Taken));
        assert!(!finishes_plan(true, Claim::Unavailable));
        assert!(!finishes_plan(false, Claim::Collect));
    }

    fn env_of(command: &std::process::Command, name: &str) -> Option<Option<String>> {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.map(|v| v.to_string_lossy().into_owned()))
    }

    fn listed() -> Vec<ChatModel> {
        vec![
            ChatModel {
                id: "deepseek-ai/DeepSeek-V3.1".into(),
                name: None,
                tier: None,
                ..Default::default()
            },
            ChatModel {
                id: "anthropic/claude-sonnet-5-5".into(),
                name: Some("Claude Sonnet 5.5".into()),
                tier: None,
                ..Default::default()
            },
        ]
    }

    /// Claude Code on a NEAR AI route starts on the default, and each of
    /// its aliases names a model NEAR AI lists; a variable that could name
    /// another id is removed.
    #[test]
    fn claude_on_a_route_names_only_listed_models() {
        let mut command = std::process::Command::new("claude");
        near_ai_route(
            &mut command,
            ToolId::Claude,
            4321,
            "deepseek-ai/DeepSeek-V3.1",
            &listed(),
            None,
        )
        .unwrap();
        let env = |name| env_of(&command, name);
        assert_eq!(
            env("ANTHROPIC_BASE_URL"),
            Some(Some("http://127.0.0.1:4321/anthropic".into()))
        );
        assert_eq!(
            env("ANTHROPIC_MODEL"),
            Some(Some("deepseek-ai/DeepSeek-V3.1".into()))
        );
        assert_eq!(
            env("ANTHROPIC_DEFAULT_SONNET_MODEL"),
            Some(Some("anthropic/claude-sonnet-5-5".into()))
        );
        assert_eq!(
            env("ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"),
            Some(Some("Claude Sonnet 5.5".into()))
        );
        assert_eq!(
            env("ANTHROPIC_DEFAULT_OPUS_MODEL"),
            Some(Some("deepseek-ai/DeepSeek-V3.1".into()))
        );
        assert_eq!(
            env("ANTHROPIC_SMALL_FAST_MODEL"),
            Some(Some("deepseek-ai/DeepSeek-V3.1".into()))
        );
        // Removed, not left to the person's shell.
        assert_eq!(env("CLAUDE_CODE_SUBAGENT_MODEL"), Some(None));
        assert_eq!(env("ANTHROPIC_CUSTOM_MODEL_OPTION"), Some(None));
    }

    /// Claude Code on a NEAR AI route, whoever hosts it, gets a status line
    /// that runs this executable for the route's home; no other tool does.
    #[test]
    fn claude_on_a_route_shows_the_routes_status_line() {
        let home = std::path::Path::new("/state dir/managed-routes/it's");
        let mut command = std::process::Command::new("claude");
        status_line(&mut command, ToolId::Claude, home);
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args.len(), 2, "{args:?}");
        assert_eq!(args[0], "--settings");
        let settings: serde_json::Value = serde_json::from_str(&args[1]).unwrap();
        let line = &settings["statusLine"];
        assert_eq!(line["type"], "command");
        let run = line["command"].as_str().unwrap();
        assert!(
            run.ends_with(r" statusline --home '/state dir/managed-routes/it'\''s'"),
            "{run}"
        );
        assert_eq!(settings.as_object().unwrap().len(), 1);
        for tool in [ToolId::Codex, ToolId::Opencode] {
            let mut command = std::process::Command::new("tool");
            status_line(&mut command, tool, home);
            assert_eq!(command.get_args().count(), 0);
        }
    }

    /// Claude Code on the NEAR AI sign-in: the daemon hosts the route, and
    /// the launcher's status line names that route's home,
    /// `<state dir>/managed-routes/<session id>`, worked out here from the
    /// session id alone. A sign-in (`login`) gets no status line.
    #[test]
    fn claude_on_the_sign_in_shows_the_daemons_route_status_line() {
        use crate::managed::profiles::NativeProfiles;
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().join("state")).unwrap();
        let account = signin::entry(ToolId::Claude).unwrap();
        let id = uuid::Uuid::new_v4();
        let context: LaunchContext = serde_json::from_value(json!({
            "session": {
                "id": id, "tool": "claude", "connection": "near_ai_signin",
                "account_id": account.id, "account_label": "NEAR AI",
                "selection_generation": 0, "project_label": "p", "cwd": home.path(),
                "started_at": "2026-10-16T00:00:00Z", "state": "starting",
                "exit_code": null, "can_focus": false,
            },
            "proof": "p", "route_port": 4100,
            "route_model": "anthropic/claude-sonnet-5-5",
            "route_models": listed(),
        }))
        .unwrap();
        let native = home.path().join("claude");
        let expected = home
            .path()
            .join("state")
            .join("managed-routes")
            .join(id.to_string());
        for login in [false, true] {
            let mut command = std::process::Command::new("claude");
            let catalog = sign_in_route_command(
                &mut command,
                &store,
                &NativeProfiles::new(&store),
                &account,
                &native,
                &context,
                login,
            )
            .unwrap();
            assert!(catalog.is_none());
            assert_eq!(
                env_of(&command, "ANTHROPIC_BASE_URL"),
                Some(Some("http://127.0.0.1:4100/anthropic".into()))
            );
            let args: Vec<String> = command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            if login {
                assert!(args.is_empty(), "{args:?}");
                continue;
            }
            assert_eq!(args.len(), 2, "{args:?}");
            assert_eq!(args[0], "--settings");
            assert_eq!(
                Some(args[1].as_str()),
                crate::managed::statusline::settings(&expected).as_deref()
            );
        }
    }

    /// Codex on a NEAR AI route reads its model list from the session's
    /// catalog file and starts on the default; without the file, no run.
    #[test]
    fn codex_on_a_route_uses_the_catalog_file_and_the_default() {
        let path = std::path::Path::new("/state dir/managed-codex-catalogs/s.json");
        let mut command = std::process::Command::new("codex");
        near_ai_route(
            &mut command,
            ToolId::Codex,
            4321,
            "anthropic/claude-sonnet-5-5",
            &listed(),
            Some(path),
        )
        .unwrap();
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let value = |key: &str| {
            args.windows(2)
                .find(|pair| pair[0] == "-c" && pair[1].starts_with(&format!("{key}=")))
                .map(|pair| pair[1][key.len() + 1..].to_owned())
        };
        assert_eq!(value("web_search").as_deref(), Some("\"disabled\""));
        assert_eq!(
            value("model_catalog_json").as_deref(),
            Some("\"/state dir/managed-codex-catalogs/s.json\"")
        );
        assert_eq!(
            value("model").as_deref(),
            Some("\"anthropic/claude-sonnet-5-5\"")
        );
        assert_eq!(
            value("model_providers.near_ai_managed.base_url").as_deref(),
            Some("\"http://127.0.0.1:4321/openai/v1\"")
        );
        let mut command = std::process::Command::new("codex");
        assert_eq!(
            near_ai_route(&mut command, ToolId::Codex, 4321, "x/a", &listed(), None).err(),
            Some(ManagedError::InvalidRequest)
        );
    }

    /// OpenCode on the NEAR AI sign-in runs against the daemon's proxy for
    /// this session, starting on its default model and offering every
    /// model it lists, with a placeholder key. Without the port, the
    /// default or the list there is no command: no run.
    #[cfg(unix)]
    #[test]
    fn opencode_on_the_sign_in_uses_the_route_port_and_models_or_does_not_run() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let native = home.path().join("opencode");
        std::fs::write(
            &native,
            b"#!/bin/sh\n[ \"$1\" = --version ] || exit 1\necho '1.19.0'\n",
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let account = signin::entry(ToolId::Opencode).unwrap();
        let id = uuid::Uuid::new_v4();
        let session = OpenCodeSession::new(&store, &account, &native, id, home.path()).unwrap();
        let context = |port: Option<u16>,
                       model: Option<&str>,
                       models: Option<Vec<ChatModel>>|
         -> LaunchContext {
            serde_json::from_value(json!({
                "session": {
                    "id": id, "tool": "opencode", "connection": "near_ai_signin",
                    "account_id": account.id, "account_label": "NEAR AI",
                    "selection_generation": 0, "project_label": "p", "cwd": home.path(),
                    "started_at": "2026-10-16T00:00:00Z", "state": "starting",
                    "exit_code": null, "can_focus": false,
                },
                "proof": "proof",
                "route_port": port,
                "route_model": model,
                "route_models": models,
            }))
            .unwrap()
        };
        let command = opencode_sign_in_command(
            &session,
            &context(
                Some(4321),
                Some("deepseek-ai/DeepSeek-V3.1"),
                Some(listed()),
            ),
        )
        .unwrap();
        assert_eq!(
            context(None, None, None).session.connection,
            ConnectionKind::NearAiSignIn
        );
        let env = |name: &str| {
            command
                .get_envs()
                .find(|(key, _)| *key == name)
                .and_then(|(_, value)| value)
                .map(|value| value.to_string_lossy().into_owned())
        };
        let config: serde_json::Value =
            serde_json::from_str(&env("OPENCODE_CONFIG_CONTENT").unwrap()).unwrap();
        assert_eq!(config["model"], "trace-managed/deepseek-ai/DeepSeek-V3.1");
        assert_eq!(
            config["small_model"],
            "trace-managed/deepseek-ai/DeepSeek-V3.1"
        );
        assert_eq!(
            config["provider"]["trace-managed"]["models"],
            json!({
                "deepseek-ai/DeepSeek-V3.1": {"name": "deepseek-ai/DeepSeek-V3.1"},
                "anthropic/claude-sonnet-5-5": {"name": "Claude Sonnet 5.5"},
            })
        );
        assert_eq!(
            config["provider"]["trace-managed"]["options"]["baseURL"],
            "http://127.0.0.1:4321/openai/v1"
        );
        assert_eq!(
            env("TRACE_COMMONS_OPENCODE_KEY").as_deref(),
            Some("managed-local-session")
        );
        for (port, model, models) in [
            (None, Some("deepseek-ai/DeepSeek-V3.1"), Some(listed())),
            (Some(4321), None, Some(listed())),
            (Some(4321), Some("deepseek-ai/DeepSeek-V3.1"), None),
            (None, None, None),
        ] {
            let missing = models.is_none();
            assert_eq!(
                opencode_sign_in_command(&session, &context(port, model, models)).err(),
                Some(ManagedError::LaunchUnknown),
                "{port:?} {model:?} {missing}"
            );
        }
        // A default the list does not hold is refused, not added.
        assert_eq!(
            opencode_sign_in_command(
                &session,
                &context(Some(4321), Some("other/model"), Some(listed()))
            )
            .err(),
            Some(ManagedError::InvalidRequest)
        );
    }

    /// OpenCode on a saved NEAR AI key runs against the launcher's own
    /// proxy through the same command as the sign-in: every model the
    /// proxy lists is in OpenCode's picker, not only the default.
    #[cfg(unix)]
    #[test]
    fn opencode_on_a_saved_key_route_offers_every_listed_model() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let native = home.path().join("opencode");
        std::fs::write(
            &native,
            b"#!/bin/sh\n[ \"$1\" = --version ] || exit 1\necho '1.19.0'\n",
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let account = signin::entry(ToolId::Opencode).unwrap();
        let session =
            OpenCodeSession::new(&store, &account, &native, uuid::Uuid::new_v4(), home.path())
                .unwrap();
        let command =
            opencode_route_command(&session, 4321, "anthropic/claude-sonnet-5-5", &listed())
                .unwrap();
        let content = command
            .get_envs()
            .find(|(key, _)| *key == "OPENCODE_CONFIG_CONTENT")
            .and_then(|(_, value)| value)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap();
        let config: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(config["model"], "trace-managed/anthropic/claude-sonnet-5-5");
        assert_eq!(
            config["provider"]["trace-managed"]["models"],
            json!({
                "deepseek-ai/DeepSeek-V3.1": {"name": "deepseek-ai/DeepSeek-V3.1"},
                "anthropic/claude-sonnet-5-5": {"name": "Claude Sonnet 5.5"},
            })
        );
        assert_eq!(
            config["provider"]["trace-managed"]["options"]["baseURL"],
            "http://127.0.0.1:4321/openai/v1"
        );
    }

    /// Codex's catalog takes the prompt from the installed Codex's own
    /// list (`debug models --bundled`), not the basic one-line prompt.
    #[cfg(unix)]
    /// A daemon from before a model was hidden from Codex can still hand
    /// the launcher one: the launcher leaves it out of the picker and,
    /// when it is the default, starts on Codex's own default instead.
    #[test]
    fn a_sign_in_route_never_offers_codex_a_hidden_model() {
        use super::sign_in_route;
        use crate::managed::{models::ChatModel, sessions::LaunchContext};
        let chat = |id: &str| ChatModel {
            id: id.into(),
            name: None,
            tier: None,
            ..Default::default()
        };
        let context = |tool: &str, model: &str| -> LaunchContext {
            serde_json::from_value(json!({
                "session": {
                    "id": uuid::Uuid::new_v4(), "tool": tool, "connection": "near_ai_signin",
                    "account_id": uuid::Uuid::new_v4(), "account_label": "NEAR AI",
                    "selection_generation": 0, "project_label": "p", "cwd": "/tmp",
                    "started_at": "2026-10-19T00:00:00Z", "state": "starting",
                    "exit_code": null, "can_focus": false,
                },
                "proof": "proof",
                "route_port": 4321,
                "route_model": model,
                "route_models": [chat("openai/gpt-6.1-sol"), chat("openai/gpt-6-sol"), chat("x/other")],
            }))
            .unwrap()
        };
        let (port, default, models) =
            sign_in_route(&context("codex", "openai/gpt-6.1-sol")).unwrap();
        assert_eq!(port, 4321);
        assert_eq!(default, "openai/gpt-6-sol");
        assert_eq!(models, vec![chat("openai/gpt-6-sol"), chat("x/other")]);
        // An offered default is kept as the daemon named it.
        let (_, default, _) = sign_in_route(&context("codex", "x/other")).unwrap();
        assert_eq!(default, "x/other");
        // Other tools are offered every model.
        let (_, default, models) = sign_in_route(&context("claude", "openai/gpt-6.1-sol")).unwrap();
        assert_eq!(default, "openai/gpt-6.1-sol");
        assert_eq!(models.len(), 3);
    }

    // The stub Codex is a shell script made executable with Unix permissions.
    #[cfg(unix)]
    #[test]
    fn codex_catalog_file_reads_the_installed_codex_list() {
        use super::codex_catalog_file;
        use crate::managed::{codex::BASIC_INSTRUCTIONS, profiles::NativeProfiles};
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().join("state")).unwrap();
        let cwd = home.path().join("project");
        std::fs::create_dir(&cwd).unwrap();
        // Answers only the bundled-list read; anything else fails.
        let native = home.path().join("codex");
        std::fs::write(
            &native,
            concat!(
                "#!/bin/sh\n",
                "[ \"$*\" = \"debug models --bundled\" ] || exit 1\n",
                "printf '%s' '{\"models\":[{\"slug\":\"gpt-stub\",",
                "\"visibility\":\"list\",\"priority\":1,",
                "\"base_instructions\":\"Stub prompt.\"}]}'\n",
            ),
        )
        .unwrap();
        std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
        let account = signin::entry(ToolId::Codex).unwrap();
        let row: crate::managed::sessions::SessionView = serde_json::from_value(json!({
            "id": uuid::Uuid::new_v4(), "tool": "codex", "connection": "near_ai_signin",
            "account_id": account.id, "account_label": "NEAR AI",
            "selection_generation": 0, "project_label": "p", "cwd": cwd,
            "started_at": "2026-10-16T00:00:00Z", "state": "starting",
            "exit_code": null, "can_focus": false,
        }))
        .unwrap();
        let file = codex_catalog_file(
            &store,
            &NativeProfiles::new(&store),
            &account,
            &native,
            &row,
            "anthropic/claude-sonnet-5-5",
            &listed(),
        )
        .unwrap()
        .unwrap();
        let catalog: serde_json::Value =
            serde_json::from_slice(&std::fs::read(file.path()).unwrap()).unwrap();
        let prompts: Vec<&str> = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["base_instructions"].as_str().unwrap())
            .collect();
        assert_eq!(prompts, ["Stub prompt.", "Stub prompt."]);
        assert_ne!(prompts[0], BASIC_INSTRUCTIONS);
    }

    /// Only the daemon's explicit `stop: true` stops the tool. A missing
    /// field, another value or a failed report leaves it running.
    #[test]
    fn only_an_explicit_stop_answer_stops_the_tool() {
        assert!(stop_asked(&Ok(json!({"accepted":true,"stop":true}))));
        assert!(!stop_asked(&Ok(json!({"accepted":true,"stop":false}))));
        assert!(!stop_asked(&Ok(json!({"accepted":true}))));
        assert!(!stop_asked(&Ok(json!({"accepted":true,"stop":"true"}))));
        assert!(!stop_asked(&Ok(json!({"accepted":true,"stop":1}))));
        assert!(!stop_asked(&Err(anyhow::anyhow!("daemon unavailable"))));
    }

    /// Run `command` the way a terminal that starts at `start_soft` open files
    /// would: a shell lowers its own limit, then execs the wrapped command.
    /// `None` when this host's hard limit forbids the start value.
    #[cfg(unix)]
    fn run_from_soft_limit(
        command: &std::process::Command,
        start_soft: u32,
    ) -> Option<std::process::Output> {
        let mut outer = std::process::Command::new("/bin/sh");
        outer
            .args([
                "-c",
                "ulimit -Sn \"$0\" 2>/dev/null || exit 99; exec \"$@\"",
            ])
            .arg(start_soft.to_string())
            .arg(command.get_program())
            .args(command.get_args());
        for (key, value) in command.get_envs() {
            match value {
                Some(value) => outer.env(key, value),
                None => outer.env_remove(key),
            };
        }
        if let Some(dir) = command.get_current_dir() {
            outer.current_dir(dir);
        }
        let out = outer.output().unwrap();
        (out.status.code() != Some(99)).then_some(out)
    }

    #[cfg(unix)]
    fn probe() -> std::process::Command {
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args([
                "-c",
                "printf '%s|%s|%s|%s|%s' \"$(ulimit -Sn)\" \"$0\" \"$1\" \"$KEPT\" \"${GONE-unset}\"; exit 7",
                "tool name",
                "a 'quoted' $HOME `arg`",
            ])
            .env("KEPT", "yes")
            .env_remove("GONE");
        command
    }

    /// A helper started from inside a Claude Code session (its Bash tool, or
    /// a terminal or daemon that one started) inherits that session's
    /// markers. The managed tool must start as its own top-level session, or
    /// Claude Code turns transcript saving off.
    #[cfg(unix)]
    #[test]
    fn the_spawned_tool_does_not_inherit_a_parent_claude_code_session() {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", "env"]);
        for key in PARENT_SESSION_ENV {
            command.env(key, "1");
        }
        command.env("KEPT", "yes");
        clear_parent_session(&mut command);
        let out = with_open_files_floor(command).output().unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        let names: Vec<&str> = text
            .lines()
            .filter_map(|line| line.split_once('=').map(|(name, _)| name))
            .collect();
        for key in PARENT_SESSION_ENV {
            assert!(!names.contains(key), "{key} reached the tool");
        }
        assert!(names.contains(&"KEPT"), "{text}");
    }

    /// A tool started from a shell at launchd's 256 open files outgrows it at
    /// startup, however it was launched, so the supervisor raises the floor
    /// at the one place it spawns.
    #[cfg(unix)]
    #[test]
    fn the_spawned_tool_gets_at_least_ten_thousand_open_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = probe();
        command.current_dir(dir.path());
        let wrapped = with_open_files_floor(command);
        let Some(out) = run_from_soft_limit(&wrapped, 256) else {
            return;
        };
        let text = String::from_utf8(out.stdout).unwrap();
        let soft: u32 = text.split('|').next().unwrap().parse().unwrap();
        assert!(soft >= 10240, "{text}");
        // The tool sees its own arguments, environment, and exit code.
        assert!(
            text.ends_with("|tool name|a 'quoted' $HOME `arg`|yes|unset"),
            "{text}"
        );
        assert_eq!(out.status.code(), Some(7));
    }

    #[cfg(unix)]
    #[test]
    fn a_higher_open_file_limit_is_never_lowered() {
        let Some(out) = run_from_soft_limit(&with_open_files_floor(probe()), 20000) else {
            return;
        };
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.starts_with("20000|"), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn the_working_directory_survives_the_wrapper() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new("/bin/pwd");
        command.current_dir(dir.path());
        let out = with_open_files_floor(command).output().unwrap();
        let printed = std::fs::canonicalize(String::from_utf8(out.stdout).unwrap().trim()).unwrap();
        assert_eq!(printed, std::fs::canonicalize(dir.path()).unwrap());
    }
    /// A row as the launcher receives it, for `tool` in `cwd`.
    fn row(
        tool: &str,
        cwd: &std::path::Path,
        collect_traces: bool,
    ) -> crate::managed::sessions::SessionView {
        serde_json::from_value(json!({
            "id": uuid::Uuid::new_v4(), "tool": tool, "connection": "subscription",
            "account_id": uuid::Uuid::new_v4(), "account_label": "Personal",
            "selection_generation": 0, "project_label": "p", "cwd": cwd,
            "started_at": "2026-10-31T00:00:00Z", "state": "starting",
            "exit_code": null, "can_focus": false, "collect_traces": collect_traces,
        }))
        .unwrap()
    }

    /// Claude Code continues the root's conversation with `--resume
    /// <root>`, and is never also given `--session-id`; a fresh launch is
    /// named by its own row id. Nothing goes before Claude's options.
    #[test]
    fn claude_resumes_the_root_and_never_names_a_new_session() {
        use super::{Resume, claude_session_args, leading_args};
        let id = uuid::Uuid::new_v4();
        let root = uuid::Uuid::new_v4();
        let resume = Some(Resume {
            root,
            codex_thread: None,
        });
        let resumed = claude_session_args(id, resume);
        assert_eq!(resumed, ["--resume".to_owned(), root.to_string()]);
        assert!(!resumed.contains(&"--session-id".to_owned()));
        assert!(!resumed.contains(&id.to_string()));
        assert_eq!(
            claude_session_args(id, None),
            ["--session-id".to_owned(), id.to_string()]
        );
        assert!(leading_args(ToolId::Claude, resume).is_empty());
        assert!(leading_args(ToolId::Opencode, resume).is_empty());
        assert!(leading_args(ToolId::Codex, None).is_empty());
    }

    /// Codex continues with `resume <thread id>` straight after the
    /// executable, read from the root's binding, and keeps the launch's
    /// profile isolation and `-c` overrides after it. Without a recorded
    /// thread there is nothing to resume.
    #[cfg(unix)]
    #[test]
    fn codex_resumes_the_recorded_thread_with_the_isolation_overrides() {
        use super::{NativeAction, leading_args, resume_for};
        use crate::managed::{
            AccountId, AccountView, AuthState, ConnectionKind, profiles::NativeProfiles,
            session_binding,
        };
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().join("state")).unwrap();
        let cwd = home.path().join("project");
        std::fs::create_dir(&cwd).unwrap();
        let native = home.path().join("codex");
        std::fs::write(&native, b"#!/bin/sh\nexit 1\n").unwrap();
        let account = AccountView {
            id: AccountId(uuid::Uuid::new_v4()),
            tool: ToolId::Codex,
            connection: ConnectionKind::Subscription,
            label: "Work".into(),
            auth_state: AuthState::Ready,
            verified_at: None,
        };
        let root = uuid::Uuid::new_v4();
        assert_eq!(
            resume_for(&store, ToolId::Codex, Some(root)).err(),
            Some(ManagedError::NotFound),
            "no binding, nothing to resume"
        );
        let binding = session_binding::prepare(&store, root).unwrap();
        let thread = uuid::Uuid::new_v4();
        let transcript = std::fs::canonicalize(home.path())
            .unwrap()
            .join("rollout.jsonl");
        std::fs::write(&transcript, b"{}\n").unwrap();
        let nonce = std::fs::read_to_string(binding.directory().join("nonce")).unwrap();
        session_binding::record(
            binding.directory(),
            &nonce,
            std::io::Cursor::new(
                serde_json::to_vec(&json!({
                    "session_id": thread, "transcript_path": transcript,
                    "hook_event_name": "SessionStart",
                }))
                .unwrap(),
            ),
        )
        .unwrap();
        let resume = resume_for(&store, ToolId::Codex, Some(root)).unwrap();
        assert_eq!(resume.unwrap().codex_thread, Some(thread));
        assert_eq!(resume_for(&store, ToolId::Codex, None).unwrap(), None);

        let profiles = NativeProfiles::new(&store);
        let mut command = profiles
            .command(&account, &native, NativeAction::Run, &cwd)
            .unwrap();
        command.args(leading_args(ToolId::Codex, resume));
        command.args(["--model", "custom"]);
        near_ai_route(
            &mut command,
            ToolId::Codex,
            4321,
            "deepseek-ai/DeepSeek-V3.1",
            &listed(),
            Some(&home.path().join("catalog.json")),
        )
        .unwrap();
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[..2], ["resume".to_owned(), thread.to_string()]);
        assert_eq!(args[2..4], ["--model".to_owned(), "custom".to_owned()]);
        assert!(args.contains(&"model_provider=\"near_ai_managed\"".to_owned()));
        assert!(args.contains(&"web_search=\"disabled\"".to_owned()));
        assert_eq!(
            env_of(&command, "CODEX_HOME"),
            Some(Some(
                profiles
                    .profile_root(account.id)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            ))
        );
    }

    /// Every Codex coding launch records its thread id, collected or not,
    /// so it can be resumed. A resumed launch reads its root's binding and
    /// never prepares one; a sign-in and a Codex before 0.164 get none.
    #[cfg(unix)]
    #[test]
    fn codex_binding_is_configured_when_collection_is_off() {
        use super::{Resume, codex_binding};
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().join("state")).unwrap();
        let version = Some((0, 166, 0));
        let fresh = row("codex", home.path(), false);
        assert!(!fresh.collect_traces);
        let mut command = std::process::Command::new("codex");
        let binding = codex_binding(&store, &fresh, false, None, version, &mut command)
            .expect("bound without collection");
        assert!(binding.directory().ends_with(fresh.id.to_string()));
        assert!(
            command
                .get_args()
                .any(|arg| { arg.to_string_lossy().starts_with("hooks.SessionStart=") })
        );
        drop(binding);
        assert!(
            store
                .dir()
                .join(crate::managed::session_binding::DIRECTORY)
                .join(fresh.id.to_string())
                .is_dir(),
            "the binding outlives the launch"
        );

        let resumed = row("codex", home.path(), false);
        let mut command = std::process::Command::new("codex");
        let resume = Some(Resume {
            root: fresh.id,
            codex_thread: Some(uuid::Uuid::new_v4()),
        });
        assert!(codex_binding(&store, &resumed, false, resume, version, &mut command).is_none());
        assert_eq!(command.get_args().count(), 0);
        assert!(
            !store
                .dir()
                .join(crate::managed::session_binding::DIRECTORY)
                .join(resumed.id.to_string())
                .exists()
        );
        for (login, version) in [
            (true, Some((0, 166, 0))),
            (false, Some((0, 163, 0))),
            (false, None),
        ] {
            let other = row("codex", home.path(), false);
            let mut command = std::process::Command::new("codex");
            assert!(codex_binding(&store, &other, login, None, version, &mut command).is_none());
            assert_eq!(command.get_args().count(), 0);
        }
        let claude = row("claude", home.path(), false);
        let mut command = std::process::Command::new("claude");
        assert!(codex_binding(&store, &claude, false, None, version, &mut command).is_none());
    }

    /// A resumed OpenCode session reuses the root's per-session profile and
    /// `ses_<root>` id, and is never seeded through import again.
    #[cfg(unix)]
    #[test]
    fn opencode_resumes_the_roots_session_without_importing() {
        use super::{Resume, opencode_session};
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().join("state")).unwrap();
        let log = home.path().join("calls.log");
        let native = home.path().join("opencode");
        std::fs::write(
            &native,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\n[ \"$1\" = --version ] || exit 1\necho '1.19.0'\n",
                log.display()
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let account = signin::entry(ToolId::Opencode).unwrap();
        let resumed = row("opencode", home.path(), false);
        let root = uuid::Uuid::new_v4();
        let session = opencode_session(
            &store,
            &account,
            &native,
            &resumed,
            Some(Resume {
                root,
                codex_thread: None,
            }),
        )
        .unwrap();
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(!calls.contains("import"), "{calls}");
        let command = session
            .command_with_models(
                "deepseek-ai/DeepSeek-V3.1",
                &listed(),
                "http://127.0.0.1:4321/openai/v1",
                "managed-local-session",
            )
            .unwrap();
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let at = args.iter().position(|a| a == "--session").unwrap();
        assert_eq!(args[at + 1], format!("ses_{}", root.simple()));
        assert!(!args.contains(&format!("ses_{}", resumed.id.simple())));
        let profile = crate::managed::profiles::NativeProfiles::new(&store)
            .profile_root(account.id)
            .unwrap()
            .join(root.to_string());
        assert_eq!(
            env_of(&command, "HOME"),
            Some(Some(profile.to_string_lossy().into_owned())),
            "the root's per-session profile"
        );
    }
}

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
    let mut route = None;
    let opencode = if row.tool == super::ToolId::Opencode {
        let session =
            super::opencode::OpenCodeSession::new(store, &account, &native, row.id, &row.cwd)?;
        // A saved key names its model; the sign-in's proxy has pinned one.
        let model = || row.model.as_deref().ok_or(ManagedError::InvalidRequest);
        session.prepare()?;
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
                let proxy = super::route::ManagedRoute::start_selected(
                    store,
                    row.id,
                    SecretStore::new(store).get(account.id)?,
                    Some(model),
                )
                .await?;
                command = session.command(
                    model,
                    &format!("http://127.0.0.1:{}/openai/v1", proxy.port()),
                    "managed-local-session",
                )?;
                route = Some(proxy);
            }
        }
        Some(session)
    } else {
        None
    };
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
                )
                .await?;
                near_ai_route(&mut command, row.tool, proxy.port())?;
                route = Some(proxy);
            }
            // The daemon holds the sign-in and hosts this session's proxy. This
            // process learns its port and never the credential; no port, no run.
            ConnectionKind::NearAiSignIn => {
                near_ai_route(
                    &mut command,
                    row.tool,
                    context.route_port.ok_or(ManagedError::LaunchUnknown)?,
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
    #[cfg(not(any(unix, windows)))]
    let _ = signals;
    #[cfg(unix)]
    let mut shutdown_at: Option<std::time::Instant> = None;
    let mut child = tokio::process::Command::from(command)
        .kill_on_drop(false)
        .spawn()?;
    *spawned = true;
    let pid = child.id().ok_or(ManagedError::LaunchUnknown)?;
    let mut system = sysinfo::System::new();
    system.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
        true,
    );
    let identity = system
        .process(sysinfo::Pid::from_u32(pid))
        .map(|p| format!("{}:{}", pid, p.start_time()))
        .unwrap_or_else(|| format!("{}:{}", pid, uuid::Uuid::new_v4()));
    let report = |state, exit_code| LifecycleReport {
        collection: None,
        session_id: row.id,
        proof: context.proof.clone(),
        state,
        process_id: Some(pid),
        process_identity: Some(identity.clone()),
        exit_code,
    };
    let mut answer: anyhow::Result<Value> = call(
        store,
        "managed_session_report",
        json!(report(SessionState::Running, None)),
    );
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(5));
    let status = loop {
        // The daemon asked for a stop: this process owns the child, so it
        // signals it exactly as a terminal's own SIGTERM would, and the
        // escalation below kills it if it has not exited in time.
        #[cfg(unix)]
        if stop_asked(&answer) && shutdown_at.is_none() {
            forward_signal(pid, &identity, sysinfo::Signal::Term);
            shutdown_at = Some(std::time::Instant::now());
        }
        #[cfg(windows)]
        if stop_asked(&answer) {
            let _ = child.start_kill();
        }
        answer = Ok(Value::Null);
        #[cfg(unix)]
        tokio::select! {
            status = child.wait() => break status?,
            _ = signals.interrupt.recv() => {},
            _ = signals.hangup.recv() => { forward_signal(pid, &identity, sysinfo::Signal::Hangup); shutdown_at.get_or_insert_with(std::time::Instant::now); },
            _ = signals.terminate.recv() => { forward_signal(pid, &identity, sysinfo::Signal::Term); shutdown_at.get_or_insert_with(std::time::Instant::now); },
            _ = heartbeat.tick() => {
                if shutdown_at.is_some_and(|at| at.elapsed() >= std::time::Duration::from_secs(5)) { let _ = child.start_kill(); }
                answer = call(store, "managed_session_report", json!(report(SessionState::Running, None)));
            },
        }
        #[cfg(windows)]
        tokio::select! {
            status = child.wait() => break status?,
            _ = signals.interrupt.recv() => {},
            _ = signals.break_signal.recv() => {},
            _ = heartbeat.tick() => { answer = call(store, "managed_session_report", json!(report(SessionState::Running, None))); },
        }
        #[cfg(not(any(unix, windows)))]
        tokio::select! {
            status = child.wait() => break status?,
            _ = heartbeat.tick() => { let _: anyhow::Result<Value> = call(store, "managed_session_report", json!(report(SessionState::Running, None))); },
        }
    };
    #[cfg(unix)]
    let code = {
        use std::os::unix::process::ExitStatusExt;
        status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
    };
    #[cfg(not(unix))]
    let code = status.code().unwrap_or(1);
    let mut receipt = report(SessionState::Exited, Some(code));
    if row.collect_traces {
        receipt.collection = Some(
            match opencode
                .as_ref()
                .ok_or(ManagedError::InvalidRequest)?
                .collect(store)
            {
                Ok(()) => super::sessions::CollectionState::Collected,
                Err(_) => {
                    eprintln!(
                        "OpenCode exited, but its trace could not be collected. The local session is preserved."
                    );
                    super::sessions::CollectionState::Failed
                }
            },
        );
    }
    let persisted = super::sessions::persist_exit(store, &receipt);
    let final_report: anyhow::Result<Value> = call(store, "managed_session_report", json!(receipt));
    if final_report.is_ok() {
        super::sessions::clear_exit(store, row.id);
    }
    if final_report.is_err() && persisted.is_err() {
        eprintln!(
            "Native session exited; {} status is unknown until the daemon reconnects.",
            crate::brand::APP_NAME
        );
    }
    if let Some(route) = route {
        route.shutdown().await;
    }
    if login {
        let _: anyhow::Result<Value> = call(
            store,
            "managed_account_verify",
            json!({"account_id":row.account_id}),
        );
    }
    Ok(code)
}

/// OpenCode on the NEAR AI sign-in: the daemon hosts this session's proxy
/// and says its loopback port and the model it pinned. This process never
/// sees the credential; without the port or the model, no run.
fn opencode_sign_in_command(
    session: &super::opencode::OpenCodeSession,
    context: &LaunchContext,
) -> Result<std::process::Command, ManagedError> {
    let port = context.route_port.ok_or(ManagedError::LaunchUnknown)?;
    let model = context
        .route_model
        .as_deref()
        .ok_or(ManagedError::LaunchUnknown)?;
    session.command(
        model,
        &format!("http://127.0.0.1:{port}/openai/v1"),
        "managed-local-session",
    )
}

/// Point the tool at a session's local NEAR AI proxy. The tool gets the
/// loopback URL and a placeholder token, never a credential.
fn near_ai_route(
    command: &mut std::process::Command,
    tool: super::ToolId,
    port: u16,
) -> Result<(), ManagedError> {
    let base = format!("http://127.0.0.1:{port}");
    match tool {
        super::ToolId::Opencode => return Err(ManagedError::InvalidRequest),
        super::ToolId::Claude => {
            command
                .env("ANTHROPIC_BASE_URL", format!("{base}/anthropic"))
                .env("ANTHROPIC_AUTH_TOKEN", "managed-local-session");
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
                ]);
        }
    }
    Ok(())
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

struct TerminalSignals {
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
    fn new() -> std::io::Result<Self> {
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
    use super::{opencode_sign_in_command, stop_asked};
    use crate::config::ConfigStore;
    use crate::managed::{
        ConnectionKind, ManagedError, ToolId, opencode::OpenCodeSession, sessions::LaunchContext,
        signin,
    };
    use serde_json::json;

    /// OpenCode on the NEAR AI sign-in runs against the daemon's proxy for
    /// this session, on the model that proxy pinned, with a placeholder
    /// key. Without the port or the model there is no command: no run.
    #[test]
    fn opencode_on_the_sign_in_uses_the_route_port_and_model_or_does_not_run() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let native = home.path().join("opencode");
        std::fs::write(&native, b"never executed").unwrap();
        let account = signin::entry(ToolId::Opencode).unwrap();
        let id = uuid::Uuid::new_v4();
        let session = OpenCodeSession::new(&store, &account, &native, id, home.path()).unwrap();
        let context = |port: Option<u16>, model: Option<&str>| -> LaunchContext {
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
            }))
            .unwrap()
        };
        let command = opencode_sign_in_command(
            &session,
            &context(Some(4321), Some("deepseek-ai/DeepSeek-V3.1")),
        )
        .unwrap();
        assert_eq!(
            context(None, None).session.connection,
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
            config["provider"]["trace-managed"]["options"]["baseURL"],
            "http://127.0.0.1:4321/openai/v1"
        );
        assert_eq!(
            env("TRACE_COMMONS_OPENCODE_KEY").as_deref(),
            Some("managed-local-session")
        );
        for (port, model) in [
            (None, Some("deepseek-ai/DeepSeek-V3.1")),
            (Some(4321), None),
            (None, None),
        ] {
            assert_eq!(
                opencode_sign_in_command(&session, &context(port, model)).err(),
                Some(ManagedError::LaunchUnknown),
                "{port:?} {model:?}"
            );
        }
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
}

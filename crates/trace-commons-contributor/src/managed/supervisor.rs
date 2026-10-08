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
    match row.connection {
        ConnectionKind::Subscription => {}
        ConnectionKind::ApiKey => {
            let key = SecretStore::new(store).get(account.id)?;
            match row.tool {
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
            let base = format!("http://127.0.0.1:{}", proxy.port());
            match row.tool {
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
            route = Some(proxy);
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
        session_id: row.id,
        proof: context.proof.clone(),
        state,
        process_id: Some(pid),
        process_identity: Some(identity.clone()),
        exit_code,
    };
    let _: anyhow::Result<Value> = call(
        store,
        "managed_session_report",
        json!(report(SessionState::Running, None)),
    );
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(5));
    let status = loop {
        #[cfg(unix)]
        tokio::select! {
            status = child.wait() => break status?,
            _ = signals.interrupt.recv() => {},
            _ = signals.hangup.recv() => { forward_signal(pid, &identity, sysinfo::Signal::Hangup); shutdown_at.get_or_insert_with(std::time::Instant::now); },
            _ = signals.terminate.recv() => { forward_signal(pid, &identity, sysinfo::Signal::Term); shutdown_at.get_or_insert_with(std::time::Instant::now); },
            _ = heartbeat.tick() => {
                if shutdown_at.is_some_and(|at| at.elapsed() >= std::time::Duration::from_secs(5)) { let _ = child.start_kill(); }
                let _: anyhow::Result<Value> = call(store, "managed_session_report", json!(report(SessionState::Running, None)));
            },
        }
        #[cfg(windows)]
        tokio::select! {
            status = child.wait() => break status?,
            _ = signals.interrupt.recv() => {},
            _ = signals.break_signal.recv() => {},
            _ = heartbeat.tick() => { let _: anyhow::Result<Value> = call(store, "managed_session_report", json!(report(SessionState::Running, None))); },
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
    let receipt = report(SessionState::Exited, Some(code));
    let persisted = super::sessions::persist_exit(store, &receipt);
    let final_report: anyhow::Result<Value> = call(store, "managed_session_report", json!(receipt));
    if final_report.is_ok() {
        super::sessions::clear_exit(store, row.id);
    }
    if final_report.is_err() && persisted.is_err() {
        eprintln!(
            "Native session exited; Trace Commons status is unknown until the daemon reconnects."
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

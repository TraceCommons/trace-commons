//! Enrolling this device from the terminal with a NEAR AI sign-in, with no
//! invite.
//!
//! This is the same two-step sequence the native macOS first run takes, over
//! the same daemon methods, so the CLI and the app share one sign-in and one
//! enrollment:
//!
//! 1. `near_ai_credential_status`, then -- only when no NEAR AI session is
//!    kept -- `near_ai_credential_start`, opening the browser on the address
//!    it hands back and polling `near_ai_credential_status` for that attempt.
//! 2. `near_ai_account_enroll` naming no commons, which enrolls against the
//!    native account's commons, then `set_consent_scopes` with the answers
//!    `login` collected.
//!
//! The methods go to the running daemon over its socket when there is one,
//! and are otherwise answered in this process from the same handlers
//! (`handle_local`), against the same configuration directory and credential
//! store. Nothing here re-implements the ceremony or the enrollment.
//!
//! On macOS the CLI bundled in the app cannot answer them itself: the NEAR
//! AI session lives in the app's keychain access group, and only the app is
//! signed with it. With no daemon running, `login` starts the app in the
//! background (no window, not brought forward) and waits for its daemon, or
//! opens the app's first run when this Mac has not been set up, since the
//! app starts no daemon until first run has the session folders.
//!
//! # Secrets
//!
//! The refresh token and NEAR AI's short-lived JWT never leave the daemon's
//! handlers; neither travels over IPC. What does travel back -- refusal
//! labels, the enrolled account id -- is not printed verbatim: refusals map
//! to fixed sentences and the account id is not shown at all.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};

use crate::commands::DaemonCaller;
use crate::config::ConfigStore;
use crate::daemon::ipc::Response;

/// Printed before the browser is asked to open.
pub(crate) const OPENING: &str = "Opening near.ai in your browser to sign in.";
/// Printed with the address, before the browser is asked to open, so a failed
/// or hung `open` never hides it.
pub(crate) const OPEN_FALLBACK_PREFIX: &str = "If it doesn't open, go to:";
/// Printed with the address when no browser is opened here (`--no-browser`).
pub(crate) const OPEN_PROMPT: &str = "Open this address in your browser to sign in:";
/// Printed when no browser on this machine can reach the person: over SSH,
/// or on a Linux machine with no display. The sign-in finishes on a loopback
/// address, so the browser on their own computer needs the port forwarded.
pub(crate) const REMOTE_INTRO: &str =
    "No browser on this machine can reach you, so sign in from your own computer:";
/// Printed first when no browser here can reach the person: a server is
/// linked more simply with a code from a signed-in computer.
pub(crate) const CODE_HINT: &str = "On a server? From a computer that's signed in, run `trace-commons-contributor devices add`, then run `trace-commons-contributor login --code <code>` here.";
/// Stands in for the host in the printed `ssh` command.
pub(crate) const REMOTE_HOST_PLACEHOLDER: &str = "<this-machine>";
/// The invite route, named wherever the NEAR AI route is taken by default.
pub(crate) const INVITE_HINT: &str =
    "Have an invite? Run `trace-commons-contributor login --invite <link>`.";
/// Signed in and enrolled.
pub(crate) const DONE: &str = "Signed in. This device is enrolled.";
/// `submit` on an unenrolled machine with nobody at a terminal to sign in.
pub(crate) const UNENROLLED_HINT: &str = "This device isn't enrolled. Run `trace-commons-contributor login` to sign in with NEAR AI, or pass an invite with `--invite <link>`.";
/// The browser sign-in ended without a NEAR AI session.
pub(crate) const SIGN_IN_UNFINISHED: &str =
    "Sign-in didn't finish. Run `trace-commons-contributor login` to try again.";
/// The browser sign-in could not be started.
/// A commons without the NEAR AI login path. The app's line for this offers
/// a wallet, which the CLI has no route to.
pub(crate) const COMMONS_UNSUPPORTED: &str =
    "This commons doesn't offer NEAR AI sign-in. Use an invite instead.";
/// `login --json` with neither a grant nor an invite.
pub(crate) const JSON_REFUSED: &str = "--json can't start a browser sign-in. Pass --invite <link>, or run `trace-commons-contributor login` without --json.";
pub(crate) const SIGN_IN_UNAVAILABLE: &str = "Couldn't start NEAR AI sign-in. Try again shortly.";
/// Enrolled, and the consent answers could not be recorded.
pub(crate) const CONSENT_NOT_SAVED: &str = "Enrolled, but your consent choices weren't saved. Run `trace-commons-contributor consent scopes` to choose them.";
/// `--allowed-hosts` on the NEAR AI route, which cannot carry it: the
/// sign-in's host list comes from TRACE_COMMONS_ALLOWED_HOSTS in the
/// environment of whichever process answers it (a running daemon, or this
/// one).
pub(crate) const ALLOWED_HOSTS_REFUSED: &str =
    "--allowed-hosts works only with --grant or --invite.";

/// How long to wait on the browser, as the macOS first run does.
pub(crate) const BROWSER_WAIT: Duration = Duration::from_secs(600);
/// How often to ask whether the browser sign-in has finished.
const POLL_EVERY: Duration = Duration::from_secs(1);

/// The consent answers `login` collected before anything reached the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Consent {
    pub scopes: Vec<String>,
    /// Whether the person chose them (an explicit `--scopes`, or answers to
    /// the menu) rather than having the default answers taken for them.
    pub chosen: bool,
}

impl Consent {
    /// The `set_consent_scopes` list. Default answers send an empty list:
    /// the daemon saves the floor scope for it and records no choice, which
    /// is what an invite enrollment with the default answers records.
    pub(crate) fn request_scopes(&self) -> Vec<String> {
        if self.chosen {
            self.scopes.clone()
        } else {
            Vec::new()
        }
    }
}

/// Where the sign-in's browser is, decided once from the flags and the
/// environment before anything is printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BrowserPlan {
    /// Ask the platform opener to open the address here.
    pub open: bool,
    /// No browser here reaches the person: print how to forward the loopback
    /// port from their own computer.
    pub remote: bool,
}

impl BrowserPlan {
    /// Open a browser here and print the address in case it doesn't.
    #[cfg(test)]
    pub(crate) const LOCAL: Self = Self {
        open: true,
        remote: false,
    };

    /// Decide from `--no-browser` and the process environment.
    pub(crate) fn detect(no_browser: bool) -> Self {
        Self::from_env(
            no_browser,
            cfg!(any(target_os = "macos", target_os = "windows")),
            &|name| std::env::var_os(name),
        )
    }

    /// The decision, with the environment passed in so it is testable.
    ///
    /// - An SSH session (`SSH_CONNECTION`, `SSH_CLIENT` or `SSH_TTY`) is
    ///   remote: a browser opened here, if any, is not in front of the
    ///   person. On macOS `open` would put it on the console's screen.
    /// - Off macOS and Windows, no `DISPLAY` and no `WAYLAND_DISPLAY` means
    ///   no graphical session, so nothing can open; `xdg-open` would only
    ///   list the text browsers it could not find.
    /// - Off macOS and Windows, `BROWSER` names an opener the person set up,
    ///   which can work over SSH (an editor's remote terminal forwards it),
    ///   and `xdg-open` falls back to it, so it is still tried; the
    ///   forwarding steps are printed either way. On macOS and Windows the
    ///   opener (`open`, `start`) never reads `BROWSER`, so it counts for
    ///   nothing there.
    pub(crate) fn from_env(
        no_browser: bool,
        graphical_os: bool,
        env: &dyn Fn(&str) -> Option<std::ffi::OsString>,
    ) -> Self {
        let set = |name: &str| env(name).is_some_and(|value| !value.is_empty());
        let ssh = set("SSH_CONNECTION") || set("SSH_CLIENT") || set("SSH_TTY");
        let display = graphical_os || set("DISPLAY") || set("WAYLAND_DISPLAY");
        let remote = ssh || !display;
        Self {
            open: !no_browser && ((!graphical_os && set("BROWSER")) || !remote),
            remote,
        }
    }
}

/// The loopback port a sign-in finishes on: the address's own port when it
/// is `http://127.0.0.1:<port>/...` (the NEAR wallet page), or the port of
/// its `frontend_callback` (an OAuth provider, which redirects there).
fn loopback_port(url: &str) -> Option<u16> {
    fn own_port(parsed: &url::Url) -> Option<u16> {
        if parsed.scheme() != "http" || parsed.host_str() != Some("127.0.0.1") {
            return None;
        }
        parsed.port()
    }
    let parsed = url::Url::parse(url).ok()?;
    own_port(&parsed).or_else(|| {
        let callback = parsed
            .query_pairs()
            .find(|(key, _)| key == "frontend_callback")?
            .1;
        own_port(&url::Url::parse(&callback).ok()?)
    })
}

/// Print where to finish the sign-in, as `plan` says.
pub(crate) fn print_sign_in_address(
    out: &mut dyn Write,
    plan: BrowserPlan,
    url: &str,
) -> std::io::Result<()> {
    match (plan.remote, loopback_port(url)) {
        (true, Some(port)) => {
            writeln!(out, "{CODE_HINT}")?;
            writeln!(out, "{REMOTE_INTRO}")?;
            writeln!(
                out,
                "  1. In a terminal on your computer, forward the sign-in port and leave it running:"
            )?;
            writeln!(
                out,
                "       ssh -N -o ExitOnForwardFailure=yes -L {port}:127.0.0.1:{port} {REMOTE_HOST_PLACEHOLDER}"
            )?;
            writeln!(
                out,
                "     Put the host you SSH to in place of {REMOTE_HOST_PLACEHOLDER}."
            )?;
            writeln!(
                out,
                "  2. Open this address in your computer's browser within 5 minutes:"
            )?;
            writeln!(out, "       {url}")?;
        }
        _ if plan.open => {
            writeln!(out, "{OPENING}")?;
            writeln!(out, "{OPEN_FALLBACK_PREFIX} {url}")?;
        }
        _ => {
            writeln!(out, "{OPEN_PROMPT}")?;
            writeln!(out, "  {url}")?;
        }
    }
    Ok(())
}

/// Everything the flow touches outside its own logic, so a test can stand in
/// for the daemon, the browser and the clock.
pub(crate) struct Ports<'a> {
    pub call: DaemonCaller<'a>,
    pub browser: BrowserPlan,
    pub open_browser: &'a mut dyn FnMut(&str) -> bool,
    pub sleep: &'a mut dyn FnMut(Duration),
    pub out: &'a mut dyn Write,
}

/// What one status poll says about the sign-in being waited on. The same
/// verdict the macOS first run draws (`NearAILoginPoll`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Poll {
    SignedIn,
    Waiting,
    Ended,
}

fn poll_verdict(status: Option<&serde_json::Value>) -> Poll {
    let Some(status) = status else {
        return Poll::Waiting;
    };
    if status.get("session_state").and_then(|v| v.as_str()) == Some(LABEL_PRESENT) {
        return Poll::SignedIn;
    }
    match status.get("attempt_status").and_then(|v| v.as_str()) {
        None | Some(WAITING_FOR_BROWSER) => Poll::Waiting,
        Some(_) => Poll::Ended,
    }
}

const LABEL_PRESENT: &str = crate::daemon::nearai_credential::LABEL_CREDENTIAL_PRESENT;
const WAITING_FOR_BROWSER: &str = "waiting_for_browser";

/// The result of a call the daemon answered, or `None` for a refusal or a
/// call that could not be made. Never the refusal's text.
fn answered(response: Result<Response>) -> Option<serde_json::Value> {
    match response {
        Ok(Response {
            error: None,
            result: Some(result),
            ..
        }) => Some(result),
        _ => None,
    }
}

/// Sign in to NEAR AI in the browser, skipping it when a session is
/// already kept. `hint` is printed with the address.
fn sign_in(ports: &mut Ports<'_>, hint: Option<&str>) -> Result<()> {
    let Ports {
        call,
        browser,
        open_browser,
        sleep,
        out,
    } = ports;

    let kept = answered(call("near_ai_credential_status", serde_json::json!({})));
    if poll_verdict(kept.as_ref()) != Poll::SignedIn {
        let started = answered(call("near_ai_credential_start", serde_json::json!({})))
            .ok_or_else(|| anyhow!(SIGN_IN_UNAVAILABLE))?;
        let (Some(attempt_id), Some(url)) = (
            started.get("attempt_id").and_then(|v| v.as_str()),
            started.get("browser_url").and_then(|v| v.as_str()),
        ) else {
            bail!(SIGN_IN_UNAVAILABLE)
        };
        print_sign_in_address(&mut **out, *browser, url)?;
        if let Some(hint) = hint {
            writeln!(out, "{hint}")?;
        }
        out.flush()?;
        if browser.open {
            // Best effort: the address is already on screen.
            let _ = open_browser(url);
        }

        let attempt = serde_json::json!({ "attempt_id": attempt_id });
        let mut waited = Duration::ZERO;
        loop {
            if waited >= BROWSER_WAIT {
                // Free the loopback port rather than leave the attempt
                // waiting on a browser nobody is using.
                let _ = call("near_ai_credential_cancel", attempt.clone());
                bail!(SIGN_IN_UNFINISHED)
            }
            sleep(POLL_EVERY);
            waited += POLL_EVERY;
            let status = answered(call("near_ai_credential_status", attempt.clone()));
            match poll_verdict(status.as_ref()) {
                Poll::SignedIn => break,
                Poll::Waiting => continue,
                Poll::Ended => bail!(SIGN_IN_UNFINISHED),
            }
        }
    }
    Ok(())
}

/// Sign in to NEAR AI (skipping the browser when a session is already kept)
/// and enroll this device on the commons, then record the consent answers.
pub(crate) fn sign_in_and_enroll(mut ports: Ports<'_>, consent: &Consent) -> Result<()> {
    sign_in(&mut ports, Some(INVITE_HINT))?;
    let Ports { call, out, .. } = ports;

    // No commons and no account are named: the daemon enrolls against the
    // native account's commons with whatever account NEAR AI's own token
    // resolves to, as the app's first run without an invite does.
    let enrolled = call("near_ai_account_enroll", serde_json::json!({}))?;
    if let Some(error) = enrolled.error {
        let label = error.message.as_str();
        if label == crate::private_inference_copy::NEAR_AI_ENROLL_COMMONS_UNSUPPORTED {
            bail!("{COMMONS_UNSUPPORTED}\n{INVITE_HINT}")
        }
        bail!(
            "{}",
            crate::private_inference_copy::near_ai_enroll_line(label)
        )
    }

    let saved = answered(call(
        "set_consent_scopes",
        serde_json::json!({ "scopes": consent.request_scopes() }),
    ));
    if saved.is_none() {
        bail!(CONSENT_NOT_SAVED)
    }
    writeln!(out, "{DONE}")?;
    Ok(())
}

/// [`sign_in_and_enroll`] against this store: over the running daemon's
/// socket when one answers, otherwise in this process with one daemon state
/// loaded for the whole flow. A process that cannot keep the session
/// hands the sign-in to the app instead ([`hand_off_to_app`]).
///
/// Which of the two is decided once. A per-call fallback would load the
/// daemon state (and read the OS credential store) on every poll, and would
/// ask a fresh in-process state about an attempt a running daemon holds.
pub(crate) fn run(store: &ConfigStore, consent: &Consent, no_browser: bool) -> Result<()> {
    use crate::daemon::client;
    use crate::daemon::ipc::{DaemonShared, handle_local};

    let running = client::is_running(store) || hand_off_to_app(store)?;
    let mut local: Option<DaemonShared> = None;
    let mut call = |method: &str, params: serde_json::Value| -> Result<Response> {
        if running {
            // A daemon that stopped part-way through holds the attempt this
            // process was waiting on; there is nothing to fall back to.
            return client::try_call(store, method, &params)?
                .ok_or_else(|| anyhow!(SIGN_IN_UNFINISHED));
        }
        let shared = match local.as_mut() {
            Some(shared) => shared,
            None => local.insert(DaemonShared::load(ConfigStore::open(
                store.dir().to_path_buf(),
            )?)?),
        };
        Ok(handle_local(shared, method, params))
    };
    let mut open = |url: &str| crate::account_auth::try_open_browser(url);
    let mut sleep = |d: Duration| std::thread::sleep(d);
    let mut stdout = std::io::stdout();
    sign_in_and_enroll(
        Ports {
            call: &mut call,
            browser: BrowserPlan::detect(no_browser),
            open_browser: &mut open,
            sleep: &mut sleep,
            out: &mut stdout,
        },
        consent,
    )
}

/// `launch` without a NEAR AI sign-in, after the person agreed to sign in.
pub(crate) const LAUNCH_SIGN_IN_UNFINISHED: &str =
    "Sign-in didn't finish. Run the same command to try again.";
/// Printed once `launch` has a NEAR AI sign-in.
pub(crate) const LAUNCH_SIGNED_IN: &str = "Signed in to NEAR AI.";

/// Sign in to NEAR AI for a managed launch: the browser step of [`run`],
/// without enrolling. Sessions run on this sign-in whether or not the
/// device contributes.
pub(crate) fn sign_in_for_launch(ports: Ports<'_>) -> Result<()> {
    let mut ports = ports;
    sign_in(&mut ports, None).map_err(|error| {
        if error.to_string() == SIGN_IN_UNFINISHED {
            anyhow!(LAUNCH_SIGN_IN_UNFINISHED)
        } else {
            error
        }
    })?;
    writeln!(ports.out, "{LAUNCH_SIGNED_IN}")?;
    Ok(())
}

/// [`sign_in_for_launch`] over the running daemon's socket. A managed
/// launch needs the daemon, so there is no in-process fallback.
pub(crate) fn run_for_launch(store: &ConfigStore) -> Result<()> {
    use crate::daemon::client;
    let mut call = |method: &str, params: serde_json::Value| -> Result<Response> {
        client::try_call(store, method, &params)?.ok_or_else(|| anyhow!(SIGN_IN_UNFINISHED))
    };
    let mut open = |url: &str| crate::account_auth::try_open_browser(url);
    let mut sleep = |d: Duration| std::thread::sleep(d);
    let mut stdout = std::io::stdout();
    sign_in_for_launch(Ports {
        call: &mut call,
        browser: BrowserPlan::detect(false),
        open_browser: &mut open,
        sleep: &mut sleep,
        out: &mut stdout,
    })
}

/// The argument the app reads as "started for the CLI": it opens no window
/// at launch, first run included (`LaunchRouting.isBackgroundLaunch`).
pub(crate) const BACKGROUND_LAUNCH_ARG: &str = "--background-launch";
/// How often `login` asks whether the launched app's daemon answers.
const DAEMON_POLL: Duration = Duration::from_millis(500);
/// How long it waits for that daemon before saying so.
const DAEMON_WAIT: Duration = Duration::from_secs(30);
/// No app around this CLI to hand the sign-in to.
pub(crate) const OPEN_APP: &str = "Open the Near.ai app, then run this command again.";

/// Who answers `login`'s daemon methods.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    /// The daemon already running for this store.
    Daemon,
    /// This process, which can keep the session itself.
    InProcess,
    /// The app around this CLI, started in the background.
    LaunchApp(PathBuf),
    /// The app around this CLI, opened on its first run: it starts no
    /// daemon until first run has the session folders.
    OpenSetup(PathBuf),
    /// The app around this CLI is already open and its daemon is not
    /// answering; launching it again would only re-activate it.
    AppNotAnswering(PathBuf),
    /// Nobody: the person opens the app.
    OpenApp,
}

/// `kept` is whether this process can keep a NEAR AI session
/// (`store_is_reachable`); `roots_declared` whether the app would start its
/// daemon; `app` the bundle this CLI runs from; `app_open` whether that
/// app is already running.
fn route(
    running: bool,
    kept: bool,
    roots_declared: bool,
    app: Option<PathBuf>,
    app_open: bool,
) -> Route {
    if running {
        return Route::Daemon;
    }
    if kept {
        return Route::InProcess;
    }
    match app {
        None => Route::OpenApp,
        Some(app) if !roots_declared => Route::OpenSetup(app),
        Some(app) if app_open => Route::AppNotAnswering(app),
        Some(app) => Route::LaunchApp(app),
    }
}

/// The `.app` this executable is bundled in (`X.app/Contents/MacOS/near-ai`),
/// or `None` for a CLI installed anywhere else.
fn app_bundle(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    let bundled = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app";
    bundled.then(|| app.to_path_buf())
}

/// Whether `ps -axo command=` output lists `exe`, the app's own executable,
/// as a process: the path alone or followed by its arguments.
fn app_running_in(ps: &str, exe: &Path) -> bool {
    let exe = exe.to_string_lossy();
    ps.lines().any(|line| {
        line.trim_start()
            .strip_prefix(exe.as_ref())
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
    })
}

/// Whether the app at `app` is running, by its `CFBundleExecutable`. A
/// failure to tell reads as not running: the launch then waits its bound
/// and says so.
fn app_is_open(app: &Path) -> bool {
    let plist = app.join("Contents/Info.plist");
    let exe = std::process::Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleExecutable", "raw", "-o", "-"])
        .arg(&plist)
        .output();
    let Ok(exe) = exe else { return false };
    let name = String::from_utf8_lossy(&exe.stdout).trim().to_string();
    if !exe.status.success() || name.is_empty() {
        return false;
    }
    let Ok(ps) = std::process::Command::new("/bin/ps")
        .args(["-axo", "command="])
        .output()
    else {
        return false;
    };
    app_running_in(
        &String::from_utf8_lossy(&ps.stdout),
        &app.join("Contents/MacOS").join(name),
    )
}

/// The store to hand the app, or `None` when it is the platform default,
/// which the app resolves itself. `--config-dir` and the override variable
/// both choose a store the app would not, so neither is special-cased.
fn forwarded_state_dir<'a>(store_dir: &'a Path, default: Option<&Path>) -> Option<&'a Path> {
    (default != Some(store_dir)).then_some(store_dir)
}

/// The app's name as Finder shows it: "Near.ai", "Near.ai Staging".
fn app_name(app: &Path) -> String {
    app.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Near.ai".to_string())
}

/// `open` arguments that start the app without bringing it forward (`-g`)
/// and, through the app's own argument, without a window. Not `-j`: an app
/// launched hidden can hold back the windows it opens later.
fn background_launch_args(app: &Path, state_dir: Option<&Path>) -> Vec<String> {
    let mut args = vec!["-g".to_string()];
    args.extend(setup_launch_args(app, state_dir));
    args.extend(["--args".to_string(), BACKGROUND_LAUNCH_ARG.to_string()]);
    args
}

/// `open` arguments that open the app as a click does, on its first run.
/// `state_dir` is the store when it is not the default
/// ([`forwarded_state_dir`]): `open` does not pass the terminal's
/// environment to the app.
fn setup_launch_args(app: &Path, state_dir: Option<&Path>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(dir) = state_dir {
        args.push("--env".to_string());
        args.push(format!(
            "{}={}",
            crate::channel::STATE_DIR_ENV,
            dir.to_string_lossy()
        ));
    }
    args.push(app.to_string_lossy().into_owned());
    args
}

fn launching_line(app: &Path) -> String {
    format!("Starting {} in the background to sign in.", app_name(app))
}

fn setup_line(app: &Path) -> String {
    format!(
        "Finish setting up in {}, which is now open. Sign-in is part of setup.",
        app_name(app)
    )
}

fn not_answering_line(app: &Path) -> String {
    let name = app_name(app);
    format!("{name} is open, but its background service isn't answering. Open {name} to see why.")
}

fn no_daemon_line(app: &Path) -> String {
    let name = app_name(app);
    format!("{name} started, but its background service didn't. Open {name} to see why.")
}

/// Polls `answers` until it is true or [`DAEMON_WAIT`] has passed.
fn wait_for_daemon(answers: &mut dyn FnMut() -> bool, sleep: &mut dyn FnMut(Duration)) -> bool {
    let mut waited = Duration::ZERO;
    loop {
        if answers() {
            return true;
        }
        if waited >= DAEMON_WAIT {
            return false;
        }
        sleep(DAEMON_POLL);
        waited += DAEMON_POLL;
    }
}

fn open_app(args: &[String]) -> Result<()> {
    match std::process::Command::new("/usr/bin/open")
        .args(args)
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        _ => bail!(OPEN_APP),
    }
}

/// With no daemon running: `Ok(true)` once the app's daemon answers for this
/// store, `Ok(false)` to sign in in this process as before. Only on macOS,
/// where the bundled CLI has no keychain entitlement and so could only fail
/// at `near_ai_credential_start` (`near_ai_credential_storage_unentitled`).
fn hand_off_to_app(store: &ConfigStore) -> Result<bool> {
    if !cfg!(target_os = "macos") {
        return Ok(false);
    }
    let kept = crate::daemon::cloud_credential_lifecycle::store_is_reachable(store).is_ok();
    // Unreadable settings are treated as not set up: opening the app shows
    // the person what it makes of them.
    let roots_declared = crate::daemon::settings::DaemonSettings::load(store)
        .map(|settings| crate::daemon::settings::roots_declared(&settings))
        .unwrap_or(false);
    let app = std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .ok()
        .and_then(|exe| app_bundle(&exe));
    let default = crate::config::platform_default_dir().ok();
    let state_dir = forwarded_state_dir(store.dir(), default.as_deref());
    let app_open = app.as_deref().is_some_and(app_is_open);
    match route(false, kept, roots_declared, app, app_open) {
        Route::Daemon | Route::InProcess => Ok(false),
        Route::OpenApp => bail!(OPEN_APP),
        Route::AppNotAnswering(app) => bail!(not_answering_line(&app)),
        Route::OpenSetup(app) => {
            open_app(&setup_launch_args(&app, state_dir))?;
            bail!(setup_line(&app))
        }
        Route::LaunchApp(app) => {
            println!("{}", launching_line(&app));
            open_app(&background_launch_args(&app, state_dir))?;
            let mut answers = || crate::daemon::client::is_running(store);
            let mut sleep = |d: Duration| std::thread::sleep(d);
            if wait_for_daemon(&mut answers, &mut sleep) {
                Ok(true)
            } else {
                bail!(no_daemon_line(&app))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ipc::IpcError;
    use serde_json::{Value, json};

    fn ok(result: Value) -> Result<Response> {
        Ok(Response {
            id: 0,
            result: Some(result),
            error: None,
        })
    }

    fn err(label: &str) -> Result<Response> {
        Ok(Response {
            id: 0,
            result: None,
            error: Some(IpcError {
                code: "unavailable".into(),
                message: label.into(),
            }),
        })
    }

    const URL: &str = "http://127.0.0.1:49152/near?state=abc";

    /// A scripted daemon: each method answers from its own queue, the last
    /// answer repeating. Records every call.
    struct Script {
        answers: Vec<(&'static str, Vec<Result<Response>>)>,
        calls: Vec<(String, Value)>,
    }

    impl Script {
        fn new(answers: Vec<(&'static str, Vec<Result<Response>>)>) -> Self {
            Self {
                answers,
                calls: Vec::new(),
            }
        }

        fn answer(&mut self, method: &str, params: Value) -> Result<Response> {
            self.calls.push((method.to_string(), params));
            let queue = self
                .answers
                .iter_mut()
                .find(|(m, _)| *m == method)
                .map(|(_, q)| q)
                .unwrap_or_else(|| panic!("unexpected daemon call {method}"));
            if queue.len() > 1 {
                queue.remove(0)
            } else {
                match &queue[0] {
                    Ok(r) => Ok(r.clone()),
                    Err(e) => Err(anyhow!("{e}")),
                }
            }
        }

        fn methods(&self) -> Vec<&str> {
            self.calls.iter().map(|(m, _)| m.as_str()).collect()
        }
    }

    struct Outcome {
        result: Result<()>,
        out: String,
        opened: Vec<(String, usize)>,
        script: Script,
    }

    fn drive(mut script: Script, consent: &Consent) -> Outcome {
        let mut out = Vec::new();
        let mut opened = Vec::new();
        let result = {
            let out_ref = &mut out;
            let mut caller = |m: &str, p: Value| script.answer(m, p);
            let mut open = |url: &str| {
                opened.push(url.to_string());
                true
            };
            let mut sleep = |_d: Duration| {};
            let ports = Ports {
                call: &mut caller,
                browser: BrowserPlan::LOCAL,
                open_browser: &mut open,
                sleep: &mut sleep,
                out: out_ref,
            };
            sign_in_and_enroll(ports, consent)
        };
        let out = String::from_utf8(out).unwrap();
        // Where in the output each open happened is not observable through
        // the closure above without sharing the buffer; the URL-before-open
        // order is checked in its own test.
        let opened = opened.into_iter().map(|u| (u, 0)).collect();
        Outcome {
            result,
            out,
            opened,
            script,
        }
    }

    fn floor() -> Consent {
        Consent {
            scopes: vec!["debugging_evaluation".into()],
            chosen: false,
        }
    }

    fn enrolled() -> Result<Response> {
        ok(json!({
            "enrolled": true,
            "tenant_id": format!("nearai-{}", "ab".repeat(32)),
            "account_id": "acct-7f3e",
            "device_key_id": "sha256:00",
        }))
    }

    fn saved_scopes() -> Result<Response> {
        ok(json!({"consent_scopes": ["debugging_evaluation"]}))
    }

    #[test]
    fn a_kept_session_skips_the_browser_and_enrolls() {
        let script = Script::new(vec![
            (
                "near_ai_credential_status",
                vec![ok(json!({"state": "present", "session_state": "present"}))],
            ),
            ("near_ai_account_enroll", vec![enrolled()]),
            ("set_consent_scopes", vec![saved_scopes()]),
        ]);
        let o = drive(script, &floor());
        o.result.unwrap();
        assert_eq!(
            o.script.methods(),
            [
                "near_ai_credential_status",
                "near_ai_account_enroll",
                "set_consent_scopes"
            ]
        );
        assert!(o.opened.is_empty(), "no browser for a kept session");
        assert!(o.out.contains(DONE), "{}", o.out);
        assert!(!o.out.contains(OPENING));
    }

    #[test]
    fn a_fresh_sign_in_prints_the_address_opens_the_browser_and_enrolls() {
        let script = Script::new(vec![
            (
                "near_ai_credential_status",
                vec![
                    ok(json!({"state": "absent", "session_state": "absent"})),
                    ok(json!({"state": "obtaining", "session_state": "obtaining",
                              "attempt_id": "a1", "attempt_status": "waiting_for_browser"})),
                    ok(json!({"state": "present", "session_state": "present",
                              "attempt_id": "a1", "attempt_status": "complete"})),
                ],
            ),
            (
                "near_ai_credential_start",
                vec![ok(
                    json!({"attempt_id": "a1", "status": "waiting_for_browser", "browser_url": URL}),
                )],
            ),
            ("near_ai_account_enroll", vec![enrolled()]),
            ("set_consent_scopes", vec![saved_scopes()]),
        ]);
        let o = drive(script, &floor());
        o.result.unwrap();
        assert_eq!(
            o.script.methods(),
            [
                "near_ai_credential_status",
                "near_ai_credential_start",
                "near_ai_credential_status",
                "near_ai_credential_status",
                "near_ai_account_enroll",
                "set_consent_scopes"
            ]
        );
        // The daemon picks the provider and the commons, as it does for the
        // app's first run without an invite.
        assert_eq!(o.script.calls[1].1, json!({}));
        assert_eq!(o.script.calls[4].1, json!({}));
        // Polls name the attempt.
        assert_eq!(o.script.calls[2].1, json!({"attempt_id": "a1"}));
        assert_eq!(o.opened.len(), 1);
        assert_eq!(o.opened[0].0, URL);
        assert!(o.out.contains(OPENING), "{}", o.out);
        assert!(
            o.out.contains(&format!("{OPEN_FALLBACK_PREFIX} {URL}")),
            "{}",
            o.out
        );
        assert!(o.out.contains(INVITE_HINT), "{}", o.out);
        assert!(o.out.contains(DONE), "{}", o.out);
    }

    /// The address is on screen before the browser is asked to open it, so a
    /// failed or hung opener never hides it.
    #[test]
    fn the_address_is_printed_before_the_browser_is_asked() {
        let mut script = Script::new(vec![
            (
                "near_ai_credential_status",
                vec![
                    ok(json!({"session_state": "absent"})),
                    ok(json!({"session_state": "present", "attempt_status": "complete"})),
                ],
            ),
            (
                "near_ai_credential_start",
                vec![ok(json!({"attempt_id": "a1", "browser_url": URL}))],
            ),
            ("near_ai_account_enroll", vec![enrolled()]),
            ("set_consent_scopes", vec![saved_scopes()]),
        ]);
        let out = std::cell::RefCell::new(Vec::<u8>::new());
        let mut seen_at_open = String::new();
        struct Shared<'a>(&'a std::cell::RefCell<Vec<u8>>);
        impl Write for Shared<'_> {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.borrow_mut().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = Shared(&out);
        let mut caller = |m: &str, p: Value| script.answer(m, p);
        let mut open = |_url: &str| {
            seen_at_open = String::from_utf8(out.borrow().clone()).unwrap();
            false
        };
        let mut sleep = |_d: Duration| {};
        sign_in_and_enroll(
            Ports {
                call: &mut caller,
                browser: BrowserPlan::LOCAL,
                open_browser: &mut open,
                sleep: &mut sleep,
                out: &mut writer,
            },
            &floor(),
        )
        .unwrap();
        assert!(seen_at_open.contains(URL), "{seen_at_open}");
    }

    fn env_of(
        vars: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<std::ffi::OsString> {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| std::ffi::OsString::from(v))
        }
    }

    #[test]
    fn the_browser_plan_follows_ssh_display_and_the_flag() {
        let plan = |no_browser, graphical, vars| {
            BrowserPlan::from_env(no_browser, graphical, &env_of(vars))
        };
        let local = BrowserPlan {
            open: true,
            remote: false,
        };
        let print_only = BrowserPlan {
            open: false,
            remote: false,
        };
        let remote = BrowserPlan {
            open: false,
            remote: true,
        };
        // A desktop session opens the browser.
        assert_eq!(plan(false, true, &[]), local);
        assert_eq!(plan(false, false, &[("DISPLAY", ":0")]), local);
        assert_eq!(
            plan(false, false, &[("WAYLAND_DISPLAY", "wayland-0")]),
            local
        );
        // --no-browser only stops the opener.
        assert_eq!(plan(true, true, &[]), print_only);
        // Linux with no display: nothing to open, forward from elsewhere.
        assert_eq!(plan(false, false, &[]), remote);
        assert_eq!(plan(false, false, &[("DISPLAY", "")]), remote);
        // Any SSH session is remote, even with a display (X forwarding) or
        // on macOS, where `open` would use the console's screen.
        for ssh in ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"] {
            let vars: &'static [(&'static str, &'static str)] = match ssh {
                "SSH_CONNECTION" => &[("SSH_CONNECTION", "1 2 3 4"), ("DISPLAY", "localhost:10")],
                "SSH_CLIENT" => &[("SSH_CLIENT", "1 2 3")],
                _ => &[("SSH_TTY", "/dev/pts/0")],
            };
            assert_eq!(plan(false, true, vars), remote, "{ssh}");
            assert_eq!(plan(false, false, vars), remote, "{ssh}");
        }
        // A BROWSER the person set up is still tried over SSH, and the
        // forwarding steps are still printed.
        assert_eq!(
            plan(
                false,
                false,
                &[("SSH_TTY", "/dev/pts/0"), ("BROWSER", "/opt/helper")]
            ),
            BrowserPlan {
                open: true,
                remote: true
            }
        );
        assert_eq!(
            plan(
                true,
                false,
                &[("SSH_TTY", "/dev/pts/0"), ("BROWSER", "/opt/helper")]
            ),
            remote
        );
        // `open` and `start` never read BROWSER: over SSH into a Mac it
        // would open on the console's screen, so nothing is opened.
        assert_eq!(
            plan(
                false,
                true,
                &[("SSH_TTY", "/dev/pts/0"), ("BROWSER", "/opt/helper")]
            ),
            remote
        );
    }

    #[test]
    fn the_loopback_port_comes_from_the_address_or_its_callback() {
        assert_eq!(loopback_port(URL), Some(49152));
        assert_eq!(
            loopback_port(
                "https://cloud-api.near.ai/v1/auth/github?frontend_callback=http%3A%2F%2F127.0.0.1%3A40001%2Ftrace-commons%2Fnear-ai%2Fcallback"
            ),
            Some(40001)
        );
        assert_eq!(
            loopback_port("https://cloud-api.near.ai/v1/auth/github"),
            None
        );
        assert_eq!(loopback_port("http://localhost:40001/x"), None);
        assert_eq!(loopback_port("not a url"), None);
    }

    /// One fresh sign-in under `plan`: what was printed, and how many times
    /// the opener was asked.
    fn fresh_sign_in_with(plan: BrowserPlan) -> (String, usize) {
        let mut script = Script::new(vec![
            (
                "near_ai_credential_status",
                vec![
                    ok(json!({"session_state": "absent"})),
                    ok(json!({"session_state": "present", "attempt_status": "complete"})),
                ],
            ),
            (
                "near_ai_credential_start",
                vec![ok(json!({"attempt_id": "a1", "browser_url": URL}))],
            ),
        ]);
        let mut out = Vec::new();
        let mut opened = 0;
        {
            let mut caller = |m: &str, p: Value| script.answer(m, p);
            let mut open = |_url: &str| {
                opened += 1;
                true
            };
            let mut sleep = |_d: Duration| {};
            sign_in_for_launch(Ports {
                call: &mut caller,
                browser: plan,
                open_browser: &mut open,
                sleep: &mut sleep,
                out: &mut out,
            })
            .unwrap();
        }
        (String::from_utf8(out).unwrap(), opened)
    }

    /// Over SSH the address alone is a dead end: it names this machine's
    /// loopback. The output says how to forward that port, with the port
    /// filled in, and no opener runs to print its own failures.
    #[test]
    fn a_remote_sign_in_prints_the_port_forward_and_opens_nothing() {
        let (out, opened) = fresh_sign_in_with(BrowserPlan {
            open: false,
            remote: true,
        });
        assert_eq!(opened, 0, "{out}");
        assert!(out.contains(REMOTE_INTRO), "{out}");
        // The code route is named first: it needs no port forward.
        assert!(
            out.find(CODE_HINT).unwrap() < out.find(REMOTE_INTRO).unwrap(),
            "{out}"
        );
        assert!(
            out.contains(&format!(
                "ssh -N -o ExitOnForwardFailure=yes -L 49152:127.0.0.1:49152 {REMOTE_HOST_PLACEHOLDER}"
            )),
            "{out}"
        );
        assert!(out.contains(URL), "{out}");
        assert!(!out.contains(OPENING), "{out}");
        // The command comes before the address it makes reachable.
        assert!(
            out.find("ssh -N").unwrap() < out.find(URL).unwrap(),
            "{out}"
        );
    }

    #[test]
    fn no_browser_prints_the_address_without_opening_it() {
        let (out, opened) = fresh_sign_in_with(BrowserPlan {
            open: false,
            remote: false,
        });
        assert_eq!(opened, 0, "{out}");
        assert!(out.contains(OPEN_PROMPT), "{out}");
        assert!(out.contains(URL), "{out}");
        assert!(!out.contains(OPENING), "{out}");
        assert!(!out.contains("ssh -N"), "{out}");
        assert!(!out.contains(CODE_HINT), "{out}");
    }

    /// Fail closed: a commons that does not advertise the NEAR AI login path
    /// refuses with its own sentence and the invite route, and records no
    /// consent against an enrollment that did not happen.
    #[test]
    fn a_commons_without_the_login_path_refuses() {
        let script = Script::new(vec![
            (
                "near_ai_credential_status",
                vec![ok(json!({"session_state": "present"}))],
            ),
            (
                "near_ai_account_enroll",
                vec![err("near_ai_enroll_commons_unsupported")],
            ),
        ]);
        let o = drive(script, &floor());
        let error = format!("{:#}", o.result.unwrap_err());
        assert!(error.contains(COMMONS_UNSUPPORTED), "{error}");
        // The app's line offers a wallet, which the CLI has no route to.
        assert!(!error.contains("wallet"), "{error}");
        assert!(error.contains(INVITE_HINT), "{error}");
        assert!(!o.script.methods().contains(&"set_consent_scopes"));
        assert!(!o.out.contains(DONE));
    }

    /// [`sign_in_for_launch`] against a scripted daemon: its result and
    /// output.
    fn drive_launch(mut script: Script) -> (Result<()>, String, usize, Script) {
        let mut out = Vec::new();
        let mut opened = 0;
        let result = {
            let mut caller = |m: &str, p: Value| script.answer(m, p);
            let mut open = |_url: &str| {
                opened += 1;
                true
            };
            let mut sleep = |_d: Duration| {};
            sign_in_for_launch(Ports {
                call: &mut caller,
                browser: BrowserPlan::LOCAL,
                open_browser: &mut open,
                sleep: &mut sleep,
                out: &mut out,
            })
        };
        (result, String::from_utf8(out).unwrap(), opened, script)
    }

    /// A launch signs in through the browser and stops there: it does not
    /// enroll, record consent, or offer an invite.
    #[test]
    fn a_launch_signs_in_without_enrolling() {
        let script = Script::new(vec![
            (
                "near_ai_credential_status",
                vec![
                    ok(json!({"state": "absent", "session_state": "absent"})),
                    ok(json!({"state": "present", "session_state": "present",
                              "attempt_id": "a1", "attempt_status": "complete"})),
                ],
            ),
            (
                "near_ai_credential_start",
                vec![ok(json!({"attempt_id": "a1", "browser_url": URL}))],
            ),
        ]);
        let (result, out, opened, script) = drive_launch(script);
        result.unwrap();
        assert_eq!(
            script.methods(),
            [
                "near_ai_credential_status",
                "near_ai_credential_start",
                "near_ai_credential_status"
            ]
        );
        assert_eq!(opened, 1);
        assert!(out.contains(URL), "{out}");
        assert!(out.ends_with(&format!("{LAUNCH_SIGNED_IN}\n")), "{out}");
        assert!(!out.contains(INVITE_HINT), "{out}");
        // Already signed in: no browser.
        let kept = Script::new(vec![(
            "near_ai_credential_status",
            vec![ok(json!({"state": "present", "session_state": "present"}))],
        )]);
        let (result, _, opened, _) = drive_launch(kept);
        result.unwrap();
        assert_eq!(opened, 0);
    }

    /// An unfinished sign-in says to run the launch again, not `login`.
    #[test]
    fn an_unfinished_launch_sign_in_says_to_run_it_again() {
        let script = Script::new(vec![
            (
                "near_ai_credential_status",
                vec![
                    ok(json!({"session_state": "absent"})),
                    ok(json!({"session_state": "absent", "attempt_status": "failed"})),
                ],
            ),
            (
                "near_ai_credential_start",
                vec![ok(json!({"attempt_id": "a1", "browser_url": URL}))],
            ),
        ]);
        let (result, out, _, _) = drive_launch(script);
        assert_eq!(result.unwrap_err().to_string(), LAUNCH_SIGN_IN_UNFINISHED);
        assert!(!out.contains(LAUNCH_SIGNED_IN), "{out}");
    }

    #[test]
    fn an_ended_sign_in_stops_before_enrolling() {
        for ended in ["failed", "cancelled", "complete"] {
            let script = Script::new(vec![
                (
                    "near_ai_credential_status",
                    vec![
                        ok(json!({"session_state": "absent"})),
                        ok(json!({"session_state": "absent", "attempt_status": ended})),
                    ],
                ),
                (
                    "near_ai_credential_start",
                    vec![ok(json!({"attempt_id": "a1", "browser_url": URL}))],
                ),
            ]);
            let o = drive(script, &floor());
            let error = o.result.unwrap_err().to_string();
            assert_eq!(error, SIGN_IN_UNFINISHED, "{ended}");
            assert!(!o.script.methods().contains(&"near_ai_account_enroll"));
        }
    }

    #[test]
    fn a_browser_that_never_finishes_is_cancelled_at_the_limit() {
        let script = Script::new(vec![
            (
                "near_ai_credential_status",
                vec![
                    ok(json!({"session_state": "absent"})),
                    ok(
                        json!({"session_state": "obtaining", "attempt_status": "waiting_for_browser"}),
                    ),
                ],
            ),
            (
                "near_ai_credential_start",
                vec![ok(json!({"attempt_id": "a1", "browser_url": URL}))],
            ),
            ("near_ai_credential_cancel", vec![ok(json!({}))]),
        ]);
        let o = drive(script, &floor());
        assert_eq!(o.result.unwrap_err().to_string(), SIGN_IN_UNFINISHED);
        let methods = o.script.methods();
        assert_eq!(methods.last(), Some(&"near_ai_credential_cancel"));
        assert_eq!(
            o.script.calls.last().unwrap().1,
            json!({"attempt_id": "a1"})
        );
        let polls = methods
            .iter()
            .filter(|m| **m == "near_ai_credential_status")
            .count();
        assert_eq!(polls as u64, 1 + BROWSER_WAIT.as_secs());
        assert!(!methods.contains(&"near_ai_account_enroll"));
    }

    #[test]
    fn a_start_the_daemon_refuses_stops_with_one_sentence() {
        let script = Script::new(vec![
            (
                "near_ai_credential_status",
                vec![ok(json!({"session_state": "absent"}))],
            ),
            (
                "near_ai_credential_start",
                vec![err("near_ai_credential_unavailable")],
            ),
        ]);
        let o = drive(script, &floor());
        assert_eq!(o.result.unwrap_err().to_string(), SIGN_IN_UNAVAILABLE);
        assert!(o.opened.is_empty());
    }

    /// `--default` answers record the floor and no choice, exactly as the
    /// invite route records them; chosen scopes are sent as chosen.
    #[test]
    fn consent_is_sent_as_the_invite_route_records_it() {
        let run = |consent: Consent| {
            let script = Script::new(vec![
                (
                    "near_ai_credential_status",
                    vec![ok(json!({"session_state": "present"}))],
                ),
                ("near_ai_account_enroll", vec![enrolled()]),
                ("set_consent_scopes", vec![saved_scopes()]),
            ]);
            let o = drive(script, &consent);
            o.result.unwrap();
            o.script.calls.last().unwrap().1.clone()
        };
        assert_eq!(run(floor()), json!({"scopes": []}));
        assert_eq!(
            run(Consent {
                scopes: vec!["debugging_evaluation".into(), "model_training".into()],
                chosen: true,
            }),
            json!({"scopes": ["debugging_evaluation", "model_training"]})
        );
    }

    #[test]
    fn consent_that_was_not_saved_says_so() {
        let script = Script::new(vec![
            (
                "near_ai_credential_status",
                vec![ok(json!({"session_state": "present"}))],
            ),
            ("near_ai_account_enroll", vec![enrolled()]),
            ("set_consent_scopes", vec![err("audit-write-failed")]),
        ]);
        let o = drive(script, &floor());
        assert_eq!(o.result.unwrap_err().to_string(), CONSENT_NOT_SAVED);
    }

    /// Nothing the daemon hands back reaches the terminal verbatim: not a
    /// token-shaped value planted in a result or a refusal, and not the
    /// enrolled account id.
    #[test]
    fn no_secret_reaches_output_or_errors() {
        const PLANTED: [&str; 5] = [
            "rt_PLANTEDREFRESH",
            "tcn1_PLANTEDSESSION",
            "sk-PLANTEDKEY",
            "eyJPLANTEDJWT",
            "acct-7f3e",
        ];
        let laced = |label: &str| err(&format!("{label} rt_PLANTEDREFRESH eyJPLANTEDJWT"));
        let enrolled_laced = || {
            ok(json!({
                "enrolled": true,
                "tenant_id": format!("nearai-{}", "ab".repeat(32)),
                "account_id": "acct-7f3e",
                "access_token": "tcn1_PLANTEDSESSION",
                "device_key_id": "sha256:00",
            }))
        };
        let status_laced = |s: &str| {
            ok(json!({"session_state": s, "attempt_id": "a1",
                      "attempt_status": "waiting_for_browser",
                      "refresh_token": "rt_PLANTEDREFRESH", "key": "sk-PLANTEDKEY"}))
        };
        let scripts = vec![
            // Success, through the browser.
            Script::new(vec![
                (
                    "near_ai_credential_status",
                    vec![status_laced("absent"), status_laced("present")],
                ),
                (
                    "near_ai_credential_start",
                    vec![ok(json!({"attempt_id": "a1", "browser_url": URL,
                                   "refresh_token": "rt_PLANTEDREFRESH"}))],
                ),
                ("near_ai_account_enroll", vec![enrolled_laced()]),
                ("set_consent_scopes", vec![saved_scopes()]),
            ]),
            // Start refused.
            Script::new(vec![
                ("near_ai_credential_status", vec![status_laced("absent")]),
                (
                    "near_ai_credential_start",
                    vec![laced("near_ai_credential_unavailable")],
                ),
            ]),
            // Every enroll refusal, and one this build has never heard of.
            Script::new(vec![
                ("near_ai_credential_status", vec![status_laced("present")]),
                (
                    "near_ai_account_enroll",
                    vec![laced("near_ai_enroll_token_unavailable")],
                ),
            ]),
            Script::new(vec![
                ("near_ai_credential_status", vec![status_laced("present")]),
                ("near_ai_account_enroll", vec![laced("tcn1_PLANTEDSESSION")]),
            ]),
            // Consent refused.
            Script::new(vec![
                ("near_ai_credential_status", vec![status_laced("present")]),
                ("near_ai_account_enroll", vec![enrolled_laced()]),
                ("set_consent_scopes", vec![laced("config-write-failed")]),
            ]),
        ];
        for script in scripts {
            let o = drive(script, &floor());
            let error = o
                .result
                .err()
                .map(|e| format!("{e:#} {e:?}"))
                .unwrap_or_default();
            for secret in PLANTED {
                assert!(!o.out.contains(secret), "{secret} in output: {}", o.out);
                assert!(!error.contains(secret), "{secret} in error: {error}");
            }
        }
    }

    #[test]
    fn the_poll_verdict_matches_the_macos_first_run() {
        assert_eq!(poll_verdict(None), Poll::Waiting);
        assert_eq!(
            poll_verdict(Some(&json!({"session_state": "present"}))),
            Poll::SignedIn
        );
        assert_eq!(
            poll_verdict(Some(&json!({"session_state": "absent"}))),
            Poll::Waiting
        );
        assert_eq!(
            poll_verdict(Some(
                &json!({"session_state": "obtaining", "attempt_status": "waiting_for_browser"})
            )),
            Poll::Waiting
        );
        assert_eq!(
            poll_verdict(Some(
                &json!({"session_state": "absent", "attempt_status": "failed"})
            )),
            Poll::Ended
        );
    }

    /// The parity `Consent::request_scopes` relies on, checked against the
    /// real handler: a NEAR AI enrollment's config (no scopes, no choice)
    /// given an empty list keeps the floor and records no choice; given a
    /// list, records the choice.
    #[test]
    fn the_daemon_records_consent_as_the_invite_route_does() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let config = crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "https://issuer.example".into(),
            ingest_url: "https://ingest.example".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: format!("nearai-{}", "ab".repeat(32)),
            instance_id: String::new(),
            user_subject: "sha256:00".into(),
            device_key_id: "sha256:00".into(),
            consent_scopes: Vec::new(),
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness_origin: None,
            witness: None,
            inference_receipt_endpoint: None,
            consent_scopes_chosen: Some(false),
            inference_receipt_check_attestation: true,
        };
        store.save_config(&config).unwrap();
        let shared = crate::daemon::ipc::DaemonShared::load(
            ConfigStore::open(dir.path().to_path_buf()).unwrap(),
        )
        .unwrap();

        let default = floor().request_scopes();
        let r = crate::daemon::ipc::handle_local(
            &shared,
            "set_consent_scopes",
            json!({ "scopes": default }),
        );
        assert!(r.error.is_none(), "{r:?}");
        let saved = store.load_config().unwrap().unwrap();
        assert_eq!(
            saved.consent_scopes,
            vec!["debugging_evaluation".to_string()]
        );
        assert_eq!(saved.consent_scopes_chosen, Some(false));

        let chosen = Consent {
            scopes: vec!["debugging_evaluation".into(), "model_training".into()],
            chosen: true,
        }
        .request_scopes();
        let r = crate::daemon::ipc::handle_local(
            &shared,
            "set_consent_scopes",
            json!({ "scopes": chosen }),
        );
        assert!(r.error.is_none(), "{r:?}");
        let saved = store.load_config().unwrap().unwrap();
        assert!(saved.consent_scopes.contains(&"model_training".to_string()));
        assert_eq!(saved.consent_scopes_chosen, Some(true));
    }
}

#[cfg(test)]
mod app_handoff_tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn app() -> PathBuf {
        PathBuf::from("/Applications/Near.ai Staging.app")
    }

    #[test]
    fn a_running_daemon_answers_whatever_else_holds() {
        assert_eq!(route(true, false, false, Some(app()), false), Route::Daemon);
        assert_eq!(route(true, true, true, None, false), Route::Daemon);
    }

    #[test]
    fn an_entitled_process_signs_in_itself_as_before() {
        assert_eq!(
            route(false, true, false, Some(app()), false),
            Route::InProcess
        );
        assert_eq!(route(false, true, true, None, false), Route::InProcess);
    }

    #[test]
    fn an_unentitled_cli_inside_a_set_up_app_starts_it_in_the_background() {
        assert_eq!(
            route(false, false, true, Some(app()), false),
            Route::LaunchApp(app())
        );
    }

    #[test]
    fn an_unentitled_cli_on_a_mac_not_set_up_opens_the_apps_setup() {
        // No folders chosen: the app will not start its daemon until first
        // run has them, so there is nothing to wait for in the background.
        assert_eq!(
            route(false, false, false, Some(app()), false),
            Route::OpenSetup(app())
        );
    }

    #[test]
    fn an_unentitled_cli_outside_an_app_is_told_to_open_one() {
        assert_eq!(route(false, false, true, None, false), Route::OpenApp);
        assert_eq!(route(false, false, false, None, false), Route::OpenApp);
    }

    /// Already open with its daemon not answering (refused, or still on
    /// first run's folders): `open` would only re-activate it, since
    /// LaunchServices drops `--args` and `--env` for a running instance, so
    /// nothing is waited for.
    #[test]
    fn an_app_already_open_without_its_daemon_is_not_waited_on() {
        assert_eq!(
            route(false, false, true, Some(app()), true),
            Route::AppNotAnswering(app())
        );
        // With no folders declared it is on first run: bring it forward.
        assert_eq!(
            route(false, false, false, Some(app()), true),
            Route::OpenSetup(app())
        );
        assert_eq!(route(true, false, true, Some(app()), true), Route::Daemon);
    }

    #[test]
    fn a_running_app_is_found_by_its_executable_path() {
        let exe = Path::new("/Applications/Near.ai Staging.app/Contents/MacOS/TraceCommonsApp");
        let ps = "/sbin/launchd\n\
                  /Applications/Near.ai Staging.app/Contents/MacOS/TraceCommonsApp --background-launch\n";
        assert!(app_running_in(ps, exe));
        assert!(app_running_in(
            "/Applications/Near.ai Staging.app/Contents/MacOS/TraceCommonsApp\n",
            exe
        ));
        // The bundled CLI itself, the production app, and a longer name
        // sharing the prefix are not it.
        assert!(!app_running_in(
            "/Applications/Near.ai Staging.app/Contents/MacOS/near-ai login\n\
             /Applications/Near.ai.app/Contents/MacOS/TraceCommonsApp\n\
             /Applications/Near.ai Staging.app/Contents/MacOS/TraceCommonsAppHelper\n",
            exe
        ));
    }

    /// `--config-dir` wins over the override variable in
    /// `ConfigStore::resolve`, so the store is forwarded whenever it is not
    /// the default, however it was chosen; the default goes unforwarded, so
    /// the app resolves it exactly as a click would.
    #[test]
    fn a_store_other_than_the_default_is_forwarded_however_it_was_chosen() {
        let default = Path::new("/Users/x/Library/Application Support/trace-commons-staging");
        let chosen = Path::new("/Users/x/tc-alt");
        assert_eq!(forwarded_state_dir(chosen, Some(default)), Some(chosen));
        assert_eq!(forwarded_state_dir(default, Some(default)), None);
        // No default to compare with: forward, so the app cannot pick another.
        assert_eq!(forwarded_state_dir(chosen, None), Some(chosen));
    }

    #[test]
    fn the_app_is_found_from_the_bundled_cli_only() {
        assert_eq!(
            app_bundle(Path::new(
                "/Applications/Near.ai Staging.app/Contents/MacOS/near-ai"
            )),
            Some(app())
        );
        assert_eq!(app_bundle(Path::new("/opt/homebrew/bin/near-ai")), None);
        assert_eq!(
            app_bundle(Path::new("/tmp/target/debug/trace-commons-contributor")),
            None
        );
        // Contents/MacOS under something that is not an app bundle.
        assert_eq!(app_bundle(Path::new("/tmp/x/Contents/MacOS/near-ai")), None);
        assert_eq!(
            app_bundle(Path::new("/tmp/x.app/Resources/MacOS/near-ai")),
            None
        );
    }

    #[test]
    fn the_app_is_launched_without_coming_forward_or_opening_a_window() {
        let args = background_launch_args(&app(), None);
        assert_eq!(
            args,
            vec![
                "-g".to_string(),
                "/Applications/Near.ai Staging.app".to_string(),
                "--args".to_string(),
                BACKGROUND_LAUNCH_ARG.to_string(),
            ]
        );
        assert!(
            !args.iter().any(|a| a == "-j"),
            "hidden apps can hold back later windows"
        );
        assert_eq!(
            BACKGROUND_LAUNCH_ARG, "--background-launch",
            "the app matches this spelling"
        );
    }

    #[test]
    fn setup_opens_the_app_normally() {
        assert_eq!(
            setup_launch_args(&app(), None),
            vec!["/Applications/Near.ai Staging.app".to_string()]
        );
    }

    /// `open` does not pass the terminal's environment on, so a store
    /// chosen with the override variable is handed to the app explicitly:
    /// otherwise the app would start its daemon on the default folder and
    /// `login` would wait on one that never answers.
    #[test]
    fn an_overridden_store_goes_to_the_app_with_it() {
        let dir = Path::new("/tmp/tc state");
        let pair = format!("{}=/tmp/tc state", crate::channel::STATE_DIR_ENV);
        assert_eq!(
            background_launch_args(&app(), Some(dir)),
            vec![
                "-g".to_string(),
                "--env".to_string(),
                pair.clone(),
                "/Applications/Near.ai Staging.app".to_string(),
                "--args".to_string(),
                BACKGROUND_LAUNCH_ARG.to_string(),
            ]
        );
        assert_eq!(
            setup_launch_args(&app(), Some(dir)),
            vec![
                "--env".to_string(),
                pair,
                "/Applications/Near.ai Staging.app".to_string()
            ]
        );
    }

    #[test]
    fn waiting_stops_once_the_daemon_answers() {
        let mut checks = 0;
        let mut slept = Duration::ZERO;
        let up = wait_for_daemon(
            &mut || {
                checks += 1;
                checks == 3
            },
            &mut |d| slept += d,
        );
        assert!(up);
        assert_eq!(checks, 3);
        assert_eq!(slept, DAEMON_POLL * 2);
    }

    #[test]
    fn waiting_gives_up_after_its_bound() {
        let mut checks = 0u32;
        let mut slept = Duration::ZERO;
        let up = wait_for_daemon(
            &mut || {
                checks += 1;
                false
            },
            &mut |d| slept += d,
        );
        assert!(!up);
        assert!(slept >= DAEMON_WAIT, "waited {slept:?}");
        assert!(slept < DAEMON_WAIT + DAEMON_POLL * 2, "waited {slept:?}");
    }

    #[test]
    fn the_messages_name_the_app() {
        assert_eq!(
            launching_line(&app()),
            "Starting Near.ai Staging in the background to sign in."
        );
        assert_eq!(
            setup_line(&app()),
            "Finish setting up in Near.ai Staging, which is now open. Sign-in is part of setup."
        );
        assert_eq!(
            not_answering_line(&app()),
            "Near.ai Staging is open, but its background service isn't answering. Open Near.ai Staging to see why."
        );
        assert_eq!(
            no_daemon_line(&app()),
            "Near.ai Staging started, but its background service didn't. Open Near.ai Staging to see why."
        );
    }
}

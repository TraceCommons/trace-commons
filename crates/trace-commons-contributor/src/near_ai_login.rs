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
//! # Secrets
//!
//! The refresh token and NEAR AI's short-lived JWT never leave the daemon's
//! handlers; neither travels over IPC. What does travel back -- refusal
//! labels, the enrolled account id -- is not printed verbatim: refusals map
//! to fixed sentences and the account id is not shown at all.

use std::io::Write;
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
    fn request_scopes(&self) -> Vec<String> {
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
    /// - `BROWSER` names an opener the person set up, which can work over SSH
    ///   (an editor's remote terminal forwards it), so it is still tried; the
    ///   forwarding steps are printed either way.
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
            open: !no_browser && (set("BROWSER") || !remote),
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
            writeln!(out, "{REMOTE_INTRO}")?;
            writeln!(
                out,
                "  1. In a terminal on your computer, forward the sign-in port and leave it running:"
            )?;
            writeln!(
                out,
                "       ssh -N -L {port}:127.0.0.1:{port} {REMOTE_HOST_PLACEHOLDER}"
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
/// loaded for the whole flow.
///
/// Which of the two is decided once. A per-call fallback would load the
/// daemon state (and read the OS credential store) on every poll, and would
/// ask a fresh in-process state about an attempt a running daemon holds.
pub(crate) fn run(store: &ConfigStore, consent: &Consent, no_browser: bool) -> Result<()> {
    use crate::daemon::client;
    use crate::daemon::ipc::{DaemonShared, handle_local};

    let running = client::is_running(store);
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
        assert!(
            out.contains(&format!(
                "ssh -N -L 49152:127.0.0.1:49152 {REMOTE_HOST_PLACEHOLDER}"
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

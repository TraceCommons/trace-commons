//! `devices` and `login --code`: the devices linked to one account, and
//! linking a server or headless machine with a one-time code.
//!
//! On a signed-in computer, `devices add` prints a code that lasts ten
//! minutes and waits until it is used. On the server, `login --code <code>`
//! enrolls that machine in the same account; it needs no browser. See
//! `daemon::device_codes` for what the daemon does with each.

use std::io::Write;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::commands::DaemonCaller;
use crate::config::ConfigStore;
use crate::daemon::ipc::Response;

/// How often `devices add` asks whether the code was used.
const POLL_EVERY: Duration = Duration::from_secs(3);
/// How long `devices add` waits: the code's own lifetime.
const ADD_WAIT: Duration =
    Duration::from_secs(trace_commons_protocol::account_device_code::DEVICE_CODE_TTL_SECS as u64);

/// The environment variable `login --code` also reads, which keeps the
/// code out of shell history.
pub(crate) const CODE_ENV: &str = "TRACE_COMMONS_DEVICE_CODE";

/// Printed once `login --code` has enrolled this machine.
pub(crate) const CODE_DONE: &str =
    "Linked. This machine is enrolled in the same account and renews its own sign-in.";

/// The error `login --code` ends with when this machine enrolled but could
/// not record that it renews its own sign-in.
pub(crate) const CODE_NO_RENEWAL: &str = "Linked, but this machine couldn't record that it renews its own sign-in, so it will be signed out in about 12 hours. Check that the state directory is writable, then run `trace-commons-contributor logout` and link it again with a new code.";

/// A daemon answer's result, or its refusal label as the error.
fn result(response: Response) -> Result<Value> {
    match response {
        Response {
            error: Some(error), ..
        } => Err(anyhow!("{}", error.message)),
        Response {
            result: Some(result),
            ..
        } => Ok(result),
        _ => Err(anyhow!("account_devices_unavailable")),
    }
}

/// The sentence for a refusal label, or the label itself.
fn refusal_line(label: &str) -> String {
    match label {
        "account-session-required" => {
            "Sign in first: run `trace-commons-contributor account login`.".into()
        }
        "not-logged-in" => {
            "This device isn't enrolled. Run `trace-commons-contributor login` first.".into()
        }
        "device_code_already_enrolled" => {
            "This machine is already enrolled. Run `trace-commons-contributor logout` first to link it to another account.".into()
        }
        "device_code_invalid" => "That doesn't look like a device code. Codes start with `tcd1_`.".into(),
        "device_code_refused" => {
            "That code was refused. It may have expired or been used; make a new one with `trace-commons-contributor devices add`.".into()
        }
        "device_key_registered_elsewhere" => {
            "This machine's device key is already linked to another account.".into()
        }
        "device_code_too_many" | "device_code_rate_limited" | "account_devices_rate_limited" => {
            "Too many codes at once. Wait a few minutes and try again.".into()
        }
        "device_code_joined_by_code" => {
            "This machine joined by code, so it can't make codes. Make one on a computer signed in with NEAR AI.".into()
        }
        "device_code_minter_not_a_device" => {
            "This device can't make codes for the account. Make one on a computer that joined with a NEAR AI login, not with a code.".into()
        }
        "account_device_not_found" => "No device with that id is linked to this account.".into(),
        "account_device_revoke_this_device" => {
            "That's this device. Use `trace-commons-contributor logout` to sign it out instead.".into()
        }
        other => other.to_string(),
    }
}

fn refused(error: anyhow::Error) -> anyhow::Error {
    anyhow!("{}", refusal_line(&error.to_string()))
}

/// `devices list`.
pub(crate) fn list_with(call: DaemonCaller<'_>, json: bool, out: &mut dyn Write) -> Result<()> {
    let value = result(call("account_devices_list", json!({}))?).map_err(refused)?;
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&value)?)?;
        return Ok(());
    }
    let devices = value["devices"].as_array().cloned().unwrap_or_default();
    if devices.is_empty() {
        writeln!(out, "No devices are linked to this account.")?;
    }
    for device in devices {
        let id = device["device_key_id"].as_str().unwrap_or("");
        let label = device["label"].as_str().unwrap_or("(no name)");
        let this = if device["this_device"] == true {
            "  (this device)"
        } else {
            ""
        };
        let seen = device["last_seen_at"]
            .as_str()
            .map(|at| format!("  last seen {at}"))
            .unwrap_or_default();
        writeln!(out, "{label}{this}")?;
        writeln!(out, "  {id}{seen}")?;
    }
    Ok(())
}

/// `devices rename <id> [label]`: no label clears it.
pub(crate) fn rename_with(
    call: DaemonCaller<'_>,
    device_key_id: &str,
    label: Option<&str>,
    json: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let value = result(call(
        "account_device_rename",
        json!({ "device_key_id": device_key_id, "label": label }),
    )?)
    .map_err(refused)?;
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&value)?)?;
    } else if value["labeled"] == true {
        writeln!(out, "Renamed.")?;
    } else {
        writeln!(out, "Name cleared.")?;
    }
    Ok(())
}

/// `devices revoke <id>`: asks first unless `yes`.
pub(crate) fn revoke_with(
    call: DaemonCaller<'_>,
    device_key_id: &str,
    yes: bool,
    json: bool,
    reader: &mut impl std::io::Read,
    out: &mut impl Write,
) -> Result<()> {
    if !yes {
        writeln!(
            out,
            "About to unlink {device_key_id}. It is signed out and can't upload or renew its sign-in."
        )?;
        if !crate::commands::read_yes_no("Unlink this device? [y/N] ", reader, out)? {
            bail!("Cancelled; nothing changed.");
        }
    }
    let value = result(call(
        "account_device_revoke",
        json!({ "device_key_id": device_key_id }),
    )?)
    .map_err(refused)?;
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&value)?)?;
    } else {
        writeln!(out, "Unlinked.")?;
    }
    Ok(())
}

/// `devices add [--label]`: print a one-time code, then wait until it is
/// used or expires (unless `wait` is false).
pub(crate) fn add_with(
    call: DaemonCaller<'_>,
    label: Option<&str>,
    wait: bool,
    json: bool,
    sleep: &mut dyn FnMut(Duration),
    out: &mut dyn Write,
) -> Result<()> {
    let minted =
        result(call("account_device_code_mint", json!({ "label": label }))?).map_err(refused)?;
    let (Some(code_id), Some(code)) = (minted["code_id"].as_str(), minted["code"].as_str()) else {
        bail!("account_devices_unavailable")
    };
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&minted)?)?;
        if !wait {
            return Ok(());
        }
    } else {
        writeln!(out, "On the machine you want to link, run:")?;
        writeln!(out, "  trace-commons-contributor login --code {code}")?;
        writeln!(
            out,
            "The code works once, within 10 minutes. Keep it private: whoever uses it joins your account."
        )?;
        if !wait {
            return Ok(());
        }
        writeln!(out, "Waiting for it to be used...")?;
    }
    out.flush()?;
    let mut waited = Duration::ZERO;
    while waited < ADD_WAIT {
        sleep(POLL_EVERY);
        waited += POLL_EVERY;
        // A missed poll is not the end: the code may still be used.
        let Ok(Ok(status)) =
            call("account_device_code_status", json!({ "code_id": code_id })).map(result)
        else {
            continue;
        };
        match status["state"].as_str() {
            Some("redeemed") => {
                if json {
                    writeln!(out, "{}", serde_json::to_string_pretty(&status)?)?;
                } else {
                    let device = status["device_key_id"].as_str().unwrap_or("");
                    writeln!(out, "Linked {device}.")?;
                }
                return Ok(());
            }
            Some("expired") => bail!("The code expired before it was used."),
            _ => continue,
        }
    }
    bail!("The code expired before it was used.")
}

/// `login --code`: enroll this machine with a code from a signed-in
/// device, then record the consent answers.
pub(crate) fn login_with_code_with(
    call: DaemonCaller<'_>,
    code: &str,
    consent: &crate::near_ai_login::Consent,
    json: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let enrolled =
        result(call("account_device_code_redeem", json!({ "code": code }))?).map_err(refused)?;
    let saved = result(call(
        "set_consent_scopes",
        json!({ "scopes": consent.request_scopes() }),
    )?);
    if saved.is_err() {
        bail!(crate::near_ai_login::CONSENT_NOT_SAVED)
    }
    // Without the marker the daemon renews nothing, and the session lapses
    // after 12 hours. Enrolled, but not as asked: say so and fail, so a
    // provisioning script does not carry on.
    let renews = enrolled
        .get("renews_by_device_key")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&enrolled)?)?;
    } else if renews {
        writeln!(out, "{CODE_DONE}")?;
    }
    if !renews {
        bail!(CODE_NO_RENEWAL)
    }
    Ok(())
}

/// [`login_with_code_with`] against this store. Refused before the consent
/// questions when this machine is already enrolled.
pub(crate) fn login_with_code(
    store: &ConfigStore,
    code: &str,
    scopes: Option<&str>,
    default_consent: bool,
    json: bool,
) -> Result<()> {
    if store.load_config()?.is_some() {
        bail!(crate::commands::ALREADY_ENROLLED);
    }
    let (scopes, chosen) = crate::commands::resolve_consent_scopes(scopes, default_consent)?;
    login_with_code_with(
        &mut |method, params| crate::commands::daemon_call(store, method, params),
        code,
        &crate::near_ai_login::Consent { scopes, chosen },
        json,
        &mut std::io::stdout(),
    )
}

/// A `login` refusal, saying where the code came from when it was read
/// from [`CODE_ENV`] rather than given: a spent code left exported would
/// otherwise fail every later login with no hint why.
pub(crate) fn naming_the_variable(error: anyhow::Error, from_env: bool) -> anyhow::Error {
    if from_env {
        anyhow!("{error} (The code came from {CODE_ENV}; unset it to sign in another way.)")
    } else {
        error
    }
}

/// The code for `login --code`: the flag, or else [`CODE_ENV`] when no
/// other sign-in route was asked for (`other_route`).
pub(crate) fn code_from(
    flag: Option<&str>,
    env: Option<&str>,
    other_route: bool,
) -> Option<String> {
    flag.or(env.filter(|_| !other_route))
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ipc::IpcError;
    use std::collections::VecDeque;

    fn ok(value: Value) -> Result<Response> {
        Ok(Response {
            id: 0,
            result: Some(value),
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

    struct Script {
        answers: VecDeque<(&'static str, Result<Response>)>,
        seen: Vec<(String, Value)>,
    }

    impl Script {
        fn new(answers: Vec<(&'static str, Result<Response>)>) -> Self {
            Self {
                answers: answers.into(),
                seen: Vec::new(),
            }
        }

        fn answer(&mut self, method: &str, params: Value) -> Result<Response> {
            self.seen.push((method.to_string(), params));
            let (expected, answer) = self.answers.pop_front().expect("an answer is scripted");
            assert_eq!(expected, method);
            answer
        }
    }

    #[test]
    fn add_prints_the_login_line_and_waits_until_the_code_is_used() {
        let mut script = Script::new(vec![
            (
                "account_device_code_mint",
                ok(json!({"code_id": "c1", "code": "tcd1_dA.s", "expires_at": "x"})),
            ),
            (
                "account_device_code_status",
                ok(json!({"state": "pending"})),
            ),
            (
                "account_device_code_status",
                err("account_devices_unavailable"),
            ),
            (
                "account_device_code_status",
                ok(json!({"state": "redeemed", "device_key_id": "sha256:ab"})),
            ),
        ]);
        let mut slept = Duration::ZERO;
        let mut out = Vec::new();
        add_with(
            &mut |m, p| script.answer(m, p),
            Some("build box"),
            true,
            false,
            &mut |d| slept += d,
            &mut out,
        )
        .unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.contains("trace-commons-contributor login --code tcd1_dA.s"),
            "{out}"
        );
        assert!(out.contains("Linked sha256:ab."), "{out}");
        assert_eq!(slept, POLL_EVERY * 3);
        assert_eq!(script.seen[0].1, json!({"label": "build box"}));
        assert_eq!(script.seen[1].1, json!({"code_id": "c1"}));
    }

    #[test]
    fn add_stops_when_the_code_expires() {
        let mut script = Script::new(vec![
            (
                "account_device_code_mint",
                ok(json!({"code_id": "c1", "code": "tcd1_dA.s"})),
            ),
            (
                "account_device_code_status",
                ok(json!({"state": "expired"})),
            ),
        ]);
        let mut out = Vec::new();
        let error = add_with(
            &mut |m, p| script.answer(m, p),
            None,
            true,
            false,
            &mut |_| {},
            &mut out,
        )
        .unwrap_err();
        assert!(error.to_string().contains("expired"), "{error}");
    }

    #[test]
    fn add_gives_up_after_the_codes_lifetime() {
        let mut calls = 0usize;
        let mut call = |method: &str, _: Value| {
            calls += 1;
            if method == "account_device_code_mint" {
                ok(json!({"code_id": "c1", "code": "tcd1_dA.s"}))
            } else {
                ok(json!({"state": "pending"}))
            }
        };
        let mut out = Vec::new();
        assert!(add_with(&mut call, None, true, false, &mut |_| {}, &mut out).is_err());
        assert_eq!(
            calls,
            1 + (ADD_WAIT.as_secs() / POLL_EVERY.as_secs()) as usize
        );
    }

    #[test]
    fn a_session_refusal_names_the_sign_in() {
        let mut script = Script::new(vec![(
            "account_device_code_mint",
            err("account-session-required"),
        )]);
        let mut out = Vec::new();
        let error = add_with(
            &mut |m, p| script.answer(m, p),
            None,
            false,
            false,
            &mut |_| {},
            &mut out,
        )
        .unwrap_err();
        assert!(error.to_string().contains("account login"), "{error}");
    }

    #[test]
    fn login_with_code_redeems_then_records_consent() {
        let mut script = Script::new(vec![
            (
                "account_device_code_redeem",
                ok(
                    json!({"enrolled": true, "tenant_id": "nearai-x", "renews_by_device_key": true}),
                ),
            ),
            ("set_consent_scopes", ok(json!({"saved": true}))),
        ]);
        let mut out = Vec::new();
        login_with_code_with(
            &mut |m, p| script.answer(m, p),
            "tcd1_dA.s",
            &crate::near_ai_login::Consent {
                scopes: vec!["debugging_evaluation".into(), "model_training".into()],
                chosen: true,
            },
            false,
            &mut out,
        )
        .unwrap();
        assert_eq!(script.seen[0].1, json!({"code": "tcd1_dA.s"}));
        assert_eq!(
            script.seen[1].1,
            json!({"scopes": ["debugging_evaluation", "model_training"]})
        );
        assert!(String::from_utf8(out).unwrap().contains(CODE_DONE));
    }

    #[test]
    fn a_link_that_cannot_renew_fails_instead_of_saying_it_renews() {
        let mut script = Script::new(vec![
            (
                "account_device_code_redeem",
                ok(json!({"enrolled": true, "renews_by_device_key": false})),
            ),
            ("set_consent_scopes", ok(json!({"saved": true}))),
        ]);
        let mut out = Vec::new();
        let error = login_with_code_with(
            &mut |m, p| script.answer(m, p),
            "tcd1_dA.s",
            &crate::near_ai_login::Consent {
                scopes: vec![],
                chosen: false,
            },
            false,
            &mut out,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), CODE_NO_RENEWAL);
        assert!(!String::from_utf8(out).unwrap().contains(CODE_DONE));
    }

    #[test]
    fn a_refused_code_records_no_consent() {
        let mut script = Script::new(vec![(
            "account_device_code_redeem",
            err("device_code_refused"),
        )]);
        let mut out = Vec::new();
        let error = login_with_code_with(
            &mut |m, p| script.answer(m, p),
            "tcd1_dA.s",
            &crate::near_ai_login::Consent {
                scopes: vec![],
                chosen: false,
            },
            false,
            &mut out,
        )
        .unwrap_err();
        assert!(error.to_string().contains("devices add"), "{error}");
        assert_eq!(script.seen.len(), 1);
    }

    #[test]
    fn revoke_asks_first_and_cancels_on_no() {
        let calls = std::cell::Cell::new(0usize);
        let mut call = |_: &str, _: Value| {
            calls.set(calls.get() + 1);
            ok(json!({"revoked": true}))
        };
        let mut out = Vec::new();
        let error = revoke_with(
            &mut call,
            "sha256:ab",
            false,
            false,
            &mut "n\n".as_bytes(),
            &mut out,
        )
        .unwrap_err();
        assert!(error.to_string().contains("nothing changed"), "{error}");
        assert_eq!(calls.get(), 0);
        let mut out = Vec::new();
        revoke_with(
            &mut call,
            "sha256:ab",
            true,
            false,
            &mut "".as_bytes(),
            &mut out,
        )
        .unwrap();
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn list_marks_this_device() {
        let mut script = Script::new(vec![(
            "account_devices_list",
            ok(json!({"devices": [
                {"device_key_id": "sha256:aa", "label": "laptop", "this_device": true},
                {"device_key_id": "sha256:bb", "this_device": false, "last_seen_at": "2026-10-27T00:00:00Z"},
            ]})),
        )]);
        let mut out = Vec::new();
        list_with(&mut |m, p| script.answer(m, p), false, &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("laptop  (this device)"), "{out}");
        assert!(out.contains("(no name)"), "{out}");
        assert!(
            out.contains("sha256:bb  last seen 2026-10-27T00:00:00Z"),
            "{out}"
        );
    }

    #[test]
    fn the_code_comes_from_the_flag_then_the_environment() {
        assert_eq!(
            code_from(Some(" a "), Some("b"), false).as_deref(),
            Some("a")
        );
        assert_eq!(code_from(None, Some("b"), false).as_deref(), Some("b"));
        assert_eq!(code_from(None, Some("  "), false), None);
        assert_eq!(code_from(None, None, false), None);
    }

    #[test]
    fn a_refused_code_from_the_environment_names_the_variable() {
        let named = naming_the_variable(anyhow!("That code was refused."), true).to_string();
        assert!(named.starts_with("That code was refused."), "{named}");
        assert!(named.contains(CODE_ENV), "{named}");
        assert_eq!(
            naming_the_variable(anyhow!("That code was refused."), false).to_string(),
            "That code was refused."
        );
    }

    #[test]
    fn another_route_wins_over_a_code_left_in_the_environment() {
        assert_eq!(code_from(None, Some("tcd1_spent"), true), None);
        assert_eq!(
            code_from(Some("tcd1_flag"), Some("tcd1_spent"), true).as_deref(),
            Some("tcd1_flag"),
            "the flag itself conflicts with other routes at parse time"
        );
    }
}

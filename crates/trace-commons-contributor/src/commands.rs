//! CLI command implementations: login, whoami, logout, mint-grant.
//!
//! These are thin orchestration layers over `config`, `identity`, and
//! `issuer_client`. They never print raw `user_subject` (only its hash) and
//! never echo issuer response bodies on error.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use trace_commons_operator_client::format::print_table;
use trace_commons_protocol::onboarding::user_subject_hash;

use trace_commons_protocol::onboarding::{
    TRACE_ONBOARD_REQUEST_SCHEMA_VERSION, TraceOnboardClientInfo, TraceOnboardRequest,
};

use crate::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig, allowlist_for,
};
use crate::consent::{
    ConsentAnswers, prompt_consent_answers, scopes_from_answers, validate_scopes,
};
use crate::identity::{
    DeviceIdentity, EnrollmentGrant, build_enroll_request, mint_grant, pem_to_pkcs8_der,
};
use crate::issuer_client::IssuerClient;
use crate::picker;
use crate::source::{SessionRef, SessionTranscript, TraceSource, all_sources};
use crate::submit::{self, SubmitOptions, SubmitOutcome};
use trace_commons_protocol::trace_contribution::ConsentScope;

const UNENROLLED_PREVIEW_NOTICE: &str = "unenrolled preview: deterministic-only redaction; identity \
    fields are placeholders, external privacy filters are ignored to keep pre-enrollment data \
    offline, and nothing was submitted";
const NEAR_AI_FIRST_USE_NOTICE: &str = "notice: this will send redacted-but-unscrubbed message text \
    to NEAR AI under your API key (one-time notice; see `--pii-filter near-ai` in the README for \
    scope).";

// These explicit placeholders exist only so an unenrolled preview can build
// the same local envelope shape without claiming a real contributor identity.
const PREVIEW_ISSUER_URL: &str = "https://unenrolled-preview.invalid";
const PREVIEW_INGEST_URL: &str = "https://unenrolled-preview.invalid";
const PREVIEW_AUDIENCE: &str = "unenrolled-preview-placeholder";
// Canonical tenant ids are `tenant-` plus a SHA-256 hex digest. Keeping the
// placeholder at that exact serialized width makes the envelope size boundary
// independent of whether enrollment has happened.
const PREVIEW_TENANT_ID: &str =
    "tenant-0000000000000000000000000000000000000000000000000000000000000000";
const PREVIEW_INSTANCE_ID: &str = "unenrolled-preview-placeholder";
const PREVIEW_USER_SUBJECT: &str = "unenrolled-preview-placeholder";
const PREVIEW_DEVICE_KEY_ID: &str = "unenrolled-preview-placeholder";

pub(crate) fn unenrolled_preview_config() -> ContributorConfig {
    ContributorConfig {
        schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
        issuer_url: PREVIEW_ISSUER_URL.to_string(),
        ingest_url: PREVIEW_INGEST_URL.to_string(),
        audience: PREVIEW_AUDIENCE.to_string(),
        tenant_id: PREVIEW_TENANT_ID.to_string(),
        instance_id: PREVIEW_INSTANCE_ID.to_string(),
        user_subject: PREVIEW_USER_SUBJECT.to_string(),
        device_key_id: PREVIEW_DEVICE_KEY_ID.to_string(),
        consent_scopes: vec!["debugging_evaluation".to_string()],
        pii_filter: None,
        allowed_hosts: None,
        display_handle: None,
        public_bio: None,
        public_since: None,
    }
}

/// Signals that a JSON submit result has already been rendered to stdout.
/// The binary uses this to return a failing exit status without appending a
/// second JSON document.
#[derive(Debug, thiserror::Error)]
#[error("one or more sessions were refused or failed")]
pub struct RenderedSubmitFailure;

/// The result of a non-interactive enrollment attempt.
///
/// The grant path can be run with no grant in hand yet, in which case it
/// only records this device's identity so an instance operator can vouch
/// for it; every other path produces a saved, usable config.
pub(crate) enum EnrollOutcome {
    Enrolled(Box<ContributorConfig>),
    AwaitingGrant { device_key_id: String },
}

/// Enroll this device with an instance-signed grant or an invite link, with
/// no interaction: `consent_scopes` must already be resolved, since nothing
/// here can prompt a terminal.
///
/// This is the single enrollment implementation shared by the interactive
/// `login` command and the daemon's `enroll` IPC method, so a socket caller
/// (a native application) and a terminal caller enrol identically rather
/// than through two hand-maintained copies of the same network calls.
///
/// When `allowed_hosts` is provided it takes precedence over the
/// `TRACE_COMMONS_ALLOWED_HOSTS` env fallback and is persisted into the
/// saved config so every later command enforces it.
pub(crate) async fn enroll_core(
    store: &ConfigStore,
    grant_b64: Option<&str>,
    invite: Option<&str>,
    allowed_hosts: Option<&str>,
    consent_scopes: Vec<String>,
) -> Result<EnrollOutcome> {
    if grant_b64.is_some() && invite.is_some() {
        anyhow::bail!("--grant and --invite are alternative enrollment paths; pass only one");
    }

    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;

    if let Some(invite) = invite {
        let cfg =
            enroll_with_invite_core(store, invite, allowed_hosts, &device, consent_scopes).await?;
        return Ok(EnrollOutcome::Enrolled(Box::new(cfg)));
    }

    let Some(grant_b64) = grant_b64 else {
        return Ok(EnrollOutcome::AwaitingGrant {
            device_key_id: device.device_key_id,
        });
    };

    // Refuse to overwrite an existing enrollment, exactly like the invite
    // path. `enroll` is now socket-reachable on a continuously-uploading
    // daemon, so without this check a single call could repoint a running
    // daemon's issuer/ingest/tenant out from under it. There is no
    // legitimate re-enrollment flow that needs this to silently overwrite;
    // `logout` first if re-enrolling is actually intended.
    if store
        .load_config()
        .context("loading contributor config")?
        .is_some()
    {
        anyhow::bail!(
            "this device is already enrolled; run `logout` first if you intend to re-enroll"
        );
    }

    let grant = EnrollmentGrant::decode(grant_b64).context("decoding enrollment grant")?;
    let req = build_enroll_request(&grant, &device).context("building enroll request")?;

    // Pre-enrollment there is no saved config yet; the flag takes
    // precedence, else fall back to the env var.
    let allowlist = allowlist_for(allowed_hosts);
    let client = IssuerClient::new(allowlist).context("building issuer client")?;
    let response = client.enroll(&grant.issuer_url, &req).await?;

    let cfg = ContributorConfig {
        schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
        issuer_url: grant.issuer_url.clone(),
        ingest_url: response.ingest_url,
        audience: response.audience,
        tenant_id: response.tenant_id,
        instance_id: grant.attestation.instance_id.clone(),
        user_subject: grant.attestation.user_subject.clone(),
        device_key_id: response.device_key_id,
        consent_scopes,
        pii_filter: None,
        allowed_hosts: allowed_hosts.map(str::to_string),
        display_handle: None,
        public_bio: None,
        public_since: None,
    };
    store
        .save_config(&cfg)
        .context("saving contributor config")?;
    Ok(EnrollOutcome::Enrolled(Box::new(cfg)))
}

/// Enroll this device with an instance-signed grant, or (with no grant)
/// print this device's key id so an instance operator can mint one.
///
/// `scopes` (a CSV of wire-name consent scopes) is validated before any
/// network call. When absent, an interactive terminal prompts for consent
/// choices; a non-interactive session, or one passing `default_consent`,
/// falls back to the `debugging_evaluation` floor only.
pub async fn login(
    store: &ConfigStore,
    grant_b64: Option<&str>,
    invite: Option<&str>,
    allowed_hosts: Option<&str>,
    scopes: Option<&str>,
    default_consent: bool,
) -> Result<()> {
    let consent_scopes = resolve_consent_scopes(scopes, default_consent)?;
    let used_invite = invite.is_some();
    match enroll_core(store, grant_b64, invite, allowed_hosts, consent_scopes).await? {
        EnrollOutcome::AwaitingGrant { device_key_id } => {
            println!("device_key_id: {device_key_id}");
            println!(
                "give this to your instance to mint an enrollment grant, then re-run \
                 `login --grant <grant>` -- or, if you were handed an invite link, run \
                 `login --invite <url>`"
            );
        }
        EnrollOutcome::Enrolled(cfg) if used_invite => {
            println!("enrolled: tenant_id={}", cfg.tenant_id);
            println!("this invite use is now spent");
            println!(
                "run `trace-commons-contributor whoami` to confirm, then \
                 `trace-commons-contributor submit --dry-run` before contributing anything"
            );
        }
        EnrollOutcome::Enrolled(cfg) => {
            println!("enrolled: tenant_id={}", cfg.tenant_id);
            println!(
                "Traces you submit carry the {} consent scope(s); secrets are removed locally \
                 (including tool payloads), and the server re-applies the same deterministic \
                 redaction on receipt. The optional NEAR AI PII pass (--pii-filter near-ai) covers \
                 message text only.",
                cfg.consent_scopes.join(", ")
            );
        }
    }
    Ok(())
}

/// Where this login's consent answers come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConsentSource<'a> {
    /// An explicit `--scopes` CSV, still to be validated.
    Explicit(&'a str),
    /// The default answer (no) to every optional scope.
    DefaultAnswers,
    /// The interactive consent menu.
    Prompt,
}

/// Decide where consent answers come from, given the flags and whether
/// stdin is a terminal. Split out from [`resolve_consent_scopes`] so the
/// precedence is testable without a real terminal.
///
/// `--scopes` wins over everything; `--default` then suppresses the prompt
/// even on a terminal, which is the case TTY detection alone cannot cover:
/// an agent driving this CLI through a pty looks interactive.
fn consent_source(
    scopes: Option<&str>,
    default_consent: bool,
    is_terminal: bool,
) -> ConsentSource<'_> {
    match (scopes, default_consent, is_terminal) {
        (Some(csv), _, _) => ConsentSource::Explicit(csv),
        (None, true, _) => ConsentSource::DefaultAnswers,
        (None, false, true) => ConsentSource::Prompt,
        (None, false, false) => ConsentSource::DefaultAnswers,
    }
}

/// Resolve the consent scopes to request for this login: an explicit
/// `--scopes` CSV wins (validated immediately, before any network call);
/// `--default` takes the default (no) answer for every optional scope; a
/// TTY prompts interactively; a non-interactive session with neither flag
/// falls back to the `debugging_evaluation` floor only.
///
/// The default answers are deliberately the most restrictive ones: nothing
/// beyond the always-on floor is granted unless someone said so.
fn resolve_consent_scopes(scopes: Option<&str>, default_consent: bool) -> Result<Vec<String>> {
    use std::io::IsTerminal;
    match consent_source(scopes, default_consent, std::io::stdin().is_terminal()) {
        ConsentSource::Explicit(csv) => {
            let names: Vec<String> = csv.split(',').map(|s| s.trim().to_string()).collect();
            validate_scopes(&names).context("invalid --scopes value")
        }
        ConsentSource::DefaultAnswers => Ok(scopes_from_answers(ConsentAnswers::default())),
        ConsentSource::Prompt => {
            let mut stdin = std::io::stdin().lock();
            let mut stdout = std::io::stdout();
            let answers = prompt_consent_answers(&mut stdin, &mut stdout)
                .context("reading interactive consent answers")?;
            Ok(scopes_from_answers(answers))
        }
    }
}

/// Print local identity: never the raw `user_subject`, only its hash.
pub fn whoami(store: &ConfigStore, json: bool) -> Result<()> {
    let cfg = store
        .load_config()
        .context("loading contributor config")?
        .context("not logged in; run `login` first")?;
    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;

    if json {
        // The raw user_subject is never emitted, in either mode: it is
        // contributor identity, and this output is exactly what an
        // automating caller will log.
        let out = serde_json::json!({
            "schema_version": "trace_commons.whoami.v1",
            "instance_id": cfg.instance_id,
            "tenant_id": cfg.tenant_id,
            "device_key_id": device.device_key_id,
            "user_subject_hash": user_subject_hash(&cfg.user_subject),
            "config_dir": store.dir().display().to_string(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!("instance_id: {}", cfg.instance_id);
    println!("tenant_id: {}", cfg.tenant_id);
    println!("device_key_id: {}", device.device_key_id);
    println!(
        "user_subject_hash: {}",
        user_subject_hash(&cfg.user_subject)
    );
    println!("config_dir: {}", store.dir().display());
    Ok(())
}

/// Sign in to the contributor's ACCOUNT, as distinct from enrolling this
/// device (`login`).
///
/// The device key can upload; it deliberately cannot withdraw traces or read
/// account history, because a stolen device key must not be worth the ability
/// to delete someone's contribution record. So withdrawal needs the account,
/// and the account is proven the only way it can be: the human completes the
/// ordinary browser login flow, and the browser hands this machine a
/// short-lived token on a loopback redirect.
///
/// The printed URL is the headless path. It carries a single-use code, so it
/// is treated as a secret: it is printed for the person at the keyboard and
/// never logged.
pub async fn account_login(store: &ConfigStore, no_browser: bool, json: bool) -> Result<()> {
    let cfg = store
        .load_config()?
        .context("not enrolled: run `login` first")?;
    let outcome = crate::account_auth::sign_in(store, &cfg, !no_browser, |url| {
        if json {
            // Machine-readable callers still need the URL: in --json mode the
            // whole point is that a wrapper drives the browser itself.
            println!(
                "{}",
                serde_json::json!({
                    "schema_version": "trace_commons.account_login_url.v1",
                    "url": url,
                })
            );
        } else {
            println!("Open this in your browser to finish signing in:\n\n  {url}\n");
            println!("Waiting for the browser...");
        }
    })
    .await?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": "trace_commons.account_login.v1",
                "account_id": outcome.account_id,
                "expires_at": outcome.expires_at,
            }))?
        );
    } else {
        println!("signed in; session expires {}", outcome.expires_at);
    }
    Ok(())
}

/// Report whether a live account session is stored, WITHOUT printing it.
pub fn account_status(store: &ConfigStore, json: bool) -> Result<()> {
    let expires_at = crate::account_auth::session_status(store);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": "trace_commons.account_status.v1",
                "signed_in": expires_at.is_some(),
                "expires_at": expires_at,
            }))?
        );
    } else {
        match expires_at {
            Some(at) => println!("signed in; session expires {at}"),
            None => println!("not signed in; run `account login`"),
        }
    }
    Ok(())
}

/// Revoke the account session server-side and forget it locally. Leaves the
/// device enrollment alone -- that is what `logout` is for.
pub async fn account_logout(store: &ConfigStore) -> Result<()> {
    let cfg = store
        .load_config()?
        .context("not enrolled: run `login` first")?;
    crate::account_auth::sign_out(store, &cfg).await?;
    println!("signed out of your account");
    Ok(())
}

/// Delete all local contributor state (config, device key, receipts).
pub fn logout(store: &ConfigStore) -> Result<()> {
    // Stop a running daemon first. It holds a minted claim that stays valid
    // for minutes, so wiping the state out from under it would leave it
    // uploading against an enrollment the contributor has just revoked, into
    // a receipts file that no longer exists.
    match stop_running_daemon(store) {
        Ok(DaemonStopOutcome::NotRunning) => {}
        Ok(DaemonStopOutcome::Stopped) => println!("stopped the background daemon"),
        Ok(DaemonStopOutcome::AcknowledgedButStillHoldingTheLock) => {
            // Not a failure, and not worth an alarming warning. An
            // FFI-hosted daemon releases `daemon.lock` only in
            // `tc_handle_free` / `tc_daemon_stop`, never on the socket's
            // `"shutdown"` -- so the supervise loop really has stopped, the
            // lock is simply still held by the hosting application. Logout
            // against an embedded daemon therefore *always* waited out the
            // full deadline and then printed a warning about a stop that
            // had in fact happened.
            println!(
                "the background daemon stopped its upload loop; the application \
                 hosting it still holds the state directory and will release it \
                 when it exits."
            );
        }
        Err(_e) => {
            // Never block a logout on this: the wipe below removes the device
            // key, and the daemon refuses to upload without one. A fixed
            // label, not the error text, which can carry a state-directory
            // path.
            tracing::warn!("could not signal the daemon");
            // Say what is actually true: the credentials are gone either way,
            // and the daemon refuses to upload without them.
            println!(
                "warning: the background daemon did not confirm it stopped. Local \
                 credentials have been removed regardless, so it cannot upload \
                 anything; it will exit on its next pass."
            );
        }
    }
    store.wipe().context("wiping contributor state")?;
    let _ = store.remove_daemon_file(crate::config::DAEMON_SOCK_FILE);
    let _ = store.remove_daemon_file(crate::config::DAEMON_LOCK_FILE);
    println!("logged out; local state removed");
    Ok(())
}

/// What actually happened when logout asked a running daemon to stop.
///
/// The distinction that matters is the last variant. A daemon embedded in a
/// native application via the C ABI releases `daemon.lock` only in
/// `tc_handle_free` / `tc_daemon_stop` -- never in response to the socket's
/// `"shutdown"`, which stops the supervise loop and nothing else. Treating
/// a still-held lock as "did not confirm it stopped" made logout against an
/// FFI-hosted daemon wait out the full deadline every single time and then
/// print an alarming warning about a stop that had in fact happened.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DaemonStopOutcome {
    /// No socket, or a stale one from a crashed daemon.
    NotRunning,
    /// It acknowledged and released its lock: fully gone.
    Stopped,
    /// It acknowledged the stop -- so its upload loop has stopped -- but
    /// the lock is still held, which is what an embedded daemon looks like.
    AcknowledgedButStillHoldingTheLock,
}

/// Ask a running daemon to stop, and wait briefly for it to let go of its
/// lock. Reports which of the three things above happened.
fn stop_running_daemon(store: &ConfigStore) -> Result<DaemonStopOutcome> {
    use std::io::{BufRead, BufReader, Write};

    // Both transports are reached the same way the one-shot client reaches
    // them, so this cannot drift from `daemon::client` when one of them
    // changes.
    let mut stream = match crate::daemon::client::connect_for_shutdown(store) {
        Some(s) => s,
        // Nothing listening, or a stale endpoint from a crashed daemon.
        None => return Ok(DaemonStopOutcome::NotRunning),
    };
    stream
        .write_all(b"{\"id\":0,\"method\":\"shutdown\"}\n")
        .context("sending shutdown")?;
    stream.flush().ok();
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply).ok();
    let acknowledged = serde_json::from_str::<crate::daemon::ipc::Response>(reply.trim())
        .map(|r| r.error.is_none())
        .unwrap_or(false);

    // Wait for the lock to be released, which is the daemon actually gone
    // rather than merely acknowledging.
    let lock_path = store.daemon_path(crate::config::DAEMON_LOCK_FILE);
    // Generous, because a standalone daemon finishes the pass it is in
    // before it stops, and a large session store makes that pass take a few
    // seconds.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if !lock_path.exists() {
            return Ok(DaemonStopOutcome::Stopped);
        }
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&lock_path) {
            if f.try_lock().is_ok() {
                return Ok(DaemonStopOutcome::Stopped);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if acknowledged {
        return Ok(DaemonStopOutcome::AcknowledgedButStillHoldingTheLock);
    }
    anyhow::bail!("daemon did not exit within 15s")
}

/// Operator/dogfood tool: mint an enrollment grant with an instance private
/// key and print it (base64) to stdout.
// Arity is fixed by the plan's interface contract for this function.
#[allow(clippy::too_many_arguments)]
pub fn mint_grant_cmd(
    store: &ConfigStore,
    instance_key_pem_path: &Path,
    instance_id: &str,
    user_subject: &str,
    audience: &str,
    issuer_url: &str,
    device_key_id: Option<&str>,
    ttl_seconds: i64,
) -> Result<()> {
    let pem = std::fs::read_to_string(instance_key_pem_path)
        .with_context(|| format!("reading {}", instance_key_pem_path.display()))?;
    let der = pem_to_pkcs8_der(&pem).context("parsing instance key PEM")?;

    let device_key_id = match device_key_id {
        Some(id) => id.to_string(),
        None => {
            DeviceIdentity::load_or_generate(store)
                .context("loading device identity")?
                .device_key_id
        }
    };

    let grant = mint_grant(
        &der,
        issuer_url,
        instance_id,
        user_subject,
        audience,
        &device_key_id,
        ttl_seconds,
        chrono::Utc::now(),
    )
    .context("minting enrollment grant")?;

    println!("{}", grant.encode());
    Ok(())
}

/// Pure predicate for the `--project` filter. Prefers the session's true
/// decoded working directory (`cwd`) for a hyphen-safe, component-wise
/// path-prefix match; falls back to the legacy basename-or-path heuristic
/// only when the true cwd is unavailable.
fn cwd_matches_project(
    cwd: Option<&str>,
    legacy_project: Option<&str>,
    path: &Path,
    project: &Path,
) -> bool {
    if let Some(cwd) = cwd {
        return Path::new(cwd).starts_with(project);
    }
    let basename = project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    legacy_project == Some(basename) || path.starts_with(project)
}

/// Discover every locally discoverable session across all sources, applying
/// optional `source`/`project`/`since` filters. `project` matches the
/// session's true decoded working directory when available; otherwise falls
/// back to the legacy heuristic (basename match or path prefix). `since`
/// filters against `started_at` (falls back to excluding sessions with no
/// timestamp when set).
fn discover_filtered(
    source_filter: Option<&str>,
    project_filter: Option<&Path>,
    since: Option<chrono::Duration>,
    trajectory: Option<&Path>,
) -> Result<Vec<SessionRef>> {
    // An explicitly-supplied path that does not exist is user error, not an
    // empty result. Silent-empty makes a typo indistinguishable from "this
    // file had no sessions". Follows the --project precedent below.
    if let Some(p) = trajectory {
        if !p.exists() {
            anyhow::bail!("--trajectory path {} does not exist", p.display());
        }
    }

    // Resolve `--project` against the real filesystem before matching. A
    // participant standing in their hackathon project types `--project .`
    // (or a relative path, or one crossing a symlink); an unresolved value
    // never prefix-matches an absolute session `cwd`, so the batch would
    // come back empty and look like "this project has no traces".
    let resolved_project = match project_filter {
        None => None,
        Some(p) => Some(std::fs::canonicalize(p).with_context(|| {
            format!("resolving --project path {} (does it exist?)", p.display())
        })?),
    };
    let project_filter = resolved_project.as_deref();

    let mut refs = Vec::new();
    for source in all_sources(None, None, trajectory.map(|p| p.to_path_buf())) {
        if let Some(sf) = source_filter {
            if source.name() != sf {
                continue;
            }
        }
        refs.extend(source.discover().context("discovering local sessions")?);
    }

    let now = Utc::now();
    refs.retain(|r| {
        let project_ok = match project_filter {
            None => true,
            Some(p) => {
                // Canonicalize the session cwd too when it still exists, so a
                // symlinked path (e.g. macOS /tmp -> /private/tmp) compares
                // equal on both sides. A cwd that no longer exists falls back
                // to the raw string.
                let cwd = r.cwd.as_deref().map(|c| {
                    std::fs::canonicalize(c)
                        .map(|abs| abs.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| c.to_string())
                });
                cwd_matches_project(cwd.as_deref(), r.project.as_deref(), &r.path, p)
            }
        };
        let since_ok = match since {
            None => true,
            Some(d) => r.started_at.map(|t| now - t <= d).unwrap_or(false),
        };
        project_ok && since_ok
    });
    Ok(refs)
}

/// Build a fresh `TraceSource` instance for the adapter named `name` (used
/// to pair a previously discovered `SessionRef` with a loadable source).
fn source_for(name: &str, trajectory: Option<&Path>) -> Option<Box<dyn TraceSource>> {
    all_sources(None, None, trajectory.map(|p| p.to_path_buf()))
        .into_iter()
        .find(|s| s.name() == name)
}

/// Human-readable "Nh"/"Nd" age, or "-" when the session has no timestamp.
fn format_age(started_at: Option<chrono::DateTime<Utc>>) -> String {
    match started_at {
        None => "-".to_string(),
        Some(t) => {
            let age = Utc::now() - t;
            if age.num_hours() < 48 {
                format!("{}h", age.num_hours().max(0))
            } else {
                format!("{}d", age.num_days())
            }
        }
    }
}

/// Human-readable byte size (bytes/KB/MB).
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes}B")
    }
}

fn session_row(idx: usize, r: &SessionRef) -> Vec<String> {
    vec![
        (idx + 1).to_string(),
        r.source.to_string(),
        r.project.clone().unwrap_or_else(|| "-".to_string()),
        format_age(r.started_at),
        format_size(r.size_bytes),
    ]
}

/// SUBMITTED marker for the interactive submit picker: `Some(true)` when a
/// receipt with an already-submitted status matches this session's hash,
/// `Some(false)` when not, `None` when the transcript failed to load (the
/// session stays selectable; `submit_sessions` will classify it).
fn submitted_marker(
    source: &dyn TraceSource,
    r: &SessionRef,
    receipts: &[crate::config::Receipt],
) -> Option<bool> {
    let transcript = source.load(r).ok()?;
    Some(receipts.iter().any(|rec| {
        rec.session_hash == transcript.session_hash
            && crate::submit::ALREADY_SUBMITTED_STATUSES.contains(&rec.status.as_str())
    }))
}

/// Row for the submit picker table: the `list` columns plus a SUBMITTED
/// cell ("yes" / "-" / "?" when the transcript could not be loaded).
fn submit_picker_row(idx: usize, r: &SessionRef, submitted: Option<bool>) -> Vec<String> {
    let mut row = session_row(idx, r);
    row.push(
        match submitted {
            Some(true) => "yes",
            Some(false) => "-",
            None => "?",
        }
        .to_string(),
    );
    row
}

/// List every discoverable local session in a numbered table. Never prints
/// full paths -- only the source name, project basename, age, and size.
pub fn list(trajectory: Option<&Path>, json: bool) -> Result<()> {
    let sessions = discover_filtered(None, None, None, trajectory)?;
    if json {
        // Never the full path: it is a local filesystem path and this output
        // is machine-consumed. Source, project basename, and size are what a
        // caller needs to choose a session.
        let items: Vec<serde_json::Value> = sessions
            .iter()
            .map(|r| {
                serde_json::json!({
                    "source": r.source,
                    "project": r.project,
                    "started_at": r.started_at,
                    "size_bytes": r.size_bytes,
                })
            })
            .collect();
        let out = serde_json::json!({
            "schema_version": "trace_commons.session_list.v1",
            "sessions": items,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    if sessions.is_empty() {
        println!("no sessions found");
        return Ok(());
    }
    let rows: Vec<Vec<String>> = sessions
        .iter()
        .enumerate()
        .map(|(i, r)| session_row(i, r))
        .collect();
    print_table(
        &mut std::io::stdout(),
        &["#", "SOURCE", "PROJECT", "AGE", "SIZE"],
        &rows,
    )
    .context("printing session table")?;
    Ok(())
}

/// Options controlling which local sessions `submit` considers and whether
/// it prompts interactively before uploading.
pub struct SubmitSelection<'a> {
    pub all: bool,
    pub since: Option<&'a str>,
    pub project: Option<&'a Path>,
    pub source: Option<&'a str>,
    pub yes: bool,
    pub dry_run: bool,
    pub pii_filter: Option<&'a str>,
    pub manifest: Option<&'a Path>,
    /// Path to a trajectory-v1 file or a directory of them. Trajectory
    /// sessions are only discoverable when this is set.
    pub trajectory: Option<&'a Path>,
    /// Emit machine-readable JSON instead of human lines, for callers
    /// driving this CLI programmatically.
    pub json: bool,
    /// Drop model reasoning from this run. Reasoning is included by default.
    pub no_reasoning: bool,
    /// Re-submit corrected envelopes for locally-known quarantined sessions
    /// under the same submission_id (server supersedes; see #214).
    pub remediate_quarantined: bool,
    /// How the contributor says these sessions went: `worked` or `failed`.
    /// Absent leaves `task_success` `Unknown`, as every envelope this client
    /// has ever sent carried.
    pub verdict: Option<&'a str>,
}

/// Drop reasoning events before envelope construction. Reasoning is captured
/// by default; this is the per-run opt-out behind `--no-reasoning`.
pub(crate) fn strip_reasoning(t: &mut SessionTranscript) {
    t.events
        .retain(|e| e.kind != crate::source::SessionEventKind::Reasoning);
}

/// Discover, filter, (optionally) interactively pick, redact, and submit
/// local sessions. Prints exactly one outcome line per session; returns an
/// error (nonzero exit) if a real submission is refused or any run fails.
pub async fn submit(store: &ConfigStore, sel: &SubmitSelection<'_>) -> Result<()> {
    // A dry run mints envelope ids locally but delivers nothing, so its ids
    // do not exist server-side. Writing them would hand an external collector
    // ids that can never be scored. Refuse up front, before any work.
    if sel.manifest.is_some() && sel.dry_run {
        anyhow::bail!(
            "--manifest cannot be combined with --dry-run: a dry run uploads nothing, \
             so its envelope ids would never exist server-side"
        );
    }
    let saved_cfg = store.load_config().context("loading contributor config")?;
    let (cfg, unenrolled_preview) = match saved_cfg {
        Some(cfg) => (cfg, false),
        None if sel.dry_run => (unenrolled_preview_config(), true),
        None => anyhow::bail!("not logged in; run `login` first"),
    };

    let selected_filter = sel.pii_filter.or(cfg.pii_filter.as_deref());
    let near_ai_notice =
        !unenrolled_preview && selected_filter == Some("near-ai") && !store.near_ai_notice_shown();
    let mut notices = Vec::new();
    if unenrolled_preview {
        notices.push(UNENROLLED_PREVIEW_NOTICE);
    }
    if near_ai_notice {
        notices.push(NEAR_AI_FIRST_USE_NOTICE);
    }
    if !sel.json {
        for notice in &notices {
            println!("{notice}");
        }
    }

    let since = sel.since.map(picker::parse_since).transpose()?;
    let mut refs = discover_filtered(sel.source, sel.project, since, sel.trajectory)?;
    refs.sort_by_key(|r| std::cmp::Reverse(r.started_at));

    if refs.is_empty() {
        println!("no sessions found");
        return Ok(());
    }

    let indices: Vec<usize> = if sel.all || sel.yes {
        (0..refs.len()).collect()
    } else {
        let receipts = store.load_receipts().context("loading receipts")?;
        let rows: Vec<Vec<String>> = refs
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let marker = source_for(r.source, sel.trajectory)
                    .and_then(|src| submitted_marker(src.as_ref(), r, &receipts));
                submit_picker_row(i, r, marker)
            })
            .collect();
        print_table(
            &mut std::io::stdout(),
            &["#", "SOURCE", "PROJECT", "AGE", "SIZE", "SUBMITTED"],
            &rows,
        )
        .context("printing session table")?;
        println!("Select sessions to submit (e.g. 3, 1,3-5, or 'all'):");
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .context("reading selection from stdin")?;
        picker::parse_selection(&line, refs.len())?
    };

    let pairs: Vec<(Box<dyn TraceSource>, SessionRef)> = indices
        .into_iter()
        .map(|i| {
            let r = refs[i].clone();
            let src = source_for(r.source, sel.trajectory)
                .with_context(|| format!("no adapter registered for source '{}'", r.source))?;
            Ok((src, r))
        })
        .collect::<Result<_>>()?;

    // Refuse an unrecognised verdict rather than silently submitting the
    // whole run as `Unknown`: a contributor who typed it meant to say
    // something, and a typo must not quietly discard the answer.
    let verdict =
        match sel.verdict {
            None => None,
            Some(name) => Some(crate::envelope::ContributorVerdict::parse(name).ok_or_else(
                || anyhow::anyhow!("unknown --outcome '{name}': use worked, partly or failed"),
            )?),
        };
    let opts = SubmitOptions {
        dry_run: sel.dry_run,
        pii_filter: sel.pii_filter.map(str::to_string),
        no_reasoning: sel.no_reasoning,
        machine_readable: sel.json,
        unenrolled_preview,
        remediate_quarantined: sel.remediate_quarantined,
        verdict,
    };
    let outcomes = submit::submit_sessions(store, &cfg, pairs, &opts).await?;

    if let Some(path) = sel.manifest {
        let entries = submit::build_manifest(&outcomes);
        let json = serde_json::to_string_pretty(&entries).context("serializing manifest")?;
        std::fs::write(path, json)
            .with_context(|| format!("writing manifest to {}", path.display()))?;
        println!("wrote {} envelope id(s) to manifest", entries.len());
    }

    if sel.json {
        let document = submit::outcomes_to_json(&outcomes, unenrolled_preview, &notices);
        println!("{}", serde_json::to_string_pretty(&document)?);
        if submit::outcomes_have_failure(&outcomes, sel.dry_run) {
            return Err(RenderedSubmitFailure.into());
        }
        return Ok(());
    }

    let preview_prefix = if unenrolled_preview {
        "unenrolled-preview "
    } else {
        ""
    };
    for outcome in &outcomes {
        match outcome {
            SubmitOutcome::Submitted {
                submission_id,
                status,
            } => {
                if unenrolled_preview {
                    println!("{preview_prefix}previewed {submission_id} {status}");
                } else {
                    println!("submitted {submission_id} {status}");
                }
                // "quarantined" reads as rejection to a first-time
                // contributor. It is not: the trace was delivered and is
                // held pending operator privacy review. Say so at the moment
                // they see the word, not only if they later run `status`.
                if status == "quarantined" {
                    println!(
                        "  held for privacy review, not rejected; credit is 0.00 until it \
                         completes. Run `status` for the server's explanation."
                    );
                }
            }
            SubmitOutcome::AlreadySubmitted {
                submission_id,
                prior_status,
            } => {
                // Name the status it already has. "already-submitted" alone
                // reads as a failure when it usually means the trace was
                // accepted on an earlier run.
                println!("{preview_prefix}already-submitted {submission_id} ({prior_status})");
            }
            SubmitOutcome::SkippedParseFailure { reason_label } => {
                println!("{preview_prefix}skipped ({reason_label})");
            }
            SubmitOutcome::Refused {
                reason_label,
                session_ref,
                size_bytes,
                limit_bytes,
            } => {
                if let (Some(size), Some(limit)) = (size_bytes, limit_bytes) {
                    println!(
                        "{preview_prefix}refused ({reason_label}) session={session_ref} \
                         size={size} limit={limit}"
                    );
                } else {
                    println!("{preview_prefix}refused ({reason_label}) session={session_ref}");
                }
            }
            SubmitOutcome::Failed { reason_label } => {
                println!("{preview_prefix}failed ({reason_label})");
            }
        }
    }

    if submit::outcomes_have_failure(&outcomes, sel.dry_run) {
        anyhow::bail!("one or more sessions were refused or failed to submit");
    }
    Ok(())
}

/// Render a comma-joined list of wire-name consent scopes for the status
/// table; an empty slice renders as `"-"`.
pub(crate) fn scopes_cell(scopes: &[ConsentScope]) -> String {
    if scopes.is_empty() {
        return "-".to_string();
    }
    scopes
        .iter()
        .map(|scope| {
            serde_json::to_value(scope)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Print server-side status for every locally recorded submission receipt.
pub async fn profile(
    store: &ConfigStore,
    handle: Option<&str>,
    bio: Option<&str>,
    no_bio: bool,
    withdraw: bool,
    json: bool,
) -> Result<()> {
    let mut cfg = store
        .load_config()
        .context("loading contributor config")?
        .context("not logged in; run `login` first")?;

    // Deliberately no local public_attribution check. `cfg.consent_scopes`
    // records what was selected for submissions, while these calls mint an
    // empty-scope claim that the issuer resolves to the caller's full grant
    // ceiling - so the local set can be narrower than what the credential
    // actually carries, and checking it here would refuse contributors the
    // server would have allowed. The server is the authority; the context
    // below carries the remedy if it refuses.

    if withdraw {
        submit::clear_profile(store, &cfg).await?;
        // Same local cache the daemon's `clear_public_profile` drops, through
        // the same helper. A CLI withdrawal that left the cache in place
        // would keep the daemon polling the roster for a row that is gone.
        let cache_written = submit::clear_cached_public_profile(store, &mut cfg);
        if json {
            println!(
                "{}",
                serde_json::json!({"withdrawn": true, "handle_persisted": cache_written})
            );
        } else {
            println!("public attribution withdrawn; the row goes at the next snapshot");
            if !cache_written {
                println!(
                    "note: withdrawn on the server, but this machine's local copy could not \
                     be updated"
                );
            }
        }
        return Ok(());
    }

    let Some(handle) = handle else {
        anyhow::bail!("nothing to do: pass --handle <name> or --withdraw");
    };
    // The server upserts with `bio = excluded.bio`, so this call replaces the
    // whole profile - there is no "leave the bio alone". Requiring the choice
    // is the difference between clearing a published bio because you were
    // asked, and clearing it because you renamed your handle.
    if bio.is_none() && !no_bio {
        anyhow::bail!(
            "setting a handle replaces your whole public profile: pass --bio <text> \
             to publish one, or --no-bio to publish none"
        );
    }

    let profile = submit::set_profile(store, &cfg, handle, bio)
        .await
        .context(
            "setting your public handle (this needs the public_attribution scope; if the \
             server refuses, re-run `login` with --scopes debugging_evaluation,public_attribution)",
        )?;
    // Cache what the server reported, through the same helper the daemon's
    // `set_public_profile` uses. This is not cosmetic bookkeeping: there is
    // no read-back endpoint, so without this write nothing on the machine
    // knows a handle was claimed. `daemon::refresh_community` polls the
    // roster only when the config names a handle, so a contributor who
    // claimed one through the CLI -- the flow that ships today -- would never
    // see their community standing appear.
    let cache_written = submit::cache_public_profile(store, &mut cfg, &profile);
    if json {
        println!(
            "{}",
            serde_json::json!({
                "display_handle": profile.display_handle,
                "bio": profile.bio,
                "public_since": profile.public_since,
                "handle_persisted": cache_written,
            })
        );
    } else {
        println!("public handle: {}", profile.display_handle);
        println!("public since: {}", profile.public_since);
        println!("your handle appears once an accepted submission lands in the window");
        if !cache_written {
            // Published either way; only the local read-back is missing.
            println!(
                "note: published, but this machine could not record it, so the desktop app \
                 will not show your community standing until you run this again"
            );
        }
    }
    Ok(())
}

pub async fn status(store: &ConfigStore) -> Result<()> {
    let cfg = store
        .load_config()
        .context("loading contributor config")?
        .context("not logged in; run `login` first")?;

    let updates = submit::status(store, &cfg).await?;
    if updates.is_empty() {
        println!("no submissions found");
        return Ok(());
    }

    let rows: Vec<Vec<String>> = updates
        .iter()
        .map(|u| {
            vec![
                u.submission_id.to_string(),
                u.status.clone(),
                scopes_cell(&u.consent_scopes),
                format!("{:.2}", u.credit_points_pending),
                u.credit_points_final
                    .map(|f| format!("{f:.2}"))
                    .unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    print_table(
        &mut std::io::stdout(),
        &["SUBMISSION", "STATUS", "SCOPES", "PENDING", "FINAL"],
        &rows,
    )
    .context("printing status table")?;

    // The server already explains a non-accepted status, and the table drops
    // it. Quarantine in particular means "held for operator privacy review",
    // not "rejected" -- a contributor who only sees the word reads it as
    // failure and has nothing to act on.
    let explained: Vec<&trace_commons_protocol::trace_contribution::TraceSubmissionStatusUpdate> =
        updates
            .iter()
            .filter(|u| !u.explanation.is_empty() || !u.delayed_credit_explanations.is_empty())
            .collect();
    if !explained.is_empty() {
        println!();
        for u in explained {
            println!("{} ({}):", u.submission_id, u.status);
            for line in u
                .explanation
                .iter()
                .chain(u.delayed_credit_explanations.iter())
            {
                println!("  {line}");
            }
        }
    }

    // #444: coverage. The server scores a bounded number of sections of a long
    // trace, so a gate result on a capped trace speaks for the part that was
    // read, not the whole. Say so, or a contributor reads it as a verdict on
    // their work.
    //
    // Best-effort by construction: this is one extra read of an endpoint the
    // CLI already calls, and a caveat is worth less than the table above. Any
    // failure -- offline, an older server, an attestation shape this build
    // does not know -- leaves `status` exactly as it was.
    if let Ok(attestation) = submit::fetch_score_attestation(store, &cfg).await {
        if let Some(lines) = partial_coverage_lines(&attestation) {
            if !lines.is_empty() {
                println!();
                println!("coverage:");
                for (submission_id, sentence) in lines {
                    println!("  {submission_id}: {sentence}");
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_cell_renders_wire_names() {
        use trace_commons_protocol::trace_contribution::ConsentScope;
        assert_eq!(scopes_cell(&[]), "-");
        assert_eq!(
            scopes_cell(&[
                ConsentScope::DebuggingEvaluation,
                ConsentScope::ModelTraining
            ]),
            "debugging_evaluation,model_training"
        );
    }

    #[test]
    fn submit_picker_marks_already_submitted_fixture_session() {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
        let src = crate::source::claude_code::ClaudeCodeSource::new(root);
        let r = src.discover().unwrap().remove(0);
        let transcript = src.load(&r).unwrap();

        // No receipts: not submitted, cell renders "-".
        assert_eq!(submitted_marker(&src, &r, &[]), Some(false));
        let row = submit_picker_row(0, &r, Some(false));
        assert_eq!(row.last().unwrap(), "-");

        // Matching receipt with an already-submitted status: "yes".
        let receipt = crate::config::Receipt {
            submission_id: uuid::Uuid::new_v4(),
            session_hash: transcript.session_hash.clone(),
            source: r.source.to_string(),
            submitted_at: chrono::Utc::now(),
            status: "accepted".into(),
        };
        assert_eq!(
            submitted_marker(&src, &r, std::slice::from_ref(&receipt)),
            Some(true)
        );
        let row = submit_picker_row(0, &r, Some(true));
        assert_eq!(row.last().unwrap(), "yes");

        // Receipt with a non-terminal status does not mark the session.
        let mut rejected = receipt;
        rejected.status = "rejected".into();
        assert_eq!(submitted_marker(&src, &r, &[rejected]), Some(false));

        // Load failure renders "?" and stays selectable.
        let row = submit_picker_row(0, &r, None);
        assert_eq!(row.last().unwrap(), "?");
    }

    #[test]
    fn scopes_flag_error_is_flag_scoped_not_stored_config() {
        let err = resolve_consent_scopes(Some("bogus"), false).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("--scopes"), "{msg}");
        assert!(
            msg.contains("bogus") && msg.contains("model_training"),
            "{msg}"
        );
        assert!(!msg.contains("stored config"), "{msg}");
    }

    #[test]
    fn default_flag_skips_the_prompt_even_on_a_terminal() {
        // The whole point of `--default`: an agent driving the CLI through a
        // pty still has a terminal on stdin, so TTY detection alone cannot
        // save it from the interactive consent menu.
        assert_eq!(
            consent_source(None, true, true),
            ConsentSource::DefaultAnswers
        );
        assert_eq!(
            consent_source(None, true, false),
            ConsentSource::DefaultAnswers
        );
        // Without the flag, a terminal still prompts.
        assert_eq!(consent_source(None, false, true), ConsentSource::Prompt);
        assert_eq!(
            consent_source(None, false, false),
            ConsentSource::DefaultAnswers
        );
        // An explicit --scopes wins over both.
        assert_eq!(
            consent_source(Some("model_training"), true, true),
            ConsentSource::Explicit("model_training")
        );
    }

    #[test]
    fn default_consent_grants_only_the_floor_scope() {
        assert_eq!(
            resolve_consent_scopes(None, true).unwrap(),
            vec!["debugging_evaluation".to_string()]
        );
    }

    #[test]
    fn non_tty_default_falls_back_to_debugging_evaluation_only() {
        // `cargo test` runs with stdin that is not a terminal, so this
        // exercises the non-interactive silent-default branch rather than
        // the interactive prompt path.
        let scopes = resolve_consent_scopes(None, false).unwrap();
        assert_eq!(scopes, vec!["debugging_evaluation".to_string()]);
    }

    #[tokio::test]
    async fn login_rejects_issuer_host_off_allowlist_and_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap();
        // Grant issuer host is 127.0.0.1; the allowlist only permits
        // api.example, so login must fail before any request is sent.
        let grant = mint_grant(
            doc.as_ref(),
            "http://127.0.0.1:9",
            "instance-1",
            "alice",
            "aud",
            &device.device_key_id,
            300,
            chrono::Utc::now(),
        )
        .unwrap();
        let err = login(
            &store,
            Some(&grant.encode()),
            None,
            Some("api.example"),
            None,
            false,
        )
        .await
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("not on the allowed-hosts list"), "{msg}");
        // No config was persisted.
        assert!(store.load_config().unwrap().is_none());
    }

    #[tokio::test]
    async fn a_grant_enrollment_refuses_to_overwrite_an_existing_one() {
        // `enroll` is socket-reachable on a continuously-uploading daemon
        // now; without this check, one call could repoint a running
        // daemon's issuer/ingest/tenant out from under it. The grant path
        // used to overwrite silently while the invite path already refused
        // -- this closes that gap.
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        let existing = ContributorConfig {
            schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
            issuer_url: "https://issuer.original.invalid".to_string(),
            ingest_url: "https://ingest.original.invalid".to_string(),
            audience: "aud".to_string(),
            tenant_id: "tenant-original".to_string(),
            instance_id: "instance-original".to_string(),
            user_subject: "alice".to_string(),
            device_key_id: device.device_key_id.clone(),
            consent_scopes: vec!["debugging_evaluation".to_string()],
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
        };
        store.save_config(&existing).unwrap();

        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap();
        let grant = mint_grant(
            doc.as_ref(),
            "https://issuer.attacker.invalid",
            "instance-attacker",
            "alice",
            "aud",
            &device.device_key_id,
            300,
            chrono::Utc::now(),
        )
        .unwrap();
        let err = login(&store, Some(&grant.encode()), None, None, None, false)
            .await
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("already enrolled"), "{msg}");
        // The original config must survive untouched: no repointing to a
        // different issuer/ingest/tenant.
        let cfg = store.load_config().unwrap().unwrap();
        assert_eq!(cfg.issuer_url, "https://issuer.original.invalid");
        assert_eq!(cfg.tenant_id, "tenant-original");
    }

    /// A local issuer that hands back a fixed claim, and a local ingest that
    /// answers the profile PUT/DELETE. Neither validates anything: these
    /// tests are about what the command does with the answer, not about the
    /// protocol, which `submit`'s own tests pin.
    async fn spawn_stub(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    fn stub_issuer() -> axum::Router {
        axum::Router::new().route(
            "/v1/trace-upload-claim",
            axum::routing::post(|| async {
                axum::Json(serde_json::json!({
                    "access_token": "stub-claim-jwt",
                    "token_type": "Bearer",
                    "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                    "expires_in": 300,
                    "consent_scopes": ["debugging_evaluation"],
                    "allowed_uses": ["debugging"],
                }))
            }),
        )
    }

    fn stub_profile_ingest() -> axum::Router {
        axum::Router::new().route(
            "/v1/community/profile",
            axum::routing::put(|| async {
                axum::Json(serde_json::json!({
                    "display_handle": "quiet-otter",
                    "handle_normalized": "quiet-otter",
                    "bio": "Ships billing systems by day.",
                    "public_since": "2026-07-09T10:30:00Z",
                    "last_updated_at": "2026-07-09T10:30:00Z",
                    "update_count": 0,
                }))
            })
            .delete(|| async { axum::http::StatusCode::NO_CONTENT }),
        )
    }

    #[tokio::test]
    async fn the_cli_profile_command_records_the_handle_it_claimed() {
        // The CLI is the flow that ships today, and there is no
        // `GET /v1/community/profile`. A `profile --handle` that publishes
        // without writing the local cache leaves the contributor on the
        // roster with nothing on the machine that knows it: the daemon's
        // `get_public_profile` answers `on_roster: false`, and
        // `refresh_community` takes its no-handle branch and never polls, so
        // the History screen's community section never appears for them.
        let issuer = spawn_stub(stub_issuer()).await;
        let ingest = spawn_stub(stub_profile_ingest()).await;
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        let mut cfg = enrolled_with_a_claimed_handle(&device.device_key_id);
        cfg.issuer_url = issuer;
        cfg.ingest_url = ingest;
        cfg.allowed_hosts = Some("127.0.0.1".to_string());
        cfg.display_handle = None;
        cfg.public_bio = None;
        cfg.public_since = None;
        store.save_config(&cfg).unwrap();

        profile(
            &store,
            Some("quiet-otter"),
            Some("Ships billing systems by day."),
            false,
            false,
            true,
        )
        .await
        .unwrap();

        let saved = store.load_config().unwrap().unwrap();
        assert_eq!(saved.display_handle.as_deref(), Some("quiet-otter"));
        assert_eq!(
            saved.public_bio.as_deref(),
            Some("Ships billing systems by day.")
        );
        assert!(saved.public_since.is_some());

        // ...and a withdrawal takes all three back off again, so the daemon
        // stops polling the roster for a row that no longer exists.
        profile(&store, None, None, false, true, true)
            .await
            .unwrap();
        let saved = store.load_config().unwrap().unwrap();
        assert!(saved.display_handle.is_none());
        assert!(saved.public_bio.is_none());
        assert!(saved.public_since.is_none());
    }

    /// A config with a public handle already claimed on this machine.
    fn enrolled_with_a_claimed_handle(device_key_id: &str) -> ContributorConfig {
        ContributorConfig {
            schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
            issuer_url: "https://issuer.original.invalid".to_string(),
            ingest_url: "https://ingest.original.invalid".to_string(),
            audience: "aud".to_string(),
            tenant_id: "tenant-original".to_string(),
            instance_id: "instance-original".to_string(),
            user_subject: "alice".to_string(),
            device_key_id: device_key_id.to_string(),
            consent_scopes: vec!["debugging_evaluation".to_string()],
            pii_filter: None,
            allowed_hosts: None,
            display_handle: Some("quiet-otter".to_string()),
            public_bio: Some("Ships billing systems by day.".to_string()),
            public_since: Some(chrono::Utc::now()),
        }
    }

    #[tokio::test]
    async fn re_enrolling_cannot_destroy_a_claimed_public_handle() {
        // Both enrollment constructors write `display_handle: None`, which
        // would be a real loss if either could run over an existing config:
        // the server row would survive with no way to read it back, so the
        // handle would exist publicly and be invisible locally forever.
        //
        // It is safe because neither path can run over an existing config --
        // both refuse before any config write, and before the network call
        // that would produce a response to write. So the `None` is the value
        // for a device with no prior enrollment, which by construction has no
        // prior handle. `logout` (`ConfigStore::wipe`) removes the config
        // file outright and is the one operation that is *meant* to take the
        // handle with it.
        //
        // That makes this a test about the refusals holding, since the day
        // one of them stops refusing is the day the `None` becomes data loss.
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        store
            .save_config(&enrolled_with_a_claimed_handle(&device.device_key_id))
            .unwrap();

        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap();
        let grant = mint_grant(
            doc.as_ref(),
            "https://issuer.other.invalid",
            "instance-other",
            "alice",
            "aud",
            &device.device_key_id,
            300,
            chrono::Utc::now(),
        )
        .unwrap();
        let err = login(&store, Some(&grant.encode()), None, None, None, false)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("already enrolled"));

        // The invite path refuses before the redemption call, so an
        // unreachable issuer host in the invite is never contacted.
        let err = enroll_with_invite_core(
            &store,
            "https://issuer.other.invalid/onboard#VQWWPGYSG8Y4LTP6",
            None,
            &device,
            vec!["debugging_evaluation".to_string()],
        )
        .await
        .unwrap_err();
        assert!(format!("{err:#}").contains("already enrolled"));

        let cfg = store.load_config().unwrap().unwrap();
        assert_eq!(cfg.display_handle.as_deref(), Some("quiet-otter"));
        assert_eq!(
            cfg.public_bio.as_deref(),
            Some("Ships billing systems by day.")
        );
        assert!(cfg.public_since.is_some());
    }

    #[test]
    fn strip_reasoning_removes_only_reasoning_events() {
        use crate::source::{SessionEvent, SessionEventKind};
        let mk = |kind: SessionEventKind| SessionEvent {
            kind,
            timestamp: None,
            content: Some("x".to_string()),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
            tool_call_id: None,
            success: None,
        };
        let mut t = crate::source::SessionTranscript {
            source: std::borrow::Cow::Borrowed("claude-code"),
            agent_version: None,
            model: None,
            project: None,
            cwd: None,
            started_at: None,
            session_hash: "sha256:aa".to_string(),
            conversation_id: None,
            events: vec![
                mk(SessionEventKind::User),
                mk(SessionEventKind::Reasoning),
                mk(SessionEventKind::Assistant),
            ],
            subagent_count: 0,
            subagents_dropped: 0,
        };
        super::strip_reasoning(&mut t);
        let kinds: Vec<_> = t.events.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![SessionEventKind::User, SessionEventKind::Assistant]
        );
    }
}

#[cfg(test)]
mod project_filter_tests {
    use super::cwd_matches_project;
    use std::path::Path;

    #[tokio::test]
    async fn manifest_with_dry_run_is_refused_before_any_upload() {
        // A dry run mints envelope ids locally but delivers nothing, so a
        // manifest written from its outcomes would hand devfolio ids the
        // server has never seen. The combination must be refused up front.
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let manifest = dir.path().join("ids.json");
        let sel = super::SubmitSelection {
            all: true,
            since: None,
            project: None,
            source: None,
            yes: true,
            dry_run: true,
            pii_filter: None,
            manifest: Some(&manifest),
            trajectory: None,
            json: false,
            no_reasoning: false,
            remediate_quarantined: false,
            verdict: None,
        };

        let error = super::submit(&store, &sel).await.expect_err("refused");
        assert!(
            error.to_string().contains("--dry-run"),
            "unexpected error: {error}"
        );
        // Refused BEFORE the not-logged-in check, i.e. before any work.
        assert!(!manifest.exists(), "no manifest is written on refusal");
    }

    #[test]
    fn project_filter_resolves_relative_and_dot_paths() {
        // The primary devfolio path: a participant stands in their project
        // and types `--project .`. An unresolved "." never prefix-matches an
        // absolute session cwd, so the filter must canonicalize first.
        let dir = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(dir.path()).unwrap();
        let resolved = std::fs::canonicalize(Path::new(".")).unwrap();
        assert!(resolved.is_absolute(), "canonicalize yields absolute paths");
        assert!(cwd_matches_project(
            Some(project.join("sub").to_str().unwrap()),
            None,
            Path::new("/x.jsonl"),
            &project,
        ));
    }

    #[test]
    fn true_cwd_prefix_matches_including_hyphenated_name() {
        // Project literally named "my-hack" — the legacy basename would decode
        // to "hack" and miss it; the true cwd matches exactly.
        let cwd = Some("/Users/dev/code/my-hack");
        assert!(cwd_matches_project(
            cwd,
            Some("hack"),
            Path::new("/Users/dev/.claude/projects/-Users-dev-code-my-hack/s.jsonl"),
            Path::new("/Users/dev/code/my-hack"),
        ));
    }

    #[test]
    fn discover_filtered_includes_trajectory_only_when_a_path_is_given() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.json");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(
            br#"[{"role":"meta","source":"pi"},
                 {"role":"user","content":"hi","timestamp":"2026-07-10T12:00:00Z"}]"#,
        )
        .unwrap();

        let without = super::discover_filtered(Some("trajectory"), None, None, None).unwrap();
        assert!(
            without.is_empty(),
            "trajectory files must never appear without an explicit path"
        );

        let with = super::discover_filtered(Some("trajectory"), None, None, Some(&p)).unwrap();
        assert_eq!(with.len(), 1);
        assert_eq!(with[0].source, crate::source::SOURCE_TRAJECTORY);
    }

    #[test]
    fn nonexistent_trajectory_path_is_an_error() {
        let err =
            super::discover_filtered(None, None, None, Some(Path::new("/nonexistent/x.json")))
                .unwrap_err()
                .to_string();
        assert!(err.contains("does not exist"), "got: {err}");
    }

    #[test]
    fn true_cwd_excludes_sibling_and_prefix_collision() {
        // Sibling dir and a "my-hack-2" name must NOT match "my-hack".
        assert!(!cwd_matches_project(
            Some("/Users/dev/code/other"),
            None,
            Path::new("/x.jsonl"),
            Path::new("/Users/dev/code/my-hack"),
        ));
        assert!(!cwd_matches_project(
            Some("/Users/dev/code/my-hack-2"),
            None,
            Path::new("/x.jsonl"),
            Path::new("/Users/dev/code/my-hack"),
        ));
    }

    #[test]
    fn falls_back_to_basename_or_path_prefix_when_cwd_unknown() {
        // No true cwd available -> legacy heuristic: basename match ...
        assert!(cwd_matches_project(
            None,
            Some("my-hack"),
            Path::new("/somewhere/s.jsonl"),
            Path::new("/Users/dev/code/my-hack"),
        ));
        // ... or session-file path prefix.
        assert!(cwd_matches_project(
            None,
            None,
            Path::new("/Users/dev/code/my-hack/s.jsonl"),
            Path::new("/Users/dev/code/my-hack"),
        ));
        // Neither matches -> false.
        assert!(!cwd_matches_project(
            None,
            Some("other"),
            Path::new("/elsewhere/s.jsonl"),
            Path::new("/Users/dev/code/my-hack"),
        ));
    }
}

/// Redeem an invite link: register this device with the issuer and write
/// `contributor.json` from the response.
///
/// This exists so an agent does not have to hand-roll `POST /v1/onboard`,
/// base64 a raw Ed25519 public key, and then know that the response has to be
/// persisted. Every one of those was a step contributors got wrong by reading
/// the source instead of a document.
async fn enroll_with_invite_core(
    store: &ConfigStore,
    invite: &str,
    allowed_hosts: Option<&str>,
    device: &DeviceIdentity,
    consent_scopes: Vec<String>,
) -> Result<ContributorConfig> {
    let parsed = parse_invite(invite)?;

    // Redeeming spends one use of the invite whether or not the config write
    // later succeeds, so refuse before the network call rather than burning
    // the invite on a device that is already enrolled.
    if store
        .load_config()
        .context("loading contributor config")?
        .is_some()
    {
        anyhow::bail!(
            "this device is already enrolled; redeeming an invite would spend one of its uses              for nothing. Run `logout` first if you intend to re-enroll."
        );
    }

    let req = TraceOnboardRequest {
        schema_version: TRACE_ONBOARD_REQUEST_SCHEMA_VERSION.to_string(),
        invite_code: parsed.code.clone(),
        device_public_key: device.public_key_b64.clone(),
        client_info: TraceOnboardClientInfo {
            agent: "trace-commons-contributor".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    };

    let client =
        IssuerClient::new(allowlist_for(allowed_hosts)).context("building issuer client")?;
    let response = client.onboard(&parsed.issuer_url, &req).await?;

    let cfg = ContributorConfig {
        schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
        issuer_url: parsed.issuer_url.clone(),
        ingest_url: response.ingest_url,
        audience: response.audience,
        tenant_id: response.tenant_id,
        // An invite enrolls a device directly, with no instance vouching for
        // it, so there is no instance identity to record.
        instance_id: String::new(),
        user_subject: response
            .contributor_label
            .clone()
            .unwrap_or_else(|| device.device_key_id.clone()),
        device_key_id: response.device_key_id,
        consent_scopes,
        pii_filter: None,
        allowed_hosts: allowed_hosts.map(str::to_string),
        display_handle: None,
        public_bio: None,
        public_since: None,
    };
    store
        .save_config(&cfg)
        .context("saving contributor config")?;
    Ok(cfg)
}

/// Fetch a server-signed attestation of this contributor's own scores and
/// write it out.
///
/// This is what a contributor hands to a collector instead of a list of
/// submission ids. An id list is forgeable by anyone who learns the ids --
/// they have been published in plain text before now -- whereas forging an
/// attestation requires the server's signing key.
/// Coverage caveats to print under the `status` table, as
/// `(submission_id, sentence)` pairs, for submissions only partly scored.
///
/// #444: the gate can only afford to score a bounded number of sections of a
/// long trace. Measured on the pilot, a capped decision reads about a third
/// of its trace, and one carried 2,362 sections against a cap of 16. A
/// contributor seeing a bare gate result has no way to know their work was
/// not read in full, which reads as a verdict on the work rather than on our
/// budget.
///
/// The attestation already carries this per submission, but `attest` prints
/// the raw signed JWT -- a thing to hand a collector, not to read.
///
/// **Display only.** The signature is deliberately NOT verified here: this is
/// a contributor reading their own scores back from their own server over an
/// already-authenticated call. Verification is the collector's job and that
/// path is untouched. Nothing here is a trust boundary, which is why a
/// malformed attestation returns `None` rather than erroring -- showing the
/// submission table matters more than showing a caveat.
fn partial_coverage_lines(attestation: &str) -> Option<Vec<(String, String)>> {
    use base64::Engine as _;

    let payload = attestation.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let submissions = claims.get("submissions")?.as_array()?;

    let mut lines = Vec::new();
    for entry in submissions {
        let Some(coverage) = entry.get("coverage") else {
            continue;
        };
        let Some(scored) = coverage.get("chunks_scored").and_then(|v| v.as_u64()) else {
            continue;
        };
        let sentence = match coverage.get("coverage_state").and_then(|v| v.as_str()) {
            // A fully scored trace needs no caveat.
            Some("complete") | None => continue,
            Some("partial") => match coverage.get("chunks_total").and_then(|v| v.as_u64()) {
                Some(total) => format!(
                    "scored {scored} of {total} sections; the rest was not read, so any gate result covers only the part that was scored"
                ),
                // State says partial but no denominator survived: report what
                // is known rather than inventing one.
                None => format!(
                    "scored {scored} sections; the rest was not read, so any gate result covers only the part that was scored"
                ),
            },
            Some(_) => format!(
                "scored {scored} sections; the rest was not read, and how many there were was not recorded for this submission"
            ),
        };
        let id = entry
            .get("submission_id")
            .and_then(|v| v.as_str())
            .unwrap_or("(unknown submission)")
            .to_string();
        lines.push((id, sentence));
    }
    Some(lines)
}

pub async fn attest(store: &ConfigStore, out: Option<&Path>, json: bool) -> Result<()> {
    let cfg = store
        .load_config()
        .context("loading contributor config")?
        .context("not logged in; run `login` first")?;

    let attestation = submit::fetch_score_attestation(store, &cfg).await?;

    if let Some(path) = out {
        std::fs::write(path, &attestation)
            .with_context(|| format!("writing attestation to {}", path.display()))?;
    }

    if json {
        let value = serde_json::json!({
            "schema_version": "trace_commons.attest_result.v1",
            "attestation": attestation,
            "written_to": out.map(|p| p.display().to_string()),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if let Some(path) = out {
        println!("wrote score attestation to {}", path.display());
        println!("hand this to your collector; they verify it against the server keyset");
    } else {
        println!("{attestation}");
    }
    Ok(())
}

/// An invite as handed to a contributor: the issuer origin plus the code.
#[derive(Debug, PartialEq)]
pub(crate) struct ParsedInvite {
    pub issuer_url: String,
    pub code: String,
}

/// Parse an invite link into its issuer origin and code.
///
/// Contributors are handed a URL like
/// `https://issuer.example.ai/onboard#VQWWPGYSG8Y4LTP6`. The code is the
/// fragment; a `?code=` query parameter is also accepted because some clients
/// strip fragments. A bare code is rejected: without an origin there is
/// nothing to POST to, and guessing a default issuer would silently send an
/// invite to the wrong host.
pub(crate) fn parse_invite(raw: &str) -> Result<ParsedInvite> {
    let raw = raw.trim();
    let url = reqwest::Url::parse(raw)
        .map_err(|_| anyhow::anyhow!("--invite must be the full invite URL, not a bare code"))?;
    if !matches!(url.scheme(), "https" | "http") {
        anyhow::bail!("--invite must be an http(s) URL");
    }
    let code = url
        .fragment()
        .map(str::to_string)
        .filter(|f| !f.is_empty())
        .or_else(|| {
            url.query_pairs()
                .find(|(k, _)| k == "code")
                .map(|(_, v)| v.into_owned())
        })
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("invite URL carries no code (expected #CODE or ?code=CODE)")
        })?;
    let mut origin = url.clone();
    origin.set_fragment(None);
    origin.set_query(None);
    origin.set_path("");
    Ok(ParsedInvite {
        issuer_url: origin.as_str().trim_end_matches('/').to_string(),
        code: code.trim().to_string(),
    })
}

/// The instance an invite names, for a shell that has to show a contributor
/// whose commons they are about to join before they commit to joining it
/// (shared design spec, "### 2. Connect": *resolve and show the instance
/// before committing*).
///
/// Returns the host only, and deliberately nothing else. The obvious API
/// here would expose [`ParsedInvite`], but that carries `code` -- the
/// credential -- and handing it to four shells is four chances to put it in
/// a label, a log line, or a window title. A shell cannot leak what it was
/// never given, so this returns the one field it has a reason to draw.
///
/// `None` for anything `parse_invite` refuses, which is what the caller
/// wants: the shared spec gives the whole invite path a single failure
/// sentence, so the distinction between "not a URL" and "no code in it" is
/// one the interface must not draw anyway.
pub fn invite_issuer_host(raw: &str) -> Option<String> {
    let parsed = parse_invite(raw).ok()?;
    reqwest::Url::parse(&parsed.issuer_url)
        .ok()?
        .host_str()
        .map(str::to_string)
}

/// The invite inside a `tracecommons://enroll?invite=…` deep link, or
/// `None` for anything else — including every other argument a shell is
/// launched with, since registering a scheme handler means this question
/// gets asked about all of them.
///
/// An issuer link cannot open a desktop app, so an invite mail carries the
/// app's own scheme with the real invite folded into the `invite`
/// parameter. Scheme and host are compared case-insensitively, matching
/// `DeepLink.inviteURL` on macOS: a handler registration elsewhere in the
/// system need not preserve the case anyone typed.
///
/// This lives here rather than in a shell so that every Rust shell agrees
/// on what a deep link is, and so none of them vendors its own URL parser
/// to find out.
pub fn invite_from_deep_link(arg: &str) -> Option<String> {
    let url = reqwest::Url::parse(arg).ok()?;
    if !url.scheme().eq_ignore_ascii_case("tracecommons") {
        return None;
    }
    if !url.host_str()?.eq_ignore_ascii_case("enroll") {
        return None;
    }
    url.query_pairs()
        .find(|(k, _)| k == "invite")
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod invite_tests {
    use super::{invite_from_deep_link, invite_issuer_host, parse_invite};

    #[test]
    fn deep_link_yields_the_invite() {
        let got = invite_from_deep_link(
            "tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE",
        );
        assert_eq!(got.as_deref(), Some("https://issuer.example/onboard#CODE"));
    }

    #[test]
    fn deep_link_scheme_and_host_are_case_insensitive() {
        let got =
            invite_from_deep_link("TraceCommons://ENROLL?invite=https%3A%2F%2Fi.example%2Fo%23C");
        assert_eq!(got.as_deref(), Some("https://i.example/o#C"));
    }

    /// The exact argv a real Windows shell delivered when a
    /// `tracecommons://` link was opened: it normalised the URL and added a
    /// slash after the host, which nothing in our code wrote. Parsing via
    /// `Url` survives that; splitting on `"://enroll?"` would not have.
    ///
    /// Kept here as well as in the Windows tests because both shells parse
    /// links produced by the same invite mail, and a desktop portal is as
    /// free to normalise as the Windows shell was.
    #[test]
    fn the_form_a_shell_actually_delivers() {
        let got = invite_from_deep_link(
            "tracecommons://enroll/?invite=https%3A%2F%2Fissuer.tracecommons.ai%2Fonboard%23VQWWPGYSG8Y4LTP6",
        );
        assert_eq!(
            got.as_deref(),
            Some("https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6")
        );
    }

    #[test]
    fn other_arguments_are_not_invites() {
        // Registering a scheme handler means this is asked about every
        // argument the shell is ever launched with, including its own.
        assert_eq!(invite_from_deep_link("https://example.com/"), None);
        assert_eq!(invite_from_deep_link("tracecommons://open?x=1"), None);
        assert_eq!(invite_from_deep_link("--state-dir"), None);
        assert_eq!(invite_from_deep_link("tracecommons://enroll?invite="), None);
    }

    #[test]
    fn issuer_host_is_shown_without_the_code() {
        let host = invite_issuer_host("https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6")
            .expect("a well-formed invite resolves to its host");
        assert_eq!(host, "issuer.tracecommons.ai");
        // The point of the narrow return type: the credential is not in it.
        assert!(!host.contains("VQWWPGYSG8Y4LTP6"));
    }

    #[test]
    fn issuer_host_refuses_what_parse_invite_refuses() {
        // A bare code, and a URL carrying no code at all. Both are `None`,
        // because the interface shows one sentence for every failure here.
        assert_eq!(invite_issuer_host("VQWWPGYSG8Y4LTP6"), None);
        assert_eq!(
            invite_issuer_host("https://issuer.tracecommons.ai/onboard"),
            None
        );
    }

    #[test]
    fn parses_fragment_form() {
        let p = parse_invite("https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6").unwrap();
        assert_eq!(p.issuer_url, "https://issuer.tracecommons.ai");
        assert_eq!(p.code, "VQWWPGYSG8Y4LTP6");
    }

    #[test]
    fn parses_query_form_when_fragment_was_stripped() {
        let p = parse_invite("https://issuer.tracecommons.ai/onboard?code=ABC123XYZ").unwrap();
        assert_eq!(p.issuer_url, "https://issuer.tracecommons.ai");
        assert_eq!(p.code, "ABC123XYZ");
    }

    #[test]
    fn rejects_a_bare_code() {
        // Guessing a default issuer would send someone's invite to the wrong
        // host, and the code is single-use.
        let err = parse_invite("VQWWPGYSG8Y4LTP6").unwrap_err().to_string();
        assert!(err.contains("full invite URL"), "got: {err}");
    }

    #[test]
    fn rejects_a_url_with_no_code() {
        let err = parse_invite("https://issuer.tracecommons.ai/onboard")
            .unwrap_err()
            .to_string();
        assert!(err.contains("no code"), "got: {err}");
    }

    #[test]
    fn rejects_a_non_http_scheme() {
        assert!(parse_invite("file:///etc/passwd#CODE").is_err());
    }
}

// ---------------------------------------------------------------------------
// Background daemon control
//
// These drive the same request handlers the IPC socket exposes. When a daemon
// is running they are sent to *it*, over that socket; only when nothing is
// running do they fall back to reading and writing the state files in-process.
// Both paths reach `ipc::handle_request_async`, so a CLI caller and a native
// application get identical answers to the same request.
//
// Every method here is reachable over the socket too. There is no
// terminal-only carve-out: arming a project for automatic upload and
// approving the whole queue at once are both available to a socket caller,
// and both append a local audit entry instead (see `daemon::audit` and the
// `ipc` module doc's "Authorization" section for why the old gate did not
// survive scrutiny). `daemon audit` is how a contributor reads that log.
// ---------------------------------------------------------------------------

use crate::daemon::ipc::{DaemonShared, ERR_UNAVAILABLE, Response, handle_local};
use crate::daemon::policy::ProjectMode;
use crate::daemon::withdraw::ERR_ACCOUNT_SESSION_REQUIRED;

/// Load daemon state for a one-shot command against a *stopped* daemon.
///
/// Only ever reached through `daemon_call`'s no-daemon-running fallback.
/// Writing the state files is the right thing to do there -- it primes the
/// next daemon start -- and is exactly the wrong thing to do while one is
/// running, which is what `daemon_call` is for. See `daemon::client`.
fn daemon_shared(store: &ConfigStore) -> Result<DaemonShared> {
    DaemonShared::load(ConfigStore::open(store.dir().to_path_buf())?)
        .context("loading daemon state")
}

/// Answer one daemon request: from the running daemon over its socket when
/// there is one, otherwise from the on-disk state directly.
///
/// The socket is not an optimization here, it is the only correct path when
/// a daemon is running: the running daemon holds the authoritative queue,
/// policy, pause flag and health in memory, re-reads none of them, and
/// overwrites the files from its own copy on its next pass. See
/// `daemon::client`'s module doc for the full list of what silently did not
/// work before this.
fn daemon_call(store: &ConfigStore, method: &str, params: serde_json::Value) -> Result<Response> {
    match crate::daemon::client::try_call(store, method, &params)? {
        Some(resp) => Ok(resp),
        None => {
            let shared = daemon_shared(store)?;
            Ok(handle_local(&shared, method, params))
        }
    }
}

/// Render an IPC response for a human or for a script.
fn render(resp: Response, json: bool, table: impl FnOnce(&serde_json::Value)) -> Result<()> {
    if let Some(err) = resp.error {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "schema_version": "trace_commons.cli_error.v1",
                    "error": err.code,
                    "detail": err.message,
                }))?
            );
        }
        anyhow::bail!("{}: {}", err.code, err.message);
    }
    let result = resp.result.unwrap_or(serde_json::Value::Null);
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        table(&result);
    }
    Ok(())
}

/// Show what the daemon is doing and whether anything is wrong.
///
/// Goes through `daemon_call`, so the health line reports the *running*
/// daemon's real health. It used to be answered from a fresh
/// `DaemonShared::load`, whose `HealthState` is always
/// `HealthState::default()` -- `HealthState` is in-memory only and is never
/// persisted -- so this command printed `health: ok` unconditionally, even
/// while the daemon was refusing every upload.
pub fn daemon_status(store: &ConfigStore, json: bool) -> Result<()> {
    let resp = daemon_call(store, "status", serde_json::json!({}))?;
    render(resp, json, |v| {
        let health = &v["health"];
        println!(
            "logged in:   {}",
            if v["logged_in"] == true { "yes" } else { "no" }
        );
        println!(
            "paused:      {}",
            if v["paused"] == true { "yes" } else { "no" }
        );
        println!("pending:     {}", v["queue_depth"]);
        match health["last_error_label"].as_str() {
            Some(label) => println!(
                "health:      {label} (since {})",
                health["since"].as_str().unwrap_or("unknown")
            ),
            None => println!("health:      ok"),
        }
    })
}

pub fn daemon_pending(store: &ConfigStore, json: bool) -> Result<()> {
    let resp = daemon_call(store, "list_pending", serde_json::json!({}))?;
    render(resp, json, |v| {
        let empty = Vec::new();
        let entries = v["pending"].as_array().unwrap_or(&empty);
        if entries.is_empty() {
            println!("nothing waiting");
            return;
        }
        let rows: Vec<Vec<String>> = entries
            .iter()
            .map(|e| {
                vec![
                    e["entry_id"].as_str().unwrap_or("-").to_string(),
                    e["project_label"].as_str().unwrap_or("-").to_string(),
                    e["source"].as_str().unwrap_or("-").to_string(),
                    format!("{}", e["size_bytes"].as_u64().unwrap_or(0)),
                    e["discovered_at"].as_str().unwrap_or("-").to_string(),
                ]
            })
            .collect();
        let _ = print_table(
            &mut std::io::stdout(),
            &["ENTRY", "PROJECT", "SOURCE", "BYTES", "READY SINCE"],
            &rows,
        );
    })
}

pub fn daemon_preview(store: &ConfigStore, entry_id: &str, json: bool) -> Result<()> {
    let resp = daemon_call(
        store,
        "preview",
        serde_json::json!({ "entry_id": entry_id }),
    )?;
    render(resp, json, |v| {
        println!(
            "project: {}",
            v["entry"]["project_label"].as_str().unwrap_or("-")
        );
        println!("source:  {}", v["entry"]["source"].as_str().unwrap_or("-"));
        println!("bytes:   {}", v["would_send_bytes"]);
        println!(
            "hash:    {}",
            v["entry"]["session_hash"].as_str().unwrap_or("-")
        );
    })
}

/// Refuse an ambiguous or empty approve selector; `Some(message)` is the
/// error to surface, `None` means exactly one selector was given.
///
/// The daemon's `approve` accepts all three of `entry_id`, `all: true`, and
/// `project_id` at once and resolves the ambiguity with a documented
/// precedence (`all` > `project_id` > `entry_id`, see
/// `docs/contributor-daemon-ipc-v1_1.md`'s approve row). That precedence
/// exists for protocol robustness, not as something a human should have to
/// know to use this CLI safely -- so the CLI refuses here rather than
/// silently picking a winner and approving something other than what was
/// asked for.
fn approve_selector_error(
    entry_id: Option<&str>,
    all: bool,
    project: Option<&str>,
) -> Option<String> {
    let selected = [entry_id.is_some(), all, project.is_some()]
        .iter()
        .filter(|b| **b)
        .count();
    if selected == 1 {
        None
    } else {
        Some("give exactly one of: an entry id, --all, or --project <id>".to_string())
    }
}

/// Manual arg-slice parser used only to unit-test the approve selector rule
/// against CLI-shaped input without depending on the `clap::Parser` type
/// defined in the `trace-commons-contributor` binary (which this lib crate
/// does not, and should not, depend on). Recognizes `--all` and `--project
/// <id>`; anything else is treated as the positional entry id.
///
/// The parsed selectors are handed to [`daemon_approve`] itself, not to
/// [`approve_selector_error`] directly: what has to hold is that the command
/// an operator actually runs refuses to act, and a test that only exercises
/// the predicate would still pass if `daemon_approve` stopped consulting it.
/// An ambiguous selector bails before any socket is touched, so the store
/// here is never connected to anything.
#[cfg(test)]
fn approve_args_error(args: &[&str]) -> String {
    let mut entry_id = None;
    let mut all = false;
    let mut project = None;
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--all" => all = true,
            "--project" => {
                i += 1;
                project = args.get(i).copied();
            }
            other => entry_id = Some(other),
        }
        i += 1;
    }
    let (_dir, store) = crate::config::tests_support::temp_store();
    match daemon_approve(&store, entry_id, all, project, false) {
        Ok(()) => String::new(),
        Err(e) => e.to_string(),
    }
}

pub fn daemon_approve(
    store: &ConfigStore,
    entry_id: Option<&str>,
    all: bool,
    project: Option<&str>,
    json: bool,
) -> Result<()> {
    if let Some(err) = approve_selector_error(entry_id, all, project) {
        anyhow::bail!(err);
    }
    let params = if all {
        serde_json::json!({ "all": true })
    } else if let Some(project) = project {
        serde_json::json!({ "project_id": project })
    } else {
        serde_json::json!({ "entry_id": entry_id })
    };
    let resp = daemon_call(store, "approve", params)?;
    render(resp, json, |v| {
        println!("approved {}", v["approved"]);
        let flagged = v["flagged"].as_u64().unwrap_or(0);
        if flagged > 0 {
            println!("flagged {flagged}");
        }
        if let Some(redactions) = v["redactions"].as_object() {
            let total: u64 = redactions.values().filter_map(|c| c.as_u64()).sum();
            if total > 0 {
                println!("redactions {total}");
            }
        }
        let empty = Vec::new();
        let skipped = v["skipped"].as_array().unwrap_or(&empty);
        if !skipped.is_empty() {
            println!("skipped {}:", skipped.len());
            for s in skipped {
                println!(
                    "  {} ({})",
                    s["entry_id"].as_str().unwrap_or("-"),
                    s["reason_label"].as_str().unwrap_or("-"),
                );
            }
        }
    })
}

pub fn daemon_dismiss(store: &ConfigStore, entry_id: &str, json: bool) -> Result<()> {
    let resp = daemon_call(
        store,
        "dismiss",
        serde_json::json!({ "entry_id": entry_id }),
    )?;
    render(resp, json, |_| println!("dismissed"))
}

/// Withdraw one submitted trace, or every quarantined one at once.
///
/// `--all-quarantined` rather than a generic `--all-status <status>`:
/// "take back everything held for privacy review" is the realistic bulk
/// case this feature exists to answer (see
/// `docs/superpowers/specs/2026-08-08-trace-withdrawal-design.md`), and the
/// daemon's `withdraw_bulk` IPC method accepts other statuses if a future
/// caller needs them.
///
/// Withdrawal is authenticated by an account session, not this device's key.
/// When there is no live session the daemon answers
/// `account-session-required`, and that is turned into a specific "run
/// `account login`" instruction here rather than falling through to
/// `render`'s generic `code: message` bail, so a contributor is not left
/// staring at a bare error code with no idea what to do about it.
pub fn daemon_withdraw(
    store: &ConfigStore,
    submission_id: Option<&str>,
    all_quarantined: bool,
    json: bool,
) -> Result<()> {
    if !all_quarantined && submission_id.is_none() {
        anyhow::bail!("give a submission id, or --all-quarantined");
    }
    let (method, params) = if all_quarantined {
        (
            "withdraw_bulk",
            serde_json::json!({ "status": "quarantined" }),
        )
    } else {
        (
            "withdraw",
            serde_json::json!({ "submission_id": submission_id }),
        )
    };
    let resp = daemon_call(store, method, params)?;
    if let Some(err) = &resp.error {
        if err.code == ERR_UNAVAILABLE && err.message == ERR_ACCOUNT_SESSION_REQUIRED {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "schema_version": "trace_commons.cli_error.v1",
                        "error": err.code,
                        "detail": err.message,
                    }))?
                );
            }
            anyhow::bail!(
                "withdrawal needs your account, not just this device's key. Run \
                 `trace-commons-contributor account login` and try again."
            );
        }
    }
    render(resp, json, |v| {
        if all_quarantined {
            println!(
                "withdrawn {} quarantined trace(s); {} failed",
                v["withdrawn"], v["failed"]
            );
        } else {
            println!(
                "withdrawn: {}",
                v["distribution_reach"].as_str().unwrap_or("-")
            );
        }
    })
}

/// Read the local audit log: when autonomy was armed, when the queue was
/// bulk-approved, when consent scopes changed, when the NEAR AI notice was
/// acknowledged.
///
/// This log is the stated replacement for a removed terminal-only
/// restriction (see `daemon::audit`), and until this command existed it was
/// reachable only over IPC -- and no native application ships in this
/// branch. A replacement nobody can read is not a replacement.
pub fn daemon_audit(store: &ConfigStore, limit: usize, json: bool) -> Result<()> {
    let resp = daemon_call(store, "list_audit", serde_json::json!({ "limit": limit }))?;
    render(resp, json, |v| {
        let empty = Vec::new();
        let entries = v["entries"].as_array().unwrap_or(&empty);
        if entries.is_empty() {
            println!("nothing audited yet");
            return;
        }
        let rows: Vec<Vec<String>> = entries
            .iter()
            .map(|e| {
                vec![
                    e["at"].as_str().unwrap_or("-").to_string(),
                    e["action"].as_str().unwrap_or("-").to_string(),
                    e["project_label"].as_str().unwrap_or("-").to_string(),
                    e["detail"].as_str().unwrap_or("-").to_string(),
                ]
            })
            .collect();
        let _ = print_table(
            &mut std::io::stdout(),
            &["WHEN", "ACTION", "PROJECT", "DETAIL"],
            &rows,
        );
    })
}

pub fn daemon_pause(store: &ConfigStore, pause: bool, json: bool) -> Result<()> {
    let resp = daemon_call(
        store,
        if pause { "pause" } else { "resume" },
        serde_json::json!({}),
    )?;
    render(resp, json, |v| {
        println!(
            "{}",
            if v["paused"] == true {
                "paused"
            } else {
                "running"
            }
        );
    })
}

pub fn daemon_projects(store: &ConfigStore, json: bool) -> Result<()> {
    let resp = daemon_call(store, "list_projects", serde_json::json!({}))?;
    render(resp, json, |v| {
        let empty = Vec::new();
        let projects = v["projects"].as_array().unwrap_or(&empty);
        if projects.is_empty() {
            println!("no projects known yet; everything defaults to notify-only");
            return;
        }
        // `list_projects` reports discovered-but-unruled projects too, so
        // the mode column alone would not say whether a `notify_only` row
        // is a decision the contributor made or just the default in force.
        // The third column says which.
        let rows: Vec<Vec<String>> = projects
            .iter()
            .map(|p| {
                vec![
                    p["project_label"].as_str().unwrap_or("-").to_string(),
                    p["mode"].as_str().unwrap_or("-").to_string(),
                    if p["configured"].as_bool().unwrap_or(true) {
                        "configured".to_string()
                    } else {
                        "discovered".to_string()
                    },
                ]
            })
            .collect();
        let _ = print_table(
            &mut std::io::stdout(),
            &["PROJECT", "MODE", "SOURCE"],
            &rows,
        );
    })
}

/// Parse the CLI's short mode words into policy modes.
pub(crate) fn parse_project_mode(s: &str) -> Result<ProjectMode> {
    match s {
        "auto" | "auto_upload" => Ok(ProjectMode::AutoUpload),
        "notify" | "notify_only" => Ok(ProjectMode::NotifyOnly),
        "ignore" => Ok(ProjectMode::Ignore),
        other => anyhow::bail!("unknown mode {other}: expected auto, notify, or ignore"),
    }
}

/// Resolve a `daemon project` path argument into a policy key.
///
/// The watcher keys policy off the session's *recorded* working directory,
/// which is always absolute and fully resolved. A relative path, a trailing
/// slash, a `.`, or a path crossing a symlink produces a key that matches
/// no session at all -- and the command still printed success. The
/// dangerous direction is `--mode ignore` silently not applying, so the
/// path is resolved against the real filesystem here, exactly as
/// `discover_filtered` already does for `submit --project`.
///
/// The locked unknown-cwd bucket is a sentinel, not a path, and is passed
/// through untouched so `policy::set_mode` can refuse to arm it by name.
fn resolve_project_key(path: &Path) -> Result<String> {
    let raw = path.to_string_lossy().to_string();
    if raw == crate::daemon::policy::UNKNOWN_PROJECT_KEY {
        return Ok(raw);
    }
    let resolved = std::fs::canonicalize(path)
        .with_context(|| format!("resolving project path {} (does it exist?)", path.display()))?;
    Ok(resolved.to_string_lossy().to_string())
}

pub fn daemon_set_project(store: &ConfigStore, path: &Path, mode: &str, json: bool) -> Result<()> {
    let mode = parse_project_mode(mode)?;
    let key = resolve_project_key(path)?;
    // No `label` is sent. The daemon derives it from the key -- it ignores
    // any label a client supplies, because a caller-chosen string reaching
    // `list_projects` and `daemon-audit.jsonl` was an injection path into
    // both. This is the same bare basename the daemon will store;
    // disambiguation happens at render time (here, and in `daemon projects`
    // / the queue) against the current known-key set, so a stored label
    // never goes stale when a colliding project shows up later.
    let label = crate::daemon::policy::project_label_for(&key);
    let resp = daemon_call(
        store,
        "set_project_mode",
        serde_json::json!({ "project_key": key, "mode": mode }),
    )?;
    // Ask the same daemon that just applied the edit what it now knows, so
    // the label shown is disambiguated against the authoritative known-key
    // set rather than against a private copy loaded from disk.
    let known_labels = daemon_call(store, "list_projects", serde_json::json!({}))
        .ok()
        .and_then(|r| r.result)
        .map(|v| {
            v["projects"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p["project_label"].as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let display_label = known_labels
        .iter()
        .find(|l| l.as_str() == label || l.starts_with(&format!("{label} (")))
        .cloned()
        .unwrap_or_else(|| label.clone());

    // A key that matches nothing the daemon has ever seen is very likely a
    // typo, and a typo'd `--mode ignore` reads as "this project is now
    // silenced" while silencing nothing at all. Say so rather than printing
    // an unqualified success.
    let matches_nothing = daemon_call(store, "list_pending", serde_json::json!({}))
        .ok()
        .and_then(|r| r.result)
        .map(|v| {
            v["pending"]
                .as_array()
                .map(|a| {
                    !a.iter()
                        .any(|e| e["project_label"].as_str() == Some(display_label.as_str()))
                })
                .unwrap_or(true)
        })
        .unwrap_or(true);

    render(resp, json, |_| {
        println!(
            "{display_label}: {}",
            serde_json::to_string(&mode).unwrap_or_default()
        );
        if matches_nothing {
            println!(
                "note: no session the daemon currently knows about comes from this \
                 project. If you meant an already-queued project, check `daemon \
                 pending` -- the mode applies to the session's recorded working \
                 directory, not to the directory you happen to be standing in."
            );
        }
    })
}

pub async fn daemon_history(
    store: &ConfigStore,
    limit: usize,
    refresh: bool,
    json: bool,
) -> Result<()> {
    if refresh {
        // Refreshing needs the network and an enrollment, so it happens here
        // rather than inside the request handler.
        refresh_history_cache(store).await?;
    }
    let resp = daemon_call(store, "list_history", serde_json::json!({ "limit": limit }))?;
    if json {
        // Emit history and rollup together so a caller gets one document.
        let rollup = daemon_call(store, "history_rollup", serde_json::json!({}))?;
        let out = serde_json::json!({
            "history": resp.result.unwrap_or(serde_json::Value::Null)["history"],
            "rollup": rollup.result.unwrap_or(serde_json::Value::Null),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    render(resp, false, |v| {
        let empty = Vec::new();
        let records = v["history"].as_array().unwrap_or(&empty);
        if records.is_empty() {
            println!("no contributions yet");
            return;
        }
        let rows: Vec<Vec<String>> = records
            .iter()
            .map(|r| {
                vec![
                    r["submitted_at"].as_str().unwrap_or("-").to_string(),
                    r["project_label"].as_str().unwrap_or("-").to_string(),
                    r["status"].as_str().unwrap_or("-").to_string(),
                    format!("{:.2}", r["credit_points_pending"].as_f64().unwrap_or(0.0)),
                    r["credit_points_final"]
                        .as_f64()
                        .map(|f| format!("{f:.2}"))
                        .unwrap_or_else(|| "-".to_string()),
                ]
            })
            .collect();
        let _ = print_table(
            &mut std::io::stdout(),
            &["WHEN", "PROJECT", "STATUS", "PENDING", "FINAL"],
            &rows,
        );
    })?;

    let rollup = daemon_call(store, "history_rollup", serde_json::json!({}))?;
    if let Some(v) = rollup.result {
        println!();
        println!(
            "this week: {} | this month: {} | all time: {}",
            v["week"]["accepted"].as_u64().unwrap_or(0)
                + v["week"]["submitted"].as_u64().unwrap_or(0)
                + v["week"]["quarantined"].as_u64().unwrap_or(0),
            v["month"]["accepted"].as_u64().unwrap_or(0)
                + v["month"]["submitted"].as_u64().unwrap_or(0)
                + v["month"]["quarantined"].as_u64().unwrap_or(0),
            v["all_time"]["accepted"].as_u64().unwrap_or(0)
                + v["all_time"]["submitted"].as_u64().unwrap_or(0)
                + v["all_time"]["quarantined"].as_u64().unwrap_or(0),
        );
        println!(
            "credit: {:.2} pending, {:.2} final",
            v["credit_pending"].as_f64().unwrap_or(0.0),
            v["credit_final"].as_f64().unwrap_or(0.0),
        );
        let quarantined = v["quarantined"].as_u64().unwrap_or(0);
        if quarantined > 0 {
            // Quarantine is "held for operator privacy review", not rejected.
            println!(
                "{quarantined} held for privacy review (not rejected; an operator \
                 has to look at these)"
            );
        }
        if let Some(t) = v["last_refreshed_at"].as_str() {
            println!("last refreshed {t}");
        } else {
            println!("never refreshed from the server; run with --refresh");
        }
    }
    Ok(())
}

/// Poll the server for submission status and rewrite the history cache.
async fn refresh_history_cache(store: &ConfigStore) -> Result<()> {
    let cfg = store
        .load_config()
        .context("loading contributor config")?
        .context("not logged in; run `login` first")?;
    let updates = submit::status(store, &cfg).await?;
    let receipts = store.load_receipts().context("loading receipts")?;
    let labels = {
        let queue = crate::daemon::queue::Queue::load(store)?;
        let mut m = std::collections::BTreeMap::new();
        for e in queue.all() {
            if let Some(id) = e.submission_id {
                m.insert(id, e.project_label.clone());
            }
        }
        m
    };
    let records = crate::daemon::history::join(&receipts, &updates, &labels, Utc::now());
    crate::daemon::history::HistoryCache::save(store, &records)
}

pub fn daemon_settings(store: &ConfigStore, set: &[String], json: bool) -> Result<()> {
    if set.is_empty() {
        let resp = daemon_call(store, "get_settings", serde_json::json!({}))?;
        return render(resp, json, |v| {
            println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
        });
    }
    let mut params = serde_json::Map::new();
    for pair in set {
        let (k, v) = pair
            .split_once('=')
            .with_context(|| format!("expected KEY=VALUE, got {pair}"))?;
        let value = if let Ok(b) = v.parse::<bool>() {
            serde_json::Value::Bool(b)
        } else if let Ok(n) = v.parse::<u64>() {
            serde_json::Value::from(n)
        } else {
            serde_json::Value::String(v.to_string())
        };
        params.insert(k.to_string(), value);
    }
    let resp = daemon_call(store, "set_settings", serde_json::Value::Object(params))?;
    render(resp, json, |v| {
        println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    })
}

pub fn daemon_install(store: &ConfigStore) -> Result<()> {
    crate::daemon::install::install(store)
}

pub fn daemon_uninstall() -> Result<()> {
    crate::daemon::install::uninstall()
}

/// The one human-readable line for each update outcome. A function rather
/// than inline `println!`s so it is testable without capturing stdout.
pub(crate) fn update_outcome_line(outcome: &crate::update::run::UpdateOutcome) -> String {
    use crate::update::run::UpdateOutcome;
    match outcome {
        UpdateOutcome::DeferredToWinget => format!(
            "winget installed this copy, so winget updates it:\n  {}",
            crate::update::source::WINGET_UPGRADE_COMMAND
        ),
        // Nothing was installed here either, but unlike winget we have no
        // idea how this copy got here (Homebrew, a distro package, a
        // read-only Nix path, `install.sh --dir`), so there is no single
        // command to point at.
        UpdateOutcome::DeferredUnrecognized => "this copy was not installed by \
            trace-commons-contributor's own updater, so it will not replace it; \
            update it the same way you installed it"
            .to_string(),
        UpdateOutcome::UpToDate { version } => format!("already up to date ({version})"),
        UpdateOutcome::NoArtifactForPlatform => {
            "no update is published for this platform yet".to_string()
        }
        UpdateOutcome::Staged { version } => format!(
            "{version} verified and staged; it is applied at the daemon's next start, \
             or now with `trace-commons-contributor update`"
        ),
        UpdateOutcome::Applied { version } => format!("installed {version}"),
        UpdateOutcome::QuiesceTimedOutStaged { version } => format!(
            "{version} verified and staged, but an upload is still in flight; \
             nothing was replaced. Try again shortly."
        ),
    }
}

/// The machine-readable name for each outcome, for `--json` callers.
fn update_outcome_kind(outcome: &crate::update::run::UpdateOutcome) -> &'static str {
    use crate::update::run::UpdateOutcome;
    match outcome {
        UpdateOutcome::DeferredToWinget => "deferred_to_winget",
        UpdateOutcome::DeferredUnrecognized => "deferred_unrecognized",
        UpdateOutcome::UpToDate { .. } => "up_to_date",
        UpdateOutcome::NoArtifactForPlatform => "no_artifact_for_platform",
        UpdateOutcome::Staged { .. } => "staged",
        UpdateOutcome::Applied { .. } => "applied",
        UpdateOutcome::QuiesceTimedOutStaged { .. } => "quiesce_timed_out_staged",
    }
}

/// Check for, verify, and install an update.
///
/// Verification is not optional and there is no flag that skips it: this tool
/// reads coding transcripts, so an updater that can be talked into installing
/// something unverified is worse than no updater.
///
/// Every outcome here -- including "nothing to do" ones like `UpToDate`,
/// `NoArtifactForPlatform`, and the two deferred variants -- is `Ok(())`, so
/// the process exits 0. Only an `UpdateError` (a verification, fetch, or
/// swap failure) is `Err`, which exits non-zero. That is the intended
/// distinction: declining to update because there is nothing to do is not a
/// failure, but failing to verify or apply one is.
pub async fn update(store: &ConfigStore, stage_only: bool, json: bool) -> Result<()> {
    use crate::update::run::{UpdateMode, check_and_install};

    let mode = if stage_only {
        UpdateMode::Stage
    } else {
        UpdateMode::Apply
    };
    let outcome = check_and_install(store, mode)
        .await
        // The error labels are fixed and carry no path, URL, or signature.
        .map_err(|e| anyhow::anyhow!("update refused: {e}"))?;

    if json {
        let version = match &outcome {
            crate::update::run::UpdateOutcome::UpToDate { version }
            | crate::update::run::UpdateOutcome::Staged { version }
            | crate::update::run::UpdateOutcome::Applied { version }
            | crate::update::run::UpdateOutcome::QuiesceTimedOutStaged { version } => {
                Some(version.clone())
            }
            crate::update::run::UpdateOutcome::DeferredToWinget
            | crate::update::run::UpdateOutcome::DeferredUnrecognized
            | crate::update::run::UpdateOutcome::NoArtifactForPlatform => None,
        };
        let out = serde_json::json!({
            "schema_version": "trace_commons.cli_update.v1",
            "outcome": update_outcome_kind(&outcome),
            "version": version,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("{}", update_outcome_line(&outcome));
    }
    Ok(())
}

#[cfg(test)]
mod daemon_command_tests {
    use super::*;

    #[test]
    fn project_mode_words_parse_to_policy_modes() {
        assert_eq!(parse_project_mode("auto").unwrap(), ProjectMode::AutoUpload);
        assert_eq!(
            parse_project_mode("auto_upload").unwrap(),
            ProjectMode::AutoUpload
        );
        assert_eq!(
            parse_project_mode("notify").unwrap(),
            ProjectMode::NotifyOnly
        );
        assert_eq!(parse_project_mode("ignore").unwrap(), ProjectMode::Ignore);
    }

    #[test]
    fn an_unknown_mode_word_is_rejected_with_the_valid_ones_named() {
        let err = parse_project_mode("yolo").unwrap_err();
        assert!(err.to_string().contains("auto"), "{err}");
        assert!(err.to_string().contains("notify"), "{err}");
        assert!(err.to_string().contains("ignore"), "{err}");
    }

    #[test]
    fn arming_the_unknown_bucket_is_refused_from_the_cli_too() {
        // The terminal carve-out grants autonomy over real projects, not over
        // sessions whose project could not be identified.
        let (_d, store) = crate::config::tests_support::temp_store();
        let err = daemon_set_project(
            &store,
            std::path::Path::new(crate::daemon::policy::UNKNOWN_PROJECT_KEY),
            "auto",
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown-project"), "{err}");
    }

    #[test]
    fn approving_with_neither_an_id_nor_all_is_an_error() {
        let (_d, store) = crate::config::tests_support::temp_store();
        let err = daemon_approve(&store, None, false, None, false).unwrap_err();
        assert!(err.to_string().contains("--all"), "{err}");
    }

    #[test]
    fn approve_accepts_a_project_or_an_id_or_all_but_not_two_at_once() {
        let err = approve_args_error(&["--all", "--project", "proj_abc"]);
        assert!(
            err.contains("one of"),
            "ambiguous selection must be refused, got: {err}"
        );
    }

    #[test]
    fn setting_a_project_to_auto_from_the_cli_is_persisted() {
        let (_d, store) = crate::config::tests_support::temp_store();
        let project = tempfile::tempdir().unwrap();
        daemon_set_project(&store, project.path(), "auto", false).unwrap();
        let key = std::fs::canonicalize(project.path())
            .unwrap()
            .to_string_lossy()
            .to_string();
        let policy = crate::daemon::policy::ProjectPolicy::load(&store).unwrap();
        assert_eq!(policy.resolve(&key), ProjectMode::AutoUpload);
    }

    #[test]
    fn a_project_path_is_canonicalized_before_it_becomes_a_policy_key() {
        // The watcher keys off the session's recorded cwd, which is always
        // absolute and resolved. A trailing slash, a `.`, or a symlinked
        // path produced a key matching no session at all -- and `--mode
        // ignore` still printed success while silencing nothing.
        let project = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(project.path()).unwrap();

        let with_trailing_slash =
            std::path::PathBuf::from(format!("{}/", project.path().display()));
        assert_eq!(
            resolve_project_key(&with_trailing_slash).unwrap(),
            canonical.to_string_lossy()
        );

        let with_dot = project.path().join(".");
        assert_eq!(
            resolve_project_key(&with_dot).unwrap(),
            canonical.to_string_lossy()
        );
    }

    #[test]
    fn a_project_path_that_does_not_exist_is_an_error_rather_than_a_silent_no_op() {
        let missing = std::path::Path::new("/nonexistent-trace-commons-project-xyz");
        let err = resolve_project_key(missing).unwrap_err();
        assert!(err.to_string().contains("does it exist?"), "{err}");
    }

    #[test]
    fn logout_distinguishes_an_embedded_daemon_from_one_that_never_answered() {
        // `start_embedded` is exactly the shape the C ABI hosts: it takes
        // the lock and serves the socket, and the socket's `"shutdown"`
        // stops the supervise loop without releasing that lock (only
        // `tc_handle_free` / `tc_daemon_stop` do). Logout used to read that
        // as "did not confirm it stopped", waiting out the full deadline
        // and then printing an alarming warning about a stop that had in
        // fact happened.
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let embedded = rt
            .block_on(crate::daemon::start_embedded(
                ConfigStore::open(dir.path().to_path_buf()).unwrap(),
            ))
            .unwrap();

        let outcome = stop_running_daemon(&store).unwrap();
        assert_eq!(
            outcome,
            DaemonStopOutcome::AcknowledgedButStillHoldingTheLock,
            "an embedded daemon acknowledges and keeps its lock; that is not \
             a failure to stop"
        );
        assert!(
            embedded
                .shared
                .shutdown
                .load(std::sync::atomic::Ordering::Relaxed),
            "and it really did receive the stop request"
        );
        embedded.close();
    }

    #[test]
    fn logout_reports_no_daemon_when_none_is_running() {
        let (_d, store) = crate::config::tests_support::temp_store();
        assert_eq!(
            stop_running_daemon(&store).unwrap(),
            DaemonStopOutcome::NotRunning
        );
    }

    #[test]
    fn the_unknown_bucket_sentinel_is_not_treated_as_a_filesystem_path() {
        assert_eq!(
            resolve_project_key(std::path::Path::new(
                crate::daemon::policy::UNKNOWN_PROJECT_KEY
            ))
            .unwrap(),
            crate::daemon::policy::UNKNOWN_PROJECT_KEY
        );
    }

    #[test]
    fn a_winget_install_is_told_the_winget_command_and_nothing_else() {
        use crate::update::run::UpdateOutcome;
        let text = crate::commands::update_outcome_line(&UpdateOutcome::DeferredToWinget);
        assert!(
            text.contains("winget upgrade TraceCommons.Contributor"),
            "{text}"
        );
        // Never suggest a manual replacement to somebody whose package
        // manager owns the file: doing it would leave winget offering a
        // phantom upgrade forever.
        assert!(!text.contains("install.ps1"), "{text}");
    }

    #[test]
    fn an_unrecognized_install_is_not_told_to_run_winget() {
        use crate::update::run::UpdateOutcome;
        let text = crate::commands::update_outcome_line(&UpdateOutcome::DeferredUnrecognized);
        // Telling a Homebrew (or distro package, or Nix, or install.ps1
        // --dir) install to run `winget upgrade` would be actively wrong.
        assert!(!text.contains("winget"), "{text}");
        assert!(!text.to_lowercase().contains("install.ps1"), "{text}");
    }

    #[test]
    fn an_up_to_date_install_says_so_without_a_version_bump_claim() {
        use crate::update::run::UpdateOutcome;
        let text = crate::commands::update_outcome_line(&UpdateOutcome::UpToDate {
            version: "0.1.0".to_string(),
        });
        assert!(text.contains("0.1.0"), "{text}");
        assert!(!text.contains("installed"), "{text}");
    }

    #[test]
    fn a_quiesce_timeout_is_reported_as_staged_not_as_installed() {
        use crate::update::run::UpdateOutcome;
        let text = crate::commands::update_outcome_line(&UpdateOutcome::QuiesceTimedOutStaged {
            version: "0.2.0".to_string(),
        });
        assert!(text.contains("staged"), "{text}");
        assert!(!text.contains("installed"), "{text}");
    }

    #[test]
    fn an_applied_update_names_the_version_installed() {
        use crate::update::run::UpdateOutcome;
        let text = crate::commands::update_outcome_line(&UpdateOutcome::Applied {
            version: "0.2.0".to_string(),
        });
        assert!(text.contains("installed"), "{text}");
        assert!(text.contains("0.2.0"), "{text}");
    }
}

#[cfg(test)]
mod coverage_tests {
    use super::*;

    /// #444: the score attestation already carries how much of each trace was
    /// actually scored, but `attest` prints the raw signed JWT -- a thing to
    /// hand a collector, not to read. `status` is what a contributor looks at,
    /// and it says nothing about coverage.
    ///
    /// The decode here is DISPLAY ONLY and deliberately does not verify the
    /// signature: this is the contributor reading their own scores back. The
    /// signature is what a collector checks, and that path is unchanged.
    #[test]
    fn coverage_is_read_from_the_attestation_for_display() {
        // payload segment only; header and signature are not inspected.
        let payload = serde_json::json!({
            "submissions": [
                {"submission_id": "11111111-1111-1111-1111-111111111111",
                 "coverage": {"coverage_state": "partial",
                              "chunks_scored": 16, "chunks_total": 2362}},
                {"submission_id": "22222222-2222-2222-2222-222222222222",
                 "coverage": {"coverage_state": "complete",
                              "chunks_scored": 4, "chunks_total": 4}},
                {"submission_id": "33333333-3333-3333-3333-333333333333",
                 "coverage": {"coverage_state": "partial_unknown_total",
                              "chunks_scored": 16}}
            ]
        });
        let jwt = fake_compact_jws(&payload);

        let lines = partial_coverage_lines(&jwt).expect("payload decodes");

        // Only the two partial ones are reported. A complete trace needs no
        // caveat, and one on every row would train people to skip the line.
        assert_eq!(lines.len(), 2, "got {lines:?}");

        let partial = &lines[0];
        assert!(partial.0.starts_with("11111111"));
        assert!(
            partial.1.contains("16") && partial.1.contains("2362"),
            "both numbers must be shown: {}",
            partial.1
        );

        let unknown = &lines[1];
        assert!(unknown.0.starts_with("33333333"));
        assert!(
            unknown.1.contains("16") && !unknown.1.contains("2362"),
            "an unknown total must not be fabricated: {}",
            unknown.1
        );
    }

    /// A malformed or unexpected attestation must not break `status`. Showing
    /// the submission table matters more than showing a coverage caveat.
    #[test]
    fn a_undecodable_attestation_yields_no_lines_rather_than_an_error() {
        assert!(partial_coverage_lines("not-a-jwt").is_none());
        assert!(partial_coverage_lines("").is_none());
        assert!(partial_coverage_lines("a.!!!notbase64!!!.c").is_none());
    }

    fn fake_compact_jws(payload: &serde_json::Value) -> String {
        use base64::Engine as _;
        let body = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(payload).expect("payload serializes"));
        format!("ignored-header.{body}.ignored-signature")
    }
}

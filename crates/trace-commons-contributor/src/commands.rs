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
use crate::consent::{prompt_consent_answers, scopes_from_answers, validate_scopes};
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
    }
}

/// Signals that a JSON submit result has already been rendered to stdout.
/// The binary uses this to return a failing exit status without appending a
/// second JSON document.
#[derive(Debug, thiserror::Error)]
#[error("one or more sessions were refused or failed")]
pub struct RenderedSubmitFailure;

/// Enroll this device with an instance-signed grant, or (with no grant)
/// print this device's key id so an instance operator can mint one.
///
/// When `allowed_hosts` is provided it takes precedence over the
/// `TRACE_COMMONS_ALLOWED_HOSTS` env fallback and is persisted into the
/// saved config so every later command enforces it.
///
/// `scopes` (a CSV of wire-name consent scopes) is validated before any
/// network call. When absent, an interactive terminal prompts for consent
/// choices; a non-interactive session falls back to the
/// `debugging_evaluation` floor only.
pub async fn login(
    store: &ConfigStore,
    grant_b64: Option<&str>,
    invite: Option<&str>,
    allowed_hosts: Option<&str>,
    scopes: Option<&str>,
) -> Result<()> {
    if grant_b64.is_some() && invite.is_some() {
        anyhow::bail!("--grant and --invite are alternative enrollment paths; pass only one");
    }
    let consent_scopes = resolve_consent_scopes(scopes)?;

    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;

    if let Some(invite) = invite {
        return login_with_invite(store, invite, allowed_hosts, &device, consent_scopes).await;
    }

    let Some(grant_b64) = grant_b64 else {
        println!("device_key_id: {}", device.device_key_id);
        println!(
            "give this to your instance to mint an enrollment grant, then re-run \
             `login --grant <grant>` -- or, if you were handed an invite link, run \
             `login --invite <url>`"
        );
        return Ok(());
    };

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
        consent_scopes: consent_scopes.clone(),
        pii_filter: None,
        allowed_hosts: allowed_hosts.map(str::to_string),
    };
    store
        .save_config(&cfg)
        .context("saving contributor config")?;

    println!("enrolled: tenant_id={}", cfg.tenant_id);
    println!(
        "Traces you submit carry the {} consent scope(s); secrets are removed locally \
         (including tool payloads), and the server re-applies the same deterministic \
         redaction on receipt. The optional NEAR AI PII pass (--pii-filter near-ai) covers \
         message text only.",
        consent_scopes.join(", ")
    );
    Ok(())
}

/// Resolve the consent scopes to request for this login: an explicit
/// `--scopes` CSV wins (validated immediately, before any network call); a
/// TTY prompts interactively; a non-interactive session with no `--scopes`
/// falls back to the `debugging_evaluation` floor only.
fn resolve_consent_scopes(scopes: Option<&str>) -> Result<Vec<String>> {
    if let Some(csv) = scopes {
        let names: Vec<String> = csv.split(',').map(|s| s.trim().to_string()).collect();
        return validate_scopes(&names).context("invalid --scopes value");
    }
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout();
        let answers = prompt_consent_answers(&mut stdin, &mut stdout)
            .context("reading interactive consent answers")?;
        Ok(scopes_from_answers(answers))
    } else {
        Ok(vec!["debugging_evaluation".to_string()])
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

/// Delete all local contributor state (config, device key, receipts).
pub fn logout(store: &ConfigStore) -> Result<()> {
    // Stop a running daemon first. It holds a minted claim that stays valid
    // for minutes, so wiping the state out from under it would leave it
    // uploading against an enrollment the contributor has just revoked, into
    // a receipts file that no longer exists.
    match stop_running_daemon(store) {
        Ok(true) => println!("stopped the background daemon"),
        Ok(false) => {}
        Err(e) => {
            // Never block a logout on this: the wipe below removes the device
            // key, and the daemon refuses to upload without one.
            tracing::warn!(error = %e, "could not signal the daemon");
            println!("warning: could not signal the background daemon; state removed anyway");
        }
    }
    store.wipe().context("wiping contributor state")?;
    let _ = store.remove_daemon_file(crate::config::DAEMON_SOCK_FILE);
    let _ = store.remove_daemon_file(crate::config::DAEMON_LOCK_FILE);
    println!("logged out; local state removed");
    Ok(())
}

/// Ask a running daemon to stop, and wait briefly for it to let go of its
/// lock. Returns whether a daemon was there to stop.
fn stop_running_daemon(store: &ConfigStore) -> Result<bool> {
    use std::io::{BufRead, BufReader, Write};

    let sock = store.daemon_path(crate::config::DAEMON_SOCK_FILE);
    if !sock.exists() {
        return Ok(false);
    }
    let mut stream = match std::os::unix::net::UnixStream::connect(&sock) {
        Ok(s) => s,
        // A stale socket from a crashed daemon: nothing is running.
        Err(_) => return Ok(false),
    };
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    stream
        .write_all(b"{\"id\":0,\"method\":\"shutdown\"}\n")
        .context("sending shutdown")?;
    stream.flush().ok();
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply).ok();

    // Wait for the lock to be released, which is the daemon actually gone
    // rather than merely acknowledging.
    let lock_path = store.daemon_path(crate::config::DAEMON_LOCK_FILE);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if !lock_path.exists() {
            return Ok(true);
        }
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&lock_path) {
            if f.try_lock().is_ok() {
                return Ok(true);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    anyhow::bail!("daemon did not exit within 5s")
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

    let opts = SubmitOptions {
        dry_run: sel.dry_run,
        pii_filter: sel.pii_filter.map(str::to_string),
        no_reasoning: sel.no_reasoning,
        machine_readable: sel.json,
        unenrolled_preview,
        remediate_quarantined: sel.remediate_quarantined,
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
    let cfg = store
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
        if json {
            println!("{}", serde_json::json!({"withdrawn": true}));
        } else {
            println!("public attribution withdrawn; the row goes at the next snapshot");
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
    if json {
        println!(
            "{}",
            serde_json::json!({
                "display_handle": profile.display_handle,
                "bio": profile.bio,
                "public_since": profile.public_since,
            })
        );
    } else {
        println!("public handle: {}", profile.display_handle);
        println!("public since: {}", profile.public_since);
        println!("your handle appears once an accepted submission lands in the window");
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
        let err = resolve_consent_scopes(Some("bogus")).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("--scopes"), "{msg}");
        assert!(
            msg.contains("bogus") && msg.contains("model_training"),
            "{msg}"
        );
        assert!(!msg.contains("stored config"), "{msg}");
    }

    #[test]
    fn non_tty_default_falls_back_to_debugging_evaluation_only() {
        // `cargo test` runs with stdin that is not a terminal, so this
        // exercises the non-interactive silent-default branch rather than
        // the interactive prompt path.
        let scopes = resolve_consent_scopes(None).unwrap();
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
        )
        .await
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("not on the allowed-hosts list"), "{msg}");
        // No config was persisted.
        assert!(store.load_config().unwrap().is_none());
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
        };
        let mut t = crate::source::SessionTranscript {
            source: std::borrow::Cow::Borrowed("claude-code"),
            agent_version: None,
            model: None,
            project: None,
            cwd: None,
            started_at: None,
            session_hash: "sha256:aa".to_string(),
            events: vec![
                mk(SessionEventKind::User),
                mk(SessionEventKind::Reasoning),
                mk(SessionEventKind::Assistant),
            ],
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
async fn login_with_invite(
    store: &ConfigStore,
    invite: &str,
    allowed_hosts: Option<&str>,
    device: &DeviceIdentity,
    consent_scopes: Vec<String>,
) -> Result<()> {
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
    };
    store
        .save_config(&cfg)
        .context("saving contributor config")?;

    println!("enrolled: tenant_id={}", cfg.tenant_id);
    println!("this invite use is now spent");
    println!("run `whoami` to confirm, then `submit --dry-run` before contributing anything");
    Ok(())
}

/// Fetch a server-signed attestation of this contributor's own scores and
/// write it out.
///
/// This is what a contributor hands to a collector instead of a list of
/// submission ids. An id list is forgeable by anyone who learns the ids --
/// they have been published in plain text before now -- whereas forging an
/// attestation requires the server's signing key.
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

#[cfg(test)]
mod invite_tests {
    use super::parse_invite;

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
// These drive the same request handlers the IPC socket exposes, but in-process
// and marked as coming from a terminal. That is what lets `daemon project
// --mode auto` work here while the identical call over the socket is refused:
// a terminal is a capability an attacker with same-user code execution does
// not have.
// ---------------------------------------------------------------------------

use crate::daemon::ipc::{DaemonShared, Response, handle_local};
use crate::daemon::policy::ProjectMode;

/// Load daemon state for a one-shot command.
///
/// Reads the same files a running daemon uses. Mutating commands write them
/// back, and a running daemon picks the change up on its next pass.
fn daemon_shared(store: &ConfigStore) -> Result<DaemonShared> {
    DaemonShared::load(ConfigStore::open(store.dir().to_path_buf())?)
        .context("loading daemon state")
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

pub fn daemon_status(store: &ConfigStore, json: bool) -> Result<()> {
    let shared = daemon_shared(store)?;
    let resp = handle_local(&shared, "status", serde_json::json!({}));
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
    let shared = daemon_shared(store)?;
    let resp = handle_local(&shared, "list_pending", serde_json::json!({}));
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
    let shared = daemon_shared(store)?;
    let resp = handle_local(
        &shared,
        "preview",
        serde_json::json!({ "entry_id": entry_id }),
    );
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

pub fn daemon_approve(
    store: &ConfigStore,
    entry_id: Option<&str>,
    all: bool,
    json: bool,
) -> Result<()> {
    if !all && entry_id.is_none() {
        anyhow::bail!("give an entry id, or --all");
    }
    let shared = daemon_shared(store)?;
    let params = if all {
        serde_json::json!({ "all": true })
    } else {
        serde_json::json!({ "entry_id": entry_id })
    };
    let resp = handle_local(&shared, "approve", params);
    render(resp, json, |v| {
        println!("approved {}", v["approved"]);
    })
}

pub fn daemon_dismiss(store: &ConfigStore, entry_id: &str, json: bool) -> Result<()> {
    let shared = daemon_shared(store)?;
    let resp = handle_local(
        &shared,
        "dismiss",
        serde_json::json!({ "entry_id": entry_id }),
    );
    render(resp, json, |_| println!("dismissed"))
}

pub fn daemon_pause(store: &ConfigStore, pause: bool, json: bool) -> Result<()> {
    let shared = daemon_shared(store)?;
    let resp = handle_local(
        &shared,
        if pause { "pause" } else { "resume" },
        serde_json::json!({}),
    );
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
    let shared = daemon_shared(store)?;
    let resp = handle_local(&shared, "list_projects", serde_json::json!({}));
    render(resp, json, |v| {
        let empty = Vec::new();
        let projects = v["projects"].as_array().unwrap_or(&empty);
        if projects.is_empty() {
            println!("no projects configured; everything defaults to notify-only");
            return;
        }
        let rows: Vec<Vec<String>> = projects
            .iter()
            .map(|p| {
                vec![
                    p["project_label"].as_str().unwrap_or("-").to_string(),
                    p["mode"].as_str().unwrap_or("-").to_string(),
                ]
            })
            .collect();
        let _ = print_table(&mut std::io::stdout(), &["PROJECT", "MODE"], &rows);
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

pub fn daemon_set_project(store: &ConfigStore, path: &Path, mode: &str, json: bool) -> Result<()> {
    let mode = parse_project_mode(mode)?;
    let shared = daemon_shared(store)?;
    let key = path.to_string_lossy().to_string();
    let label = crate::daemon::policy::project_label_for(&key);
    let resp = handle_local(
        &shared,
        "set_project_mode",
        serde_json::json!({ "project_key": key, "label": label, "mode": mode }),
    );
    render(resp, json, |_| {
        println!(
            "{label}: {}",
            serde_json::to_string(&mode).unwrap_or_default()
        );
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
    let shared = daemon_shared(store)?;
    let resp = handle_local(
        &shared,
        "list_history",
        serde_json::json!({ "limit": limit }),
    );
    if json {
        // Emit history and rollup together so a caller gets one document.
        let rollup = handle_local(&shared, "history_rollup", serde_json::json!({}));
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

    let rollup = handle_local(&shared, "history_rollup", serde_json::json!({}));
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
    let shared = daemon_shared(store)?;
    if set.is_empty() {
        let resp = handle_local(&shared, "get_settings", serde_json::json!({}));
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
    let resp = handle_local(&shared, "set_settings", serde_json::Value::Object(params));
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
        let err = daemon_approve(&store, None, false, false).unwrap_err();
        assert!(err.to_string().contains("--all"), "{err}");
    }

    #[test]
    fn setting_a_project_to_auto_from_the_cli_is_persisted() {
        let (_d, store) = crate::config::tests_support::temp_store();
        daemon_set_project(
            &store,
            std::path::Path::new("/Users/z/code/proj"),
            "auto",
            false,
        )
        .unwrap();
        let policy = crate::daemon::policy::ProjectPolicy::load(&store).unwrap();
        assert_eq!(
            policy.resolve("/Users/z/code/proj"),
            ProjectMode::AutoUpload
        );
    }
}

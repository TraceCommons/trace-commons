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
pub fn whoami(store: &ConfigStore) -> Result<()> {
    let cfg = store
        .load_config()
        .context("loading contributor config")?
        .context("not logged in; run `login` first")?;
    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;

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
    store.wipe().context("wiping contributor state")?;
    println!("logged out; local state removed");
    Ok(())
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
pub fn list(trajectory: Option<&Path>) -> Result<()> {
    let sessions = discover_filtered(None, None, None, trajectory)?;
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
    /// Drop model reasoning from this run. Reasoning is included by default.
    pub no_reasoning: bool,
}

/// Drop reasoning events before envelope construction. Reasoning is captured
/// by default; this is the per-run opt-out behind `--no-reasoning`.
pub(crate) fn strip_reasoning(t: &mut SessionTranscript) {
    t.events
        .retain(|e| e.kind != crate::source::SessionEventKind::Reasoning);
}

/// Discover, filter, (optionally) interactively pick, redact, and submit
/// local sessions. Prints exactly one outcome line per session; returns an
/// error (nonzero exit) if any outcome was refused or failed.
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
    let cfg = store
        .load_config()
        .context("loading contributor config")?
        .context("not logged in; run `login` first")?;

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
    };
    let outcomes = submit::submit_sessions(store, &cfg, pairs, &opts).await?;

    if let Some(path) = sel.manifest {
        let entries = submit::build_manifest(&outcomes);
        let json = serde_json::to_string_pretty(&entries).context("serializing manifest")?;
        std::fs::write(path, json)
            .with_context(|| format!("writing manifest to {}", path.display()))?;
        println!("wrote {} envelope id(s) to manifest", entries.len());
    }

    let mut had_failure = false;
    for outcome in &outcomes {
        match outcome {
            SubmitOutcome::Submitted {
                submission_id,
                status,
            } => {
                println!("submitted {submission_id} {status}");
            }
            SubmitOutcome::AlreadySubmitted {
                submission_id,
                prior_status,
            } => {
                // Name the status it already has. "already-submitted" alone
                // reads as a failure when it usually means the trace was
                // accepted on an earlier run.
                println!("already-submitted {submission_id} ({prior_status})");
            }
            SubmitOutcome::SkippedParseFailure { reason_label } => {
                println!("skipped ({reason_label})");
            }
            SubmitOutcome::Refused { reason_label } => {
                println!("refused ({reason_label})");
                had_failure = true;
            }
            SubmitOutcome::Failed { reason_label } => {
                println!("failed ({reason_label})");
                had_failure = true;
            }
        }
    }

    if had_failure {
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
            no_reasoning: false,
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

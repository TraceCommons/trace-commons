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

use crate::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig, allowlist_for,
};
use crate::consent::{prompt_consent_answers, scopes_from_answers, validate_scopes};
use crate::identity::{
    DeviceIdentity, EnrollmentGrant, build_enroll_request, mint_grant, pem_to_pkcs8_der,
};
use crate::issuer_client::IssuerClient;
use crate::picker;
use crate::source::{SessionRef, TraceSource, all_sources};
use crate::submit::{self, SubmitOptions, SubmitOutcome};

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
    allowed_hosts: Option<&str>,
    scopes: Option<&str>,
) -> Result<()> {
    let consent_scopes = resolve_consent_scopes(scopes)?;

    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;

    let Some(grant_b64) = grant_b64 else {
        println!("device_key_id: {}", device.device_key_id);
        println!(
            "give this to your instance to mint an enrollment grant, then re-run \
             `login --grant <grant>`"
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

/// Discover every locally discoverable session across all sources, applying
/// optional `source`/`project`/`since` filters. `project` matches either the
/// session's basename (`SessionRef.project`) or a prefix of its full path;
/// `since` filters against `started_at` (falls back to excluding sessions
/// with no timestamp when set).
fn discover_filtered(
    source_filter: Option<&str>,
    project_filter: Option<&Path>,
    since: Option<chrono::Duration>,
) -> Result<Vec<SessionRef>> {
    let mut refs = Vec::new();
    for source in all_sources(None, None) {
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
                let basename = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                let basename_match = r.project.as_deref() == Some(basename);
                let prefix_match = r.path.starts_with(p);
                basename_match || prefix_match
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
fn source_for(name: &str) -> Option<Box<dyn TraceSource>> {
    all_sources(None, None)
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
pub fn list() -> Result<()> {
    let sessions = discover_filtered(None, None, None)?;
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
}

/// Discover, filter, (optionally) interactively pick, redact, and submit
/// local sessions. Prints exactly one outcome line per session; returns an
/// error (nonzero exit) if any outcome was refused or failed.
pub async fn submit(store: &ConfigStore, sel: &SubmitSelection<'_>) -> Result<()> {
    let cfg = store
        .load_config()
        .context("loading contributor config")?
        .context("not logged in; run `login` first")?;

    let since = sel.since.map(picker::parse_since).transpose()?;
    let mut refs = discover_filtered(sel.source, sel.project, since)?;
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
                let marker = source_for(r.source)
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
            let src = source_for(r.source)
                .with_context(|| format!("no adapter registered for source '{}'", r.source))?;
            Ok((src, r))
        })
        .collect::<Result<_>>()?;

    let opts = SubmitOptions {
        dry_run: sel.dry_run,
        pii_filter: sel.pii_filter.map(str::to_string),
    };
    let outcomes = submit::submit_sessions(store, &cfg, pairs, &opts).await?;

    let mut had_failure = false;
    for outcome in &outcomes {
        match outcome {
            SubmitOutcome::Submitted {
                submission_id,
                status,
            } => {
                println!("submitted {submission_id} {status}");
            }
            SubmitOutcome::AlreadySubmitted { submission_id } => {
                println!("already-submitted {submission_id}");
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
                format!("{:.2}", u.credit_points_pending),
                u.credit_points_final
                    .map(|f| format!("{f:.2}"))
                    .unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    print_table(
        &mut std::io::stdout(),
        &["SUBMISSION", "STATUS", "PENDING", "FINAL"],
        &rows,
    )
    .context("printing status table")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let err = login(&store, Some(&grant.encode()), Some("api.example"), None)
            .await
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("not on the allowed-hosts list"), "{msg}");
        // No config was persisted.
        assert!(store.load_config().unwrap().is_none());
    }
}

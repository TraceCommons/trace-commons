use chrono::{Duration as ChronoDuration, Utc};
use trace_commons_server::config::DatabaseConfig;
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::db::{Database, InviteGrantInsertOutcome, InviteGrantWrite};
use trace_commons_server::trace_invite_registry::{
    InviteTenantMode, generate_invite_code, import_file_invites,
};
use trace_commons_server::trace_upload_claim_allowlist::hash_invite_code;
use trace_commons_server::trace_upload_claim_issuer::{
    TraceUploadClaimIssuerConfig, UploadClaimIssuerHealthCheck, configure_invite_admin_from_env,
    configure_onboarding_device_key_registry_from_env, configure_tenant_access_grants_from_env,
    generate_upload_claim_keypair, mint_test_upload_claim, run_upload_claim_issuer_health_check,
    serve_trace_upload_claim_issuer,
};

const HELP_TEXT: &str = "trace-commons-upload-claim-issuer

Standalone Trace Commons Ed25519 upload-claim issuer.

USAGE:
    trace-commons-upload-claim-issuer [SUBCOMMAND]

SUBCOMMANDS:
    (none)                       Start the HTTP issuer (default)
    --generate-keypair           Print a fresh Ed25519 keypair (PKCS#8 + SPKI PEM)
                                 and a suggested kid (UUID v4) to stdout
    --health-check               Load env config, verify keys, exit 0 on success
                                 and 1 with a hash-only reason on failure
    --mint-test-claim            Mint a test upload claim for a hardcoded test
                                 tenant/principal and print the JWT to stdout
                                 (FOR TESTING / DEPLOY PROBES ONLY).
                                 Set TRACE_COMMONS_MINT_TEST_CLAIM_CONSENT_SCOPES
                                 and TRACE_COMMONS_MINT_TEST_CLAIM_ALLOWED_USES
                                 to comma-separated snake_case variant lists to
                                 populate the corresponding claim fields (both
                                 default to empty).
    --hash-invite-code <CODE>    Print the canonical sha256: hash of an invite
                                 code (the value the operator pastes into the
                                 pilot allowlist JSON file). Reads CODE from the
                                 next argument; use this rather than rolling a
                                 local sha256 helper so the hashing function
                                 stays in lockstep with the issuance handler.
    --import-file-invites <PATH> --policy-label <LABEL>
                                 One-time migration of an existing pilot
                                 allowlist JSON file's invite entries into the
                                 database. Idempotent on the invite hash, so a
                                 partial run is safe to repeat. Instance
                                 entries stay in the file and are counted, not
                                 imported. Prints counts only. Connects to the
                                 database directly via DATABASE_URL and
                                 TRACE_COMMONS_INVITE_REGISTRY_DATABASE_URL, so
                                 an operator can bootstrap before any admin
                                 token exists.
    --mint-invites <COUNT> --mint-tenant-template <TEMPLATE> --policy-label <LABEL>
                                 [--mint-max-uses <N>] [--mint-expires-in-days <N>]
                                 [--mint-note-label <LABEL>]
                                 [--mint-consent-scopes <a,b,c>] [--mint-allowed-uses <a,b,c>]
                                 Mint COUNT server-side invites directly against
                                 the database, replacing
                                 scripts/operator/generate-pilot-invites.py.
                                 Prints one raw 16-character code per line to
                                 stdout and nothing else, so an operator can
                                 redirect it to a file and delete it after
                                 handing the codes out. The raw code is never
                                 stored or logged; only its hash reaches the
                                 database.
    -V, --version                Print the version, the commit this binary was
                                 built from, and the build time. The same
                                 identity is on GET /health.
    -h, --help                   Print this help text

Environment variables are documented in docs/upload-claim-issuer.md.
Pilot allowlist operator guide: docs/operator/pilot-allowlist.md.
";

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let subcommand = args.first().map(String::as_str);

    match subcommand {
        Some("-h") | Some("--help") => {
            print!("{HELP_TEXT}");
            Ok(())
        }
        Some("-V") | Some("--version") => {
            println!(
                "{}",
                trace_commons_build_info::identity(
                    env!("CARGO_BIN_NAME"),
                    env!("CARGO_PKG_VERSION")
                )
            );
            Ok(())
        }
        Some("--generate-keypair") => run_generate_keypair(),
        Some("--health-check") => run_health_check(),
        Some("--mint-test-claim") => run_mint_test_claim(),
        Some("--hash-invite-code") => run_hash_invite_code(args.get(1).map(String::as_str)),
        Some("--import-file-invites") => run_import_file_invites(&args[1..]),
        Some("--mint-invites") => run_mint_invites(&args[1..]),
        Some(other) if other.starts_with("--") => {
            eprintln!("unknown subcommand: {other}\n");
            eprint!("{HELP_TEXT}");
            std::process::exit(2);
        }
        _ => run_server(),
    }
}

fn run_hash_invite_code(code: Option<&str>) -> anyhow::Result<()> {
    let Some(code) = code.map(str::trim).filter(|s| !s.is_empty()) else {
        eprintln!(
            "--hash-invite-code requires a CODE argument. Example:\n  \
             trace-commons-upload-claim-issuer --hash-invite-code INV-PILOT-001"
        );
        std::process::exit(2);
    };
    println!("{}", hash_invite_code(code));
    Ok(())
}

/// Find `--name value` in `args`. Only the flag pattern, not `--name=value`,
/// matching the rest of this binary's ad hoc arg handling.
fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn parse_comma_separated(value: Option<String>) -> Vec<String> {
    value
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Build a database config from the environment for the two subcommands that
/// run directly against the database rather than through the admin API.
/// Requires DATABASE_URL and, separately,
/// TRACE_COMMONS_INVITE_REGISTRY_DATABASE_URL (read internally by
/// `DatabaseConfig::from_postgres_url`) since invite writes/reads run on the
/// narrow registry pool, not the runtime pool.
fn database_config_from_env() -> anyhow::Result<DatabaseConfig> {
    let database_url =
        std::env::var("DATABASE_URL").map_err(|_| anyhow::anyhow!("DATABASE_URL must be set"))?;
    let pool_size: usize = std::env::var("DATABASE_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    Ok(DatabaseConfig::from_postgres_url(
        &database_url,
        pool_size.max(1),
    ))
}

/// `--import-file-invites <path> --policy-label <label>`: one-time migration.
/// Prints counts only.
fn run_import_file_invites(rest: &[String]) -> anyhow::Result<()> {
    let Some(path) = rest
        .first()
        .map(String::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        eprintln!(
            "--import-file-invites requires a PATH argument and --policy-label <LABEL>. Example:\n  \
             trace-commons-upload-claim-issuer --import-file-invites allowlist.json --policy-label pilot"
        );
        std::process::exit(2);
    };
    let Some(policy_label) = flag_value(rest, "--policy-label") else {
        eprintln!("--import-file-invites requires --policy-label <LABEL>");
        std::process::exit(2);
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let config = database_config_from_env()?;
        let backend = PgBackend::new(&config).await?;
        backend.run_migrations().await?;
        let summary =
            import_file_invites(&backend, std::path::Path::new(path), &policy_label).await?;
        println!(
            "imported={} already_present={} skipped_non_invite={}",
            summary.imported, summary.already_present, summary.skipped_non_invite
        );
        Ok::<(), anyhow::Error>(())
    })
}

/// `--mint-invites <count> --mint-tenant-template <template> --policy-label <label>
///   [--mint-max-uses <n>] [--mint-expires-in-days <n>] [--mint-note-label <label>]
///   [--mint-consent-scopes <a,b,c>] [--mint-allowed-uses <a,b,c>]`
///
/// Server-side operator batch, replacing generate-pilot-invites.py. Prints
/// one raw code per line to stdout and nothing else, so an operator can
/// redirect it to a file and delete it after handing the codes out. The raw
/// code is never stored or logged; only its hash reaches the database.
fn run_mint_invites(rest: &[String]) -> anyhow::Result<()> {
    let Some(count) = rest.first().and_then(|s| s.trim().parse::<u32>().ok()) else {
        eprintln!(
            "--mint-invites requires a COUNT argument, --mint-tenant-template <TEMPLATE>, \
             and --policy-label <LABEL>. Example:\n  \
             trace-commons-upload-claim-issuer --mint-invites 2 \
             --mint-tenant-template tmpl-pilot --policy-label pilot"
        );
        std::process::exit(2);
    };
    let Some(policy_label) = flag_value(rest, "--policy-label") else {
        eprintln!("--mint-invites requires --policy-label <LABEL>");
        std::process::exit(2);
    };
    let Some(mint_tenant_template) = flag_value(rest, "--mint-tenant-template") else {
        eprintln!("--mint-invites requires --mint-tenant-template <TEMPLATE>");
        std::process::exit(2);
    };
    let mint_max_uses: u32 = flag_value(rest, "--mint-max-uses")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let mint_expires_in_days: Option<i64> =
        flag_value(rest, "--mint-expires-in-days").and_then(|s| s.parse().ok());
    let mint_note_label = flag_value(rest, "--mint-note-label");
    let mint_consent_scopes = parse_comma_separated(flag_value(rest, "--mint-consent-scopes"));
    let mint_allowed_uses = parse_comma_separated(flag_value(rest, "--mint-allowed-uses"));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let config = database_config_from_env()?;
        let backend = PgBackend::new(&config).await?;
        backend.run_migrations().await?;
        for _ in 0..count {
            let code = generate_invite_code();
            let write = InviteGrantWrite {
                invite_subject_hash: hash_invite_code(&code),
                policy_label: policy_label.clone(),
                tenant_mode: InviteTenantMode::Derived,
                fixed_tenant_id: None,
                tenant_template_id: Some(mint_tenant_template.clone()),
                policy_version: "v1".to_string(),
                allowed_consent_scopes: mint_consent_scopes.clone(),
                allowed_uses: mint_allowed_uses.clone(),
                max_uses: mint_max_uses,
                expires_at: mint_expires_in_days.map(|d| Utc::now() + ChronoDuration::days(d)),
                issuance_source: "operator".to_string(),
                issued_by_label: None,
                credential_binding_hash: None,
                note_label: mint_note_label.clone(),
            };
            match backend.insert_invite_grant(write).await? {
                InviteGrantInsertOutcome::Inserted => println!("{code}"),
                other => anyhow::bail!("invite mint failed: {other:?}"),
            }
        }
        Ok::<(), anyhow::Error>(())
    })
}

fn run_generate_keypair() -> anyhow::Result<()> {
    let keypair = generate_upload_claim_keypair()?;
    print!("{}", keypair.private_key_pem);
    print!("{}", keypair.public_key_pem);
    println!("kid: {}", keypair.suggested_kid);
    Ok(())
}

fn run_health_check() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run_upload_claim_issuer_health_check());
    match result {
        UploadClaimIssuerHealthCheck::Ok => {
            println!("OK");
            Ok(())
        }
        UploadClaimIssuerHealthCheck::Fail(reason) => {
            println!("FAIL: {reason}");
            std::process::exit(1);
        }
    }
}

fn run_mint_test_claim() -> anyhow::Result<()> {
    let token = mint_test_upload_claim()?;
    println!("{token}");
    Ok(())
}

fn run_server() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| {
            "trace_commons_upload_claim_issuer=info,trace_commons_server=info".into()
        }))
        .init();
    let mut config = TraceUploadClaimIssuerConfig::from_env()?;
    configure_tenant_access_grants_from_env(&mut config).await?;
    configure_onboarding_device_key_registry_from_env(&mut config).await?;
    configure_invite_admin_from_env(&mut config).await?;
    serve_trace_upload_claim_issuer(config).await
}

use trace_commons_server::trace_upload_claim_allowlist::hash_invite_code;
use trace_commons_server::trace_upload_claim_issuer::{
    TraceUploadClaimIssuerConfig, UploadClaimIssuerHealthCheck,
    configure_tenant_access_grants_from_env, generate_upload_claim_keypair, mint_test_upload_claim,
    run_upload_claim_issuer_health_check, serve_trace_upload_claim_issuer,
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
                                 (FOR TESTING / DEPLOY PROBES ONLY)
    --hash-invite-code <CODE>    Print the canonical sha256: hash of an invite
                                 code (the value the operator pastes into the
                                 pilot allowlist JSON file). Reads CODE from the
                                 next argument; use this rather than rolling a
                                 local sha256 helper so the hashing function
                                 stays in lockstep with the issuance handler.
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
        Some("--generate-keypair") => run_generate_keypair(),
        Some("--health-check") => run_health_check(),
        Some("--mint-test-claim") => run_mint_test_claim(),
        Some("--hash-invite-code") => run_hash_invite_code(args.get(1).map(String::as_str)),
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
    serve_trace_upload_claim_issuer(config).await
}

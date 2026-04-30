use tracedao_server::trace_upload_claim_issuer::{
    TraceUploadClaimIssuerConfig, configure_tenant_access_grants_from_env,
    serve_trace_upload_claim_issuer,
};

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| {
                "tracedao_upload_claim_issuer=info,tracedao_server=info".into()
            }),
        )
        .init();
    let mut config = TraceUploadClaimIssuerConfig::from_env()?;
    configure_tenant_access_grants_from_env(&mut config).await?;
    serve_trace_upload_claim_issuer(config).await
}

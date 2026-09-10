//! Retry one durable lease-release intent per pass, including after restart.
use super::ipc::DaemonShared;
use crate::{token_bundle::BundleJournal, token_capture_client::TokenCaptureClient};

pub(super) async fn pass(shared: &DaemonShared) -> anyhow::Result<()> {
    let root = shared.store.dir().join("token-bundles");
    if !root.exists() {
        return Ok(());
    }
    let declaration = shared
        .settings
        .lock()
        .expect("settings lock")
        .ironwire
        .clone();
    let Some(declaration) = declaration else {
        return Ok(());
    };
    let port = declaration
        .port()
        .filter(|port| *port > 0)
        .ok_or_else(|| anyhow::anyhow!("token-cleanup-proxy-unavailable"))?;
    let path = super::settings::ironwire_token_path(declaration.token_dir())
        .ok_or_else(|| anyhow::anyhow!("token-cleanup-proxy-unavailable"))?;
    let metadata = super::ironwire_pointer::trustworthy_file(&path)
        .ok_or_else(|| anyhow::anyhow!("token-cleanup-proxy-untrusted"))?;
    if metadata.len() > 4096 {
        anyhow::bail!("token-cleanup-proxy-untrusted");
    }
    let token = std::fs::read_to_string(path)?;
    let client = TokenCaptureClient::new(&format!("http://127.0.0.1:{port}"), token.trim().into())?;
    let journal = BundleJournal::open(&root)?;
    if let Some(id) = journal.pending_cleanup()?.into_iter().next() {
        journal.cleanup(id, &client).await?;
    }
    Ok(())
}

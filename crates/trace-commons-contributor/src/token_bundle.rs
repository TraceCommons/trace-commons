//! Durable acknowledgement and lease-release journal for token bundles.
//!
//! Payload ownership remains with Ironwire. A receipt from an authenticated,
//! configured destination must be persisted before this module releases a lease.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};
use trace_commons_protocol::token_distribution::{
    ContributionBundleManifest, DurableBundleReceipt,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleDestination {
    pub server_id: String,
    pub tenant_id: String,
    pub account_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleLease {
    pub capture_store_id: String,
    pub lease_id: String,
    pub owner: String,
    pub snapshot_digest: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    manifest: ContributionBundleManifest,
    destination: BundleDestination,
    lease: BundleLease,
    receipt: Option<DurableBundleReceipt>,
    released: bool,
    #[serde(default)]
    approved_payload: Option<CertifiedBundleUpload>,
}

/// Only the caller's immutable lease is released. Implementations must not
/// delete a session directory or release another destination's lease.
#[async_trait::async_trait]
pub trait BundleLeaseReleaser: Send + Sync {
    async fn release(&self, lease: &BundleLease) -> Result<()>;
}

pub struct BundleJournal {
    root: PathBuf,
}
impl BundleJournal {
    /// The directory must be inside the contributor's private state directory.
    pub fn open(root: &Path) -> Result<Self> {
        if !root.exists() {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(root).context("bundle-journal-create")?;
        }
        let metadata = fs::symlink_metadata(root).context("bundle-journal-metadata")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!("bundle-journal-directory");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                bail!("bundle-journal-permissions");
            }
        }
        Ok(Self { root: root.into() })
    }
    fn path(&self, id: uuid::Uuid) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }
    fn lock(&self) -> Result<File> {
        let path = self.root.join("journal.lock");
        if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
            bail!("bundle-journal-lock");
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path).context("bundle-journal-lock")?;
        file.lock().context("bundle-journal-lock")?;
        Ok(file)
    }
    fn read(&self, id: uuid::Uuid) -> Result<Entry> {
        let path = self.path(id);
        let m = fs::symlink_metadata(&path).context("bundle-journal-read")?;
        if !m.is_file() || m.file_type().is_symlink() || m.len() > 128 * 1024 * 1024 {
            bail!("bundle-journal-invalid");
        }
        serde_json::from_slice(&fs::read(path).context("bundle-journal-read")?)
            .context("bundle-journal-invalid")
    }
    fn write(&self, id: uuid::Uuid, entry: &Entry) -> Result<()> {
        let bytes = serde_json::to_vec(entry).context("bundle-journal-encode")?;
        if bytes.len() > 128 * 1024 * 1024 {
            bail!("bundle-journal-limit");
        }
        let mut total = bytes.len() as u64;
        for item in fs::read_dir(&self.root)? {
            let item = item?;
            if item.path() != self.path(id) && item.path().extension().is_some_and(|s| s == "json")
            {
                total = total.saturating_add(item.metadata()?.len());
            }
        }
        if total > 256 * 1024 * 1024 {
            bail!("bundle-journal-capacity");
        }
        crate::config::write_atomic_0600(&self.root, &self.path(id), &bytes)
            .context("bundle-journal-write")?;
        #[cfg(unix)]
        File::open(&self.root)?
            .sync_all()
            .context("bundle-journal-sync")?;
        Ok(())
    }
    /// Call before upload. Retries must reuse the returned journal ID.
    pub fn prepare(
        &self,
        manifest: ContributionBundleManifest,
        destination: BundleDestination,
        lease: BundleLease,
    ) -> Result<uuid::Uuid> {
        manifest.validate().context("bundle-manifest-invalid")?;
        for value in [
            &destination.server_id,
            &destination.tenant_id,
            &destination.account_id,
            &lease.capture_store_id,
            &lease.lease_id,
            &lease.owner,
            &lease.snapshot_digest,
        ] {
            if value.is_empty() || value.len() > 1024 {
                bail!("bundle-binding-invalid");
            }
        }
        let _lock = self.lock()?;
        if fs::read_dir(&self.root)?
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|s| s == "json"))
            .count()
            >= 4096
        {
            bail!("bundle-journal-limit");
        }
        let id = uuid::Uuid::new_v4();
        self.write(
            id,
            &Entry {
                manifest,
                destination,
                lease,
                receipt: None,
                released: false,
                approved_payload: None,
            },
        )?;
        Ok(id)
    }
    /// This method is called only after authentication of the response transport.
    /// An upload status, local JSON file, or transcript receipt is insufficient.
    pub fn acknowledge_authenticated(
        &self,
        id: uuid::Uuid,
        receipt: DurableBundleReceipt,
        now: u64,
    ) -> Result<()> {
        let _lock = self.lock()?;
        let mut entry = self.read(id)?;
        receipt
            .matches_bundle(
                &entry.manifest,
                &entry.destination.server_id,
                &entry.destination.tenant_id,
                &entry.destination.account_id,
                now,
            )
            .context("bundle-receipt-mismatch")?;
        if let Some(existing) = &entry.receipt {
            if existing != &receipt {
                bail!("bundle-receipt-conflict");
            }
            return Ok(());
        }
        entry.receipt = Some(receipt);
        self.write(id, &entry)
    }
    /// Safe to retry after a crash, including a crash after remote release.
    /// Expired receipts remain evidence of an earlier durable commit.
    pub async fn cleanup(&self, id: uuid::Uuid, releaser: &dyn BundleLeaseReleaser) -> Result<()> {
        let lease = {
            let _lock = self.lock()?;
            let entry = self.read(id)?;
            if entry.released {
                return Ok(());
            }
            if entry.receipt.is_none() {
                bail!("bundle-not-acknowledged");
            }
            entry.lease
        };
        releaser.release(&lease).await?;
        let _lock = self.lock()?;
        let mut entry = self.read(id)?;
        entry.released = true;
        entry.approved_payload = None;
        self.write(id, &entry)
    }
    /// Only acknowledged, unfinished intents are retried at daemon startup.
    pub fn pending_cleanup(&self) -> Result<Vec<uuid::Uuid>> {
        let _lock = self.lock()?;
        let mut pending = Vec::new();
        for item in fs::read_dir(&self.root)? {
            let path = item?.path();
            if path.extension().is_none_or(|s| s != "json") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|v| v.to_str())
                .and_then(|v| uuid::Uuid::parse_str(v).ok())
                .context("bundle-journal-invalid")?;
            let entry = self.read(id)?;
            if entry.receipt.is_some() && !entry.released {
                pending.push(id);
            }
        }
        Ok(pending)
    }
}

/// Certified bytes are uploaded without reserialization. The journal ID must
/// have been prepared for this destination and immutable capture snapshot.
impl BundleJournal {
    /// Pin a reviewed payload to the already prepared immutable manifest.
    /// Existing approval is never overwritten with newly witnessed bytes.
    pub fn approve_payload(&self, id: uuid::Uuid, payload: CertifiedBundleUpload) -> Result<()> {
        use crate::witness::transport::verify_certificate;
        verify_certificate(&payload.envelope, &payload.pinned_witness_address)
            .context("bundle-envelope-certificate")?;
        verify_certificate(&payload.manifest, &payload.pinned_witness_address)
            .context("bundle-manifest-certificate")?;
        let manifest = ContributionBundleManifest::decode(&payload.manifest.envelope_bytes)?;
        let _lock = self.lock()?;
        let mut entry = self.read(id)?;
        if manifest.digest()? != entry.manifest.digest()?
            || !manifest
                .envelope_digest
                .matches(&payload.envelope.envelope_bytes)
        {
            bail!("bundle-approval-mismatch");
        }
        if entry.approved_payload.is_some() {
            bail!("bundle-already-approved");
        }
        entry.approved_payload = Some(payload);
        self.write(id, &entry)
    }
    /// Upload only the exact bytes persisted during approval, including after
    /// restart. This never calls the witness again to rebuild approved content.
    pub async fn upload_approved(
        &self,
        id: uuid::Uuid,
        client: &trace_commons_operator_client::Client,
    ) -> Result<DurableBundleReceipt> {
        let payload = {
            let _lock = self.lock()?;
            self.read(id)?
                .approved_payload
                .context("bundle-not-approved")?
        };
        self.upload(id, client, &payload).await
    }
    pub async fn upload(
        &self,
        id: uuid::Uuid,
        client: &trace_commons_operator_client::Client,
        payload: &CertifiedBundleUpload,
    ) -> Result<DurableBundleReceipt> {
        use crate::witness::transport::{
            WITNESS_CERTIFICATE_HEADER, WITNESS_SIGNATURE_HEADER, verify_certificate,
        };
        use reqwest::Method;
        let entry = {
            let _lock = self.lock()?;
            self.read(id)?
        };
        verify_certificate(&payload.envelope, &payload.pinned_witness_address)
            .context("bundle-envelope-certificate")?;
        verify_certificate(&payload.manifest, &payload.pinned_witness_address)
            .context("bundle-manifest-certificate")?;
        let manifest = ContributionBundleManifest::decode(&payload.manifest.envelope_bytes)
            .context("bundle-manifest-invalid")?;
        if manifest.digest()? != entry.manifest.digest()?
            || !manifest
                .envelope_digest
                .matches(&payload.envelope.envelope_bytes)
            || payload.attachments.len() != manifest.attachments.len()
        {
            bail!("bundle-upload-mismatch");
        }
        let envelope: trace_commons_protocol::trace_contribution::TraceContributionEnvelope =
            serde_json::from_slice(&payload.envelope.envelope_bytes)
                .context("bundle-envelope-invalid")?;
        for descriptor in &manifest.attachments {
            let bytes = payload
                .attachments
                .get(&descriptor.artifact_id)
                .context("bundle-attachment-missing")?;
            let text = envelope
                .events
                .iter()
                .find(|e| e.event_id.to_string() == descriptor.event_id)
                .and_then(|e| e.redacted_content.as_deref())
                .context("bundle-event-missing")?;
            manifest
                .verify_sanitized_attachment(&descriptor.artifact_id, bytes, text.as_bytes())
                .context("bundle-attachment-invalid")?;
        }
        // Encode path components independently: identifiers are never URL paths.
        let component = |value: &str| -> Result<String> {
            if !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            {
                bail!("bundle-path-invalid");
            }
            Ok(value.into())
        };
        let base = format!(
            "/v1/token-bundles/{}/{}",
            component(&manifest.submission_id)?,
            component(&manifest.bundle_revision)?
        );
        let witness_headers = |value: &crate::witness::transport::WitnessedEnvelope| {
            vec![
                (
                    WITNESS_CERTIFICATE_HEADER.to_string(),
                    value.certificate_json.clone(),
                ),
                (
                    WITNESS_SIGNATURE_HEADER.to_string(),
                    value.signature_hex.clone(),
                ),
            ]
        };
        let manifest_headers = witness_headers(&payload.manifest);
        let headers: Vec<_> = manifest_headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let response = client
            .call_bytes(
                Method::POST,
                "/v1/token-bundles",
                &[],
                &payload.manifest.envelope_bytes,
                &headers,
            )
            .await
            .map_err(|_| anyhow::anyhow!("bundle-upload-unavailable"))?;
        let response: serde_json::Value =
            serde_json::from_str(&response).context("bundle-response-invalid")?;
        if response["server_id"].as_str() != Some(&entry.destination.server_id)
            || response["tenant_id"].as_str() != Some(&entry.destination.tenant_id)
            || response["account_id"].as_str() != Some(&entry.destination.account_id)
        {
            bail!("bundle-destination-mismatch");
        }
        if response["state"].as_str() == Some("committed") {
            let receipt = serde_json::from_value(response["receipt"].clone())
                .context("bundle-receipt-invalid")?;
            self.acknowledge_authenticated(
                id,
                receipt,
                chrono::Utc::now().timestamp().max(0) as u64,
            )?;
            return self.read(id)?.receipt.context("bundle-receipt-missing");
        }
        let ready: Vec<String> =
            serde_json::from_value(response["ready"].clone()).context("bundle-response-invalid")?;
        for (artifact, bytes) in std::iter::once(("envelope", &payload.envelope.envelope_bytes))
            .chain(payload.attachments.iter().map(|(k, v)| (k.as_str(), v)))
        {
            if ready.iter().any(|v| v == artifact) {
                continue;
            }
            client
                .call_bytes(
                    Method::PUT,
                    &format!("{base}/{}", component(artifact)?),
                    &[],
                    bytes,
                    &[],
                )
                .await
                .map_err(|_| anyhow::anyhow!("bundle-upload-unavailable"))?;
        }
        let mut envelope_headers = witness_headers(&payload.envelope);
        if let Some(admission) = &payload.envelope.admission {
            envelope_headers.push((
                "x-trace-admission-evidence".into(),
                admission.evidence_json.clone(),
            ));
            envelope_headers.push((
                "x-trace-admission-signature".into(),
                admission.signature_hex.clone(),
            ));
        }
        let headers: Vec<_> = envelope_headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let response = client
            .call_bytes(
                Method::POST,
                &base,
                &[],
                &payload.envelope.envelope_bytes,
                &headers,
            )
            .await
            .map_err(|_| anyhow::anyhow!("bundle-upload-unavailable"))?;
        let receipt = serde_json::from_str(&response).context("bundle-receipt-invalid")?;
        self.acknowledge_authenticated(id, receipt, chrono::Utc::now().timestamp().max(0) as u64)?;
        self.read(id)?.receipt.context("bundle-receipt-missing")
    }
}

/// Contains private content; deliberately does not implement Debug.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedBundleUpload {
    pub pinned_witness_address: String,
    pub envelope: crate::witness::transport::WitnessedEnvelope,
    pub manifest: crate::witness::transport::WitnessedEnvelope,
    pub attachments: std::collections::BTreeMap<String, Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use trace_commons_protocol::token_distribution::*;
    struct Release {
        calls: AtomicUsize,
        fail: bool,
    }
    #[async_trait::async_trait]
    impl BundleLeaseReleaser for Release {
        async fn release(&self, lease: &BundleLease) -> Result<()> {
            assert_eq!(lease.owner, "destination-owner");
            assert_eq!(lease.lease_id, "immutable-lease");
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                bail!("offline");
            }
            Ok(())
        }
    }
    fn fixture() -> (
        ContributionBundleManifest,
        BundleDestination,
        BundleLease,
        DurableBundleReceipt,
    ) {
        let manifest = ContributionBundleManifest {
            version: 1,
            usage_profile: TokenUsageProfile::RestrictedResearch,
            submission_id: uuid::Uuid::new_v4().to_string(),
            bundle_revision: "revision".into(),
            envelope_digest: ContentDigest::of(b"envelope"),
            consent_digest: ContentDigest::of(b"consent"),
            policy_version: "policy".into(),
            attachments: vec![AttachmentDescriptor {
                artifact_id: "tokens".into(),
                event_id: "event".into(),
                content_digest: ContentDigest::of(b"tokens"),
                size_bytes: 6,
            }],
        };
        let destination = BundleDestination {
            server_id: "server".into(),
            tenant_id: "tenant".into(),
            account_id: "account".into(),
        };
        let lease = BundleLease {
            capture_store_id: "store".into(),
            lease_id: "immutable-lease".into(),
            owner: "destination-owner".into(),
            snapshot_digest: "snapshot".into(),
        };
        let receipt = DurableBundleReceipt {
            version: 1,
            server_id: "server".into(),
            tenant_id: "tenant".into(),
            account_id: "account".into(),
            submission_id: manifest.submission_id.clone(),
            bundle_revision: manifest.bundle_revision.clone(),
            manifest_digest: manifest.digest().unwrap(),
            committed_at_unix: 100,
            retain_until_unix: 200,
            retention_policy_version: "policy".into(),
        };
        (manifest, destination, lease, receipt)
    }
    #[tokio::test]
    async fn release_requires_bound_durable_receipt_and_recovers_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("journal");
        let journal = BundleJournal::open(&root).unwrap();
        let (manifest, destination, lease, receipt) = fixture();
        let id = journal.prepare(manifest, destination, lease).unwrap();
        let release = Release {
            calls: AtomicUsize::new(0),
            fail: false,
        };
        assert!(journal.cleanup(id, &release).await.is_err());
        assert_eq!(release.calls.load(Ordering::SeqCst), 0);
        let mut wrong = receipt.clone();
        wrong.account_id = "someone-else".into();
        assert!(journal.acknowledge_authenticated(id, wrong, 101).is_err());
        assert!(journal.pending_cleanup().unwrap().is_empty());
        journal.acknowledge_authenticated(id, receipt, 101).unwrap();
        drop(journal);
        let journal = BundleJournal::open(&root).unwrap();
        assert_eq!(journal.pending_cleanup().unwrap(), vec![id]);
        assert!(
            journal
                .cleanup(
                    id,
                    &Release {
                        calls: AtomicUsize::new(0),
                        fail: true
                    }
                )
                .await
                .is_err()
        );
        assert_eq!(journal.pending_cleanup().unwrap(), vec![id]);
        journal.cleanup(id, &release).await.unwrap();
        journal.cleanup(id, &release).await.unwrap();
        assert_eq!(release.calls.load(Ordering::SeqCst), 1);
        assert!(journal.pending_cleanup().unwrap().is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_journal_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path(), &link).unwrap();
        assert!(BundleJournal::open(&link).is_err());
    }
}

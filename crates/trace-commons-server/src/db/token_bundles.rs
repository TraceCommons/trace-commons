// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::postgres::PgBackend;
use crate::{error::DatabaseError, token_bundle_store::*};
use chrono::{DateTime, Utc};
use trace_commons_protocol::token_distribution::DurableBundleReceipt;
use uuid::Uuid;
type Result<T> = std::result::Result<T, DatabaseError>;
fn invalid() -> DatabaseError {
    DatabaseError::Query("TokenBundleConflict".into())
}
fn encode<T: serde::Serialize>(value: &T) -> Result<serde_json::Value> {
    serde_json::to_value(value).map_err(|_| invalid())
}
fn decode<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| invalid())
}
async fn row(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    submission: Uuid,
    revision: &str,
    owner: &str,
) -> Result<Option<StoredTokenBundle>> {
    let Some(r)=tx.query_opt("SELECT * FROM trace_token_bundles WHERE tenant_id=$1 AND submission_id=$2 AND revision=$3 AND owner_ref=$4",&[&tenant,&submission,&revision,&owner]).await? else {return Ok(None)};
    let mut attachments = Vec::new();
    for a in tx.query("SELECT artifact_id,object_ref,deleted,ready,prepared FROM trace_token_attachments WHERE tenant_id=$1 AND submission_id=$2 AND revision=$3 ORDER BY artifact_id",&[&tenant,&submission,&revision]).await? {
        attachments.push(StoredTokenObject{artifact_id:a.get(0),object_ref:decode(a.get(1))?,deleted:a.get(2),ready:a.get(3),prepared:a.get(4)});
    }
    Ok(Some(StoredTokenBundle {
        tenant_id: tenant.into(),
        submission_id: submission,
        revision: revision.into(),
        owner_ref: owner.into(),
        manifest: decode(r.get("manifest"))?,
        witness_headers: decode(r.get("witness_headers"))?,
        state: r.get("state"),
        expires_at: r.get("expires_at"),
        receipt: r
            .get::<_, Option<serde_json::Value>>("receipt")
            .map(decode)
            .transpose()?,
        attachments,
    }))
}
async fn lock(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    submission: Uuid,
) -> Result<()> {
    // Shared by every bundle revision and parent withdrawal's row lock.
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 734892))",
        &[&format!("{}:{}:{}", tenant.len(), tenant, submission)],
    )
    .await?;
    Ok(())
}
pub(super) async fn begin_token_bundle(
    db: &PgBackend,
    bundle: StoredTokenBundle,
) -> Result<StoredTokenBundle> {
    bundle.manifest.validate().map_err(|_| invalid())?;
    if bundle.manifest.submission_id != bundle.submission_id.to_string()
        || bundle.manifest.bundle_revision != bundle.revision
        || bundle.state != "staging"
        || !bundle.attachments.is_empty()
        || bundle.receipt.is_some()
    {
        return Err(invalid());
    }
    db.ensure_trace_tenant(&bundle.tenant_id).await?;
    let mut client = db.trace_pool().get().await?;
    let tx = PgBackend::begin_trace_tenant_transaction(&mut client, &bundle.tenant_id).await?;
    // Same parent-before-bundle lock order as finalize and revocation. A
    // begin racing withdrawal must see the committed tombstone, rather than
    // insert a new revision after the withdrawal trigger scanned old rows.
    tx.query_opt("SELECT submission_id FROM trace_submissions WHERE tenant_id=$1 AND submission_id=$2 FOR UPDATE", &[&bundle.tenant_id, &bundle.submission_id]).await?;
    lock(&tx, &bundle.tenant_id, bundle.submission_id).await?;
    let withdrawn:bool=tx.query_one("SELECT EXISTS(SELECT 1 FROM trace_withdrawals WHERE tenant_id=$1 AND submission_id=$2) OR EXISTS(SELECT 1 FROM trace_submissions WHERE tenant_id=$1 AND submission_id=$2 AND (status='revoked' OR withdrawn_at IS NOT NULL OR purged_at IS NOT NULL))",&[&bundle.tenant_id,&bundle.submission_id]).await?.get(0);
    if withdrawn {
        return Err(invalid());
    }
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 734893))",
        &[&bundle.tenant_id],
    )
    .await?;
    let active: i64 = tx.query_one("SELECT count(*) FROM trace_token_bundles WHERE tenant_id=$1 AND state IN ('staging','committed')", &[&bundle.tenant_id]).await?.get(0);
    if active >= 128
        && row(
            &tx,
            &bundle.tenant_id,
            bundle.submission_id,
            &bundle.revision,
            &bundle.owner_ref,
        )
        .await?
        .is_none()
    {
        return Err(invalid());
    }
    let pending: i64 = tx
        .query_one(
            "SELECT count(*) FROM trace_token_bundles WHERE tenant_id=$1 AND state='staging'",
            &[&bundle.tenant_id],
        )
        .await?
        .get(0);
    if pending >= 16
        && row(
            &tx,
            &bundle.tenant_id,
            bundle.submission_id,
            &bundle.revision,
            &bundle.owner_ref,
        )
        .await?
        .is_none()
    {
        return Err(invalid());
    }
    let digest = bundle.manifest.digest().map_err(|_| invalid())?;
    tx.execute("INSERT INTO trace_token_bundles(tenant_id,submission_id,revision,owner_ref,manifest_digest,manifest,state,expires_at,witness_headers) VALUES($1,$2,$3,$4,$5,$6,'staging',$7,$8) ON CONFLICT DO NOTHING",&[&bundle.tenant_id,&bundle.submission_id,&bundle.revision,&bundle.owner_ref,&digest.as_str(),&encode(&bundle.manifest)?,&bundle.expires_at,&encode(&bundle.witness_headers)?]).await?;
    let stored = row(
        &tx,
        &bundle.tenant_id,
        bundle.submission_id,
        &bundle.revision,
        &bundle.owner_ref,
    )
    .await?
    .ok_or_else(invalid)?;
    if stored.manifest.digest().map_err(|_| invalid())? != digest || stored.state == "revoked" {
        return Err(invalid());
    }
    tx.commit().await?;
    Ok(stored)
}
pub(super) async fn get_token_bundle(
    db: &PgBackend,
    tenant: &str,
    submission: Uuid,
    revision: &str,
    owner: &str,
) -> Result<Option<StoredTokenBundle>> {
    let mut client = db.trace_pool().get().await?;
    let tx = PgBackend::begin_trace_tenant_transaction(&mut client, tenant).await?;
    let value = row(&tx, tenant, submission, revision, owner).await?;
    tx.commit().await?;
    Ok(value)
}
pub(super) async fn stage_token_object(
    db: &PgBackend,
    tenant: &str,
    submission: Uuid,
    revision: &str,
    owner: &str,
    object: StoredTokenObject,
) -> Result<()> {
    let mut client = db.trace_pool().get().await?;
    let tx = PgBackend::begin_trace_tenant_transaction(&mut client, tenant).await?;
    lock(&tx, tenant, submission).await?;
    let bundle = row(&tx, tenant, submission, revision, owner)
        .await?
        .ok_or_else(invalid)?;
    if bundle.state != "staging"
        || bundle.expires_at <= Utc::now()
        || (object.artifact_id != "envelope"
            && !bundle
                .manifest
                .attachments
                .iter()
                .any(|a| a.artifact_id == object.artifact_id))
        || object.deleted
        || object.ready
        || object
            .prepared
            .as_ref()
            .is_none_or(|p| p.len() > 12 * 1024 * 1024)
    {
        return Err(invalid());
    }
    let existing=tx.query_opt("SELECT object_ref FROM trace_token_attachments WHERE tenant_id=$1 AND submission_id=$2 AND revision=$3 AND artifact_id=$4",&[&tenant,&submission,&revision,&object.artifact_id]).await?;
    if let Some(existing) = existing {
        if existing.get::<_, serde_json::Value>(0) != encode(&object.object_ref)? {
            return Err(invalid());
        }
    } else {
        tx.execute("INSERT INTO trace_token_attachments(tenant_id,submission_id,revision,artifact_id,object_ref,prepared) VALUES($1,$2,$3,$4,$5,$6)",&[&tenant,&submission,&revision,&object.artifact_id,&encode(&object.object_ref)?,&object.prepared]).await?;
    }
    tx.commit().await?;
    Ok(())
}
pub(super) async fn commit_token_bundle(
    db: &PgBackend,
    tenant: &str,
    submission: Uuid,
    revision: &str,
    owner: &str,
    receipt: DurableBundleReceipt,
) -> Result<DurableBundleReceipt> {
    let mut client = db.trace_pool().get().await?;
    let tx = PgBackend::begin_trace_tenant_transaction(&mut client, tenant).await?;
    // Parent first: revocation's trigger takes the bundle lock in this order.
    let parent=tx.query_opt("SELECT auth_principal_ref,status,withdrawn_at,purged_at FROM trace_submissions WHERE tenant_id=$1 AND submission_id=$2 FOR UPDATE",&[&tenant,&submission]).await?.ok_or_else(invalid)?;
    if parent.get::<_, String>(0) != owner
        || parent.get::<_, String>(1) == "revoked"
        || parent.get::<_, Option<DateTime<Utc>>>(2).is_some()
        || parent.get::<_, Option<DateTime<Utc>>>(3).is_some()
    {
        return Err(invalid());
    }
    lock(&tx, tenant, submission).await?;
    let bundle = row(&tx, tenant, submission, revision, owner)
        .await?
        .ok_or_else(invalid)?;
    if bundle.state == "revoked" {
        return Err(invalid());
    }
    if let Some(existing) = bundle.receipt {
        tx.commit().await?;
        return Ok(existing);
    }
    if bundle.expires_at <= Utc::now()
        || bundle.attachments.len() != bundle.manifest.attachments.len() + 1
        || bundle.attachments.iter().any(|a| a.deleted || !a.ready)
    {
        return Err(invalid());
    }
    if receipt.submission_id != submission.to_string()
        || receipt.bundle_revision != revision
        || receipt.tenant_id != tenant
        || receipt.account_id != owner
        || receipt.manifest_digest != bundle.manifest.digest().map_err(|_| invalid())?
    {
        return Err(invalid());
    }
    receipt
        .matches_bundle(
            &bundle.manifest,
            &receipt.server_id,
            tenant,
            owner,
            Utc::now().timestamp() as u64,
        )
        .map_err(|_| invalid())?;
    let expires = DateTime::from_timestamp(
        i64::try_from(receipt.retain_until_unix).map_err(|_| invalid())?,
        0,
    )
    .ok_or_else(invalid)?;
    tx.execute("UPDATE trace_token_bundles SET state='committed',receipt=$4,expires_at=$5 WHERE tenant_id=$1 AND submission_id=$2 AND revision=$3",&[&tenant,&submission,&revision,&encode(&receipt)?,&expires]).await?;
    tx.commit().await?;
    Ok(receipt)
}
pub(super) async fn pending_token_bundle_deletions(
    db: &PgBackend,
    tenant: &str,
    submission: Option<Uuid>,
) -> Result<Vec<StoredTokenBundle>> {
    let mut client = db.trace_pool().get().await?;
    let tx = PgBackend::begin_trace_tenant_transaction(&mut client, tenant).await?;
    tx.execute("UPDATE trace_token_bundles b SET state='revoked' WHERE tenant_id=$1 AND ($2::uuid IS NULL OR submission_id=$2) AND (expires_at<=NOW() OR EXISTS(SELECT 1 FROM trace_withdrawals w WHERE w.tenant_id=b.tenant_id AND w.submission_id=b.submission_id) OR EXISTS(SELECT 1 FROM trace_submissions p WHERE p.tenant_id=b.tenant_id AND p.submission_id=b.submission_id AND (p.status='revoked' OR p.withdrawn_at IS NOT NULL OR p.purged_at IS NOT NULL)))",&[&tenant,&submission]).await?;
    let keys=tx.query("SELECT submission_id,revision,owner_ref FROM trace_token_bundles WHERE tenant_id=$1 AND state='revoked' AND EXISTS(SELECT 1 FROM trace_token_attachments a WHERE a.tenant_id=trace_token_bundles.tenant_id AND a.submission_id=trace_token_bundles.submission_id AND a.revision=trace_token_bundles.revision AND a.deleted=FALSE) AND ($2::uuid IS NULL OR submission_id=$2) ORDER BY created_at LIMIT 128",&[&tenant,&submission]).await?;
    let mut result = Vec::new();
    for key in keys {
        if let Some(value) = row(
            &tx,
            tenant,
            key.get(0),
            key.get::<_, String>(1).as_str(),
            key.get::<_, String>(2).as_str(),
        )
        .await?
        {
            result.push(value);
        }
    }
    tx.commit().await?;
    Ok(result)
}
pub(super) async fn mark_token_object_deleted(
    db: &PgBackend,
    tenant: &str,
    submission: Uuid,
    revision: &str,
    artifact: &str,
) -> Result<()> {
    let mut client = db.trace_pool().get().await?;
    let tx = PgBackend::begin_trace_tenant_transaction(&mut client, tenant).await?;
    tx.execute("UPDATE trace_token_attachments a SET deleted=TRUE WHERE tenant_id=$1 AND submission_id=$2 AND revision=$3 AND artifact_id=$4 AND EXISTS(SELECT 1 FROM trace_token_bundles b WHERE b.tenant_id=a.tenant_id AND b.submission_id=a.submission_id AND b.revision=a.revision AND b.state='revoked')",&[&tenant,&submission,&revision,&artifact]).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn publish_token_object(
    db: &PgBackend,
    tenant: &str,
    submission: Uuid,
    revision: &str,
    owner: &str,
    artifact: &str,
    store: &dyn crate::trace_artifact_store::TraceArtifactStore,
) -> Result<()> {
    use crate::trace_artifact_store::{PreparedBundleArtifact, TraceArtifactScope};
    let mut client = db.trace_pool().get().await?;
    let tx = PgBackend::begin_trace_tenant_transaction(&mut client, tenant).await?;
    lock(&tx, tenant, submission).await?;
    tx.query_opt("SELECT revision FROM trace_token_bundles WHERE tenant_id=$1 AND submission_id=$2 AND revision=$3 FOR UPDATE",&[&tenant,&submission,&revision]).await?.ok_or_else(invalid)?;
    let bundle = row(&tx, tenant, submission, revision, owner)
        .await?
        .ok_or_else(invalid)?;
    if bundle.state != "staging" || bundle.expires_at <= Utc::now() {
        return Err(invalid());
    }
    let object = bundle
        .attachments
        .iter()
        .find(|o| o.artifact_id == artifact && !o.deleted)
        .ok_or_else(invalid)?;
    if object.ready {
        return Ok(());
    }
    let prepared: PreparedBundleArtifact =
        serde_json::from_slice(object.prepared.as_deref().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
    if prepared.object_ref != object.object_ref {
        return Err(invalid());
    }
    let scope = TraceArtifactScope {
        tenant_storage_ref: object.object_ref.tenant_storage_ref.clone(),
        submission_storage_ref: object.object_ref.submission_storage_ref.clone(),
    };
    // Keep the DB row lock through synchronous publication and readback. No
    // detached upload task may survive cancellation and race deletion.
    store
        .publish_bundle_bytes(&scope, &prepared)
        .map_err(|_| invalid())?;
    let bytes = store
        .read_bundle_bytes(&scope, &object.object_ref)
        .map_err(|_| invalid())?;
    if artifact == "envelope" {
        if !bundle.manifest.envelope_digest.matches(&bytes) {
            return Err(invalid());
        }
    } else {
        bundle
            .manifest
            .verify_attachment(artifact, &bytes)
            .map_err(|_| invalid())?;
    }
    tx.execute("UPDATE trace_token_attachments SET ready=TRUE,prepared=NULL WHERE tenant_id=$1 AND submission_id=$2 AND revision=$3 AND artifact_id=$4",&[&tenant,&submission,&revision,&artifact]).await?;
    tx.commit().await?;
    Ok(())
}
pub(super) async fn delete_token_objects(
    db: &PgBackend,
    tenant: &str,
    submission: Uuid,
    revision: &str,
    store: &dyn crate::trace_artifact_store::TraceArtifactStore,
) -> Result<()> {
    use crate::trace_artifact_store::TraceArtifactScope;
    let mut client = db.trace_pool().get().await?;
    let tx = PgBackend::begin_trace_tenant_transaction(&mut client, tenant).await?;
    lock(&tx, tenant, submission).await?;
    let Some(record)=tx.query_opt("SELECT owner_ref,state FROM trace_token_bundles WHERE tenant_id=$1 AND submission_id=$2 AND revision=$3 FOR UPDATE",&[&tenant,&submission,&revision]).await? else {return Ok(())};
    if record.get::<_, String>(1) != "revoked" {
        return Err(invalid());
    }
    let bundle = row(
        &tx,
        tenant,
        submission,
        revision,
        record.get::<_, String>(0).as_str(),
    )
    .await?
    .ok_or_else(invalid)?;
    for object in bundle.attachments {
        if object.deleted {
            continue;
        }
        let scope = TraceArtifactScope {
            tenant_storage_ref: object.object_ref.tenant_storage_ref.clone(),
            submission_storage_ref: object.object_ref.submission_storage_ref.clone(),
        };
        store
            .delete_bundle_bytes(&scope, &object.object_ref)
            .map_err(|_| invalid())?;
        tx.execute("UPDATE trace_token_attachments SET deleted=TRUE,prepared=NULL WHERE tenant_id=$1 AND submission_id=$2 AND revision=$3 AND artifact_id=$4",&[&tenant,&submission,&revision,&object.artifact_id]).await?;
    }
    tx.commit().await?;
    Ok(())
}

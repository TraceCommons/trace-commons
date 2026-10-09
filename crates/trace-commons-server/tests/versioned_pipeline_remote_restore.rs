// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The artifact half of `pipeline_remote_restore` (spec B-D2), over the
//! in-memory `GcsObjectClient`: the provider-level ciphertext copy from a
//! source store into a scratch store, the stored-object fingerprint taken on
//! each side, the key-wrapper unwrap count, and the bucket versioning both
//! sides report. No bucket, no network.

use std::sync::Arc;

use secrecy::SecretString;
use serde_json::json;
use trace_commons_server::secrets::SecretsCrypto;
use trace_commons_server::trace_artifact_gcs::{
    GcsObjectClient, GcsRemoteTraceArtifactProvider, InMemoryGcsObjectClient,
    PrefixedGcsObjectClient,
};
use trace_commons_server::trace_artifact_kek::LocalMasterKeyWrapper;
use trace_commons_server::trace_artifact_store::{
    RemoteTraceArtifactProvider, ServiceOwnedTraceArtifactStore, TraceArtifactKind,
    TraceArtifactProviderConfig, TraceArtifactScope,
};
use trace_commons_server::versioned_pipeline_remote_restore::{
    restore_remote_artifacts, stored_artifact_fingerprint,
};

const OBJECT_STORE: &str = "trace-commons-remote-restore-test";

type Client = PrefixedGcsObjectClient<Arc<InMemoryGcsObjectClient>>;

fn crypto(key: &str) -> SecretsCrypto {
    SecretsCrypto::new(SecretString::from(key.to_string())).expect("test crypto")
}

fn kek(key: &str) -> LocalMasterKeyWrapper {
    LocalMasterKeyWrapper::new(crypto(key), "remote-restore-test")
}

fn provider(
    bucket: &Arc<InMemoryGcsObjectClient>,
    prefix: &str,
) -> GcsRemoteTraceArtifactProvider<Client> {
    GcsRemoteTraceArtifactProvider::new(
        PrefixedGcsObjectClient::new(Arc::clone(bucket), prefix).expect("a valid prefix"),
        "bucket-label",
        OBJECT_STORE,
    )
}

/// Writes `count` objects for two tenants through the store, as the seed
/// does, and returns their refs.
fn seed(
    bucket: &Arc<InMemoryGcsObjectClient>,
    prefix: &str,
    key: &str,
    count: usize,
) -> Vec<trace_commons_server::trace_artifact_store::TraceArtifactObjectRef> {
    let store = ServiceOwnedTraceArtifactStore::new(
        TraceArtifactProviderConfig::service_owned_remote(OBJECT_STORE).unwrap(),
        crypto(key),
        kek(key),
        provider(bucket, prefix),
    );
    (0..count)
        .map(|index| {
            let tenant = if index % 2 == 0 {
                "tenant:sha256:a"
            } else {
                "tenant:sha256:b"
            };
            store
                .put_scoped_json(
                    &TraceArtifactScope::new(tenant, format!("submission-{index}")),
                    TraceArtifactKind::ContributionEnvelope,
                    &format!("object-{index}"),
                    &json!({ "index": index }),
                )
                .expect("the seed writes")
                .object_ref
        })
        .collect()
}

#[test]
fn the_in_memory_client_lists_live_keys_under_a_prefix_and_reports_versioning() {
    let client = InMemoryGcsObjectClient::default();
    for key in ["a/1", "a/2", "ab/3", "b/4"] {
        client
            .put_object(key, bytes::Bytes::from_static(b"{}"), Default::default())
            .unwrap();
    }
    assert!(client.delete_object("a/2").unwrap());
    assert_eq!(
        client.list_object_keys("a/").unwrap(),
        vec!["a/1".to_string()]
    );
    assert_eq!(client.list_object_keys("").unwrap().len(), 3);
    // Off until the test says otherwise: an unknown policy is not versioning.
    assert!(!client.bucket_versioning_enabled().unwrap());
    client.set_versioning_enabled(true);
    assert!(client.bucket_versioning_enabled().unwrap());
}

#[test]
fn a_prefixed_client_keeps_its_objects_under_its_prefix() {
    let bucket = Arc::new(InMemoryGcsObjectClient::default());
    let scratch = PrefixedGcsObjectClient::new(Arc::clone(&bucket), "scratch/run-1").unwrap();
    scratch
        .put_object(
            "alias/x",
            bytes::Bytes::from_static(b"{}"),
            Default::default(),
        )
        .unwrap();
    assert_eq!(
        bucket.list_object_keys("").unwrap(),
        vec!["scratch/run-1/alias/x".to_string()]
    );
    assert_eq!(
        scratch.list_object_keys("alias/").unwrap(),
        vec!["alias/x".to_string()]
    );
    assert_eq!(&scratch.get_object("alias/x").unwrap().body[..], b"{}");
    // A sibling prefix sharing the string start is not inside it.
    let sibling = PrefixedGcsObjectClient::new(Arc::clone(&bucket), "scratch/run").unwrap();
    assert!(sibling.list_object_keys("").unwrap().is_empty());
    for prefix in ["", "/a", "a/", "a//b", "a/../b", "a/./b"] {
        assert!(
            PrefixedGcsObjectClient::new(Arc::clone(&bucket), prefix).is_err(),
            "{prefix:?} is not a prefix"
        );
    }
}

#[test]
fn a_provider_level_copy_keeps_every_fingerprint_and_unwraps_every_object() {
    let key = trace_commons_server::secrets::keychain::generate_master_key_hex();
    let live = Arc::new(InMemoryGcsObjectClient::default());
    let scratch = Arc::new(InMemoryGcsObjectClient::default());
    live.set_versioning_enabled(true);
    scratch.set_versioning_enabled(true);
    seed(&live, "drill", &key, 5);
    let source = provider(&live, "drill");
    let before = stored_artifact_fingerprint(&source).unwrap();
    assert_eq!(before.object_count, 5);

    let copy = restore_remote_artifacts(
        &source,
        provider(&scratch, "restore/drill"),
        OBJECT_STORE,
        crypto(&key),
        kek(&key),
    )
    .expect("the copy runs");
    assert_eq!(copy.object_count, 5);
    assert_eq!(copy.artifact_fingerprint, before.fingerprint);
    assert_eq!(copy.restored_artifact_fingerprint, before.fingerprint);
    assert_eq!(copy.kek_unwrap_verified_count, 5);
    assert!(copy.versioning_enabled);
    // The source is read, never written.
    assert_eq!(stored_artifact_fingerprint(&source).unwrap(), before);
    // The fingerprint names the objects by their key under the store, so
    // the same objects under another prefix fingerprint the same.
    assert_eq!(
        stored_artifact_fingerprint(&provider(&scratch, "restore/drill"))
            .unwrap()
            .fingerprint,
        before.fingerprint
    );
}

/// PR #1293, gap 3: a copy that decrypts and puts again encrypts under a
/// fresh DEK, so the stored ciphertext, and with it the fingerprint, moves.
/// That is why the copy works on ciphertext at the provider.
#[test]
fn a_decrypt_and_put_copy_would_not_keep_the_fingerprint() {
    let key = trace_commons_server::secrets::keychain::generate_master_key_hex();
    let live = Arc::new(InMemoryGcsObjectClient::default());
    let scratch = Arc::new(InMemoryGcsObjectClient::default());
    let refs = seed(&live, "drill", &key, 2);
    let reader = ServiceOwnedTraceArtifactStore::new(
        TraceArtifactProviderConfig::service_owned_remote(OBJECT_STORE).unwrap(),
        crypto(&key),
        kek(&key),
        provider(&live, "drill"),
    );
    let writer = ServiceOwnedTraceArtifactStore::new(
        TraceArtifactProviderConfig::service_owned_remote(OBJECT_STORE).unwrap(),
        crypto(&key),
        kek(&key),
        provider(&scratch, "drill"),
    );
    for object_ref in &refs {
        let scope = TraceArtifactScope::new(
            object_ref.tenant_storage_ref.clone(),
            object_ref.submission_storage_ref.clone(),
        );
        let value: serde_json::Value = reader.read_scoped_json(&scope, object_ref).unwrap();
        writer
            .put_scoped_json(&scope, object_ref.artifact_kind.clone(), "re-put", &value)
            .unwrap();
    }
    assert_ne!(
        stored_artifact_fingerprint(&provider(&live, "drill")).unwrap(),
        stored_artifact_fingerprint(&provider(&scratch, "drill")).unwrap(),
    );
}

#[test]
fn the_copy_refuses_a_scratch_store_that_already_holds_objects() {
    let key = trace_commons_server::secrets::keychain::generate_master_key_hex();
    let live = Arc::new(InMemoryGcsObjectClient::default());
    let scratch = Arc::new(InMemoryGcsObjectClient::default());
    seed(&live, "drill", &key, 1);
    seed(&scratch, "drill", &key, 1);
    let error = restore_remote_artifacts(
        &provider(&live, "drill"),
        provider(&scratch, "drill"),
        OBJECT_STORE,
        crypto(&key),
        kek(&key),
    )
    .expect_err("an occupied scratch store is refused");
    assert_eq!(error.to_string(), "remote_restore_scratch_not_empty");
}

#[test]
fn an_object_that_does_not_unwrap_under_the_configured_key_is_not_counted() {
    let key = trace_commons_server::secrets::keychain::generate_master_key_hex();
    let other = trace_commons_server::secrets::keychain::generate_master_key_hex();
    let live = Arc::new(InMemoryGcsObjectClient::default());
    let scratch = Arc::new(InMemoryGcsObjectClient::default());
    seed(&live, "drill", &key, 3);
    let copy = restore_remote_artifacts(
        &provider(&live, "drill"),
        provider(&scratch, "drill"),
        OBJECT_STORE,
        crypto(&other),
        kek(&other),
    )
    .expect("the copy itself needs no key");
    assert_eq!(copy.object_count, 3);
    assert_eq!(
        copy.restored_artifact_fingerprint,
        copy.artifact_fingerprint
    );
    assert_eq!(copy.kek_unwrap_verified_count, 0);
}

#[test]
fn versioning_is_reported_only_when_both_buckets_have_it() {
    let key = trace_commons_server::secrets::keychain::generate_master_key_hex();
    for (live_on, scratch_on) in [(true, true), (true, false), (false, true), (false, false)] {
        let live = Arc::new(InMemoryGcsObjectClient::default());
        let scratch = Arc::new(InMemoryGcsObjectClient::default());
        live.set_versioning_enabled(live_on);
        scratch.set_versioning_enabled(scratch_on);
        seed(&live, "drill", &key, 1);
        let copy = restore_remote_artifacts(
            &provider(&live, "drill"),
            provider(&scratch, "drill"),
            OBJECT_STORE,
            crypto(&key),
            kek(&key),
        )
        .unwrap();
        assert_eq!(
            copy.versioning_enabled,
            live_on && scratch_on,
            "{live_on} {scratch_on}"
        );
    }
}

#[test]
fn an_empty_source_copies_nothing() {
    let key = trace_commons_server::secrets::keychain::generate_master_key_hex();
    let live = Arc::new(InMemoryGcsObjectClient::default());
    let scratch = Arc::new(InMemoryGcsObjectClient::default());
    let copy = restore_remote_artifacts(
        &provider(&live, "drill"),
        provider(&scratch, "drill"),
        OBJECT_STORE,
        crypto(&key),
        kek(&key),
    )
    .unwrap();
    assert_eq!((copy.object_count, copy.kek_unwrap_verified_count), (0, 0));
}

#[test]
fn a_copied_object_reads_back_at_the_provider_with_the_same_ref() {
    let key = trace_commons_server::secrets::keychain::generate_master_key_hex();
    let live = Arc::new(InMemoryGcsObjectClient::default());
    let scratch = Arc::new(InMemoryGcsObjectClient::default());
    let refs = seed(&live, "drill", &key, 2);
    restore_remote_artifacts(
        &provider(&live, "drill"),
        provider(&scratch, "drill"),
        OBJECT_STORE,
        crypto(&key),
        kek(&key),
    )
    .unwrap();
    for object_ref in &refs {
        let restored = provider(&scratch, "drill")
            .read_encrypted_artifact(object_ref)
            .unwrap();
        assert_eq!(restored.object_ref, *object_ref);
        assert!(restored.invalidated_at.is_none());
    }
}

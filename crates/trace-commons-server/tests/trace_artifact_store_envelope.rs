// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Integration tests for the v2 KEK envelope path in
//! `ServiceOwnedTraceArtifactStore`.
//!
//! Covers the round-trip through `KmsKeyWrapper::wrap_dek` /
//! `unwrap_dek` and the downgrade-defense gating that prevents v1/v2
//! cross-shape envelopes from being read.

use std::path::PathBuf;

use secrecy::SecretString;
use serde_json::{Value, json};
use trace_commons_server::secrets::SecretsCrypto;
use trace_commons_server::trace_artifact_kek::LocalMasterKeyWrapper;
use trace_commons_server::trace_artifact_store::{
    FileRemoteTraceArtifactProvider, ServiceOwnedTraceArtifactStore,
    TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_V2, TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_VERSION,
    TraceArtifactKind, TraceArtifactObjectRef, TraceArtifactProviderConfig, TraceArtifactScope,
};

fn build_store(
    temp_root: PathBuf,
) -> ServiceOwnedTraceArtifactStore<FileRemoteTraceArtifactProvider, LocalMasterKeyWrapper> {
    let key = trace_commons_server::secrets::keychain::generate_master_key_hex();
    let crypto =
        SecretsCrypto::new(SecretString::from(key.clone())).expect("crypto for envelope test");
    let kek_crypto = SecretsCrypto::new(SecretString::from(key)).expect("kek crypto");
    let kek = LocalMasterKeyWrapper::new(kek_crypto, "trace-commons-envelope-test");
    let config = TraceArtifactProviderConfig::service_owned_remote("trace-commons-prod")
        .expect("remote config");
    let provider = FileRemoteTraceArtifactProvider::new(temp_root);
    ServiceOwnedTraceArtifactStore::new(config, crypto, kek, provider)
}

fn object_file_path(temp_root: &std::path::Path, object_ref: &TraceArtifactObjectRef) -> PathBuf {
    temp_root
        .join(&object_ref.object_store)
        .join(&object_ref.object_key)
}

#[test]
fn put_writes_v2_envelope_with_wrapped_dek_and_round_trips_plaintext() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = build_store(temp.path().to_path_buf());
    let scope = TraceArtifactScope::new("tenant:sha256:alpha", "submission-alpha");
    let payload = json!({"safe": true, "summary": "<redacted>"});

    let receipt = store
        .put_scoped_json(
            &scope,
            TraceArtifactKind::ContributionEnvelope,
            "envelope-id",
            &payload,
        )
        .expect("v2 artifact writes");
    let artifact = store
        .read_scoped_artifact(&scope, &receipt.object_ref)
        .expect("v2 artifact envelope reads");
    assert_eq!(artifact.schema_version, TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_V2);
    assert!(
        artifact.wrapped_dek.is_some(),
        "v2 record must carry a wrapped DEK"
    );
    assert!(
        artifact.salt_base64.is_empty(),
        "v2 record must not reuse the legacy v1 salt slot"
    );

    let round_trip: serde_json::Value = store
        .read_scoped_json(&scope, &receipt.object_ref)
        .expect("v2 artifact round-trips through KEK unwrap");
    assert_eq!(round_trip, payload);
}

#[test]
fn read_refuses_v2_envelope_with_wrapped_dek_stripped() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = build_store(temp.path().to_path_buf());
    let scope = TraceArtifactScope::new("tenant:sha256:alpha", "submission-alpha");
    let payload = json!({"safe": true});
    let receipt = store
        .put_scoped_json(
            &scope,
            TraceArtifactKind::ContributionEnvelope,
            "envelope-id",
            &payload,
        )
        .expect("v2 artifact writes");

    // Read the on-disk JSON, strip the wrapped_dek, write it back.
    let path = object_file_path(temp.path(), &receipt.object_ref);
    let body = std::fs::read_to_string(&path).expect("read on-disk record");
    let mut record: Value = serde_json::from_str(&body).expect("record parses");
    let artifact = record
        .get_mut("artifact")
        .expect("record has artifact field");
    let obj = artifact.as_object_mut().expect("artifact is object");
    obj.remove("wrapped_dek");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&record).expect("reserialize"),
    )
    .expect("write tampered record");

    let err = store
        .read_scoped_artifact(&scope, &receipt.object_ref)
        .expect_err("v2 record without wrapped_dek must be refused");
    assert!(
        err.to_string().contains("KekDowngradeRejected"),
        "expected KekDowngradeRejected, got: {err}"
    );
}

#[test]
fn read_refuses_v1_envelope_with_injected_wrapped_dek() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = build_store(temp.path().to_path_buf());
    let scope = TraceArtifactScope::new("tenant:sha256:alpha", "submission-alpha");
    let payload = json!({"safe": true});
    let receipt = store
        .put_scoped_json(
            &scope,
            TraceArtifactKind::ContributionEnvelope,
            "envelope-id",
            &payload,
        )
        .expect("v2 artifact writes");

    // Read on-disk, downgrade schema_version to v1 while leaving wrapped_dek in place.
    let path = object_file_path(temp.path(), &receipt.object_ref);
    let body = std::fs::read_to_string(&path).expect("read on-disk record");
    let mut record: Value = serde_json::from_str(&body).expect("record parses");
    record["artifact"]["schema_version"] =
        Value::String(TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_VERSION.to_string());
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&record).expect("reserialize"),
    )
    .expect("write tampered record");

    let err = store
        .read_scoped_artifact(&scope, &receipt.object_ref)
        .expect_err("v1 record with wrapped_dek must be refused");
    assert!(
        err.to_string().contains("KekDowngradeRejected"),
        "expected KekDowngradeRejected, got: {err}"
    );
}

#[test]
fn read_refuses_envelope_with_unknown_schema_version() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = build_store(temp.path().to_path_buf());
    let scope = TraceArtifactScope::new("tenant:sha256:alpha", "submission-alpha");
    let payload = json!({"safe": true});
    let receipt = store
        .put_scoped_json(
            &scope,
            TraceArtifactKind::ContributionEnvelope,
            "envelope-id",
            &payload,
        )
        .expect("v2 artifact writes");

    // Read on-disk, replace schema_version with an unrecognised future version.
    let path = object_file_path(temp.path(), &receipt.object_ref);
    let body = std::fs::read_to_string(&path).expect("read on-disk record");
    let mut record: Value = serde_json::from_str(&body).expect("record parses");
    record["artifact"]["schema_version"] =
        Value::String("ironclaw.trace_artifact_ciphertext.v3".to_string());
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&record).expect("reserialize"),
    )
    .expect("write tampered record");

    let err = store
        .read_scoped_artifact(&scope, &receipt.object_ref)
        .expect_err("unknown schema_version must be refused");
    assert!(
        err.to_string().contains("KekDowngradeRejected"),
        "expected KekDowngradeRejected: unknown schema_version, got: {err}"
    );
}

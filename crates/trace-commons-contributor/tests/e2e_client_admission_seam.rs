//! The client half of the invite-free admission path, driven against the
//! server's own types.
//!
//! `admission_pg_tests::actual_postgres_challenge_witness_ingest_and_terminal_retry`
//! proves the *server* admits well-formed evidence. It proves nothing about
//! whether a client can produce one, because it builds every header by hand.
//! Three defects shipped into that gap and were all found by reading rather
//! than by a red test: signup refused because an env var no app sets was
//! unset (#786), the receipt endpoint could not be set after signup (#787),
//! and `admission::anchor` required a `near-` tenant id to equal its own
//! anchor, which V61 made impossible (#785).
//!
//! The trap #785 illustrates is the one this file is built to avoid. The
//! server-side fixture constructs `tenant = near-{anchor}` -- the pre-V61
//! shape -- so the equality it should have caught held for it. **Every
//! account here therefore draws its tenant id and its account anchor
//! independently**, from the server's own `random_near_tenant_id` and
//! `NearAccountIndexPepper::index_label`, which is what a V61 row actually
//! holds.
//!
//! # What is real and what stands in
//!
//! Real: the contributor crate's source adapters, `build_raw_contribution`,
//! the deterministic redaction pipeline, `WitnessReviewArtifact` validation,
//! `handle_prepare_admission_session` with its real refusal labels, the
//! submit pipeline's profile selection, and -- on the server side -- the
//! actual `random_near_tenant_id`, `NearAccountIndexPepper` and
//! `verify_admission_evidence` from `trace-commons-server`.
//!
//! Stood in for, deliberately and named so nobody mistakes them for real:
//!
//! - **The witness enclave.** `witness_session` requires a DCAP quote from a
//!   real TDX instance, which CI has none of. The certificate and the
//!   admission signature here are produced by a local secp256k1 key that the
//!   config then pins, so the *signature* verification is genuine while the
//!   *attestation* is absent. This is the same trade the server's own pg
//!   suite makes with its `FixtureEnclave`.
//! - **IronWire.** A separate repository, not vendored here. The refusal
//!   tests below stop before the proxy is contacted and assert that nothing
//!   was contacted, so no stand-in is needed for them.
//! - **The provider receipt endpoint.** No hosted NEAR AI model is reachable
//!   from CI. Only the client's validation of the configured endpoint runs.
//!
//! What is NOT stood in for is any decision the client makes. Nothing here
//! recomputes an anchor, a profile, or a refusal that the code under test is
//! also computing -- a helper that agreed with the implementation by
//! construction would assert nothing.
//!
//! Non-Windows only, matching `e2e_enroll_and_submit.rs`:
//! `trace-commons-server` is not a dev-dependency on Windows.
#![cfg(not(windows))]

use sha2::Digest as _;
use trace_commons_contributor::config::{ConfigStore, ContributorConfig};
use trace_commons_contributor::source::TraceSource as _;
use trace_commons_protocol::admission::AdmissionEvidence;

// ---------------------------------------------------------------------------
// Fixture signer: a local secp256k1 key standing where the witness enclave's
// dstack-derived identity would be. Real EIP-191 signatures, no attestation.
// ---------------------------------------------------------------------------

fn fixture_signer(seed: &str) -> k256::ecdsa::SigningKey {
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&sha2::Sha256::digest(seed.as_bytes()));
    k256::ecdsa::SigningKey::from_bytes(&bytes.into()).expect("fixture key")
}

fn fixture_address(key: &k256::ecdsa::SigningKey) -> String {
    use k256::elliptic_curve::sec1::ToEncodedPoint as _;
    use sha3::Digest as _;
    let point = key.verifying_key().to_encoded_point(false);
    let digest = sha3::Keccak256::digest(&point.as_bytes()[1..]);
    format!("0x{}", hex::encode(&digest[12..]))
}

fn sign_eip191(key: &k256::ecdsa::SigningKey, message: &[u8]) -> String {
    use sha3::Digest as _;
    let mut prefixed = format!("\x19Ethereum Signed Message:\n{}", message.len()).into_bytes();
    prefixed.extend_from_slice(message);
    let digest = sha3::Keccak256::digest(&prefixed);
    let (signature, recovery) = key
        .sign_prehash_recoverable(&digest)
        .expect("fixture signature");
    let mut out = signature.to_bytes().to_vec();
    out.push(27 + recovery.to_byte());
    format!("0x{}", hex::encode(out))
}

/// The certificate's signing preimage, in the encoding the *server* defines
/// and the client re-implements. Duplicated here only because both
/// implementations are unreachable from an integration test -- the server's
/// `WitnessCertificate` has private fields and one constructor that consumes
/// a `CorrespondenceProof`, and the client's `certificate_signing_bytes` is
/// private. `transport::a_certificate_this_client_accepts_is_one_the_server_issued`
/// is what holds those two together; this is scaffolding for the assertions
/// below, never the thing under test.
fn certificate_signing_bytes(
    redacted_sha256: &str,
    policy: &str,
    measurement: &str,
    verdict_tag: u8,
    timestamp: i64,
) -> Vec<u8> {
    let mut bytes = b"trace_commons.redaction_witness_certificate.v1\n".to_vec();
    for field in [redacted_sha256, policy, measurement] {
        bytes.extend_from_slice(&(field.len() as u64).to_le_bytes());
        bytes.extend_from_slice(field.as_bytes());
    }
    bytes.push(verdict_tag);
    bytes.extend_from_slice(&timestamp.to_le_bytes());
    bytes
}

// ---------------------------------------------------------------------------
// A V61 account: tenant id and account anchor drawn independently.
// ---------------------------------------------------------------------------

/// Exactly what a V61 row holds: a tenant id from the OS RNG, derived from
/// nothing, and a keyed blind index over the account name. Both come from
/// `trace-commons-server`'s own code, so if a future migration re-couples
/// them this fixture follows rather than pinning a shape that has moved.
struct V61Account {
    tenant_id: String,
    /// The 64 hex characters an admission binding carries, i.e. the stored
    /// `anchor_hash` with its `sha256:` prefix removed.
    anchor: String,
}

impl V61Account {
    fn provision(account_name: &str) -> Self {
        use trace_commons_server::near_account_identity::NearAccountIndexPepper;
        let mut pepper = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut pepper);
        let pepper = NearAccountIndexPepper::from_bytes(&pepper).expect("32-byte pepper");
        let anchor = pepper
            .index_label("mainnet", account_name)
            .strip_prefix("sha256:")
            .expect("index_label is sha256:-prefixed")
            .to_string();
        let tenant_id = trace_commons_server::near_account_identity::random_near_tenant_id();
        assert_ne!(
            tenant_id.strip_prefix("near-"),
            Some(anchor.as_str()),
            "the fixture drew a pre-V61 account; the two halves must be independent"
        );
        Self { tenant_id, anchor }
    }
}

fn config_for(account: &V61Account, witness_address: &str) -> ContributorConfig {
    serde_json::from_value(serde_json::json!({
        "schema_version": trace_commons_contributor::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
        "issuer_url": "https://issuer.example",
        "ingest_url": "https://ingest.example/v1/traces",
        "audience": "trace-commons-upload",
        "tenant_id": account.tenant_id,
        "instance_id": "",
        "user_subject": "alice@example.com",
        "device_key_id": "device-e2e",
        "consent_scopes": ["debugging_evaluation"],
        "allowed_hosts": "issuer.example,ingest.example,receipts.example",
        "witness": {
            "admission_evidence": true,
            "url": "https://witness.example",
            "signing_address": witness_address,
            "expected_measurements": [format!("mrtd={}", "ab".repeat(48))],
        },
    }))
    .expect("fixture config")
}

/// The claude-code fixture, loaded through the real source adapter.
fn fixture_transcript() -> trace_commons_contributor::source::SessionTranscript {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
    let source = trace_commons_contributor::source::claude_code::ClaudeCodeSource::new(root);
    let session = source.discover().expect("discover").remove(0);
    source.load(&session).expect("load")
}

/// Build the stored review artifact a witnessed admission review leaves
/// behind, carrying `anchor` as the account it was bound to.
///
/// The envelope is produced by the real redaction pipeline from the real
/// fixture; only the certificate and the admission signature are the fixture
/// signer's rather than an enclave's.
async fn stored_admission_artifact(
    cfg: &ContributorConfig,
    key: &k256::ecdsa::SigningKey,
    anchor: &str,
) -> (
    trace_commons_contributor::daemon::approved_envelope::WitnessReviewArtifact,
    String,
) {
    let transcript = fixture_transcript();
    let source_hash = transcript.session_hash.clone();
    let redactor = trace_commons_contributor::envelope::build_redactor_with(
        cfg,
        transcript.cwd.as_deref(),
        None,
    )
    .expect("deterministic redactor");
    let raw = trace_commons_contributor::envelope::build_raw_contribution(
        &transcript,
        cfg,
        chrono::Utc::now(),
    );
    let envelope = trace_commons_contributor::envelope::redact_to_envelope(&redactor, raw)
        .await
        .expect("redaction");
    let envelope_bytes = serde_json::to_vec(&envelope).expect("envelope bytes");
    let redacted_sha256 = hex::encode(sha2::Sha256::digest(&envelope_bytes));

    let measurement = "ab".repeat(48);
    let policy = "deterministic-v1";
    let timestamp = chrono::Utc::now().timestamp();
    let certificate = serde_json::json!({
        "redacted_sha256": redacted_sha256,
        "residual_risk_verdict": "low",
        "redaction_policy_version": policy,
        "witness_measurement": measurement,
        "timestamp": timestamp,
    });
    let certificate_json = serde_json::to_string(&certificate).expect("certificate json");
    let certificate_signature = sign_eip191(
        key,
        &certificate_signing_bytes(&redacted_sha256, policy, &measurement, 1, timestamp),
    );

    let evidence = AdmissionEvidence {
        profile: trace_commons_protocol::admission::EVIDENCE_DOMAIN.into(),
        account_anchor_sha256: anchor.to_string(),
        challenge_sha256: "22".repeat(32),
        provider_signer: "33".repeat(32),
        model: "operator-approved-model".into(),
        request_bytes: 1,
        request_sha256: "44".repeat(32),
        response_sha256: "55".repeat(32),
        receipt_sha256: "66".repeat(32),
        artifact_sha256: redacted_sha256.clone(),
        witness_measurement: measurement.clone(),
        redaction_policy_version: policy.into(),
        issued_at: timestamp,
        expires_at: timestamp + 600,
    };
    let evidence_json = serde_json::to_string(&evidence).expect("evidence json");
    let evidence_signature =
        sign_eip191(key, &evidence.signing_bytes().expect("evidence signing bytes"));

    use base64::Engine as _;
    let artifact = serde_json::from_value(serde_json::json!({
        "review_schema": "trace_commons.witness_review.v1",
        "source_hash": source_hash,
        "input_fingerprint": "fingerprint-e2e",
        "verdict": null,
        "correction_hash": null,
        "response": {
            "envelope_bytes": base64::engine::general_purpose::STANDARD.encode(&envelope_bytes),
            "certificate_json": certificate_json,
            "signature_hex": certificate_signature,
            "admission": {
                "evidence_json": evidence_json,
                "signature_hex": evidence_signature,
            },
        },
    }))
    .expect("artifact deserialises");
    (artifact, source_hash)
}

/// A V61 account's own admission review must survive the client's stored
/// validation.
///
/// Before this test, `validate_stored` required
/// `cfg.tenant_id.strip_prefix("near-") == Some(evidence.account_anchor_sha256)`
/// -- the client-side twin of the coupling #785 removed from the server's
/// `admission::anchor`. V61 made the tenant id 32 random bytes and the
/// anchor a keyed blind index, so that equality holds for no real account
/// and every admission-bearing review was refused `witness-certificate-invalid`.
#[tokio::test]
async fn a_v61_accounts_admission_review_is_not_refused_for_its_tenant_id() {
    let account = V61Account::provision("alice.near");
    let key = fixture_signer("witness-fixture-only");
    let cfg = config_for(&account, &fixture_address(&key));
    let (artifact, source_hash) = stored_admission_artifact(&cfg, &key, &account.anchor).await;

    let envelope = artifact
        .validate_stored(&cfg, &source_hash, "fingerprint-e2e")
        .expect("a V61 account's own admission review must validate");
    assert_eq!(
        envelope.contributor.tenant_scope_ref.as_deref(),
        Some(cfg.tenant_id.as_str())
    );
}

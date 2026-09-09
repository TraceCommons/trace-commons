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
use trace_commons_contributor::config::ContributorConfig;
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
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut pepper)
            .expect("os rng");
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
        receipt_sha256: trace_commons_protocol::admission::receipt_identity(
            &"33".repeat(32),
            &"44".repeat(32),
            &"55".repeat(32),
        )
        .expect("receipt identity"),
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

/// The control that makes the failure above legible, and the fixture trap
/// itself, written down.
///
/// `validate_stored` collapses two distinct refusals onto the same
/// `witness-certificate-invalid` string: a certificate that does not verify,
/// and an account anchor the client would not accept. Without this control a
/// red run above could be read as "the fixture's certificate is wrong".
///
/// So: the same artifact, the same signer, the same code path -- with only
/// the account changed to the **pre-V61** shape the server-side fixture also
/// builds, `tenant = near-{anchor}`. That passes. Which means the
/// certificate is sound and the account is the only thing the client
/// objected to, and it means a fixture written in the old shape can never
/// express the new world's failure.
#[tokio::test]
async fn the_pre_v61_account_shape_validates_which_is_why_this_was_missed() {
    let key = fixture_signer("witness-fixture-only");
    let anchor = "ab".repeat(32);
    let legacy = V61Account {
        tenant_id: format!("near-{anchor}"),
        anchor: anchor.clone(),
    };
    let cfg = config_for(&legacy, &fixture_address(&key));
    let (artifact, source_hash) = stored_admission_artifact(&cfg, &key, &anchor).await;

    artifact
        .validate_stored(&cfg, &source_hash, "fingerprint-e2e")
        .expect(
            "the pre-V61 shape must validate; if this fails the fixture's own \
             certificate is broken and the test above proves nothing",
        );
}

/// The server's real verifier over the headers the client actually carries.
///
/// This is the seam in the other direction. The evidence and certificate the
/// client stored -- read back off the artifact in its on-disk form, not
/// re-derived -- are handed to `trace-commons-server`'s own
/// `verify_witness_certificate` and then `verify_admission_evidence`, bound
/// to the V61 anchor. Nothing between the two halves is re-implemented.
///
/// The witness pin and the provider policy name the fixture signer rather
/// than a real enclave and a real NEAR AI model (see the module docs), so
/// what this asserts is the *binding*: signature, artifact, measurement,
/// policy, receipt identity and account anchor, all checked by the server's
/// code over bytes the client produced.
///
/// It also cross-checks this file's hand-written certificate preimage: the
/// server recovers the signer from its own `signing_bytes`, so a wrong
/// encoding here fails rather than passing quietly.
#[tokio::test]
async fn the_evidence_a_client_carries_is_admitted_by_the_servers_own_verifier() {
    use trace_commons_protocol::trace_contribution::ResidualPiiRisk;
    use trace_commons_server::admission_evidence::{
        AdmissionProviderTrust, verify_admission_evidence,
    };
    use trace_commons_server::redaction_witness::certificate::{
        CertificateDetails, WitnessCertificate,
    };
    use trace_commons_server::redaction_witness::verification::{
        WitnessPin, verify_witness_certificate,
    };

    let account = V61Account::provision("bob.near");
    let key = fixture_signer("witness-fixture-only");
    let address = fixture_address(&key);
    let cfg = config_for(&account, &address);
    let (artifact, _) = stored_admission_artifact(&cfg, &key, &account.anchor).await;

    // Read the stored artifact back in its on-disk form, which is what the
    // uploader attaches to `POST /v1/traces`.
    let stored = serde_json::to_value(&artifact).expect("artifact serialises");
    let response = &stored["response"];
    use base64::Engine as _;
    let envelope_bytes = base64::engine::general_purpose::STANDARD
        .decode(response["envelope_bytes"].as_str().expect("envelope bytes"))
        .expect("base64");
    let certificate: serde_json::Value =
        serde_json::from_str(response["certificate_json"].as_str().expect("certificate"))
            .expect("certificate json");
    let evidence: AdmissionEvidence = serde_json::from_str(
        response["admission"]["evidence_json"]
            .as_str()
            .expect("evidence"),
    )
    .expect("evidence parses server-side");

    assert_eq!(
        evidence.account_anchor_sha256, account.anchor,
        "the client must carry the account's anchor, not anything derived \
         from its tenant id"
    );

    let pin = WitnessPin::new(&address, [certificate["witness_measurement"]
        .as_str()
        .expect("measurement")
        .to_string()])
    .expect("witness pin");
    let verified = verify_witness_certificate(
        WitnessCertificate::from_wire(
            certificate["redacted_sha256"]
                .as_str()
                .expect("digest")
                .to_string(),
            CertificateDetails {
                residual_risk_verdict: ResidualPiiRisk::Low,
                redaction_policy_version: certificate["redaction_policy_version"]
                    .as_str()
                    .expect("policy")
                    .to_string(),
                witness_measurement: certificate["witness_measurement"]
                    .as_str()
                    .expect("measurement")
                    .to_string(),
                timestamp: certificate["timestamp"].as_i64().expect("timestamp"),
            },
        ),
        response["signature_hex"].as_str().expect("signature"),
        Some(&pin),
        &envelope_bytes,
    )
    .expect("the server must verify the certificate the client stored");

    let trust = AdmissionProviderTrust::new(
        [evidence.provider_signer.clone()],
        Vec::new(),
        ["operator-approved-model".to_string()],
        1,
    )
    .expect("fixture provider policy");
    let signature = response["admission"]["signature_hex"]
        .as_str()
        .expect("admission signature");
    let now = chrono::Utc::now().timestamp();

    verify_admission_evidence(
        &evidence,
        signature,
        &verified,
        &pin,
        &trust,
        &account.anchor,
        now,
    )
    .expect("the server must admit evidence bound to this account's anchor");

    // The negative control, and the reason the client cannot simply drop its
    // own anchor check: the same evidence is refused for a different
    // account, so the binding is doing work.
    let other = V61Account::provision("carol.near");
    assert_ne!(account.anchor, other.anchor);
    assert!(
        verify_admission_evidence(
            &evidence,
            signature,
            &verified,
            &pin,
            &trust,
            &other.anchor,
            now,
        )
        .is_err(),
        "evidence bound to one account must not admit another"
    );
}

// ---------------------------------------------------------------------------
// The preparation path's refusals, through the real IPC entry point.
// ---------------------------------------------------------------------------

fn daemon_with(cfg: &ContributorConfig, dir: &std::path::Path) -> trace_commons_contributor::daemon::ipc::DaemonShared {
    let store = trace_commons_contributor::config::ConfigStore::open(dir.join("state"))
        .expect("config store");
    store.save_config(cfg).expect("save config");
    trace_commons_contributor::daemon::ipc::DaemonShared::load(store).expect("daemon shared")
}

fn prepare_request(entry_id: uuid::Uuid) -> trace_commons_contributor::daemon::ipc::Request {
    trace_commons_contributor::daemon::ipc::Request {
        id: 1,
        method: "prepare_admission_session".into(),
        params: serde_json::json!({
            "entry_id": entry_id,
            "backend": "near",
            "confirmed": true,
        }),
    }
}

/// A client with no receipt endpoint refuses, by name, before it opens a
/// socket.
///
/// #787 was that the receipt endpoint could not be set after signup. This is
/// the consequence of that on the path that needs it: without one, admission
/// preparation is unreachable. The assertion that nothing was contacted is
/// made against a listener that would have accepted the connection, not
/// inferred from the check existing in the source.
#[tokio::test]
async fn a_client_with_no_receipt_endpoint_refuses_before_contacting_the_proxy() {
    let account = V61Account::provision("dave.near");
    let key = fixture_signer("witness-fixture-only");
    let cfg = config_for(&account, &fixture_address(&key));
    assert!(
        cfg.inference_receipt_endpoint.is_none(),
        "this fixture is about the absent endpoint"
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let shared = daemon_with(&cfg, dir.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    {
        let mut settings = shared.settings.lock().expect("settings lock");
        settings.ironwire_attested_bodies = true;
        settings.ironwire = Some(
            trace_commons_contributor::daemon::settings::IronWireDeclaration::Watch {
                port: listener.local_addr().expect("addr").port(),
                token_dir: Some(dir.path().into()),
            },
        );
    }

    let response = trace_commons_contributor::daemon::admission_setup::handle_prepare_admission_session(
        &shared,
        &prepare_request(uuid::Uuid::new_v4()),
    )
    .await;
    assert_eq!(
        response.error.expect("refused").message,
        "admission_receipt_endpoint_required"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), listener.accept())
            .await
            .is_err(),
        "the proxy was contacted before the endpoint was required"
    );
}

/// Preparation refuses on an unsupported harness -- and the caller cannot
/// tell that from any other refusal.
///
/// Read what this can and cannot show. `handle_prepare_admission_session`
/// maps every reason but the two receipt-endpoint ones onto the single label
/// `admission_setup_unavailable`, so from outside the daemon an unsupported
/// harness, a session that could not be found, an absent proxy and an
/// untrusted one are indistinguishable. This test therefore does NOT claim
/// to observe which branch fired -- it could not -- and the discrimination
/// itself is held by `admission_setup::extracts_source_metadata_not_queue_id_or_filename`,
/// which calls `exact_session_id` directly.
///
/// What it does show is the collapse, by driving two materially different
/// sessions -- one on a harness the path cannot address, one on a harness it
/// can -- through the same entry point and getting the same word back. That
/// is the thing a contributor actually experiences, and it is not visible
/// from any unit test of the branches.
#[tokio::test]
async fn every_preparation_refusal_reaches_the_caller_as_the_same_word() {
    let account = V61Account::provision("erin.near");
    let key = fixture_signer("witness-fixture-only");
    let mut cfg = config_for(&account, &fixture_address(&key));
    cfg.inference_receipt_endpoint = Some("https://receipts.example/v1".into());

    let dir = tempfile::tempdir().expect("tempdir");
    let shared = daemon_with(&cfg, dir.path());

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/gemini-cli");
    let source = trace_commons_contributor::source::gemini_cli::GeminiCliSource::new(root.clone());
    let session = source.discover().expect("discover").remove(0);

    let entry_id = uuid::Uuid::new_v4();
    {
        let mut settings = shared.settings.lock().expect("settings lock");
        settings.ironwire_attested_bodies = true;
        settings.gemini_source = Some(
            trace_commons_contributor::daemon::settings::SourceDeclaration::Watch { path: root },
        );
    }
    shared
        .queue
        .lock()
        .expect("queue lock")
        .upsert(
            trace_commons_contributor::daemon::queue::QueueEntry {
                entry_id,
                source: "gemini-cli".into(),
                path: session.path.clone(),
                ..Default::default()
            },
            16,
        )
        .expect("queue upsert");

    let response = trace_commons_contributor::daemon::admission_setup::handle_prepare_admission_session(
        &shared,
        &prepare_request(entry_id),
    )
    .await;
    assert_eq!(
        response.error.expect("refused").message,
        "admission_setup_unavailable"
    );

    // The other half of the collapse: a harness the path CAN address, whose
    // preparation fails later and for an unrelated reason (no proxy is
    // declared), reports the identical word.
    let claude_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
    let claude = trace_commons_contributor::source::claude_code::ClaudeCodeSource::new(
        claude_root.clone(),
    );
    let claude_session = claude.discover().expect("discover").remove(0);
    let claude_entry = uuid::Uuid::new_v4();
    shared.settings.lock().expect("settings lock").claude_source = Some(
        trace_commons_contributor::daemon::settings::SourceDeclaration::Watch { path: claude_root },
    );
    shared
        .queue
        .lock()
        .expect("queue lock")
        .upsert(
            trace_commons_contributor::daemon::queue::QueueEntry {
                entry_id: claude_entry,
                source: "claude-code".into(),
                path: claude_session.path.clone(),
                ..Default::default()
            },
            16,
        )
        .expect("queue upsert");
    let supported =
        trace_commons_contributor::daemon::admission_setup::handle_prepare_admission_session(
            &shared,
            &prepare_request(claude_entry),
        )
        .await;
    assert_eq!(
        supported.error.expect("refused").message,
        "admission_setup_unavailable",
        "two unrelated failures must be reporting the same word, or this \
         test is no longer about the collapse"
    );
}

/// A contributor without the evidence flag takes the ordinary profile, and
/// the wire shows it.
///
/// The server decides between `EvidencePlan::Verify` and
/// `RefuseNewSubmission` from the presence of these two headers alone. This
/// drives the real submit pipeline for a contributor with no witness
/// configured and asserts that neither header is on the request -- i.e. that
/// such a client lands on the branch that refuses a new submission, rather
/// than one that offers half-formed evidence.
///
/// The issuer and the ingest are stubs here: the issuer returns a claim, the
/// ingest records what arrived. Neither reproduces a decision the client
/// makes. The claim-minting path itself is covered end to end against the
/// real issuer router by `e2e_enroll_and_submit.rs`.
#[tokio::test]
async fn a_contributor_without_the_evidence_flag_sends_no_admission_headers() {
    use axum::{Json, Router, routing::post};

    let seen: std::sync::Arc<std::sync::Mutex<Vec<Vec<(String, String)>>>> = Default::default();
    let sink = seen.clone();
    let router = Router::new()
        .route(
            "/v1/traces",
            post(move |headers: axum::http::HeaderMap, _: String| {
                let sink = sink.clone();
                async move {
                    sink.lock().expect("sink").push(
                        headers
                            .iter()
                            .map(|(name, value)| {
                                (
                                    name.as_str().to_ascii_lowercase(),
                                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                                )
                            })
                            .collect(),
                    );
                    Json(serde_json::json!({
                        "status": "accepted",
                        "credit_points_pending": 0.0,
                        "explanation": [],
                    }))
                }
            }),
        )
        .route(
            "/v1/trace-upload-claim",
            post(|| async move {
                Json(serde_json::json!({
                    "access_token": "stub-claim-token-for-this-test-only",
                    "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                    "consent_scopes": ["debugging_evaluation"],
                    "allowed_uses": ["debugging"],
                }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    tokio::spawn(async move { axum::serve(listener, router).await.expect("serve") });

    let account = V61Account::provision("frank.near");
    let cfg: ContributorConfig = serde_json::from_value(serde_json::json!({
        "schema_version": trace_commons_contributor::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
        "issuer_url": base,
        "ingest_url": format!("{base}/v1/traces"),
        "audience": "trace-commons-upload",
        "tenant_id": account.tenant_id,
        "instance_id": "",
        "user_subject": "frank@example.com",
        "device_key_id": "device-e2e",
        "consent_scopes": ["debugging_evaluation"],
        "allowed_hosts": "127.0.0.1",
        // No `witness` key at all: this contributor was never granted the
        // evidence profile, which is the case under test.
    }))
    .expect("fixture config");
    assert!(cfg.witness.is_none());

    let dir = tempfile::tempdir().expect("tempdir");
    let store = trace_commons_contributor::config::ConfigStore::open(dir.path().join("state"))
        .expect("config store");
    store.save_config(&cfg).expect("save config");

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
    let source = trace_commons_contributor::source::claude_code::ClaudeCodeSource::new(root.clone());
    let session = source.discover().expect("discover").remove(0);
    let outcomes = trace_commons_contributor::submit::submit_sessions(
        &store,
        &cfg,
        vec![(
            Box::new(trace_commons_contributor::source::claude_code::ClaudeCodeSource::new(root))
                as _,
            session,
        )],
        &Default::default(),
    )
    .await
    .expect("submit");
    assert!(
        matches!(
            outcomes[0],
            trace_commons_contributor::submit::SubmitOutcome::Submitted { .. }
        ),
        "{:?}",
        outcomes[0]
    );

    let requests = seen.lock().expect("sink").clone();
    assert_eq!(requests.len(), 1, "exactly one upload");
    for name in [
        trace_commons_protocol::admission::EVIDENCE_HEADER,
        trace_commons_protocol::admission::SIGNATURE_HEADER,
    ] {
        assert!(
            !requests[0].iter().any(|(header, _)| header == name),
            "an ordinary-profile client must not offer {name}; the server \
             reads exactly these two headers to choose its evidence plan"
        );
    }
}

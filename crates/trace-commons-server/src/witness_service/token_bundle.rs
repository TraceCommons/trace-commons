// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Restricted token bundles derived from the final verified inference exchange.
//! Earlier calls never inherit the final call's receipt. No raw token arrays
//! supplied by the contributor are accepted by this service.
use super::*;
use trace_commons_protocol::{token_distribution::*, token_distribution_chat::extract_chat_tokens};

pub const TOKEN_BUNDLE_POLICY: &str = "token-distribution-restricted-v1";
const MAX_CANDIDATE_CHECKS: usize = 512;
const MAX_CANDIDATE_BYTES: usize = 65536;

/// Local source identities are attribution; provider evidence binds the bytes.
pub struct TokenBundleOptions {
    pub capture_store_id: String,
    pub capture_id: String,
    pub bundle_revision: String,
    pub restricted_token_consent: bool,
}
pub struct WitnessTokenBundle {
    pub contribution: WitnessContributionResponse,
    pub manifest_bytes: Vec<u8>,
    pub attachment_bytes: Vec<u8>,
    pub certificate: WitnessCertificate,
    pub signature_hex: String,
    pub admission: Option<(trace_commons_protocol::admission::AdmissionEvidence, String)>,
}

pub async fn witness_token_bundle(
    request: WitnessContributionRequest,
    options: TokenBundleOptions,
    policy: &InferenceAttestationPolicy,
    redactor: &dyn ContributionRedactor,
    alternative_redactor: &dyn TranscriptRedactor,
    signer: &dyn Signer,
    enclave: &dyn Enclave,
) -> Result<WitnessTokenBundle, WitnessError> {
    use trace_commons_protocol::trace_contribution::TraceContributionEventType;
    let refuse = || WitnessError::ArtifactBindingFailed;
    for id in [&options.capture_store_id, &options.capture_id] {
        if id.len() != 32
            || !id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(refuse());
        }
    }
    if !options.restricted_token_consent || request.offered_receipt.is_none() {
        return Err(WitnessError::InferenceAttestationMissing);
    }
    let verified = check_inference_attestation(
        policy,
        request.offered_receipt.as_ref(),
        &WitnessedSession::Contribution(&request.raw_contribution),
    )?;
    if verified.verified != 1 {
        return Err(WitnessError::InferenceAttestationMissing);
    }
    let exchange = request
        .raw_contribution
        .events
        .iter()
        .rev()
        .find(|e| e.event_type == TraceContributionEventType::HttpExchange)
        .ok_or_else(refuse)?;
    let (request_body, response_body) = inference::exchange_bodies(exchange).ok_or_else(refuse)?;
    let request_json: serde_json::Value =
        serde_json::from_str(request_body).map_err(|_| refuse())?;
    let streaming = request_json
        .get("stream")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let requested_model = request_json
        .get("model")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(refuse)?
        .to_string();
    let mut segments =
        extract_chat_tokens(response_body.as_bytes(), streaming).map_err(|_| refuse())?;
    if segments.len() != 1 {
        return Err(refuse());
    }
    let segment = segments.remove(0);
    let event = request
        .raw_contribution
        .events
        .iter()
        .rev()
        .find(|e| e.event_type == TraceContributionEventType::AssistantMessage)
        .ok_or_else(refuse)?;
    if event.content.as_deref().map(str::as_bytes) != Some(segment.text.as_slice()) {
        return Err(refuse());
    }
    let event_id = event.event_id.to_string();
    let source = TokenDistribution {
        version: SCHEMA_VERSION,
        capture_store_id: options.capture_store_id,
        exchange_id: options.capture_id,
        event_id: event_id.clone(),
        choice: segment.choice,
        segment: 0,
        requested_model,
        // A friendly alias is not a verified tokenizer or model revision.
        served_model: None,
        tokenizer: None,
        semantics: ProbabilitySemantics::Unknown,
        conditioning: Conditioning::Unknown,
        requested_alternatives: request_json
            .get("top_logprobs")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            .min(20) as u32,
        response_digest: ContentDigest::of(&segment.text),
        records: segment.records,
    };
    source.validate(&segment.text).map_err(|_| refuse())?;
    let contribution = witness_contribution(request, policy, redactor, signer, enclave).await?;
    let envelope: TraceContributionEnvelope =
        serde_json::from_slice(&contribution.envelope_bytes).map_err(|_| refuse())?;
    if contribution.certificate.claimed_redaction_policy_version()
        == DETERMINISTIC_REDACTION_PIPELINE_VERSION
    {
        return Err(WitnessError::RedactionFailed);
    }
    let consent_bytes = serde_json::to_vec(&(
        &envelope.consent.scopes,
        &envelope.trace_card.allowed_uses,
        TOKEN_BUNDLE_POLICY,
    ))
    .map_err(|_| refuse())?;
    let sanitized = envelope
        .events
        .iter()
        .find(|e| e.event_id.to_string() == event_id)
        .and_then(|e| e.redacted_content.as_deref())
        .ok_or_else(refuse)?;
    // Until every pipeline stage returns a composed edit map, a changed
    // segment loses all token records. Never infer offsets by heuristic diff.
    let edits = if sanitized.as_bytes() != segment.text {
        vec![RedactionEdit {
            original: ByteSpan {
                start: 0,
                end: segment.text.len() as u64,
            },
            replacement: sanitized.as_bytes().to_vec(),
        }]
    } else {
        Vec::new()
    };
    let mut checks = 0usize;
    let mut decisions = Vec::with_capacity(source.records.len());
    for record in &source.records {
        let mut keep = vec![false; record.alternatives.len()];
        if edits.is_empty() && segment.text.len() <= MAX_CANDIDATE_BYTES {
            for (index, alternative) in record.alternatives.iter().enumerate() {
                if checks >= MAX_CANDIDATE_CHECKS {
                    break;
                }
                let mut candidate = Vec::new();
                candidate.extend_from_slice(&segment.text[..record.span.start as usize]);
                candidate.extend_from_slice(&alternative.bytes);
                candidate.extend_from_slice(&segment.text[record.span.end as usize..]);
                if candidate.len() > MAX_CANDIDATE_BYTES {
                    continue;
                }
                let Ok(candidate) = String::from_utf8(candidate) else {
                    continue;
                };
                checks += 1;
                let filtered = alternative_redactor
                    .redact(&candidate)
                    .await
                    .map_err(|_| WitnessError::RedactionFailed)?;
                // The alternative pass must use the same redaction pipeline as
                // the certified transcript. A missing classifier is not success.
                if filtered.policy_version
                    != contribution.certificate.claimed_redaction_policy_version()
                {
                    return Err(WitnessError::RedactionFailed);
                }
                keep[index] = filtered.redacted == candidate;
            }
        }
        decisions.push(keep);
    }
    let attachment = filter_with_edits(
        &source,
        &segment.text,
        sanitized.as_bytes(),
        &edits,
        TOKEN_BUNDLE_POLICY,
        |_, index, _| Some(decisions[index].clone()),
    )
    .map_err(|_| refuse())?;
    let attachment_bytes = serde_json::to_vec(&attachment).map_err(|_| refuse())?;
    if attachment_bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(refuse());
    }
    let manifest = ContributionBundleManifest {
        version: SCHEMA_VERSION,
        usage_profile: TokenUsageProfile::RestrictedResearch,
        submission_id: envelope.submission_id.to_string(),
        bundle_revision: options.bundle_revision,
        envelope_digest: ContentDigest::of(&contribution.envelope_bytes),
        consent_digest: ContentDigest::of(&consent_bytes),
        policy_version: TOKEN_BUNDLE_POLICY.into(),
        attachments: vec![AttachmentDescriptor {
            artifact_id: uuid::Uuid::new_v4().to_string(),
            event_id,
            content_digest: ContentDigest::of(&attachment_bytes),
            size_bytes: attachment_bytes.len() as u64,
        }],
    };
    let manifest_bytes = manifest.canonical_bytes().map_err(|_| refuse())?;
    let text = std::str::from_utf8(&manifest_bytes).map_err(|_| refuse())?;
    let proof = check_correspondence(text, text, &[]).map_err(|_| refuse())?;
    let certificate = WitnessCertificate::from_proof(
        proof,
        CertificateDetails {
            residual_risk_verdict: contribution.residual_risk_verdict(),
            redaction_policy_version: TOKEN_BUNDLE_POLICY.into(),
            witness_measurement: enclave
                .measurement()
                .await
                .map_err(|_| WitnessError::MeasurementUnavailable)?,
            timestamp: chrono::Utc::now().timestamp(),
        },
    );
    let signature_hex = signer
        .sign_eip191(&certificate.signing_bytes())
        .map_err(|_| WitnessError::SigningUnavailable)?;
    Ok(WitnessTokenBundle {
        admission: None,
        contribution,
        manifest_bytes,
        attachment_bytes,
        certificate,
        signature_hex,
    })
}

//! The actual native start request must include the server-required PKCE method.
//! Contract: near_provisioning.rs at 4fbe60f1, NearAiStartRequest and near_ai_start.

use crate::daemon::nearai_onboarding::start_payload;
use base64::Engine;
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn start_payload_matches_the_merged_servers_s256_contract() {
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(b"synthetic-contract-verifier"));
    let device = base64::engine::general_purpose::STANDARD.encode([0x42; 32]);
    assert_eq!(
        start_payload(&challenge, &device),
        json!({
            "device_public_key": device,
            "code_challenge": challenge,
            "code_challenge_method": "S256"
        }),
        "the server requires S256 and rejects unknown start fields"
    );
}

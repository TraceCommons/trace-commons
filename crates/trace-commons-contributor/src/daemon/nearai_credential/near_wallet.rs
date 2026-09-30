//! One NEP-413 login challenge for the browser wallet connector.
//!
//! The browser receives public signing parameters; the daemon verifies the
//! resulting signature before submitting it once to Cloud. Cloud verifies
//! current on-chain ownership. No transaction or wallet access key is requested.
//! The local fragment binds the browser ceremony; the random timestamped nonce
//! binds the signature. NEAR Connect signs without a callback URL.

use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ring::{
    rand::{SecureRandom, SystemRandom},
    signature,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use url::Url;

pub const CALLBACK_PATH: &str = "/near-ai/near-wallet/callback";
pub const DEPOSIT_PATH: &str = "/near-ai/near-wallet/result";
pub const MAX_CALLBACK_BYTES: usize = 2048;
pub const MAX_AGE_MS: u64 = 300_000;
const MESSAGE: &str = "Sign in to NEAR AI Cloud";
const RECIPIENT: &str = "cloud.near.ai";
const NEP_413_TAG: u32 = (1 << 31) + 413;

/// Fieldless errors cannot leak a wallet callback, account or signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NearWalletError {
    #[error("near_ai_wallet_invalid")]
    Invalid,
    #[error("near_ai_wallet_expired")]
    Expired,
    #[error("near_ai_wallet_cancelled")]
    Cancelled,
    #[error("near_ai_wallet_already_used")]
    AlreadyUsed,
    #[error("near_ai_wallet_unavailable")]
    Unavailable,
}

/// One in-memory signing attempt. Deliberately neither Clone nor Debug nor
/// serializable: no ceremony state belongs in settings or logs.
pub struct NearWalletChallenge {
    nonce: [u8; 32],
    state: String,
    callback_url: String,
    created_at_ms: u64,
    expires_at_ms: u64,
    consumed: bool,
}

impl NearWalletChallenge {
    pub fn new(port: u16, now_ms: u64) -> Result<Self, NearWalletError> {
        if port == 0 || now_ms == 0 || now_ms > i64::MAX as u64 - MAX_AGE_MS {
            return Err(NearWalletError::Invalid);
        }
        let mut nonce = [0; 32];
        let mut state = [0; 32];
        let random = SystemRandom::new();
        random
            .fill(&mut nonce[8..])
            .map_err(|_| NearWalletError::Unavailable)?;
        random
            .fill(&mut state)
            .map_err(|_| NearWalletError::Unavailable)?;
        nonce[..8].copy_from_slice(&now_ms.to_be_bytes());
        Ok(Self {
            nonce,
            state: URL_SAFE_NO_PAD.encode(state),
            callback_url: format!("http://127.0.0.1:{port}{CALLBACK_PATH}"),
            created_at_ms: now_ms,
            expires_at_ms: now_ms + MAX_AGE_MS,
            consumed: false,
        })
    }

    pub fn browser_url(&self) -> Result<String, NearWalletError> {
        if self.consumed {
            return Err(NearWalletError::AlreadyUsed);
        }
        let mut url = Url::parse(&self.callback_url).map_err(|_| NearWalletError::Unavailable)?;
        url.set_fragment(Some(&self.state));
        Ok(url.into())
    }

    pub fn browser_configuration(&self) -> serde_json::Value {
        serde_json::json!({
            "message": MESSAGE,
            "recipient": RECIPIENT,
            "nonce": self.nonce,
            "expires_at_ms": self.expires_at_ms,
        })
    }

    pub fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    /// Verify the connector's signed result and its original ceremony state.
    /// Invalid probes do not consume the attempt. A valid signature or a
    /// state-bound cancellation does; neither can be retried through this value.
    pub fn verify_callback(
        &mut self,
        body: &str,
        now_ms: u64,
    ) -> Result<VerifiedNearWalletSignIn, NearWalletError> {
        if self.consumed {
            return Err(NearWalletError::AlreadyUsed);
        }
        if now_ms < self.created_at_ms || now_ms >= self.expires_at_ms {
            return Err(NearWalletError::Expired);
        }
        let callback = Callback::parse(body)?;
        if !same_bytes(callback.state.as_bytes(), self.state.as_bytes()) {
            return Err(NearWalletError::Invalid);
        }
        if callback.error {
            self.consumed = true;
            return Err(NearWalletError::Cancelled);
        }
        let account_id = callback.account_id.ok_or(NearWalletError::Invalid)?;
        let public_key = callback.public_key.ok_or(NearWalletError::Invalid)?;
        let encoded_signature = callback.signature.ok_or(NearWalletError::Invalid)?;
        if !valid_account_id(&account_id) {
            return Err(NearWalletError::Invalid);
        }
        let key = decode_public_key(&public_key)?;
        let signature_bytes = STANDARD
            .decode(&encoded_signature)
            .map_err(|_| NearWalletError::Invalid)?;
        if signature_bytes.len() != 64 || STANDARD.encode(&signature_bytes) != encoded_signature {
            return Err(NearWalletError::Invalid);
        }
        let digest = signing_digest(MESSAGE, &self.nonce, RECIPIENT, None);
        signature::UnparsedPublicKey::new(&signature::ED25519, key)
            .verify(&digest, &signature_bytes)
            .map_err(|_| NearWalletError::Invalid)?;
        self.consumed = true;
        Ok(VerifiedNearWalletSignIn {
            signed_message: SignedMessage {
                account_id,
                public_key,
                signature: encoded_signature,
                state: callback.state,
            },
            payload: Payload {
                message: MESSAGE,
                nonce: self.nonce,
                recipient: RECIPIENT,
            },
        })
    }
}

/// Only `verify_callback` constructs this POST `/v1/auth/near` request. Cloud
/// remains responsible for proving that the signing key belongs to the account.
/// It is deliberately not Clone, Deserialize or Debug.
#[derive(Serialize)]
pub struct VerifiedNearWalletSignIn {
    signed_message: SignedMessage,
    payload: Payload,
}

impl VerifiedNearWalletSignIn {
    pub fn account_id(&self) -> &str {
        &self.signed_message.account_id
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignedMessage {
    account_id: String,
    public_key: String,
    signature: String,
    state: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload {
    message: &'static str,
    nonce: [u8; 32],
    recipient: &'static str,
}

struct Callback {
    state: String,
    account_id: Option<String>,
    public_key: Option<String>,
    signature: Option<String>,
    error: bool,
}

impl Callback {
    fn parse(body: &str) -> Result<Self, NearWalletError> {
        if body.is_empty() || body.len() > MAX_CALLBACK_BYTES || !body.is_ascii() {
            return Err(NearWalletError::Invalid);
        }
        let mut fields: [Option<String>; 5] = Default::default();
        for (key, value) in url::form_urlencoded::parse(body.as_bytes()) {
            let index = match key.as_ref() {
                "state" => 0,
                "accountId" => 1,
                "publicKey" => 2,
                "signature" => 3,
                "error" => 4,
                _ => return Err(NearWalletError::Invalid),
            };
            if fields[index].is_some() || value.is_empty() {
                return Err(NearWalletError::Invalid);
            }
            fields[index] = Some(value.into_owned());
        }
        let [state, account_id, public_key, signature, error] = fields;
        let state = state.ok_or(NearWalletError::Invalid)?;
        if state.len() != 43
            || !state
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        {
            return Err(NearWalletError::Invalid);
        }
        if error.is_some() && (account_id.is_some() || public_key.is_some() || signature.is_some())
        {
            return Err(NearWalletError::Invalid);
        }
        Ok(Self {
            state,
            account_id,
            public_key,
            signature,
            error: error.is_some(),
        })
    }
}

fn same_bytes(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

fn valid_account_id(account: &str) -> bool {
    if !(2..=64).contains(&account.len()) {
        return false;
    }
    let mut separator = true;
    for byte in account.bytes() {
        let next_separator = matches!(byte, b'.' | b'-' | b'_');
        if next_separator && separator
            || !(next_separator || byte.is_ascii_lowercase() || byte.is_ascii_digit())
        {
            return false;
        }
        separator = next_separator;
    }
    !separator
}

/// Fixed-width base58 decoding avoids adding a wallet dependency for 32 public
/// bytes. The alphabet, overflow and leading-zero checks enforce canonical form.
fn decode_public_key(value: &str) -> Result<[u8; 32], NearWalletError> {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let value = value
        .strip_prefix("ed25519:")
        .ok_or(NearWalletError::Invalid)?;
    if !(32..=44).contains(&value.len()) {
        return Err(NearWalletError::Invalid);
    }
    let mut bytes = [0u8; 32];
    for digit in value.bytes() {
        let mut carry = ALPHABET
            .iter()
            .position(|byte| *byte == digit)
            .ok_or(NearWalletError::Invalid)? as u16;
        for byte in bytes.iter_mut().rev() {
            carry += u16::from(*byte) * 58;
            *byte = carry as u8;
            carry >>= 8;
        }
        if carry != 0 {
            return Err(NearWalletError::Invalid);
        }
    }
    if value.bytes().take_while(|b| *b == b'1').count()
        != bytes.iter().take_while(|b| **b == 0).count()
    {
        return Err(NearWalletError::Invalid);
    }
    Ok(bytes)
}

/// NEP-413: SHA256(u32-LE tag || Borsh(message, nonce, recipient, callback)).
/// All variable strings here are bounded, locally generated constants/URLs.
fn signing_digest(
    message: &str,
    nonce: &[u8; 32],
    recipient: &str,
    callback: Option<&str>,
) -> [u8; 32] {
    let mut encoded = Vec::with_capacity(192);
    encoded.extend_from_slice(&NEP_413_TAG.to_le_bytes());
    push_string(&mut encoded, message);
    encoded.extend_from_slice(nonce);
    push_string(&mut encoded, recipient);
    encoded.push(u8::from(callback.is_some()));
    if let Some(callback) = callback {
        push_string(&mut encoded, callback);
    }
    Sha256::digest(encoded).into()
}

fn push_string(encoded: &mut Vec<u8>, value: &str) {
    encoded.extend_from_slice(&(value.len() as u32).to_le_bytes());
    encoded.extend_from_slice(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use crate::daemon::nearai_credential::near_wallet::*;
    use ring::signature::KeyPair;

    const NOW: u64 = 1_780_000_000_000;

    fn encode_key(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        let mut input = bytes.to_vec();
        let zeros = input.iter().take_while(|byte| **byte == 0).count();
        let mut result = Vec::new();
        let mut start = zeros;
        while start < input.len() {
            let mut remainder = 0u16;
            for byte in &mut input[start..] {
                let value = remainder * 256 + u16::from(*byte);
                *byte = (value / 58) as u8;
                remainder = value % 58;
            }
            result.push(ALPHABET[remainder as usize]);
            while start < input.len() && input[start] == 0 {
                start += 1;
            }
        }
        result.extend(std::iter::repeat_n(b'1', zeros));
        result.reverse();
        format!("ed25519:{}", String::from_utf8(result).unwrap())
    }

    fn signed(challenge: &NearWalletChallenge) -> String {
        let key = signature::Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let digest = signing_digest(MESSAGE, &challenge.nonce, RECIPIENT, None);
        let signed = STANDARD.encode(key.sign(&digest).as_ref());
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("accountId", "alice.near")
            .append_pair("publicKey", &encode_key(key.public_key().as_ref()))
            .append_pair("signature", &signed)
            .append_pair("state", &challenge.state)
            .finish()
    }

    #[test]
    fn local_chooser_url_and_cloud_dto_match_the_existing_contract() {
        let mut challenge = NearWalletChallenge::new(54321, NOW).unwrap();
        let url = Url::parse(&challenge.browser_url().unwrap()).unwrap();
        assert_eq!(url.origin().ascii_serialization(), "http://127.0.0.1:54321");
        assert_eq!(url.path(), CALLBACK_PATH);
        assert_eq!(url.fragment(), Some(challenge.state.as_str()));
        assert!(url.query().is_none());
        let config = challenge.browser_configuration();
        assert_eq!(config["message"], MESSAGE);
        assert_eq!(config["recipient"], RECIPIENT);
        assert_eq!(config["nonce"], serde_json::json!(challenge.nonce));
        assert!(config.get("state").is_none());
        assert_eq!(challenge.nonce[..8], NOW.to_be_bytes());
        let body = signed(&challenge);
        let result = challenge.verify_callback(&body, NOW + 1).unwrap();
        assert_eq!(result.account_id(), "alice.near");
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 2);
        assert!(json.get("signedMessage").is_none());
        assert_eq!(json["signed_message"]["accountId"], "alice.near");
        assert_eq!(json["payload"]["nonce"].as_array().unwrap().len(), 32);
        assert!(json["payload"].get("callbackUrl").is_none());
        assert_eq!(
            challenge.verify_callback(&body, NOW + 2).err(),
            Some(NearWalletError::AlreadyUsed)
        );
        assert!(challenge.browser_url().is_err());
    }

    #[test]
    fn signatures_bind_nonce_message_recipient_and_absent_callback() {
        let mut challenge = NearWalletChallenge::new(54321, NOW).unwrap();
        let key = signature::Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let mut other_nonce = challenge.nonce;
        other_nonce[31] ^= 1;
        for digest in [
            signing_digest("other message", &challenge.nonce, RECIPIENT, None),
            signing_digest(MESSAGE, &other_nonce, RECIPIENT, None),
            signing_digest(MESSAGE, &challenge.nonce, "other.example", None),
            signing_digest(
                MESSAGE,
                &challenge.nonce,
                RECIPIENT,
                Some("http://127.0.0.1:54322/near-ai/near-wallet/callback"),
            ),
        ] {
            let mut url = Url::parse("http://127.0.0.1/").unwrap();
            url.query_pairs_mut()
                .append_pair("accountId", "alice.near")
                .append_pair("publicKey", &encode_key(key.public_key().as_ref()))
                .append_pair("signature", &STANDARD.encode(key.sign(&digest).as_ref()))
                .append_pair("state", &challenge.state);
            assert_eq!(
                challenge.verify_callback(url.query().unwrap(), NOW).err(),
                Some(NearWalletError::Invalid)
            );
        }
        let body = signed(&challenge);
        assert!(
            challenge.verify_callback(&body, NOW).is_ok(),
            "invalid probes must not consume a valid attempt"
        );
    }

    #[test]
    fn state_deadline_and_signature_are_independently_required() {
        let mut challenge = NearWalletChallenge::new(54321, NOW).unwrap();
        let body = signed(&challenge);
        let other = NearWalletChallenge::new(54321, NOW).unwrap();
        assert_ne!(challenge.nonce, other.nonce);
        assert_ne!(challenge.state, other.state);
        assert!(
            challenge
                .verify_callback(&body.replace(&challenge.state, &other.state), NOW)
                .is_err()
        );
        assert_eq!(
            challenge.verify_callback(&body, NOW - 1).err(),
            Some(NearWalletError::Expired)
        );
        assert_eq!(
            challenge
                .verify_callback(&body, challenge.expires_at_ms())
                .err(),
            Some(NearWalletError::Expired)
        );
        let altered = body.replace("signature=", "signature=A");
        assert!(challenge.verify_callback(&altered, NOW).is_err());
        assert!(
            challenge
                .verify_callback(&body, NOW + MAX_AGE_MS - 1)
                .is_ok()
        );
        for (port, now) in [(0, NOW), (54321, 0), (54321, u64::MAX)] {
            assert!(NearWalletChallenge::new(port, now).is_err());
        }
    }

    #[test]
    fn duplicates_unknown_fields_missing_state_and_malformed_values_fail_closed() {
        let mut challenge = NearWalletChallenge::new(54321, NOW).unwrap();
        let valid = signed(&challenge);
        for bad in [
            String::new(),
            "x".repeat(MAX_CALLBACK_BYTES + 1),
            valid.replace("state=", "notState="),
            format!("{valid}&state={}", challenge.state),
            format!("{valid}&accountId=bob.near"),
            format!("{valid}&public%4Bey=ignored"),
            format!("{valid}&error=cancelled"),
            valid.replace("alice.near", "Alice.near"),
            valid.replace("alice.near", "alice..near"),
            valid.replace("ed25519%3A", "secp256k1%3A"),
            valid.replace("signature=", "signature=%EF%BF%BD"),
        ] {
            assert!(challenge.verify_callback(&bad, NOW).is_err());
        }
        assert!(challenge.verify_callback(&valid, NOW).is_ok());
    }

    #[test]
    fn a_state_bound_wallet_error_consumes_only_this_ceremony() {
        let mut challenge = NearWalletChallenge::new(54321, NOW).unwrap();
        assert!(challenge.verify_callback("error=cancelled", NOW).is_err());
        let body = format!("error=cancelled&state={}", challenge.state);
        assert_eq!(
            challenge.verify_callback(&body, NOW).err(),
            Some(NearWalletError::Cancelled)
        );
        assert_eq!(
            challenge.verify_callback(&body, NOW).err(),
            Some(NearWalletError::AlreadyUsed)
        );
    }

    #[test]
    fn public_key_decoding_is_fixed_width_and_canonical() {
        for bytes in [[0u8; 32], [1; 32], [255; 32]] {
            assert_eq!(decode_public_key(&encode_key(&bytes)).unwrap(), bytes);
        }
        let bytes = [7; 32];
        let valid = encode_key(&bytes);
        for bad in [
            valid.replace("ed25519:", "ed25519:1"),
            format!("ed25519:{}", "1".repeat(31)),
            format!("ed25519:{}", "z".repeat(44)),
            format!("ed25519:{}", "0".repeat(44)),
            "ed25519:é".into(),
        ] {
            assert!(decode_public_key(&bad).is_err());
        }
    }

    #[test]
    fn nep_413_serialization_matches_the_standards_worked_example() {
        // NEP-413's hi / bytes 0..31 / myapp.com / myapp.com/callback example,
        // independently spelled as Borsh bytes, including Some(callback).
        let mut bytes = vec![157, 1, 0, 128, 2, 0, 0, 0, b'h', b'i'];
        bytes.extend(0u8..32);
        bytes.extend([9, 0, 0, 0]);
        bytes.extend(b"myapp.com");
        bytes.extend([1, 18, 0, 0, 0]);
        bytes.extend(b"myapp.com/callback");
        let nonce = std::array::from_fn(|i| i as u8);
        assert_eq!(
            signing_digest("hi", &nonce, "myapp.com", Some("myapp.com/callback")),
            <[u8; 32]>::from(Sha256::digest(bytes))
        );
    }
}

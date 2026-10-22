// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The judgement service's enclave seam (spec section 2, "Keys", and
//! section 3, "Per-boot keys").
//!
//! Three keys:
//!
//! | Key | Source | Use |
//! |---|---|---|
//! | limiter root | `GetKey` on [`JUDGEMENT_LIMITER_KEY_PATH`], the pinned v0 surface | 32-byte root of the limiter keys ([`LimiterKeys::from_root`]). |
//! | sealing key | generated in memory at boot | key agreement for the sealed request and response bodies. |
//! | response-signing key | generated in memory at boot | secp256k1 key that signs every response. |
//!
//! Only the limiter root comes from `GetKey`, because the ledger it seals
//! must outlive a restart. `GetKey` material is released to every image the
//! app's KMS admits, so a key derived there cannot tell the pinned image
//! from another admitted one (#1501 review point 1). The sealing and
//! signing keys are therefore born in this process's memory from the system
//! random source, never written, never logged, and returned by no route
//! except as public values. A restart, an upgrade and a second instance each
//! have different ones.
//!
//! The limiter path is not the witness's `vms`, so the limiter root never
//! equals the witness's key even under one app id.
//!
//! The quote this service serves on `GET /v1/attestation` binds both
//! per-boot public values with the judgement layout of
//! [`ReportDataLayout::Judgement`]:
//! `"tcjudge1" | SHA-256(label || seal_pk || signing_address)[0..24] | nonce`.
//! A quote from this process is therefore the only thing that vouches for
//! the sealing key a contributor seals to.
//!
//! The measurement (`mrtd` + `mrconfigid`) is read once at boot out of a
//! signed TD report, as the witness does, and every later quote must carry
//! the same MRTD and exactly the report data asked for.

use async_trait::async_trait;
use k256::ecdsa::SigningKey;
use ring::rand::{SecureRandom, SystemRandom};
use trace_commons_attestation::judgement_seal::{
    ResponseSealer, SealError, SealSecretKey, open_request,
};
use trace_commons_attestation::report_data::{
    REPORT_DATA_LEN, ReportDataLayout, SEAL_PUBLIC_KEY_LEN, expected_report_data,
};
use trace_commons_protocol::insights_judgement::JudgementNonce;
use zeroize::Zeroizing;

use super::limiter::LimiterKeys;
use crate::witness_service::enclave::{
    EnclaveError, QuoteFacts, SIGNING_KEY_ALGORITHM, address_of, sign_eip191_with,
};

/// `GetKey` path of the limiter root. The only `GetKey` path this service
/// asks for.
pub const JUDGEMENT_LIMITER_KEY_PATH: &str = "judgement-limiter";
/// Echoed into the limiter key's chain claim; not part of the derivation.
pub const JUDGEMENT_LIMITER_KEY_PURPOSE: &str = "limiter";
/// The `algorithm` argument for the limiter key: the one the witness
/// already sends on a live agent. It does not enter the derivation, so the
/// limiter root is the same 32 bytes whatever is asked for here; reusing
/// the proven value avoids depending on what else an agent accepts.
pub const JUDGEMENT_KEY_ALGORITHM: &str = SIGNING_KEY_ALGORITHM;
/// A uniformly random 32-byte string is a valid secp256k1 scalar with
/// probability about 1 - 2^-128, so this bound is never reached by a
/// working generator; it only stops a broken one from looping.
const KEY_GENERATION_ATTEMPTS: usize = 16;

/// The transport to a dstack guest agent, as this service uses it.
///
/// Separate from the witness's `GuestAgent`, which asks for exactly one key
/// path; this one names the path, so a test can observe every `GetKey`
/// call.
#[async_trait]
pub trait JudgementAgent: Send + Sync {
    /// `GetKey(path, purpose, algorithm = JUDGEMENT_KEY_ALGORITHM)`: the 32
    /// derived bytes.
    async fn derive_key(
        &self,
        path: &str,
        purpose: &str,
    ) -> Result<Zeroizing<[u8; 32]>, EnclaveError>;

    /// `GetQuote(report_data)`, raw quote bytes.
    async fn quote(&self, report_data: &[u8; REPORT_DATA_LEN]) -> Result<Vec<u8>, EnclaveError>;
}

#[cfg(unix)]
#[async_trait]
impl JudgementAgent for crate::witness_service::enclave::DstackSocketAgent {
    async fn derive_key(
        &self,
        path: &str,
        purpose: &str,
    ) -> Result<Zeroizing<[u8; 32]>, EnclaveError> {
        Ok(self
            .get_key(path, purpose, JUDGEMENT_KEY_ALGORITHM)
            .await?
            .scalar)
    }

    async fn quote(&self, report_data: &[u8; REPORT_DATA_LEN]) -> Result<Vec<u8>, EnclaveError> {
        self.get_quote(report_data).await
    }
}

/// The 64 report-data bytes for the per-boot `seal_public_key` and
/// `signing_address` and `nonce`, in the judgement layout.
pub fn judgement_report_data(
    seal_public_key: &[u8; SEAL_PUBLIC_KEY_LEN],
    signing_address: &[u8; 20],
    nonce: &[u8; 32],
) -> [u8; REPORT_DATA_LEN] {
    expected_report_data(
        ReportDataLayout::Judgement {
            seal_public_key,
            signing_address,
        },
        nonce,
    )
}

/// A fresh response-signing key from the system random source.
fn generate_signing_key() -> Result<SigningKey, EnclaveError> {
    let rng = SystemRandom::new();
    for _ in 0..KEY_GENERATION_ATTEMPTS {
        let mut bytes = Zeroizing::new([0u8; 32]);
        rng.fill(bytes.as_mut_slice())
            .map_err(|_| EnclaveError::SigningKeyUnusable)?;
        if let Ok(key) = SigningKey::from_slice(bytes.as_slice()) {
            return Ok(key);
        }
    }
    Err(EnclaveError::SigningKeyUnusable)
}

/// The judgement service's identity inside a dstack guest: the per-boot
/// sealing and signing keys, the limiter root, and the boot measurement.
pub struct JudgementEnclave {
    agent: Box<dyn JudgementAgent>,
    seal_secret_key: SealSecretKey,
    seal_public_key: [u8; SEAL_PUBLIC_KEY_LEN],
    signing_key: SigningKey,
    signing_address: String,
    address: [u8; 20],
    limiter_root: Zeroizing<[u8; 32]>,
    measurement: String,
    boot_mr_td: [u8; 48],
}

/// The measurement only. The per-boot public values are not secret, but
/// they are linkable to one boot, and nothing needs them in a debug line.
impl std::fmt::Debug for JudgementEnclave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JudgementEnclave")
            .field("measurement", &self.measurement)
            .finish_non_exhaustive()
    }
}

impl JudgementEnclave {
    /// Derive the limiter root, generate the two per-boot keys, and read the
    /// boot measurement from a quote binding them. Two agent calls, once:
    /// one `GetKey` (the limiter path) and one `GetQuote`.
    pub async fn connect(agent: Box<dyn JudgementAgent>) -> Result<Self, EnclaveError> {
        let limiter_root = agent
            .derive_key(JUDGEMENT_LIMITER_KEY_PATH, JUDGEMENT_LIMITER_KEY_PURPOSE)
            .await?;
        let seal_secret_key =
            SealSecretKey::generate().map_err(|_| EnclaveError::SigningKeyUnusable)?;
        let seal_public_key = seal_secret_key.public_key().to_bytes();
        let signing_key = generate_signing_key()?;
        let signing_address = address_of(&signing_key);
        let address: [u8; 20] = hex::decode(&signing_address[2..])
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(EnclaveError::SigningAddressMalformed)?;

        // A zero nonce: this quote is served to nobody. It exists so the
        // measurement is read from a signed TD report, the only place
        // MRCONFIGID exists as a measured value (the witness's reason), and
        // so a platform that does not sign the judgement layout refuses the
        // boot.
        let boot_report_data = judgement_report_data(&seal_public_key, &address, &[0u8; 32]);
        let facts = QuoteFacts::parse(&agent.quote(&boot_report_data).await?)?;
        if facts.report_data != boot_report_data {
            return Err(EnclaveError::ReportDataMismatch);
        }

        Ok(Self {
            agent,
            seal_secret_key,
            seal_public_key,
            signing_key,
            signing_address,
            address,
            limiter_root,
            measurement: facts.measurement_label(),
            boot_mr_td: facts.mr_td,
        })
    }

    /// `0x` + 40 lowercase hex. Per boot.
    pub fn signing_address(&self) -> &str {
        &self.signing_address
    }

    /// The per-boot sealing public key, SEC1 compressed.
    pub fn seal_public_key(&self) -> [u8; SEAL_PUBLIC_KEY_LEN] {
        self.seal_public_key
    }

    /// `mrtd:<hex>+mrconfigid:<hex>`, the value an operator pins.
    pub fn measurement(&self) -> &str {
        &self.measurement
    }

    /// The limiter keys, expanded from the `judgement-limiter` root.
    pub fn limiter_keys(&self) -> LimiterKeys {
        LimiterKeys::from_root(&self.limiter_root)
    }

    /// A quote whose report data binds this boot's two public keys and
    /// `nonce` in the judgement layout. Refused unless the quote carries
    /// exactly that and the boot MRTD.
    pub async fn attest(&self, nonce: &JudgementNonce) -> Result<Vec<u8>, EnclaveError> {
        let report_data =
            judgement_report_data(&self.seal_public_key, &self.address, nonce.as_bytes());
        let quote = self.agent.quote(&report_data).await?;
        let facts = QuoteFacts::parse(&quote)?;
        // The replay guard: a stored quote, or one taken for another
        // caller, does not carry this nonce.
        if facts.report_data != report_data {
            return Err(EnclaveError::ReportDataMismatch);
        }
        if facts.mr_td != self.boot_mr_td {
            return Err(EnclaveError::MeasurementChanged);
        }
        Ok(quote)
    }

    /// Open a sealed request with this boot's sealing key. Returns the
    /// plaintext and the one sealer for its response.
    pub fn open_request(&self, sealed: &[u8]) -> Result<(Vec<u8>, ResponseSealer), SealError> {
        open_request(&self.seal_secret_key, sealed)
    }

    /// An EIP-191 signature over `message` by the per-boot response-signing
    /// key.
    pub fn sign_eip191(&self, message: &[u8]) -> Result<String, EnclaveError> {
        sign_eip191_with(&self.signing_key, message).map_err(|_| EnclaveError::SigningFailed)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::witness_service::enclave::quote_offsets;
    use std::sync::Mutex;
    use trace_commons_attestation::judgement_seal::{SealPublicKey, seal_request};
    use trace_commons_attestation::receipt::recover_eip191_signer;
    use trace_commons_attestation::report_data::{ReportDataError, check_report_data};

    const REAL_REPORT: &str = include_str!(
        "../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_report.json"
    );

    const LIMITER_ROOT: [u8; 32] = [0x42; 32];
    const MR_TD: [u8; 48] = [0xa1; 48];
    const MR_CONFIG_ID: [u8; 48] = [0xb2; 48];

    fn quote_with(report_data: &[u8; REPORT_DATA_LEN], mr_td: &[u8; 48]) -> Vec<u8> {
        let value: serde_json::Value = serde_json::from_str(REAL_REPORT).expect("fixture parses");
        let hex_quote = value["intel_quote"].as_str().expect("fixture has a quote");
        let mut quote =
            hex::decode(hex_quote.strip_prefix("0x").unwrap_or(hex_quote)).expect("quote is hex");
        quote[quote_offsets::MR_TD..quote_offsets::MR_TD + 48].copy_from_slice(mr_td);
        quote[quote_offsets::MR_CONFIG_ID..quote_offsets::MR_CONFIG_ID + 48]
            .copy_from_slice(&MR_CONFIG_ID);
        quote[quote_offsets::REPORT_DATA..quote_offsets::REPORT_DATA + REPORT_DATA_LEN]
            .copy_from_slice(report_data);
        quote
    }

    /// What every quote after the boot quote does.
    #[derive(Default, Clone, Copy)]
    enum AfterBoot {
        #[default]
        Honest,
        WrongReportData,
        WrongMrTd,
    }

    pub(crate) struct StubAgent {
        limiter: [u8; 32],
        after_boot: AfterBoot,
        pub(crate) keys_asked: Mutex<Vec<(String, String)>>,
        quotes_asked: Mutex<Vec<[u8; REPORT_DATA_LEN]>>,
    }

    impl StubAgent {
        pub(crate) fn new() -> Self {
            Self {
                limiter: LIMITER_ROOT,
                after_boot: AfterBoot::Honest,
                keys_asked: Mutex::new(Vec::new()),
                quotes_asked: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl JudgementAgent for std::sync::Arc<StubAgent> {
        async fn derive_key(
            &self,
            path: &str,
            purpose: &str,
        ) -> Result<Zeroizing<[u8; 32]>, EnclaveError> {
            self.keys_asked
                .lock()
                .expect("lock")
                .push((path.to_string(), purpose.to_string()));
            // Any path answers, as a real agent does: the test asserts which
            // paths were asked, not which were refused.
            Ok(Zeroizing::new(self.limiter))
        }

        async fn quote(
            &self,
            report_data: &[u8; REPORT_DATA_LEN],
        ) -> Result<Vec<u8>, EnclaveError> {
            let boot = {
                let mut asked = self.quotes_asked.lock().expect("lock");
                asked.push(*report_data);
                asked.len() == 1
            };
            if boot {
                return Ok(quote_with(report_data, &MR_TD));
            }
            Ok(match self.after_boot {
                AfterBoot::Honest => quote_with(report_data, &MR_TD),
                AfterBoot::WrongReportData => quote_with(&[0x99; REPORT_DATA_LEN], &MR_TD),
                AfterBoot::WrongMrTd => quote_with(report_data, &[0xee; 48]),
            })
        }
    }

    async fn connected(agent: StubAgent) -> (JudgementEnclave, std::sync::Arc<StubAgent>) {
        let agent = std::sync::Arc::new(agent);
        let enclave = JudgementEnclave::connect(Box::new(agent.clone()))
            .await
            .expect("the stub answers every call");
        (enclave, agent)
    }

    fn address_bytes(address: &str) -> [u8; 20] {
        hex::decode(&address[2..])
            .expect("hex")
            .try_into()
            .expect("20 bytes")
    }

    fn layout_of(enclave: &JudgementEnclave) -> ([u8; SEAL_PUBLIC_KEY_LEN], [u8; 20]) {
        (
            enclave.seal_public_key(),
            address_bytes(enclave.signing_address()),
        )
    }

    /// #1501 review point 1, #1531 review point 1: the signing key is not a
    /// `GetKey` key. The only path asked for is the limiter's, and it is
    /// not the witness's.
    #[tokio::test]
    async fn the_only_get_key_call_is_the_limiter_root() {
        let (_, agent) = connected(StubAgent::new()).await;
        let asked = agent.keys_asked.lock().expect("lock").clone();
        assert_eq!(
            asked,
            vec![(
                JUDGEMENT_LIMITER_KEY_PATH.to_string(),
                JUDGEMENT_LIMITER_KEY_PURPOSE.to_string()
            )]
        );
        assert_eq!(JUDGEMENT_LIMITER_KEY_PATH, "judgement-limiter");
        assert_ne!(
            JUDGEMENT_LIMITER_KEY_PATH,
            crate::witness_service::enclave::SIGNING_KEY_PATH,
            "a judgement key on the witness's path would be the witness's key"
        );
    }

    /// The structural half of the same rule: this module's production code
    /// names exactly one `GetKey` path and no signing path. A later edit
    /// that derives the signing key again has to delete this test.
    #[test]
    fn the_production_code_names_no_signing_get_key_path() {
        let source = include_str!("enclave.rs");
        let production = &source[..source
            .find("#[cfg(test)]\npub(crate) mod tests")
            .expect("the test module marker")];
        assert_eq!(production.matches(".derive_key(").count(), 1);
        assert!(!production.contains(concat!("\"judgement-", "signing\"")));
        assert!(!production.contains(concat!("SIGNING_KEY", "_PATH")));
    }

    /// Two boots, even against one agent and one limiter root, have two
    /// different sealing keys and two different signing addresses: the
    /// keys are born in memory, not derived.
    #[tokio::test]
    async fn two_connects_give_different_per_boot_keys() {
        let (first, _) = connected(StubAgent::new()).await;
        let (second, _) = connected(StubAgent::new()).await;
        assert_ne!(first.seal_public_key(), second.seal_public_key());
        assert_ne!(first.signing_address(), second.signing_address());
        // The sealing key and the signing key are two key pairs: the seal
        // key's x is not the signing key's.
        let signing_public = first.signing_key.verifying_key().to_encoded_point(true);
        assert_ne!(
            first.seal_public_key().as_slice(),
            signing_public.as_bytes()
        );
        // The limiter root, which must survive a restart, does not change.
        assert_eq!(
            first.limiter_keys().account_key("near-acct").as_bytes(),
            second.limiter_keys().account_key("near-acct").as_bytes()
        );
    }

    #[tokio::test]
    async fn the_limiter_keys_come_from_the_limiter_path() {
        let (enclave, _) = connected(StubAgent::new()).await;
        let expected = LimiterKeys::from_root(&LIMITER_ROOT).account_key("near-acct");
        assert_eq!(
            enclave.limiter_keys().account_key("near-acct").as_bytes(),
            expected.as_bytes()
        );
    }

    #[tokio::test]
    async fn the_boot_quote_binds_both_per_boot_keys_in_the_judgement_layout() {
        let (enclave, agent) = connected(StubAgent::new()).await;
        let boot = agent.quotes_asked.lock().expect("lock")[0];
        assert_eq!(&boot[..8], b"tcjudge1");
        let (seal, address) = layout_of(&enclave);
        assert_eq!(
            check_report_data(
                &boot,
                ReportDataLayout::Judgement {
                    seal_public_key: &seal,
                    signing_address: &address
                },
                &[0u8; 32]
            ),
            Ok(())
        );
        assert_eq!(
            enclave.measurement(),
            format!(
                "mrtd:{}+mrconfigid:{}",
                hex::encode(MR_TD),
                hex::encode(MR_CONFIG_ID)
            )
        );
    }

    #[tokio::test]
    async fn an_attestation_binds_both_keys_and_the_nonce_and_never_reads_as_a_witness_quote() {
        let (enclave, _) = connected(StubAgent::new()).await;
        let nonce = JudgementNonce::from_bytes(std::array::from_fn(|i| i as u8 * 7 + 3));
        let quote = enclave.attest(&nonce).await.expect("attested");
        let facts = QuoteFacts::parse(&quote).expect("parses");
        let (seal, address) = layout_of(&enclave);
        assert_eq!(
            check_report_data(
                &facts.report_data,
                ReportDataLayout::Judgement {
                    seal_public_key: &seal,
                    signing_address: &address
                },
                nonce.as_bytes()
            ),
            Ok(())
        );
        // Another boot's sealing key with this boot's address does not
        // match: the quote vouches for this sealing key only.
        let (other, _) = connected(StubAgent::new()).await;
        assert_eq!(
            check_report_data(
                &facts.report_data,
                ReportDataLayout::Judgement {
                    seal_public_key: &other.seal_public_key(),
                    signing_address: &address
                },
                nonce.as_bytes()
            ),
            Err(ReportDataError::SignerUnexpected)
        );
        // The same quote does not read as a witness quote.
        assert_eq!(
            check_report_data(
                &facts.report_data,
                ReportDataLayout::Witness {
                    signing_address: &address
                },
                nonce.as_bytes()
            ),
            Err(ReportDataError::SignerUnexpected)
        );
        // And another nonce gives another quote.
        let other_quote = enclave
            .attest(&JudgementNonce::from_bytes([0x5c; 32]))
            .await
            .expect("attested");
        assert_ne!(quote, other_quote);
    }

    #[tokio::test]
    async fn a_quote_without_the_asked_report_data_is_refused() {
        let mut agent = StubAgent::new();
        agent.after_boot = AfterBoot::WrongReportData;
        let (enclave, _) = connected(agent).await;
        assert_eq!(
            enclave
                .attest(&JudgementNonce::from_bytes([1; 32]))
                .await
                .unwrap_err(),
            EnclaveError::ReportDataMismatch
        );
    }

    #[tokio::test]
    async fn a_quote_with_another_mrtd_is_refused() {
        let mut agent = StubAgent::new();
        agent.after_boot = AfterBoot::WrongMrTd;
        let (enclave, _) = connected(agent).await;
        assert_eq!(
            enclave
                .attest(&JudgementNonce::from_bytes([1; 32]))
                .await
                .unwrap_err(),
            EnclaveError::MeasurementChanged
        );
    }

    #[tokio::test]
    async fn a_signature_recovers_to_the_per_boot_address() {
        let (enclave, _) = connected(StubAgent::new()).await;
        let signature = enclave.sign_eip191(b"judgement bytes").expect("signed");
        assert_eq!(signature.len(), 2 + 130);
        let recovered = recover_eip191_signer(b"judgement bytes", &signature).expect("recovers");
        assert_eq!(recovered, address_bytes(enclave.signing_address()));
    }

    /// A request sealed to this boot's key opens here and nowhere else, and
    /// the sealer it yields answers only that request's sender.
    #[tokio::test]
    async fn a_request_sealed_to_this_boot_opens_here_and_not_at_another_boot() {
        let (here, _) = connected(StubAgent::new()).await;
        let (elsewhere, _) = connected(StubAgent::new()).await;
        let key = SealPublicKey::from_bytes(&here.seal_public_key()).expect("a point");
        let (sealed, opener) = seal_request(&key, b"plaintext").expect("sealed");
        assert_eq!(
            elsewhere.open_request(&sealed).unwrap_err(),
            SealError::DoesNotOpen
        );
        let (plaintext, sealer) = here.open_request(&sealed).expect("opens");
        assert_eq!(plaintext, b"plaintext");
        let reply = sealer.seal(b"answer").expect("sealed");
        assert_eq!(opener.open(&reply).expect("opens"), b"answer");
    }

    #[tokio::test]
    async fn an_agent_that_refuses_the_limiter_key_refuses_the_boot() {
        struct NoKeys;
        #[async_trait]
        impl JudgementAgent for NoKeys {
            async fn derive_key(
                &self,
                _path: &str,
                _purpose: &str,
            ) -> Result<Zeroizing<[u8; 32]>, EnclaveError> {
                Err(EnclaveError::Transport)
            }
            async fn quote(
                &self,
                _report_data: &[u8; REPORT_DATA_LEN],
            ) -> Result<Vec<u8>, EnclaveError> {
                Err(EnclaveError::Transport)
            }
        }
        assert_eq!(
            JudgementEnclave::connect(Box::new(NoKeys))
                .await
                .unwrap_err(),
            EnclaveError::Transport
        );
    }

    #[tokio::test]
    async fn a_boot_quote_without_the_judgement_layout_refuses_the_boot() {
        let mut agent = StubAgent::new();
        // Every quote, the boot quote included, carries other report data.
        struct Wrong(std::sync::Arc<StubAgent>);
        #[async_trait]
        impl JudgementAgent for Wrong {
            async fn derive_key(
                &self,
                path: &str,
                purpose: &str,
            ) -> Result<Zeroizing<[u8; 32]>, EnclaveError> {
                self.0.derive_key(path, purpose).await
            }
            async fn quote(
                &self,
                _report_data: &[u8; REPORT_DATA_LEN],
            ) -> Result<Vec<u8>, EnclaveError> {
                Ok(quote_with(&[0x99; REPORT_DATA_LEN], &MR_TD))
            }
        }
        agent.after_boot = AfterBoot::Honest;
        assert_eq!(
            JudgementEnclave::connect(Box::new(Wrong(std::sync::Arc::new(agent))))
                .await
                .unwrap_err(),
            EnclaveError::ReportDataMismatch
        );
    }
}

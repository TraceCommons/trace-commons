// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The judgement service's enclave seam (spec section 2, "Keys").
//!
//! Two keys, both from the dstack guest agent's `GetKey` on the pinned v0
//! surface (`/v0/GetKey`, see [`crate::witness_service::enclave::GET_KEY_PATH`]),
//! each on its own path:
//!
//! | Path | Use |
//! |---|---|
//! | [`JUDGEMENT_SIGNING_KEY_PATH`] | secp256k1 key that signs every response. |
//! | [`JUDGEMENT_LIMITER_KEY_PATH`] | 32-byte root of the limiter keys ([`LimiterKeys::from_root`]). |
//!
//! Neither path is the witness's `vms`, so neither equals the witness's key
//! even under one app id. dstack derives a key from the app id and the path;
//! `purpose` and `algorithm` do not enter the derivation, so the path is the
//! only separator, and this module refuses to start if the two paths came
//! back with the same bytes.
//!
//! The quote this service serves on `GET /v1/attestation` binds
//! `"tcjudge1" | signing address (20) | nonce (32) | zeros (4)`, built with
//! [`trace_commons_attestation::report_data::expected_report_data`] under
//! [`JUDGEMENT_QUOTE_DOMAIN`]. A new tag, because the tag is domain
//! separation: a witness quote must never read as a judgement quote, or the
//! reverse.
//!
//! The measurement (`mrtd` + `mrconfigid`) is read once at boot out of a
//! signed TD report, as the witness does, and every later quote must carry
//! the same MRTD and exactly the report data asked for.

use async_trait::async_trait;
use k256::ecdsa::SigningKey;
use trace_commons_attestation::report_data::{
    JUDGEMENT_QUOTE_DOMAIN, REPORT_DATA_LEN, expected_report_data,
};
use trace_commons_protocol::insights_judgement::JudgementNonce;
use zeroize::Zeroizing;

use super::limiter::LimiterKeys;
use crate::witness_service::enclave::{
    EnclaveError, QuoteFacts, SIGNING_KEY_ALGORITHM, address_of, sign_eip191_with,
};

/// `GetKey` path of the response-signing key.
pub const JUDGEMENT_SIGNING_KEY_PATH: &str = "judgement-signing";
/// `GetKey` path of the limiter root.
pub const JUDGEMENT_LIMITER_KEY_PATH: &str = "judgement-limiter";
/// Echoed into the signing key's chain claim; not part of the derivation.
pub const JUDGEMENT_SIGNING_KEY_PURPOSE: &str = "signing";
/// Echoed into the limiter key's chain claim; not part of the derivation.
pub const JUDGEMENT_LIMITER_KEY_PURPOSE: &str = "limiter";
/// The `algorithm` argument for both keys: the one the witness already
/// sends on a live agent. It does not enter the derivation, so the limiter
/// root is the same 32 bytes whatever is asked for here; reusing the proven
/// value avoids depending on what else an agent accepts.
pub const JUDGEMENT_KEY_ALGORITHM: &str = SIGNING_KEY_ALGORITHM;

/// The transport to a dstack guest agent, as this service uses it.
///
/// Separate from the witness's `GuestAgent`, which asks for exactly one key
/// path; this one names the path, so the two judgement keys are two calls a
/// test can observe.
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

/// The 64 report-data bytes for `address` and `nonce` under `tcjudge1`.
pub fn judgement_report_data(address: &[u8; 20], nonce: &[u8; 32]) -> [u8; REPORT_DATA_LEN] {
    expected_report_data(JUDGEMENT_QUOTE_DOMAIN, address, nonce)
}

/// The judgement service's identity inside a dstack guest: the signing key,
/// the limiter root, and the boot measurement.
pub struct JudgementEnclave {
    agent: Box<dyn JudgementAgent>,
    signing_key: SigningKey,
    signing_address: String,
    address: [u8; 20],
    limiter_root: Zeroizing<[u8; 32]>,
    measurement: String,
    boot_mr_td: [u8; 48],
}

/// Public values only: the address and the measurement.
impl std::fmt::Debug for JudgementEnclave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JudgementEnclave")
            .field("signing_address", &self.signing_address)
            .field("measurement", &self.measurement)
            .finish_non_exhaustive()
    }
}

impl JudgementEnclave {
    /// Derive both keys and read the boot measurement. Three agent calls,
    /// once.
    pub async fn connect(agent: Box<dyn JudgementAgent>) -> Result<Self, EnclaveError> {
        let signing = agent
            .derive_key(JUDGEMENT_SIGNING_KEY_PATH, JUDGEMENT_SIGNING_KEY_PURPOSE)
            .await?;
        let limiter_root = agent
            .derive_key(JUDGEMENT_LIMITER_KEY_PATH, JUDGEMENT_LIMITER_KEY_PURPOSE)
            .await?;
        // Different paths derive different keys. The same bytes on both
        // means the agent did not separate them, and a limiter root equal to
        // the signing key would put the ledger key one step from a key whose
        // signatures are public.
        if *signing == *limiter_root {
            return Err(EnclaveError::MalformedResponse);
        }
        let signing_key = SigningKey::from_slice(signing.as_slice())
            .map_err(|_| EnclaveError::SigningKeyUnusable)?;
        let signing_address = address_of(&signing_key);
        let address: [u8; 20] = hex::decode(&signing_address[2..])
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(EnclaveError::SigningAddressMalformed)?;

        // A zero nonce: this quote is served to nobody. It exists so the
        // measurement is read from a signed TD report, the only place
        // MRCONFIGID exists as a measured value (the witness's reason).
        let boot_report_data = judgement_report_data(&address, &[0u8; 32]);
        let facts = QuoteFacts::parse(&agent.quote(&boot_report_data).await?)?;
        if facts.report_data != boot_report_data {
            return Err(EnclaveError::ReportDataMismatch);
        }

        Ok(Self {
            agent,
            signing_key,
            signing_address,
            address,
            limiter_root,
            measurement: facts.measurement_label(),
            boot_mr_td: facts.mr_td,
        })
    }

    /// `0x` + 40 lowercase hex.
    pub fn signing_address(&self) -> &str {
        &self.signing_address
    }

    /// The signing address as raw bytes, for the ledger's associated data.
    pub fn signing_address_bytes(&self) -> [u8; 20] {
        self.address
    }

    /// `mrtd:<hex>+mrconfigid:<hex>`, the value an operator pins.
    pub fn measurement(&self) -> &str {
        &self.measurement
    }

    /// The limiter keys, expanded from the `judgement-limiter` root.
    pub fn limiter_keys(&self) -> LimiterKeys {
        LimiterKeys::from_root(&self.limiter_root)
    }

    /// A quote whose report data binds this service's address and `nonce`
    /// under `tcjudge1`. Refused unless the quote carries exactly that and
    /// the boot MRTD.
    pub async fn attest(&self, nonce: &JudgementNonce) -> Result<Vec<u8>, EnclaveError> {
        let report_data = judgement_report_data(&self.address, nonce.as_bytes());
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

    /// An EIP-191 signature over `message` by the response-signing key.
    pub fn sign_eip191(&self, message: &[u8]) -> Result<String, EnclaveError> {
        sign_eip191_with(&self.signing_key, message).map_err(|_| EnclaveError::SigningFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::witness_service::enclave::quote_offsets;
    use std::sync::Mutex;
    use trace_commons_attestation::receipt::recover_eip191_signer;
    use trace_commons_attestation::report_data::{
        ReportDataError, WITNESS_QUOTE_DOMAIN, check_report_data,
    };

    const REAL_REPORT: &str = include_str!(
        "../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_report.json"
    );

    /// secp256k1 scalar 1 and its published address.
    const SCALAR_ONE: [u8; 32] = {
        let mut scalar = [0u8; 32];
        scalar[31] = 1;
        scalar
    };
    const ADDRESS_OF_SCALAR_ONE: &str = "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf";
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

    struct StubAgent {
        signing: [u8; 32],
        limiter: [u8; 32],
        after_boot: AfterBoot,
        keys_asked: Mutex<Vec<(String, String)>>,
        quotes_asked: Mutex<Vec<[u8; REPORT_DATA_LEN]>>,
    }

    impl StubAgent {
        fn new() -> Self {
            Self {
                signing: SCALAR_ONE,
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
            match path {
                JUDGEMENT_SIGNING_KEY_PATH => Ok(Zeroizing::new(self.signing)),
                JUDGEMENT_LIMITER_KEY_PATH => Ok(Zeroizing::new(self.limiter)),
                _ => Err(EnclaveError::AgentRefused { status: 400 }),
            }
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

    #[tokio::test]
    async fn both_keys_come_from_their_own_paths_and_neither_is_the_witness_path() {
        let (_, agent) = connected(StubAgent::new()).await;
        let asked = agent.keys_asked.lock().expect("lock").clone();
        assert_eq!(
            asked,
            vec![
                (
                    JUDGEMENT_SIGNING_KEY_PATH.to_string(),
                    JUDGEMENT_SIGNING_KEY_PURPOSE.to_string()
                ),
                (
                    JUDGEMENT_LIMITER_KEY_PATH.to_string(),
                    JUDGEMENT_LIMITER_KEY_PURPOSE.to_string()
                ),
            ]
        );
        assert_eq!(JUDGEMENT_SIGNING_KEY_PATH, "judgement-signing");
        assert_eq!(JUDGEMENT_LIMITER_KEY_PATH, "judgement-limiter");
        for (path, _) in &asked {
            assert_ne!(
                path,
                crate::witness_service::enclave::SIGNING_KEY_PATH,
                "a judgement key on the witness's path would be the witness's key"
            );
        }
    }

    #[tokio::test]
    async fn the_signing_address_comes_from_the_signing_path() {
        let (enclave, _) = connected(StubAgent::new()).await;
        assert_eq!(enclave.signing_address(), ADDRESS_OF_SCALAR_ONE);
        assert_eq!(
            enclave.signing_address_bytes(),
            address_bytes(ADDRESS_OF_SCALAR_ONE)
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
        // Positive control: the signing key is not what the limiter used.
        let from_signing = LimiterKeys::from_root(&SCALAR_ONE).account_key("near-acct");
        assert_ne!(expected.as_bytes(), from_signing.as_bytes());
    }

    #[tokio::test]
    async fn one_value_on_both_paths_refuses_the_boot() {
        let mut agent = StubAgent::new();
        agent.limiter = agent.signing;
        let refused = JudgementEnclave::connect(Box::new(std::sync::Arc::new(agent)))
            .await
            .unwrap_err();
        assert_eq!(refused, EnclaveError::MalformedResponse);
    }

    #[tokio::test]
    async fn the_boot_quote_is_bound_under_the_judgement_tag() {
        let (enclave, agent) = connected(StubAgent::new()).await;
        let boot = agent.quotes_asked.lock().expect("lock")[0];
        assert_eq!(&boot[..8], b"tcjudge1");
        assert_eq!(
            boot,
            judgement_report_data(&enclave.signing_address_bytes(), &[0u8; 32])
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
    async fn an_attestation_binds_the_address_and_nonce_under_tcjudge1_only() {
        let (enclave, _) = connected(StubAgent::new()).await;
        let nonce = JudgementNonce::from_bytes(std::array::from_fn(|i| i as u8 * 7 + 3));
        let quote = enclave.attest(&nonce).await.expect("attested");
        let facts = QuoteFacts::parse(&quote).expect("parses");
        let address = enclave.signing_address_bytes();
        assert_eq!(
            check_report_data(
                &facts.report_data,
                JUDGEMENT_QUOTE_DOMAIN,
                &address,
                nonce.as_bytes()
            ),
            Ok(())
        );
        // The same quote does not read as a witness quote.
        assert_eq!(
            check_report_data(
                &facts.report_data,
                WITNESS_QUOTE_DOMAIN,
                &address,
                nonce.as_bytes()
            ),
            Err(ReportDataError::SignerUnexpected)
        );
        // And another nonce gives another quote.
        let other = JudgementNonce::from_bytes([0x5c; 32]);
        let other_quote = enclave.attest(&other).await.expect("attested");
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
    async fn a_signature_recovers_to_the_signing_address() {
        let (enclave, _) = connected(StubAgent::new()).await;
        let signature = enclave.sign_eip191(b"judgement bytes").expect("signed");
        assert_eq!(signature.len(), 2 + 130);
        let recovered = recover_eip191_signer(b"judgement bytes", &signature).expect("recovers");
        assert_eq!(recovered, address_bytes(ADDRESS_OF_SCALAR_ONE));
    }

    #[tokio::test]
    async fn an_agent_that_refuses_a_key_refuses_the_boot() {
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
}

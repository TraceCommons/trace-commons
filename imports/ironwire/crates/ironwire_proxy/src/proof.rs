//! Settling each NEAR AI exchange's proof status, after the fact.
//!
//! An exchange served by NEAR AI is recorded as `pending`. This module is what
//! later moves it to a settled label (`ironwire_ledger::proof`), and it is the
//! only thing that can write `verified`.
//!
//! # Off the response path, always
//!
//! Nothing here runs while a request is in flight. The check reads rows that
//! are already in the ledger -- which means responses that have already been
//! delivered -- on a timer, with bounded concurrency and a bounded retry
//! budget. It cannot delay, alter or fail an answer, because by the time it
//! sees a row there is no answer left to affect.
//!
//! # What `verified` requires, all of it
//!
//! 1. A receipt for the row's upstream id, `signing_algo = ed25519`.
//! 2. `signature_kind = provider_tee`. A `gateway` receipt proves which gateway
//!    relayed the call, not which model answered, and settles as
//!    `gateway_only` however well it verifies.
//! 3. The ed25519 signature verifies over the receipt's raw `text` under the
//!    key the receipt names.
//! 4. Both digests in `text` equal the ones this row recorded for the bytes
//!    that actually crossed the wire.
//! 5. The model the receipt binds, when it binds one, is the model that served.
//! 6. The signing key is in the set a [`SignerAttestor`] returns for that
//!    model -- keys a **verified** TDX quote binds. A signature under a key we
//!    cannot tie to a quote is `unattested`, never `verified`.
//!
//! Any failure short of a positive answer lands on a label that is not proof.
//! In particular: verification switched off leaves rows `pending`, and an
//! attestor that cannot say anything yields `unattested`.
//!
//! # Nothing is logged but labels
//!
//! Row ids and status labels. Never the upstream id, the model, a digest, a
//! key or a signature.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use ed25519_dalek::{Signature, VerifyingKey};
use futures_util::StreamExt;
use ironwire_ledger::{Ledger, ProofCandidate, ProofStatus};
use ironwire_upstream::receipt::{RawReceipt, ReceiptFetch};

/// Where receipts come from. In the daemon, the backend that served the row.
pub trait ReceiptSource: Send + Sync {
    /// Fetch the receipt for `upstream_id` from `backend`.
    fn fetch(
        &self,
        backend: &str,
        upstream_id: &str,
        model: &str,
    ) -> impl Future<Output = ReceiptFetch> + Send;
}

impl ReceiptSource for crate::state::BackendRegistry {
    async fn fetch(&self, backend: &str, upstream_id: &str, model: &str) -> ReceiptFetch {
        match self.get(&ironwire_core::protocol::BackendId::from(backend)) {
            Some(backend) => backend.fetch_receipt(upstream_id, model).await,
            // Disconnected since the row was written. Nothing to ask.
            None => ReceiptFetch::NotOffered,
        }
    }
}

/// What an attestor knows about a model's signing keys.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Attestation {
    /// The ed25519 keys (64 lowercase hex characters each) that verified TDX
    /// quotes bind for this model, right now. Only keys whose quote passed
    /// DCAP verification may appear here.
    Keys(Vec<String>),
    /// The attestation report carries no entry for this model.
    NotAttested,
    /// Could not find out this time. Worth retrying.
    Unavailable,
    /// This build has no way to verify a quote, so it cannot tie any key to
    /// one.
    Unsupported,
}

/// Which keys a verified quote binds for a model.
///
/// Object-safe, because the implementation usually comes from outside this
/// crate: an embedding host that already verifies TDX quotes hands one in
/// through `EmbedOptions::with_signer_attestor`, and IronWire itself carries
/// no DCAP verifier.
///
/// An implementation must return only **per-model** keys (the report's
/// model attestations), never the gateway's: a gateway key signs `gateway`
/// receipts, which prove nothing about the model, and letting it into this
/// set would let a relabelled gateway receipt pass as proof.
#[async_trait::async_trait]
pub trait SignerAttestor: Send + Sync {
    /// The attested keys for `model` on `backend`.
    async fn model_keys(&self, backend: &str, model: &str) -> Attestation;
}

/// The attestor this build ships with: none.
///
/// Tying a receipt's key to hardware needs Intel DCAP quote verification, which
/// IronWire does not implement yet. Until it does, every receipt that otherwise
/// checks out settles as `unattested` -- a true statement -- rather than being
/// promoted on a key nothing has vouched for.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoQuoteVerification;

#[async_trait::async_trait]
impl SignerAttestor for NoQuoteVerification {
    async fn model_keys(&self, _backend: &str, _model: &str) -> Attestation {
        Attestation::Unsupported
    }
}

/// What the receipt alone establishes, before any attestation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judgement {
    /// Does not check out. See [`ProofStatus::Failed`].
    Failed,
    /// Checks out, and is the gateway's.
    GatewayOnly,
    /// Checks out, and names itself as the serving model's own.
    ProviderTee {
        /// The verified signing key, lowercase hex.
        signer: String,
        /// The model whose key set the signer has to be in.
        model: String,
    },
}

/// Check a receipt against the row it is for. Pure: no I/O, no clock.
#[must_use]
pub fn judge(candidate: &ProofCandidate, receipt: &RawReceipt) -> Judgement {
    // Asked for ed25519; anything else is not the receipt that was asked for,
    // and the ECDSA signer is not one any attestation binds.
    if receipt.signing_algo != "ed25519" {
        return Judgement::Failed;
    }
    // Exact spelling. An unrecognised or absent kind names no key source, so
    // there is nothing it could be checked against.
    let gateway = match receipt.signature_kind.as_deref() {
        Some("provider_tee") => false,
        Some("gateway") => true,
        _ => return Judgement::Failed,
    };
    let parts: Vec<&str> = receipt.text.split(':').collect();
    let (bound_model, request_hex, response_hex) = match parts.as_slice() {
        [request, response] => (None, *request, *response),
        [model, request, response] => (Some(*model), *request, *response),
        _ => return Judgement::Failed,
    };
    let Some(signer) = verify_signature(receipt) else {
        return Judgement::Failed;
    };
    let (Some(request_sha256), Some(response_sha256)) = (
        candidate.request_sha256.as_deref(),
        candidate.response_sha256.as_deref(),
    ) else {
        return Judgement::Failed;
    };
    if !same_digest(request_hex, request_sha256) || !same_digest(response_hex, response_sha256) {
        return Judgement::Failed;
    }
    let Some(served) = serving_model(candidate) else {
        return Judgement::Failed;
    };
    if bound_model.is_some_and(|bound| bound != served) {
        return Judgement::Failed;
    }
    if gateway {
        return Judgement::GatewayOnly;
    }
    Judgement::ProviderTee {
        signer,
        model: served.to_string(),
    }
}

/// The model a row was served by: what the provider said, else what was asked
/// for.
fn serving_model(candidate: &ProofCandidate) -> Option<&str> {
    candidate
        .served_model
        .as_deref()
        .or(candidate.requested_model.as_deref())
}

/// Verify the ed25519 signature over the raw `text` -- not EIP-191 prefixed --
/// and return the key, lowercase, when it holds.
///
/// Strict verification: a signature malleable under the looser check, or a
/// small-order key, is refused.
fn verify_signature(receipt: &RawReceipt) -> Option<String> {
    let key: [u8; 32] = hex_bytes(&receipt.signing_address)?.try_into().ok()?;
    let signature = receipt
        .signature
        .strip_prefix("0x")
        .unwrap_or(&receipt.signature);
    let signature: [u8; 64] = hex_bytes(signature)?.try_into().ok()?;
    let key = VerifyingKey::from_bytes(&key).ok()?;
    key.verify_strict(receipt.text.as_bytes(), &Signature::from_bytes(&signature))
        .ok()?;
    Some(hex::encode(key.as_bytes()))
}

fn hex_bytes(text: &str) -> Option<Vec<u8>> {
    hex::decode(text).ok()
}

/// Two SHA-256 hex digests name the same bytes: compared decoded, so case does
/// not matter, and exact otherwise.
fn same_digest(a: &str, b: &str) -> bool {
    match (hex_bytes(a), hex_bytes(b)) {
        (Some(a), Some(b)) => a.len() == 32 && a == b,
        _ => false,
    }
}

/// What one check decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Write this label now.
    Settle(ProofStatus),
    /// Try again next round; once the retry budget is spent, write `then`.
    Defer {
        /// The label to settle on when retries run out. Never `verified`.
        then: ProofStatus,
    },
}

/// How many times a 404 is looked at again before it is believed. See
/// [`check`].
pub const NOT_FOUND_RETRIES: u32 = 1;

/// Decide one row.
pub async fn check<S: ReceiptSource>(
    candidate: &ProofCandidate,
    source: &S,
    attestor: &dyn SignerAttestor,
) -> Outcome {
    // The provider rewrote this body to carry a warning, and by its own
    // account the rewritten bytes no longer match what the enclave signed. A
    // receipt fetched for it would read as a failure that is really ours.
    if candidate.model_alias_resolved.is_some() {
        return Outcome::Settle(ProofStatus::Unavailable);
    }
    let (Some(upstream_id), Some(_), Some(_), Some(model)) = (
        candidate.upstream_id.as_deref(),
        candidate.request_sha256.as_deref(),
        candidate.response_sha256.as_deref(),
        serving_model(candidate),
    ) else {
        // Nothing to key a fetch by, or nothing to check a receipt against --
        // body capture off, or a response that did not finish.
        return Outcome::Settle(ProofStatus::Unavailable);
    };
    let receipt = match source.fetch(&candidate.backend, upstream_id, model).await {
        // A 404 is usually permanent -- a brokered model never has a receipt --
        // but a hosted call checked moments after it finished could be ahead
        // of the provider writing its record. One more look, a round later,
        // costs a single extra `GET` per brokered call and turns that race
        // from a wrong `unavailable` into a right answer.
        ReceiptFetch::NotFound if candidate.attempts < NOT_FOUND_RETRIES => {
            return Outcome::Defer {
                then: ProofStatus::Unavailable,
            };
        }
        ReceiptFetch::NotOffered | ReceiptFetch::NotFound => {
            return Outcome::Settle(ProofStatus::Unavailable);
        }
        ReceiptFetch::Unavailable => {
            return Outcome::Defer {
                then: ProofStatus::Unavailable,
            };
        }
        ReceiptFetch::Malformed => return Outcome::Settle(ProofStatus::Failed),
        ReceiptFetch::Receipt(receipt) => receipt,
    };
    let (signer, model) = match judge(candidate, &receipt) {
        Judgement::Failed => return Outcome::Settle(ProofStatus::Failed),
        Judgement::GatewayOnly => return Outcome::Settle(ProofStatus::GatewayOnly),
        Judgement::ProviderTee { signer, model } => (signer, model),
    };
    match attestor.model_keys(&candidate.backend, &model).await {
        // An empty set is an attestor that knows nothing right now -- a stale
        // report reads empty rather than old -- not a verdict against the key.
        Attestation::Keys(keys) if keys.is_empty() => Outcome::Defer {
            then: ProofStatus::Unattested,
        },
        Attestation::Keys(keys) => {
            if keys.iter().any(|key| key.eq_ignore_ascii_case(&signer)) {
                Outcome::Settle(ProofStatus::Verified)
            } else {
                // Signed, but by a key no verified quote binds for this model.
                Outcome::Settle(ProofStatus::Failed)
            }
        }
        Attestation::NotAttested | Attestation::Unsupported => {
            Outcome::Settle(ProofStatus::Unattested)
        }
        Attestation::Unavailable => Outcome::Defer {
            then: ProofStatus::Unattested,
        },
    }
}

/// How hard the background check works.
///
/// Built from [`ProofSettings::default`] and the `with_` methods, so a later
/// knob does not break a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProofSettings {
    /// Rows read per round.
    pub batch: usize,
    /// Checks in flight at once.
    pub concurrency: usize,
    /// Deferrals before a row is settled on its fallback label.
    pub max_attempts: u32,
    /// Time between rounds.
    pub period: Duration,
}

impl Default for ProofSettings {
    fn default() -> Self {
        Self {
            batch: 32,
            concurrency: 4,
            max_attempts: 5,
            period: Duration::from_secs(30),
        }
    }
}

impl ProofSettings {
    /// Rows read per round.
    #[must_use]
    pub fn with_batch(mut self, batch: usize) -> Self {
        self.batch = batch;
        self
    }

    /// Checks in flight at once. Zero is read as one.
    #[must_use]
    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency;
        self
    }

    /// Deferrals before a row settles on its fallback label.
    #[must_use]
    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    /// Time between rounds.
    #[must_use]
    pub fn with_period(mut self, period: Duration) -> Self {
        self.period = period;
        self
    }
}

/// What one round did, for tests and the debug log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Round {
    /// Rows settled, by label.
    pub settled: Vec<(i64, ProofStatus)>,
    /// Rows deferred to a later round.
    pub deferred: Vec<i64>,
}

/// Check one batch of pending rows.
pub async fn run_once<S: ReceiptSource>(
    ledger: &Ledger,
    source: &S,
    attestor: &dyn SignerAttestor,
    settings: &ProofSettings,
) -> Round {
    let candidates = match ledger.proof_candidates(settings.batch, settings.max_attempts) {
        Ok(candidates) => candidates,
        Err(error) => {
            tracing::debug!(%error, "could not read rows awaiting a proof check");
            return Round::default();
        }
    };
    let outcomes: Vec<(ProofCandidate, Outcome)> = futures_util::stream::iter(candidates)
        .map(|candidate| async move {
            let outcome = check(&candidate, source, attestor).await;
            (candidate, outcome)
        })
        .buffer_unordered(settings.concurrency.max(1))
        .collect()
        .await;

    let mut round = Round::default();
    for (candidate, outcome) in outcomes {
        let status = match outcome {
            Outcome::Settle(status) => Some(status),
            Outcome::Defer { then } => match ledger.defer_proof(candidate.id) {
                Ok(spent) if spent >= settings.max_attempts => Some(then),
                Ok(_) => {
                    round.deferred.push(candidate.id);
                    None
                }
                Err(error) => {
                    tracing::debug!(%error, "could not defer a proof check");
                    None
                }
            },
        };
        let Some(status) = status else {
            continue;
        };
        // `verified` is only ever reached through `Outcome::Settle` from a
        // full check; a deferral's fallback is never proof.
        match ledger.settle_proof(candidate.id, status) {
            Ok(true) => {
                tracing::debug!(
                    id = candidate.id,
                    status = status.as_str(),
                    "settled proof status"
                );
                round.settled.push((candidate.id, status));
            }
            Ok(false) => {}
            Err(error) => tracing::debug!(%error, "could not settle a proof status"),
        }
    }
    round
}

/// Run [`run_once`] on a timer, forever.
pub fn spawn<S>(
    ledger: Ledger,
    source: S,
    attestor: Arc<dyn SignerAttestor>,
    settings: ProofSettings,
) -> tokio::task::JoinHandle<()>
where
    S: ReceiptSource + 'static,
{
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(settings.period);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            run_once(&ledger, &source, attestor.as_ref(), &settings).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const MODEL: &str = "Qwen/Qwen3.6-35B-A3B-FP8";
    const REQUEST: &str = "0b8d9018e79795ad139eea72ec6d916599c9c598a87f4616254ead91388d0f1d";
    const RESPONSE: &str = "2f2e5f7676838b5e4de141f7715964b8ed4e9e9b22cde2860c2326df81a0914d";

    /// A real `provider_tee` receipt NEAR AI returned on 2026-09-06 for
    /// `Qwen/Qwen3.6-35B-A3B-FP8`, verbatim. The keys the tests below mint
    /// prove the logic; this proves the logic agrees with the provider about
    /// what is signed -- the raw text, not an EIP-191 digest of it.
    const LIVE_RECEIPT: &str = r#"{
      "text": "Qwen/Qwen3.6-35B-A3B-FP8:0b8d9018e79795ad139eea72ec6d916599c9c598a87f4616254ead91388d0f1d:2f2e5f7676838b5e4de141f7715964b8ed4e9e9b22cde2860c2326df81a0914d",
      "signature": "3b1118b41e6e3226ff33c6a2b75eba6e644843eb1b7e7cace38f30e46d9109d75d348fbbe3e07e47fe000903026f821b777655e22008a1c16d43e8fca25e7b04",
      "signing_address": "aba45f0b8f90869baab26db02e8b01354bb8f8730769c60650cb7a635da602d4",
      "signing_algo": "ed25519",
      "signature_kind": "provider_tee"
    }"#;

    fn candidate() -> ProofCandidate {
        ProofCandidate {
            id: 1,
            backend: "nearai".into(),
            served_model: Some(MODEL.into()),
            requested_model: Some(MODEL.into()),
            upstream_id: Some("c54961ab1d594cf591e5566caa21196b".into()),
            request_sha256: Some(REQUEST.into()),
            response_sha256: Some(RESPONSE.into()),
            model_alias_resolved: None,
            attempts: 0,
        }
    }

    fn live() -> RawReceipt {
        serde_json::from_str(LIVE_RECEIPT).expect("fixture parses")
    }

    fn signed(key: &SigningKey, text: &str, kind: &str) -> RawReceipt {
        RawReceipt {
            text: text.to_string(),
            signature: hex::encode(key.sign(text.as_bytes()).to_bytes()),
            signing_address: hex::encode(key.verifying_key().as_bytes()),
            signing_algo: "ed25519".into(),
            signature_kind: Some(kind.into()),
        }
    }

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn text() -> String {
        format!("{MODEL}:{REQUEST}:{RESPONSE}")
    }

    #[test]
    fn a_live_provider_receipt_verifies_over_its_raw_text() {
        assert_eq!(
            judge(&candidate(), &live()),
            Judgement::ProviderTee {
                signer: "aba45f0b8f90869baab26db02e8b01354bb8f8730769c60650cb7a635da602d4".into(),
                model: MODEL.into(),
            }
        );
    }

    #[test]
    fn a_receipt_over_other_bytes_fails() {
        let mut row = candidate();
        row.response_sha256 = Some("00".repeat(32));
        assert_eq!(judge(&row, &live()), Judgement::Failed);
        let mut row = candidate();
        row.request_sha256 = Some("00".repeat(32));
        assert_eq!(judge(&row, &live()), Judgement::Failed);
    }

    #[test]
    fn a_tampered_text_fails() {
        let mut receipt = live();
        receipt.text = receipt.text.replace("Qwen3.6", "Qwen3.7");
        let mut row = candidate();
        row.served_model = Some("Qwen/Qwen3.7-35B-A3B-FP8".into());
        assert_eq!(judge(&row, &receipt), Judgement::Failed);
    }

    #[test]
    fn a_receipt_for_another_model_fails() {
        let mut row = candidate();
        row.served_model = Some("Qwen/Qwen3.8-27B".into());
        assert_eq!(judge(&row, &live()), Judgement::Failed);
    }

    #[test]
    fn a_gateway_receipt_is_never_more_than_gateway_only() {
        let receipt = signed(&key(7), &format!("{REQUEST}:{RESPONSE}"), "gateway");
        assert_eq!(judge(&candidate(), &receipt), Judgement::GatewayOnly);
    }

    /// Relabelling a real provider receipt as the gateway's must not make it
    /// more than a gateway receipt, and relabelling a gateway receipt as the
    /// model's must not let it past the attested-key check (below).
    #[test]
    fn the_kind_is_read_off_the_wire_and_an_unknown_one_fails() {
        for kind in [None, Some("PROVIDER_TEE"), Some("model"), Some("")] {
            let mut receipt = live();
            receipt.signature_kind = kind.map(str::to_string);
            assert_eq!(judge(&candidate(), &receipt), Judgement::Failed, "{kind:?}");
        }
    }

    #[test]
    fn an_ecdsa_receipt_fails() {
        let mut receipt = live();
        receipt.signing_algo = "ecdsa".into();
        assert_eq!(judge(&candidate(), &receipt), Judgement::Failed);
    }

    #[test]
    fn a_prefixed_signature_is_not_the_raw_text_signature() {
        let signer = key(9);
        let text = text();
        let prefixed = format!("\x19Ethereum Signed Message:\n{}{text}", text.len());
        let mut receipt = signed(&signer, &text, "provider_tee");
        receipt.signature = hex::encode(signer.sign(prefixed.as_bytes()).to_bytes());
        assert_eq!(judge(&candidate(), &receipt), Judgement::Failed);
    }

    #[test]
    fn malformed_fields_fail_rather_than_panic() {
        for mutate in [
            (|r: &mut RawReceipt| r.signing_address = "0x".to_string() + &r.signing_address)
                as fn(&mut RawReceipt),
            |r| r.signing_address.truncate(10),
            |r| r.signature = "zz".into(),
            |r| r.signature.truncate(64),
            |r| r.text = "only-one-part".into(),
            |r| r.text = "a:b:c:d".into(),
        ] {
            let mut receipt = live();
            mutate(&mut receipt);
            assert_eq!(judge(&candidate(), &receipt), Judgement::Failed);
        }
    }

    #[test]
    fn digests_compare_decoded_so_case_does_not_matter() {
        let receipt = signed(
            &key(3),
            &format!("{MODEL}:{}:{RESPONSE}", REQUEST.to_ascii_uppercase()),
            "provider_tee",
        );
        assert!(matches!(
            judge(&candidate(), &receipt),
            Judgement::ProviderTee { .. }
        ));
    }

    // ---- the whole check, with fakes for both network edges ----

    struct Fixed(ReceiptFetch);
    impl ReceiptSource for Fixed {
        async fn fetch(&self, _: &str, _: &str, _: &str) -> ReceiptFetch {
            self.0.clone()
        }
    }

    struct Attests(Attestation);
    #[async_trait::async_trait]
    impl SignerAttestor for Attests {
        async fn model_keys(&self, _: &str, _: &str) -> Attestation {
            self.0.clone()
        }
    }

    const LIVE_KEY: &str = "aba45f0b8f90869baab26db02e8b01354bb8f8730769c60650cb7a635da602d4";

    async fn outcome(fetch: ReceiptFetch, attestation: Attestation) -> Outcome {
        check(&candidate(), &Fixed(fetch), &Attests(attestation)).await
    }

    #[tokio::test]
    async fn verified_needs_a_quote_bound_key() {
        assert_eq!(
            outcome(
                ReceiptFetch::Receipt(live()),
                Attestation::Keys(vec![LIVE_KEY.to_ascii_uppercase()])
            )
            .await,
            Outcome::Settle(ProofStatus::Verified)
        );
    }

    #[tokio::test]
    async fn a_signer_outside_the_attested_set_fails() {
        assert_eq!(
            outcome(
                ReceiptFetch::Receipt(live()),
                Attestation::Keys(vec!["cd".repeat(32)])
            )
            .await,
            Outcome::Settle(ProofStatus::Failed)
        );
    }

    /// The build as shipped: no quote verification. A perfectly good receipt
    /// is still not proof.
    #[tokio::test]
    async fn without_quote_verification_nothing_is_verified() {
        let status = check(
            &candidate(),
            &Fixed(ReceiptFetch::Receipt(live())),
            &NoQuoteVerification,
        )
        .await;
        assert_eq!(status, Outcome::Settle(ProofStatus::Unattested));
    }

    #[tokio::test]
    async fn a_gateway_receipt_settles_gateway_only_even_with_its_key_attested() {
        let gateway = key(5);
        let receipt = signed(&gateway, &format!("{REQUEST}:{RESPONSE}"), "gateway");
        let attested = hex::encode(gateway.verifying_key().as_bytes());
        assert_eq!(
            outcome(
                ReceiptFetch::Receipt(receipt),
                Attestation::Keys(vec![attested])
            )
            .await,
            Outcome::Settle(ProofStatus::GatewayOnly)
        );
    }

    #[tokio::test]
    async fn an_empty_or_unreachable_key_set_defers_and_never_verifies() {
        for attestation in [Attestation::Keys(Vec::new()), Attestation::Unavailable] {
            assert_eq!(
                outcome(ReceiptFetch::Receipt(live()), attestation).await,
                Outcome::Defer {
                    then: ProofStatus::Unattested
                }
            );
        }
    }

    /// A brokered model has no receipt, permanently. That is not a failure.
    #[tokio::test]
    async fn no_receipt_is_unavailable_not_failed() {
        assert_eq!(
            outcome(ReceiptFetch::NotOffered, Attestation::Unsupported).await,
            Outcome::Settle(ProofStatus::Unavailable)
        );
        // A 404 is looked at once more before it is believed...
        assert_eq!(
            outcome(ReceiptFetch::NotFound, Attestation::Unsupported).await,
            Outcome::Defer {
                then: ProofStatus::Unavailable
            }
        );
        // ...and then it is.
        let retried = ProofCandidate {
            attempts: NOT_FOUND_RETRIES,
            ..candidate()
        };
        assert_eq!(
            check(
                &retried,
                &Fixed(ReceiptFetch::NotFound),
                &Attests(Attestation::Unsupported)
            )
            .await,
            Outcome::Settle(ProofStatus::Unavailable)
        );
        assert_eq!(
            outcome(ReceiptFetch::Unavailable, Attestation::Unsupported).await,
            Outcome::Defer {
                then: ProofStatus::Unavailable
            }
        );
        assert_eq!(
            outcome(ReceiptFetch::Malformed, Attestation::Unsupported).await,
            Outcome::Settle(ProofStatus::Failed)
        );
    }

    /// Nothing is fetched for a row that could not be checked anyway.
    #[tokio::test]
    async fn a_row_without_digests_or_with_a_rewritten_body_is_unavailable() {
        struct Refuses;
        impl ReceiptSource for Refuses {
            async fn fetch(&self, _: &str, _: &str, _: &str) -> ReceiptFetch {
                panic!("fetched a receipt for a row that cannot be checked")
            }
        }
        let rows = [
            ProofCandidate {
                request_sha256: None,
                ..candidate()
            },
            ProofCandidate {
                upstream_id: None,
                ..candidate()
            },
            ProofCandidate {
                model_alias_resolved: Some("qwen -> Qwen/Qwen3.6-35B-A3B-FP8".into()),
                ..candidate()
            },
        ];
        for row in rows {
            assert_eq!(
                check(&row, &Refuses, &NoQuoteVerification).await,
                Outcome::Settle(ProofStatus::Unavailable)
            );
        }
    }
}

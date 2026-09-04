// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Explicit NEAR provisioning verification, separate from existing-account login.
//!
//! This module writes nothing and grants no tenant, session, funds, or admission.
//! A handler must atomically take a server-held ceremony before verification and
//! atomically persist the resulting account/device link. Consuming `self` prevents
//! accidental local reuse; it does not replace durable replay protection.
//! Native handoff must retain the existing PKCE and exact-loopback checks.

use base64::Engine;
use ring::rand::SecureRandom;
use sha2::{Digest, Sha256};

use crate::account_near::{near_account_has_full_access_key, verify_nep413};
use crate::config::NearConfig;

pub const PROVISIONING_TTL_SECONDS: i64 = 300;
const PURPOSE: &str = "Create or recover my Trace Commons contributor account";
const DEVICE_DOMAIN: &[u8] = b"trace_commons.near_provisioning_device.v1\n";

/// Uniform safe label. Never attach account, key, RPC, or assertion text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("near_provisioning_refused")]
pub struct ProvisioningRefused;

/// Public wallet-signing payload. It contains identity and is deliberately not Debug.
pub struct ProvisioningChallenge<'a> {
    pub message: &'a str,
    pub nonce: &'a [u8; 32],
    pub recipient: &'a str,
    pub expires_at: i64,
}

/// Untrusted finish input. Neither account nor recipient can be switched here.
pub struct ProvisioningAssertion<'a> {
    pub wallet_public_key: &'a str,
    pub wallet_signature: &'a str,
    pub device_signature: &'a str,
}

/// A server-created pending ceremony. No Clone/Deserialize/Debug: it is not an
/// authorization object that callers may reconstruct from a request body.
///
/// ```compile_fail
/// use trace_commons_server::account_onboarding::PendingNearProvisioning;
/// fn duplicate(p: &PendingNearProvisioning) -> PendingNearProvisioning { p.clone() }
/// ```
pub struct PendingNearProvisioning {
    account_id: String,
    network: String,
    recipient: String,
    config_hash: [u8; 32],
    message: String,
    nonce: [u8; 32],
    device_public_key: [u8; 32],
    browser_binding: [u8; 32],
    issued_at: i64,
    expires_at: i64,
}

/// Evidence of wallet ownership and device possession only. Construction is
/// private, and persistence must consume it under the server's tenant policy.
///
/// ```compile_fail
/// use trace_commons_server::account_onboarding::VerifiedNearProvisioning;
/// let forged: VerifiedNearProvisioning = serde_json::from_str("{}").unwrap();
/// ```
pub struct VerifiedNearProvisioning {
    account_id: String,
    network: String,
    wallet_public_key: String,
    device_public_key: [u8; 32],
    anchor_hash: [u8; 32],
    ceremony_hash: [u8; 32],
    expires_at: i64,
}

impl VerifiedNearProvisioning {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }
    pub fn network(&self) -> &str {
        &self.network
    }
    pub fn wallet_public_key(&self) -> &str {
        &self.wallet_public_key
    }
    pub fn device_public_key(&self) -> &[u8; 32] {
        &self.device_public_key
    }
    /// Stable across key rotation/devices; not proof of a unique human.
    pub fn anchor_hash(&self) -> &[u8; 32] {
        &self.anchor_hash
    }
    pub fn ceremony_hash(&self) -> &[u8; 32] {
        &self.ceremony_hash
    }
    /// Persistence must recheck expiry after the asynchronous ownership lookup.
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
}

impl PendingNearProvisioning {
    /// `cfg` and `browser_binding` must come from server configuration and a
    /// browser-bound native authorization ceremony, never finish-request fields.
    /// Network labels namespace identities; operators must configure the matching
    /// RPC chain. This function does not attest the remote chain's identity.
    pub fn issue(
        cfg: &NearConfig,
        account_id: &str,
        device_public_key: [u8; 32],
        browser_binding: [u8; 32],
        now: i64,
    ) -> Result<Self, ProvisioningRefused> {
        if !canonical_account_id(account_id)
            || !matches!(cfg.network.as_str(), "mainnet" | "testnet")
            || cfg.recipient.is_empty()
            || cfg.recipient.trim() != cfg.recipient
            || cfg.recipient.chars().any(char::is_control)
            || cfg.rpc_url.trim().is_empty()
            || browser_binding == [0; 32]
        {
            return Err(ProvisioningRefused);
        }
        let expires_at = now
            .checked_add(PROVISIONING_TTL_SECONDS)
            .ok_or(ProvisioningRefused)?;
        let mut nonce = [0; 32];
        ring::rand::SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| ProvisioningRefused)?;
        let device_hash = hex::encode(Sha256::digest(device_public_key));
        let message = format!(
            "{PURPOSE}\nPurpose: trace_commons.near_provisioning.v1\nNetwork: {}\nAccount: {account_id}\nDevice: sha256:{device_hash}\nBrowser binding: sha256:{}\nExpires: {expires_at}",
            cfg.network,
            hex::encode(browser_binding)
        );
        Ok(Self {
            account_id: account_id.into(),
            network: cfg.network.clone(),
            recipient: cfg.recipient.clone(),
            config_hash: config_hash(cfg),
            message,
            nonce,
            device_public_key,
            browser_binding,
            issued_at: now,
            expires_at,
        })
    }

    pub fn challenge(&self) -> ProvisioningChallenge<'_> {
        ProvisioningChallenge {
            message: &self.message,
            nonce: &self.nonce,
            recipient: &self.recipient,
            expires_at: self.expires_at,
        }
    }

    /// Exact bytes the named device signs to prove possession. Length-prefixing
    /// and a dedicated domain prevent replay as wallet login or inference evidence.
    pub fn device_signing_bytes(&self) -> Vec<u8> {
        let mut out = DEVICE_DOMAIN.to_vec();
        for field in [
            &self.nonce[..],
            self.message.as_bytes(),
            self.recipient.as_bytes(),
            &self.browser_binding[..],
        ] {
            out.extend_from_slice(&(field.len() as u64).to_le_bytes());
            out.extend_from_slice(field);
        }
        out
    }

    /// Production entry point: signature and local binding checks precede RPC.
    /// All errors collapse to a non-identifying denial. The server must check the
    /// returned expiry again at commit; RPC can take time.
    pub async fn verify(
        self,
        cfg: &NearConfig,
        assertion: ProvisioningAssertion<'_>,
        browser_binding: &[u8; 32],
        now: i64,
    ) -> Result<VerifiedNearProvisioning, ProvisioningRefused> {
        self.verify_using(cfg, assertion, browser_binding, now, |account, key| {
            near_account_has_full_access_key(cfg, account, key)
        })
        .await
    }

    async fn verify_using<'a, F, Fut>(
        &'a self,
        cfg: &NearConfig,
        assertion: ProvisioningAssertion<'a>,
        browser_binding: &[u8; 32],
        now: i64,
        ownership: F,
    ) -> Result<VerifiedNearProvisioning, ProvisioningRefused>
    where
        F: FnOnce(&'a str, &'a str) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<bool>>,
    {
        if now < self.issued_at
            || now >= self.expires_at
            || self.config_hash != config_hash(cfg)
            || &self.browser_binding != browser_binding
        {
            return Err(ProvisioningRefused);
        }
        verify_nep413(
            assertion.wallet_public_key,
            &self.message,
            &self.nonce,
            &self.recipient,
            None,
            assertion.wallet_signature,
        )
        .map_err(|_| ProvisioningRefused)?;
        let signature = base64::engine::general_purpose::STANDARD
            .decode(assertion.device_signature)
            .map_err(|_| ProvisioningRefused)?;
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, self.device_public_key)
            .verify(&self.device_signing_bytes(), &signature)
            .map_err(|_| ProvisioningRefused)?;
        if !ownership(&self.account_id, assertion.wallet_public_key)
            .await
            .map_err(|_| ProvisioningRefused)?
        {
            return Err(ProvisioningRefused);
        }
        Ok(VerifiedNearProvisioning {
            account_id: self.account_id.clone(),
            network: self.network.clone(),
            wallet_public_key: assertion.wallet_public_key.into(),
            device_public_key: self.device_public_key,
            anchor_hash: framed_hash(
                b"trace_commons.near_account_anchor.v1\n",
                &[self.network.as_bytes(), self.account_id.as_bytes()],
            ),
            ceremony_hash: framed_hash(
                b"trace_commons.near_provisioning_ceremony.v1\n",
                &[&self.nonce],
            ),
            expires_at: self.expires_at,
        })
    }
}

fn framed_hash(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(domain);
    for field in fields {
        hash.update((field.len() as u64).to_le_bytes());
        hash.update(field);
    }
    hash.finalize().into()
}

fn config_hash(cfg: &NearConfig) -> [u8; 32] {
    framed_hash(
        b"trace_commons.near_provisioning_config.v1\n",
        &[
            cfg.network.as_bytes(),
            cfg.recipient.as_bytes(),
            cfg.rpc_url.as_bytes(),
        ],
    )
}

fn canonical_account_id(value: &str) -> bool {
    if !(2..=64).contains(&value.len()) {
        return false;
    }
    let mut separator = true;
    for byte in value.bytes() {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' => separator = false,
            b'.' | b'-' | b'_' if !separator => separator = true,
            _ => return false,
        }
    }
    !separator
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn cfg() -> NearConfig {
        NearConfig {
            rpc_url: "https://rpc.invalid".into(),
            network: "mainnet".into(),
            recipient: "app.tracecommons.test".into(),
        }
    }
    fn key(seed: u8) -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
    }
    fn pending(account: &str, device: &Ed25519KeyPair) -> PendingNearProvisioning {
        PendingNearProvisioning::issue(
            &cfg(),
            account,
            device.public_key().as_ref().try_into().unwrap(),
            [8; 32],
            100,
        )
        .unwrap()
    }
    fn wallet_signature(
        p: &PendingNearProvisioning,
        wallet: &Ed25519KeyPair,
        message: Option<&str>,
        recipient: Option<&str>,
    ) -> String {
        // Independently build the NEP-413 Borsh preimage used by wallets.
        let mut bytes = 2_147_484_061_u32.to_le_bytes().to_vec();
        let message = message.unwrap_or(&p.message);
        bytes.extend_from_slice(&(message.len() as u32).to_le_bytes());
        bytes.extend_from_slice(message.as_bytes());
        bytes.extend_from_slice(&p.nonce);
        let recipient = recipient.unwrap_or(&p.recipient);
        bytes.extend_from_slice(&(recipient.len() as u32).to_le_bytes());
        bytes.extend_from_slice(recipient.as_bytes());
        bytes.push(0); // Borsh Option::None callback
        base64::engine::general_purpose::STANDARD
            .encode(wallet.sign(&Sha256::digest(bytes)).as_ref())
    }
    fn assertion<'a>(
        wallet_key: &'a str,
        signature: &'a str,
        device_signature: &'a str,
    ) -> ProvisioningAssertion<'a> {
        ProvisioningAssertion {
            wallet_public_key: wallet_key,
            wallet_signature: signature,
            device_signature,
        }
    }
    fn wallet_key(wallet: &Ed25519KeyPair) -> String {
        format!(
            "ed25519:{}",
            bs58::encode(wallet.public_key().as_ref()).into_string()
        )
    }
    fn device_signature(p: &PendingNearProvisioning, device: &Ed25519KeyPair) -> String {
        base64::engine::general_purpose::STANDARD
            .encode(device.sign(&p.device_signing_bytes()).as_ref())
    }

    #[tokio::test]
    async fn verified_anchor_survives_keys_devices_and_has_network_separation() {
        let mut anchors = Vec::new();
        for seed in [1, 2] {
            let wallet = key(seed);
            let device = key(seed + 10);
            let p = pending("alice.near", &device);
            let wallet_key = wallet_key(&wallet);
            let sig = wallet_signature(&p, &wallet, None, None);
            let dsig = device_signature(&p, &device);
            let result = p
                .verify_using(
                    &cfg(),
                    assertion(&wallet_key, &sig, &dsig),
                    &[8; 32],
                    101,
                    |account, key| async move {
                        assert_eq!(account, "alice.near");
                        assert!(key.starts_with("ed25519:"));
                        Ok(true)
                    },
                )
                .await
                .unwrap();
            assert_eq!(result.account_id(), "alice.near");
            assert_eq!(result.network(), "mainnet");
            assert_eq!(result.wallet_public_key(), wallet_key);
            assert_eq!(
                result.device_public_key().as_slice(),
                device.public_key().as_ref()
            );
            assert_eq!(result.expires_at(), 400);
            anchors.push(*result.anchor_hash());
        }
        assert_eq!(anchors[0], anchors[1]);
        assert_eq!(
            hex::encode(anchors[0]),
            "9c2335d9afa6312a1b75700f1baf786dd207823002eaff79da64dd572cf53463"
        );
        assert_ne!(
            anchors[0],
            framed_hash(
                b"trace_commons.near_account_anchor.v1\n",
                &[b"testnet", b"alice.near"]
            )
        );
        assert_ne!(
            anchors[0],
            framed_hash(
                b"trace_commons.near_account_anchor.v1\n",
                &[b"mainnet", b"bob.near"]
            )
        );
        assert_ne!(
            framed_hash(b"x", &[b"a", b"bc"]),
            framed_hash(b"x", &[b"ab", b"c"])
        );
    }

    #[tokio::test]
    async fn all_local_failures_deny_before_ownership_lookup() {
        let calls = AtomicUsize::new(0);
        for scenario in 0..10 {
            let wallet = key(1);
            let device = key(2);
            let p = pending("alice.near", &device);
            let wallet_key = wallet_key(&wallet);
            let sig = wallet_signature(
                &p,
                &wallet,
                (scenario == 0).then_some("Sign in to Trace Commons"),
                (scenario == 1).then_some("attacker.test"),
            );
            let dsig = if scenario == 2 {
                device_signature(&p, &key(3))
            } else {
                device_signature(&p, &device)
            };
            let mut config = cfg();
            if scenario == 3 {
                config.network = "testnet".into();
            }
            if scenario == 4 {
                config.rpc_url = "https://different.invalid".into();
            }
            if scenario == 5 {
                config.recipient = "different.test".into();
            }
            let now = match scenario {
                6 => 99,
                7 => 400,
                8 => 401,
                _ => 101,
            };
            let binding = if scenario == 9 { [9; 32] } else { [8; 32] };
            let result = p
                .verify_using(
                    &config,
                    assertion(&wallet_key, &sig, &dsig),
                    &binding,
                    now,
                    |_, _| async {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(true)
                    },
                )
                .await;
            assert!(result.is_err(), "scenario {scenario}");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn ownership_absent_or_unavailable_is_not_verified() {
        for unavailable in [false, true] {
            let wallet = key(1);
            let device = key(2);
            let p = pending("alice.near", &device);
            let wallet_key = wallet_key(&wallet);
            let sig = wallet_signature(&p, &wallet, None, None);
            let dsig = device_signature(&p, &device);
            let result = p
                .verify_using(
                    &cfg(),
                    assertion(&wallet_key, &sig, &dsig),
                    &[8; 32],
                    101,
                    |_, _| async {
                        if unavailable {
                            Err(anyhow::anyhow!("unavailable"))
                        } else {
                            Ok(false)
                        }
                    },
                )
                .await;
            assert!(result.is_err());
        }
    }

    #[tokio::test]
    async fn signatures_cannot_move_between_accounts_devices_or_ceremonies() {
        let wallet = key(1);
        let device = key(2);
        let original = pending("alice.near", &device);
        let wallet_key = wallet_key(&wallet);
        let sig = wallet_signature(&original, &wallet, None, None);
        let dsig = device_signature(&original, &device);
        for other in [
            pending("bob.near", &device),
            pending("alice.near", &key(3)),
            pending("alice.near", &device),
        ] {
            assert_ne!(other.nonce, original.nonce);
            assert!(
                other
                    .verify_using(
                        &cfg(),
                        assertion(&wallet_key, &sig, &dsig),
                        &[8; 32],
                        101,
                        |_, _| async { panic!("must not reach RPC") }
                    )
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn exact_lifetime_boundaries_and_browser_commitment() {
        for now in [100, 399] {
            let wallet = key(1);
            let device = key(2);
            let p = pending("alice.near", &device);
            assert!(p.message.contains(&hex::encode([8; 32])));
            let wallet_key = wallet_key(&wallet);
            let sig = wallet_signature(&p, &wallet, None, None);
            let dsig = device_signature(&p, &device);
            assert!(
                p.verify_using(
                    &cfg(),
                    assertion(&wallet_key, &sig, &dsig),
                    &[8; 32],
                    now,
                    |_, _| async { Ok(true) }
                )
                .await
                .is_ok()
            );
        }
    }
}

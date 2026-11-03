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

/// Uniform safe label. Never attach account, key, RPC, or assertion text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("near_provisioning_refused")]
pub struct ProvisioningRefused;

/// Why a wallet provisioning proof was not verified. Labels only.
///
/// `Refused` is every failure of the proof itself and every answer the NEAR
/// RPC gave about the account. `Unavailable` is the NEAR RPC not answering
/// (unreachable, timed out, a body that never arrived, a `5xx`, `429`, `408`,
/// `421` or `425`), or this process unable to build a client to ask it: the
/// same for every account at that moment. Both refuse provisioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProvisioningFailure {
    #[error("near_provisioning_refused")]
    Refused,
    #[error("near_provisioning_rpc_unavailable")]
    Unavailable,
}

/// How a failed NEAR RPC ownership check reads. An account that does not
/// exist, or does not carry the key, is a well-formed answer (`Ok(false)`),
/// never here, so this cannot tell "no such account" from "key not on it".
/// An error here is the RPC not answering -- no response, a status that says
/// the request was not handled, or a handler error about the node itself
/// ([`crate::account_near::NearRpcNotAnswered`]) -- unless the RPC answered
/// with a status about the request.
fn ownership_check_failure(error: &anyhow::Error) -> ProvisioningFailure {
    if error
        .chain()
        .any(|cause| cause.is::<crate::account_near::NearRpcNotAnswered>())
    {
        return ProvisioningFailure::Unavailable;
    }
    let Some(error) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>())
    else {
        return ProvisioningFailure::Refused;
    };
    match error.status() {
        Some(status) if crate::near_ai_login::status_means_not_answered(status) => {
            ProvisioningFailure::Unavailable
        }
        Some(_) => ProvisioningFailure::Refused,
        // No status: the request was not answered (connect, timeout, a body
        // that stopped arriving, which `json()` reports as a decode error), or
        // no client could be built.
        None => ProvisioningFailure::Unavailable,
    }
}

/// Public wallet-signing payload. It contains identity and is deliberately not Debug.
pub struct ProvisioningChallenge<'a> {
    pub message: &'a str,
    pub nonce: &'a [u8; 32],
    pub recipient: &'a str,
    pub expires_at: i64,
    pub callback_url: Option<&'a str>,
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
    callback_url: Option<String>,
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
    pub fn ceremony_hash(&self) -> &[u8; 32] {
        &self.ceremony_hash
    }
    /// Persistence must recheck expiry after the asynchronous ownership lookup.
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
}

/// Hash-only durable ceremony payload. Raw account/key identities are supplied
/// again at finish and checked against these server-stored commitments.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredProvisioningCeremony {
    callback_hash: Option<[u8; 32]>,
    nonce: [u8; 32],
    /// Issue-time commitment binding this ceremony to the account name and
    /// network presented at start, rechecked in [`PendingNearProvisioning::restore`]
    /// so finish cannot switch accounts.
    ///
    /// This is deliberately NOT the durable anchor. Since the anchor became a
    /// peppered blind index (`near_account_identity`), an unkeyed digest of
    /// public inputs no longer belongs anywhere identity is stored -- but this
    /// row lives for [`PROVISIONING_TTL_SECONDS`], is deleted when taken, and is
    /// reachable only by possession of the ceremony handle, so keying it would
    /// buy a five-minute window in exchange for threading a secret through the
    /// pure issue/restore path. It keeps the unkeyed form and a name that says
    /// what it is.
    account_commitment: [u8; 32],
    device_hash: [u8; 32],
    browser_binding: [u8; 32],
    config_hash: [u8; 32],
    issued_at: i64,
    expires_at: i64,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct NativeProvisioningPending {
    pub ceremony: StoredProvisioningCeremony,
    pub code_challenge: String,
}

/// A NEAR AI login ceremony between `start` and `finish` (#836).
///
/// Everything the finish step must check against, and **nothing the client
/// supplies at finish**: the nonce and expiry are server-chosen, the device key
/// and PKCE challenge are what start committed to. A finish request that names
/// a different device or a different challenge is checked against this rather
/// than trusted, which is what makes the ceremony a binding rather than a
/// formality.
///
/// No account identifier of any kind. The account is whatever the introspected
/// token's subject resolves to, decided at finish and never asserted by the
/// caller -- which is the client-asserted identity #836 exists to prevent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NearAiLoginPending {
    /// Server-chosen, 32 bytes, hex. What makes the device proof unreplayable.
    pub nonce_hex: String,
    /// The S256 challenge from start; finish presents the verifier for it.
    pub code_challenge: String,
    /// The device public key this ceremony enrolls, base64.
    pub device_public_key: String,
    /// Unix seconds. Enforced by the row's own expiry as well, so a stale
    /// ceremony is unusable even if this field were ignored.
    pub expires_at: i64,
}

/// The device key is already registered under a different tenant.
///
/// `device_keys.device_key_id` is a global primary key, so provisioning
/// cannot place the key under this tenant, and the key would go on authenticating into the tenant that holds
/// it. Named, so it is not one more `near_provisioning_refused`; label-only,
/// because which tenant holds the key is exactly what must not be disclosed.
/// Carried as `DatabaseError::Pool(DEVICE_KEY_REGISTERED_ELSEWHERE)`; test it
/// with [`is_device_key_registered_elsewhere`].
pub const DEVICE_KEY_REGISTERED_ELSEWHERE: &str =
    "near_provisioning_device_key_registered_elsewhere";

/// Whether a provisioning error is the named
/// [`DEVICE_KEY_REGISTERED_ELSEWHERE`] refusal.
pub fn is_device_key_registered_elsewhere(error: &crate::error::DatabaseError) -> bool {
    matches!(error, crate::error::DatabaseError::Pool(label) if label == DEVICE_KEY_REGISTERED_ELSEWHERE)
}

/// A sign-in presented a device key its account revoked
/// (`DELETE /v1/account/devices/{id}`). Revocation is never undone; the
/// device signs in again with a new key, as a new device. Named so the
/// client can tell this from the uniform refusal and say so. Label-only.
///
/// Carried as `DatabaseError::Pool(DEVICE_KEY_REVOKED)`; test it with
/// [`is_device_key_revoked`].
pub const DEVICE_KEY_REVOKED: &str = "near_provisioning_device_key_revoked";

/// Whether `error` is the [`DEVICE_KEY_REVOKED`] refusal.
pub fn is_device_key_revoked(error: &crate::error::DatabaseError) -> bool {
    matches!(error, crate::error::DatabaseError::Pool(label) if label == DEVICE_KEY_REVOKED)
}

/// Whether a provisioning error is the database unable to answer rather than
/// a refusal. `true` means the request was not decided, and the caller says
/// "unavailable" (`503`); `false` means the uniform refusal.
///
/// Three kinds are unavailable:
///
/// - **Down**: no connection from the pool (it timed out, is closed, or could
///   not connect), a connection lost mid-statement, or PostgreSQL saying it is
///   shutting down, out of resources, timing an idle session out (`57P05`),
///   or read-only after a failover (`25006`).
/// - **Misconfigured**: a pool that could not be built or run; anything the
///   pool met while *creating* a connection (credentials PostgreSQL rejects,
///   TLS, configuration, a database that does not exist); a dropped database
///   (`57P04`); a grant missing or revoked (`42501`), or a deploy whose
///   migration has not run (`42P01` undefined table, `42703` undefined
///   column); or a required pool the deployment did not configure (a
///   `missing-control:` label). The same for every request. `42501` is also
///   the code of a row-level-security violation; on these paths every row is
///   written under the tenant the transaction is scoped to and nothing
///   upserts with `DO UPDATE`, so a `42501` here can only be a missing grant
///   or policy, not a fact about a row.
/// - **Contended**: a deadlock (`40P01`), a serialization failure (`40001`),
///   a lock that could not be had in time (`55P03`) or a cancelled statement
///   (`57014`). On the provisioning paths the rows that can contend are the
///   advisory lock on the login's anchor (reached only with a NEAR AI token
///   for that login, introspected first), the device key's row (reached only
///   with the device proof, verified first), and rows under the tenant those
///   two decide. So only a requester already holding the key or the login can
///   meet contention on them, and "unavailable" tells it nothing it did not
///   hold. A `57014` from a query that is merely slow is not about any
///   identity at all.
///
/// What stays a refusal:
///
/// - Every decided refusal: `DatabaseError::Pool(label)` (the refusal labels
///   above and in `db::postgres_account_onboarding`), `Query`, `Constraint`,
///   `Serialization`.
/// - A client-side `tokio_postgres` error from a statement that is not a
///   closed connection: a row count, a column, a type, an encode or parse
///   failure. These follow the data.
/// - Any other server error (an integrity violation, say).
///
/// Anything not positively recognised stays a refusal. Nothing here reads an
/// error's `Display` text.
pub fn is_database_unavailable(error: &crate::error::DatabaseError) -> bool {
    use crate::error::DatabaseError;
    match error {
        DatabaseError::PoolBuild(_) => true,
        // deadpool raises `Backend` only while creating a connection (a
        // recycle failure is dropped and a new connection made), so every
        // pool-runtime error is a connect-time fault: down or misconfigured,
        // never an answer about a row.
        DatabaseError::PoolRuntime(_) => true,
        DatabaseError::Postgres(error) => {
            error.is_closed()
                || error
                    .code()
                    .is_some_and(|code| sqlstate_unavailable(code.code()))
        }
        DatabaseError::Pool(label) => label.starts_with("missing-control:"),
        _ => false,
    }
}

/// The SQLSTATEs [`is_database_unavailable`] counts as the database unable to
/// answer: class `28` (invalid authorization) and the codes listed there.
fn sqlstate_unavailable(code: &str) -> bool {
    code.starts_with("28")
        || matches!(
            code,
            // Connection exceptions.
            "08000" | "08001" | "08003" | "08004" | "08006"
            // Out of resources: general, disk, memory, connections.
            | "53000" | "53100" | "53200" | "53300"
            // Admin and crash shutdown, cannot connect now, database
            // dropped, idle session timeout.
            | "57P01" | "57P02" | "57P03" | "57P04" | "57P05"
            // Read-only after a failover.
            | "25006"
            // No such database; grant missing; migration not run.
            | "3D000" | "42501" | "42P01" | "42703"
            // Contention: deadlock, serialization, lock not available,
            // statement cancelled.
            | "40P01" | "40001" | "55P03" | "57014"
        )
}

/// The I/O failures that mean the network path to PostgreSQL, rather than a
/// credential, a certificate or a setting.
fn io_means_unreachable(error: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        error.kind(),
        ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::NotConnected
            | ErrorKind::BrokenPipe
            | ErrorKind::TimedOut
            | ErrorKind::UnexpectedEof
            | ErrorKind::HostUnreachable
            | ErrorKind::NetworkUnreachable
            | ErrorKind::AddrNotAvailable
    )
}

/// A label for a provisioning database error, safe for a log line: a fixed
/// refusal label, a SQLSTATE, or a fixed name. Never the error's text, which
/// can carry a connection string or a wrapped error message.
///
/// A connect-time failure is `postgres_unreachable` when the network did not
/// answer and `postgres_connect_misconfigured` otherwise (authentication,
/// TLS, configuration), so an operator looking at a rotated password or a
/// broken root store is not sent looking for an outage. A server's own
/// answer at connect is its SQLSTATE (`sqlstate_28P01`, `sqlstate_3D000`).
pub fn database_error_label(error: &crate::error::DatabaseError) -> String {
    use crate::error::DatabaseError;
    use deadpool_postgres::PoolError;
    match error {
        DatabaseError::Pool(label) => {
            let label_shaped = !label.is_empty()
                && label.len() <= 64
                && label.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b':')
                });
            if label_shaped {
                label.clone()
            } else if label.starts_with("missing-control:") {
                "missing_control".to_string()
            } else {
                "pool_unlabelled".to_string()
            }
        }
        DatabaseError::Postgres(error) => match error.code() {
            Some(code) => format!("sqlstate_{}", code.code()),
            None if error.is_closed() => "postgres_connection_closed".to_string(),
            None => "postgres_client_error".to_string(),
        },
        DatabaseError::PoolRuntime(PoolError::Backend(error)) => match error.code() {
            Some(code) => format!("sqlstate_{}", code.code()),
            None => {
                let unreachable = std::error::Error::source(error)
                    .and_then(|cause| cause.downcast_ref::<std::io::Error>())
                    .is_some_and(io_means_unreachable);
                if unreachable {
                    "postgres_unreachable".to_string()
                } else {
                    "postgres_connect_misconfigured".to_string()
                }
            }
        },
        DatabaseError::PoolRuntime(PoolError::Timeout(_)) => "pool_timeout".to_string(),
        DatabaseError::PoolRuntime(PoolError::Closed) => "pool_closed".to_string(),
        DatabaseError::PoolRuntime(_) => "pool_runtime".to_string(),
        DatabaseError::PoolBuild(_) => "pool_build".to_string(),
        DatabaseError::Query(_) => "query".to_string(),
        DatabaseError::Constraint(_) => "constraint".to_string(),
        DatabaseError::Serialization(_) => "serialization".to_string(),
        DatabaseError::NotFound { .. } => "not_found".to_string(),
        DatabaseError::Migration(_) => "migration".to_string(),
        DatabaseError::UploadPrincipalAuthenticationDenied => {
            "upload_principal_authentication_denied".to_string()
        }
    }
}

/// Whether `error` is a decided refusal: one of the provisioning code's own
/// refusal labels, rather than a fault. A caller logs these quietly; any
/// requester holding a device key can produce one per request.
///
/// An explicit list, not "any `Pool` label": a `Pool` can also carry a trait
/// default that means this backend cannot provision at all
/// (`near_provisioning_unconfigured`), or free text from a wrapped error,
/// and those are faults an operator must see.
pub fn is_decided_refusal(error: &crate::error::DatabaseError) -> bool {
    use crate::error::DatabaseError;
    match error {
        DatabaseError::Pool(label) => matches!(
            label.as_str(),
            "near_provisioning_refused"
                | "near_provisioning_wallet_key_registered_elsewhere"
                | "near_provisioning_device_bound_to_other_account"
                | DEVICE_KEY_REGISTERED_ELSEWHERE
                | DEVICE_KEY_REVOKED
        ),
        DatabaseError::Query(label) => label == "near_provisioning_ambiguous",
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProvisionedNearAccount {
    pub tenant_id: String,
    pub account_id: uuid::Uuid,
    pub device_key_id: String,
    pub anchor_hash: String,
}

impl StoredProvisioningCeremony {
    /// Unix seconds after which the ceremony is unusable; the row is stored
    /// with this expiry, and a put-back keeps it.
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
}

impl PendingNearProvisioning {
    pub fn into_stored(self) -> StoredProvisioningCeremony {
        StoredProvisioningCeremony {
            callback_hash: self
                .callback_url
                .as_ref()
                .map(|s| Sha256::digest(s.as_bytes()).into()),
            nonce: self.nonce,
            account_commitment: framed_hash(
                b"trace_commons.near_account_anchor.v1\n",
                &[self.network.as_bytes(), self.account_id.as_bytes()],
            ),
            device_hash: Sha256::digest(self.device_public_key).into(),
            browser_binding: self.browser_binding,
            config_hash: self.config_hash,
            issued_at: self.issued_at,
            expires_at: self.expires_at,
        }
    }

    /// Rebuild only after atomically taking a server-held durable record. This
    /// is not an endpoint accepting arbitrary stored records from a client.
    pub fn restore(
        stored: StoredProvisioningCeremony,
        cfg: &NearConfig,
        account_id: &str,
        device_public_key: [u8; 32],
        callback_url: Option<&str>,
    ) -> Result<Self, ProvisioningRefused> {
        let mut pending = Self::issue(
            cfg,
            account_id,
            device_public_key,
            stored.browser_binding,
            stored.issued_at,
        )?;
        let commitment = framed_hash(
            b"trace_commons.near_account_anchor.v1\n",
            &[cfg.network.as_bytes(), account_id.as_bytes()],
        );
        let device_hash: [u8; 32] = Sha256::digest(device_public_key).into();
        if stored.config_hash != pending.config_hash
            || stored.account_commitment != commitment
            || stored.device_hash != device_hash
            || stored.expires_at != pending.expires_at
        {
            return Err(ProvisioningRefused);
        }
        if stored.callback_hash != callback_url.map(|s| Sha256::digest(s.as_bytes()).into()) {
            return Err(ProvisioningRefused);
        }
        pending.callback_url = callback_url.map(str::to_string);
        pending.nonce = stored.nonce;
        Ok(pending)
    }

    pub fn with_wallet_callback(mut self, callback_url: &str) -> Result<Self, ProvisioningRefused> {
        let url = reqwest::Url::parse(callback_url).map_err(|_| ProvisioningRefused)?;
        if url.path() != "/account/near/provision/wallet"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || !(url.scheme() == "https"
                || (url.scheme() == "http" && url.host_str() == Some("127.0.0.1")))
        {
            return Err(ProvisioningRefused);
        }
        self.callback_url = Some(callback_url.to_string());
        Ok(self)
    }

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
        let message = trace_commons_protocol::onboarding::near_provisioning_message(
            &cfg.network,
            account_id,
            &device_public_key,
            &browser_binding,
            expires_at,
        );
        Ok(Self {
            account_id: account_id.into(),
            network: cfg.network.clone(),
            recipient: cfg.recipient.clone(),
            config_hash: config_hash(cfg),
            callback_url: None,
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
            callback_url: self.callback_url.as_deref(),
        }
    }

    /// Exact bytes the named device signs to prove possession. Length-prefixing
    /// and a dedicated domain prevent replay as wallet login or inference evidence.
    pub fn device_signing_bytes(&self) -> Vec<u8> {
        trace_commons_protocol::onboarding::near_provisioning_device_bytes(
            &self.nonce,
            &self.message,
            &self.recipient,
            &self.browser_binding,
            self.callback_url.as_deref(),
        )
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
        self.verify_classified(cfg, assertion, browser_binding, now)
            .await
            .map_err(|_| ProvisioningRefused)
    }

    /// [`Self::verify`], saying whether a failure is the NEAR RPC not
    /// answering ([`ProvisioningFailure::Unavailable`]) rather than a refusal.
    pub async fn verify_classified(
        self,
        cfg: &NearConfig,
        assertion: ProvisioningAssertion<'_>,
        browser_binding: &[u8; 32],
        now: i64,
    ) -> Result<VerifiedNearProvisioning, ProvisioningFailure> {
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
    ) -> Result<VerifiedNearProvisioning, ProvisioningFailure>
    where
        F: FnOnce(&'a str, &'a str) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<bool>>,
    {
        if now < self.issued_at
            || now >= self.expires_at
            || self.config_hash != config_hash(cfg)
            || &self.browser_binding != browser_binding
        {
            return Err(ProvisioningFailure::Refused);
        }
        verify_nep413(
            assertion.wallet_public_key,
            &self.message,
            &self.nonce,
            &self.recipient,
            self.callback_url.as_deref(),
            assertion.wallet_signature,
        )
        .map_err(|_| ProvisioningFailure::Refused)?;
        let signature = base64::engine::general_purpose::STANDARD
            .decode(assertion.device_signature)
            .map_err(|_| ProvisioningFailure::Refused)?;
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, self.device_public_key)
            .verify(&self.device_signing_bytes(), &signature)
            .map_err(|_| ProvisioningFailure::Refused)?;
        // Last, because it is the only check that leaves this machine.
        if !ownership(&self.account_id, assertion.wallet_public_key)
            .await
            .map_err(|error| ownership_check_failure(&error))?
        {
            return Err(ProvisioningFailure::Refused);
        }
        Ok(VerifiedNearProvisioning {
            account_id: self.account_id.clone(),
            network: self.network.clone(),
            wallet_public_key: assertion.wallet_public_key.into(),
            device_public_key: self.device_public_key,
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

    /// No decided refusal reads as the database not answering. Each label the
    /// provisioning code refuses with is listed, so a new one has to be
    /// looked at here; pool and deployment failures are the unavailable ones.
    #[test]
    fn no_refusal_reads_as_the_database_unavailable() {
        use crate::error::DatabaseError;
        for label in [
            DEVICE_KEY_REGISTERED_ELSEWHERE,
            DEVICE_KEY_REVOKED,
            "near_provisioning_refused",
            "near_provisioning_wallet_key_registered_elsewhere",
            "near_provisioning_device_bound_to_other_account",
        ] {
            assert!(
                !is_database_unavailable(&DatabaseError::Pool(label.into())),
                "{label}"
            );
        }
        for refusal in [
            DatabaseError::Query("near_provisioning_ambiguous".into()),
            DatabaseError::Constraint("unique".into()),
            DatabaseError::Serialization("payload".into()),
            DatabaseError::NotFound {
                entity: "ceremony".into(),
                id: "x".into(),
            },
            DatabaseError::UploadPrincipalAuthenticationDenied,
        ] {
            assert!(!is_database_unavailable(&refusal), "{refusal:?}");
        }
        assert!(is_database_unavailable(&DatabaseError::PoolRuntime(
            deadpool_postgres::PoolError::Closed
        )));
        assert!(is_database_unavailable(&DatabaseError::PoolRuntime(
            deadpool_postgres::PoolError::Timeout(deadpool_postgres::TimeoutType::Wait)
        )));
        // Deployment state, the same for every request: a pool that cannot
        // run, or a required pool the deployment did not configure.
        assert!(is_database_unavailable(&DatabaseError::PoolRuntime(
            deadpool_postgres::PoolError::NoRuntimeSpecified
        )));
        assert!(is_database_unavailable(&DatabaseError::Pool(
            "missing-control: login-resolver-pool-unconfigured".into()
        )));
    }

    /// Only the provisioning code's own refusal labels log quietly; a trait
    /// default or free text in a `Pool` is a fault.
    #[test]
    fn only_the_listed_refusals_are_decided() {
        use crate::error::DatabaseError;
        for label in [
            "near_provisioning_refused",
            "near_provisioning_wallet_key_registered_elsewhere",
            "near_provisioning_device_bound_to_other_account",
            DEVICE_KEY_REGISTERED_ELSEWHERE,
            DEVICE_KEY_REVOKED,
        ] {
            assert!(
                is_decided_refusal(&DatabaseError::Pool(label.into())),
                "{label}"
            );
        }
        assert!(is_decided_refusal(&DatabaseError::Query(
            "near_provisioning_ambiguous".into()
        )));
        for fault in [
            DatabaseError::Pool("near_provisioning_unconfigured".into()),
            DatabaseError::Pool("near_ai_login_provisioning_unconfigured".into()),
            DatabaseError::Pool("connection reset by peer".into()),
            DatabaseError::Pool("missing-control: login-resolver-pool-unconfigured".into()),
            DatabaseError::Query("other".into()),
        ] {
            assert!(!is_decided_refusal(&fault), "{fault:?}");
        }
    }

    /// A log label never carries the error's text: a refusal label is kept,
    /// anything else becomes a fixed name.
    #[test]
    fn a_database_error_label_is_label_only() {
        use crate::error::DatabaseError;
        assert_eq!(
            database_error_label(&DatabaseError::Pool(DEVICE_KEY_REGISTERED_ELSEWHERE.into())),
            DEVICE_KEY_REGISTERED_ELSEWHERE
        );
        for (error, label) in [
            (
                DatabaseError::Pool("connect to postgres://user:secret@host/db failed".into()),
                "pool_unlabelled",
            ),
            (
                DatabaseError::Pool("missing-control: login-resolver-pool-unconfigured".into()),
                "missing_control",
            ),
            (
                DatabaseError::Query("near_provisioning_ambiguous".into()),
                "query",
            ),
            (DatabaseError::Constraint("detail".into()), "constraint"),
            (
                DatabaseError::PoolRuntime(deadpool_postgres::PoolError::Closed),
                "pool_closed",
            ),
        ] {
            assert_eq!(database_error_label(&error), label, "{error:?}");
        }
    }

    /// A failed NEAR RPC ownership check is unavailable only when the RPC did
    /// not answer; an answer about the request, or a failure with no HTTP
    /// error in it, stays the refusal.
    #[tokio::test]
    async fn only_the_near_rpc_not_answering_reads_as_unavailable() {
        let cfg_at = |rpc_url: String| NearConfig {
            rpc_url,
            network: "mainnet".into(),
            recipient: "app.tracecommons.test".into(),
        };
        let closed = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            drop(listener);
            url
        };
        let error = near_account_has_full_access_key(&cfg_at(closed), "alice.near", "ed25519:x")
            .await
            .expect_err("nothing listens");
        assert_eq!(
            ownership_check_failure(&error),
            ProvisioningFailure::Unavailable
        );
        for (status, expected) in [
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                ProvisioningFailure::Unavailable,
            ),
            (
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                ProvisioningFailure::Unavailable,
            ),
            (
                axum::http::StatusCode::BAD_REQUEST,
                ProvisioningFailure::Refused,
            ),
        ] {
            let router =
                axum::Router::new().route("/", axum::routing::post(move || async move { status }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let error = near_account_has_full_access_key(&cfg_at(url), "alice.near", "ed25519:x")
                .await
                .expect_err("a non-success status");
            assert_eq!(ownership_check_failure(&error), expected, "{status}");
        }
        assert_eq!(
            ownership_check_failure(&anyhow::anyhow!("not an HTTP failure")),
            ProvisioningFailure::Refused
        );

        // A node that is not ready answers `200` with a handler error about
        // itself: not answered. One that answers about the account decides.
        for (cause, expected) in [
            ("NO_SYNCED_BLOCKS", Err(ProvisioningFailure::Unavailable)),
            ("UNAVAILABLE_SHARD", Err(ProvisioningFailure::Unavailable)),
            ("UNKNOWN_ACCOUNT", Ok(false)),
        ] {
            let router = axum::Router::new().route(
                "/",
                axum::routing::post(move || async move {
                    axum::Json(serde_json::json!({"jsonrpc": "2.0", "id": "tc", "error": {
                        "name": "HANDLER_ERROR", "cause": {"name": cause, "info": {}},
                        "code": -32000, "message": "Server error"
                    }}))
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let outcome = near_account_has_full_access_key(&cfg_at(url), "alice.near", "ed25519:x")
                .await
                .map_err(|error| ownership_check_failure(&error));
            assert_eq!(outcome, expected, "{cause}");
        }
    }

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

    /// The proof no longer carries an anchor -- the durable anchor is a peppered
    /// blind index computed in `near_account_identity` from the fields asserted
    /// here. What still has to hold at this layer is that those fields are the
    /// ceremony's own account and network and survive a change of wallet key and
    /// device, because that is what makes the index stable for a returning user.
    #[tokio::test]
    async fn verified_identity_fields_survive_keys_and_devices() {
        let mut identities = Vec::new();
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
            identities.push((
                result.account_id().to_string(),
                result.network().to_string(),
            ));
        }
        assert_eq!(identities[0], identities[1]);
        assert_eq!(
            identities[0],
            ("alice.near".to_string(), "mainnet".to_string())
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

    /// A ceremony row left by the removed passkey bind routes (it carries a
    /// `purpose`, a tenant and an account) never parses as a NEAR AI login or
    /// provisioning ceremony, so a leftover row finishes nothing.
    #[test]
    fn a_leftover_bind_ceremony_row_does_not_parse_as_a_login_ceremony() {
        let bind = serde_json::json!({
            "purpose": "bind",
            "tenant_id": "nearai-t",
            "account_id": uuid::Uuid::nil(),
            "nonce_hex": "00".repeat(32),
            "code_challenge": "c",
            "device_public_key": "d",
            "expires_at": 1,
        });
        assert!(serde_json::from_value::<NearAiLoginPending>(bind.clone()).is_err());
        assert!(serde_json::from_value::<NativeProvisioningPending>(bind).is_err());
    }
}

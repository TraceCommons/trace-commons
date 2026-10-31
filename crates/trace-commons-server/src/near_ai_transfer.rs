// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A client for NEAR AI's org-to-org credit transfers (nearai/cloud-api
//! #1271), cumulative mode only (payout spec section 4, "The NEAR AI
//! client").
//!
//! It sends from the commons' reseller org with a scoped `oc_` credential
//! held as a [`SecretString`]. Its host is the compiled-in
//! [`NEAR_AI_API_BASE_URL`], shared with login introspection; no setting
//! names a host. [`NearAiTransferClient::at_loopback`] exists for tests and
//! accepts a plain-HTTP loopback base and nothing else, so it cannot send a
//! credential anywhere off the machine.
//!
//! The client copies the NEAR AI scorer client's shape: TLS or loopback
//! only, redirects never followed, a fixed user agent, a 15 s timeout
//! (cloud-api bounds the send transaction at 10 s), bearer auth. From an
//! error reply it keeps the status and `error.type`, parsed into a closed
//! enum, and nothing else. Its logs carry the call, the outcome label and
//! the `sha256:` of the credential id, never an org id, a token, a program,
//! a transfer id or a URL (a `reqwest::Error` prints its URL, so none is
//! ever logged).

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::near_ai_api::NEAR_AI_API_BASE_URL;

/// The fixed user agent on every call.
pub const NEAR_AI_TRANSFER_USER_AGENT: &str = "TraceCommons-payout/1";
/// The bound on one call. cloud-api bounds the send transaction at 10 s.
pub const NEAR_AI_TRANSFER_TIMEOUT: Duration = Duration::from_secs(15);
/// The largest page `GET .../credit/transfers` serves.
pub const NEAR_AI_HISTORY_PAGE_LIMIT: u8 = 100;
/// The largest error or reply body read, in bytes. A send reply is a few
/// kilobytes; a history page of 100 records with 16 KiB metadata each is the
/// largest legitimate reply.
const MAX_REPLY_BYTES: usize = 4 * 1024 * 1024;

/// The credit type the commons sends from: an operator setting
/// (`TRACE_COMMONS_NEAR_AI_PAYOUT_SOURCE`) with no default. Which one is
/// right depends on how the sender org was funded (paid credit is
/// `payment`) and what the org's policy and the credential allow, so the
/// code assumes none of them; the capacity probe reports the maximum per
/// source, and a source the credential may not use reports 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NearAiCreditSource {
    Grant,
    Payment,
    StakingFarm,
}

impl NearAiCreditSource {
    /// Every source #1271 names, in its own spelling.
    pub const ALL: [Self; 3] = [Self::Payment, Self::Grant, Self::StakingFarm];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grant => "grant",
            Self::Payment => "payment",
            Self::StakingFarm => "staking_farm",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|source| source.as_str() == value)
    }
}

/// What the client sends with. `Debug` prints the source only: the token is
/// a secret, and the org and credential ids stay out of logs.
#[derive(Clone)]
pub struct NearAiTransferCredentials {
    /// `oc_` plus 64 hex.
    pub token: SecretString,
    /// The credential's `id`. Not secret; used only for its log hash, and
    /// optional: unset, log lines carry `credential=unset`.
    pub credential_id: Option<Uuid>,
    /// The commons' reseller org.
    pub sender_org: Uuid,
    pub source: NearAiCreditSource,
}

impl fmt::Debug for NearAiTransferCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NearAiTransferCredentials")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

/// Whether `token` has the shape cloud-api accepts: `oc_` plus 64 hex.
pub fn is_near_ai_org_credential(token: &str) -> bool {
    token
        .strip_prefix("oc_")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The `error.type` codes #1271 answers with that the rail tells apart.
/// Anything else is [`NearAiErrorCode::Other`]; its text is not kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NearAiErrorCode {
    TransferRateLimited,
    TransferUnavailable,
    AccountingBusy,
    InsufficientPostedCapacity,
    SenderNotReconciled,
    CredentialTransferLimitExceeded,
    OrganizationTransferLimitExceeded,
    CapacityOverflow,
    OrganizationUnavailable,
    CredentialRefused,
    SenderNotEnabled,
    Other,
}

impl NearAiErrorCode {
    fn parse(code: &str) -> Self {
        match code {
            "transfer_rate_limited" => Self::TransferRateLimited,
            "transfer_unavailable" => Self::TransferUnavailable,
            "accounting_busy" => Self::AccountingBusy,
            "insufficient_posted_capacity" => Self::InsufficientPostedCapacity,
            "sender_not_reconciled" => Self::SenderNotReconciled,
            "credential_transfer_limit_exceeded" => Self::CredentialTransferLimitExceeded,
            "organization_transfer_limit_exceeded" => Self::OrganizationTransferLimitExceeded,
            "capacity_overflow" => Self::CapacityOverflow,
            "organization_unavailable" => Self::OrganizationUnavailable,
            "credential_refused" => Self::CredentialRefused,
            "sender_not_enabled" => Self::SenderNotEnabled,
            _ => Self::Other,
        }
    }

    pub fn as_label(self) -> &'static str {
        match self {
            Self::TransferRateLimited => "transfer_rate_limited",
            Self::TransferUnavailable => "transfer_unavailable",
            Self::AccountingBusy => "accounting_busy",
            Self::InsufficientPostedCapacity => "insufficient_posted_capacity",
            Self::SenderNotReconciled => "sender_not_reconciled",
            Self::CredentialTransferLimitExceeded => "credential_transfer_limit_exceeded",
            Self::OrganizationTransferLimitExceeded => "organization_transfer_limit_exceeded",
            Self::CapacityOverflow => "capacity_overflow",
            Self::OrganizationUnavailable => "organization_unavailable",
            Self::CredentialRefused => "credential_refused",
            Self::SenderNotEnabled => "sender_not_enabled",
            Self::Other => "other",
        }
    }
}

/// Every way a call can fail, closed. Nothing from a reply is kept but its
/// status and parsed code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NearAiTransferError {
    /// A non-2xx, non-3xx reply.
    #[error("near_ai_refused")]
    Refused { status: u16, code: NearAiErrorCode },
    /// A 3xx reply. Redirects are never followed.
    #[error("near_ai_redirect_refused")]
    Redirect,
    #[error("near_ai_timeout")]
    Timeout,
    /// No reply: connect or transport failure.
    #[error("near_ai_unreachable")]
    Unreachable,
    /// A 2xx whose body could not be read as the expected shape.
    #[error("near_ai_reply_unreadable")]
    ReplyUnreadable,
    /// A 2xx that parsed but cannot be right (a 200 send without `no_op`, a
    /// negative amount, a 201 with nothing sent).
    #[error("near_ai_reply_unexpected")]
    ReplyUnexpected,
    /// The base is not TLS or loopback, or the HTTP client could not be built.
    #[error("near_ai_client_config_invalid")]
    ClientConfig,
}

impl NearAiTransferError {
    pub fn label(self) -> &'static str {
        match self {
            Self::Refused { .. } => "near_ai_refused",
            Self::Redirect => "near_ai_redirect_refused",
            Self::Timeout => "near_ai_timeout",
            Self::Unreachable => "near_ai_unreachable",
            Self::ReplyUnreadable => "near_ai_reply_unreadable",
            Self::ReplyUnexpected => "near_ai_reply_unexpected",
            Self::ClientConfig => "near_ai_client_config_invalid",
        }
    }
}

/// What a cumulative send did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CumulativeReply {
    /// 201: `amount` was sent; the program now holds `recipient_cumulative`.
    Committed {
        id: Uuid,
        amount: i64,
        recipient_cumulative: i64,
    },
    /// 200 `no_op`: nothing was sent; the program holds
    /// `recipient_cumulative`, at or above the target.
    NoOp { recipient_cumulative: i64 },
}

/// `GET .../credit/transfer/capacity`, reduced to what the rail uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NearAiCapacity {
    pub sending_enabled: bool,
    pub sender_reconciled: bool,
    pub credential_can_transfer: bool,
    /// `sources[].max_amount_per_transfer` for the configured source. A
    /// reply without that source is `ReplyUnexpected`.
    pub per_send_max: i64,
    pub sends_remaining_in_window: i64,
}

/// A history cursor, passed back as it came.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NearAiHistoryCursor {
    pub before_time: DateTime<Utc>,
    pub before_id: Uuid,
}

/// One outgoing transfer record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearAiTransferRecord {
    pub id: Uuid,
    pub sender_organization_id: Uuid,
    pub recipient_organization_id: Uuid,
    pub payout_program: Option<String>,
    pub amount: i64,
    pub recipient_cumulative: Option<i64>,
}

/// One page of outgoing history, newest first. `next_cursor` is returned on
/// every page that has a record, the last page included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearAiHistoryPage {
    pub transfers: Vec<NearAiTransferRecord>,
    pub next_cursor: Option<NearAiHistoryCursor>,
}

/// The #1271 client. Construct with [`NearAiTransferClient::new`].
pub struct NearAiTransferClient {
    http: reqwest::Client,
    base: String,
    credentials: NearAiTransferCredentials,
    credential_hash: String,
}

impl fmt::Debug for NearAiTransferClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NearAiTransferClient")
            .field("credential", &self.credential_hash)
            .finish_non_exhaustive()
    }
}

/// True when `base` is TLS, or plain HTTP to a loopback IP literal (never
/// a name such as `localhost`, which resolves through hosts and DNS), with
/// no userinfo, query or fragment. Parsed, not matched as
/// a string, so `http://127.0.0.1@elsewhere/` is refused.
fn base_is_tls_or_loopback(base: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base) else {
        return false;
    };
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || base.ends_with('/')
    {
        return false;
    }
    match url.scheme() {
        "https" => url.host().is_some(),
        "http" => is_loopback_host(&url),
        _ => false,
    }
}

fn is_loopback_host(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let literal = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    literal
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

impl NearAiTransferClient {
    /// A client against the compiled-in [`NEAR_AI_API_BASE_URL`].
    pub fn new(credentials: NearAiTransferCredentials) -> Result<Self, NearAiTransferError> {
        Self::build(NEAR_AI_API_BASE_URL, credentials, NEAR_AI_TRANSFER_TIMEOUT)
    }

    /// A client against a plain-HTTP loopback IP literal, for a local mock.
    /// Any other base, TLS or a host name included, is refused: the only
    /// TLS host this client talks to is the compiled-in one. Public only
    /// because the mock suite sits outside the crate; not for production.
    #[doc(hidden)]
    pub fn at_loopback(
        base: &str,
        credentials: NearAiTransferCredentials,
        timeout: Duration,
    ) -> Result<Self, NearAiTransferError> {
        let loopback = reqwest::Url::parse(base)
            .is_ok_and(|url| url.scheme() == "http" && is_loopback_host(&url));
        if !loopback {
            return Err(NearAiTransferError::ClientConfig);
        }
        Self::build(base, credentials, timeout)
    }

    fn build(
        base: &str,
        credentials: NearAiTransferCredentials,
        timeout: Duration,
    ) -> Result<Self, NearAiTransferError> {
        if !base_is_tls_or_loopback(base)
            || !is_near_ai_org_credential(credentials.token.expose_secret())
        {
            return Err(NearAiTransferError::ClientConfig);
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(NEAR_AI_TRANSFER_USER_AGENT)
            .timeout(timeout)
            .build()
            .map_err(|_| NearAiTransferError::ClientConfig)?;
        let credential_hash = Self::credential_label(credentials.credential_id);
        Ok(Self {
            http,
            base: base.to_string(),
            credentials,
            credential_hash,
        })
    }

    /// What log lines carry for the credential: the hash of its id, or
    /// `unset` when no id is configured.
    pub fn credential_label(credential_id: Option<Uuid>) -> String {
        credential_id.map_or_else(|| "unset".to_string(), Self::credential_id_hash)
    }

    /// The `sha256:` of a credential id, the only form of it that is logged.
    pub fn credential_id_hash(credential_id: Uuid) -> String {
        format!(
            "sha256:{}",
            hex::encode(Sha256::digest(
                format!("nearai-payout-credential-v1:{credential_id}").as_bytes()
            ))
        )
    }

    /// The configured source.
    pub fn source(&self) -> NearAiCreditSource {
        self.credentials.source
    }

    fn url(&self, suffix: &str) -> String {
        format!(
            "{}/organizations/{}/credit/{suffix}",
            self.base, self.credentials.sender_org
        )
    }

    /// `POST .../credit/transfer` with `recipient_cumulative = target` on
    /// `program`. Memo empty (the field is left out), no `Idempotency-Key`
    /// (cumulative mode forbids one), and `metadata` exactly as given.
    pub async fn send_cumulative(
        &self,
        recipient: Uuid,
        program: &str,
        target: i64,
        metadata: Map<String, Value>,
    ) -> Result<CumulativeReply, NearAiTransferError> {
        let body = json!({
            "recipient_organization_id": recipient,
            "source_credit_type": self.credentials.source.as_str(),
            "recipient_cumulative": target,
            "payout_program": program,
            "metadata": metadata,
        });
        let request = self.http.post(self.url("transfer")).json(&body);
        let result = self.call(request).await.and_then(|(status, bytes)| {
            let reply: SendReply =
                serde_json::from_slice(&bytes).map_err(|_| NearAiTransferError::ReplyUnreadable)?;
            match (status, reply) {
                (
                    201,
                    SendReply {
                        id: Some(id),
                        amount,
                        recipient_cumulative: Some(total),
                        ..
                    },
                ) if amount > 0 && total >= amount => Ok(CumulativeReply::Committed {
                    id,
                    amount,
                    recipient_cumulative: total,
                }),
                (
                    200,
                    SendReply {
                        no_op: true,
                        amount: 0,
                        recipient_cumulative: Some(total),
                        ..
                    },
                ) if total >= 0 => Ok(CumulativeReply::NoOp {
                    recipient_cumulative: total,
                }),
                _ => Err(NearAiTransferError::ReplyUnexpected),
            }
        });
        self.log("send_cumulative", &result);
        result
    }

    /// `GET .../credit/transfer/capacity`, for the configured source.
    pub async fn capacity(&self) -> Result<NearAiCapacity, NearAiTransferError> {
        let request = self.http.get(self.url("transfer/capacity"));
        let result = self.call(request).await.and_then(|(status, bytes)| {
            if status != 200 {
                return Err(NearAiTransferError::ReplyUnexpected);
            }
            let reply: CapacityReply =
                serde_json::from_slice(&bytes).map_err(|_| NearAiTransferError::ReplyUnreadable)?;
            // #1271 lists every source, a disallowed one at 0, so a reply
            // without ours is not #1271.
            let per_send_max = reply
                .sources
                .iter()
                .find(|source| source.source_credit_type == self.credentials.source.as_str())
                .ok_or(NearAiTransferError::ReplyUnexpected)?
                .max_amount_per_transfer;
            if per_send_max < 0 || reply.sends_remaining_in_window < 0 {
                return Err(NearAiTransferError::ReplyUnexpected);
            }
            Ok(NearAiCapacity {
                sending_enabled: reply.sending_enabled,
                sender_reconciled: reply.sender_reconciled,
                credential_can_transfer: reply.credential_can_transfer,
                per_send_max,
                sends_remaining_in_window: reply.sends_remaining_in_window,
            })
        });
        self.log("capacity", &result);
        result
    }

    /// One page of `GET .../credit/transfers?direction=outgoing`, newest
    /// first, starting before `cursor`.
    pub async fn history_page(
        &self,
        cursor: Option<NearAiHistoryCursor>,
        limit: u8,
    ) -> Result<NearAiHistoryPage, NearAiTransferError> {
        let limit = limit.clamp(1, NEAR_AI_HISTORY_PAGE_LIMIT).to_string();
        let mut query: Vec<(&str, String)> =
            vec![("direction", "outgoing".to_string()), ("limit", limit)];
        if let Some(cursor) = cursor {
            query.push((
                "before_time",
                cursor
                    .before_time
                    .to_rfc3339_opts(SecondsFormat::AutoSi, true),
            ));
            query.push(("before_id", cursor.before_id.to_string()));
        }
        let request = self.http.get(self.url("transfers")).query(&query);
        let sender = self.credentials.sender_org;
        let result = self.call(request).await.and_then(|(status, bytes)| {
            if status != 200 {
                return Err(NearAiTransferError::ReplyUnexpected);
            }
            let reply: HistoryReply =
                serde_json::from_slice(&bytes).map_err(|_| NearAiTransferError::ReplyUnreadable)?;
            let mut transfers = Vec::with_capacity(reply.transfers.len());
            for record in reply.transfers {
                if record.amount < 0 || record.recipient_cumulative.is_some_and(|total| total < 0) {
                    return Err(NearAiTransferError::ReplyUnexpected);
                }
                // `direction=outgoing` limits the page to our sends. A record
                // from elsewhere is not #1271; dropping it would make a full
                // page look short and end a walk early, so refuse the page.
                if record.sender_organization_id != sender {
                    return Err(NearAiTransferError::ReplyUnexpected);
                }
                transfers.push(NearAiTransferRecord {
                    id: record.id,
                    sender_organization_id: record.sender_organization_id,
                    recipient_organization_id: record.recipient_organization_id,
                    payout_program: record.payout_program,
                    amount: record.amount,
                    recipient_cumulative: record.recipient_cumulative,
                });
            }
            Ok(NearAiHistoryPage {
                transfers,
                next_cursor: reply.next_cursor.map(|cursor| NearAiHistoryCursor {
                    before_time: cursor.before_time,
                    before_id: cursor.before_id,
                }),
            })
        });
        self.log("history_page", &result);
        result
    }

    /// Sends one request and returns a 2xx status with its body. Every
    /// other outcome becomes a closed [`NearAiTransferError`].
    async fn call(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<(u16, Vec<u8>), NearAiTransferError> {
        let response = request
            .bearer_auth(self.credentials.token.expose_secret())
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    NearAiTransferError::Timeout
                } else {
                    NearAiTransferError::Unreachable
                }
            })?;
        let status = response.status();
        if status.is_redirection() {
            return Err(NearAiTransferError::Redirect);
        }
        let bytes = read_bounded(response).await?;
        if status.is_success() {
            return Ok((status.as_u16(), bytes));
        }
        let code = serde_json::from_slice::<ErrorReply>(&bytes)
            .map(|reply| NearAiErrorCode::parse(&reply.error.kind))
            .unwrap_or(NearAiErrorCode::Other);
        Err(NearAiTransferError::Refused {
            status: status.as_u16(),
            code,
        })
    }

    fn log<T>(&self, call: &'static str, result: &Result<T, NearAiTransferError>) {
        match result {
            Ok(_) => tracing::info!(
                rail = "near_ai",
                call,
                credential = %self.credential_hash,
                outcome = "ok",
                "near_ai payout call"
            ),
            Err(NearAiTransferError::Refused { status, code }) => tracing::warn!(
                rail = "near_ai",
                call,
                credential = %self.credential_hash,
                outcome = "refused",
                status,
                code = code.as_label(),
                "near_ai payout call"
            ),
            Err(error) => tracing::warn!(
                rail = "near_ai",
                call,
                credential = %self.credential_hash,
                outcome = error.label(),
                "near_ai payout call"
            ),
        }
    }
}

async fn read_bounded(mut response: reqwest::Response) -> Result<Vec<u8>, NearAiTransferError> {
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len() + chunk.len() > MAX_REPLY_BYTES {
                    return Err(NearAiTransferError::ReplyUnreadable);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(bytes),
            Err(error) if error.is_timeout() => return Err(NearAiTransferError::Timeout),
            Err(_) => return Err(NearAiTransferError::Unreachable),
        }
    }
}

#[derive(Deserialize)]
struct SendReply {
    id: Option<Uuid>,
    amount: i64,
    recipient_cumulative: Option<i64>,
    #[serde(default)]
    no_op: bool,
}

#[derive(Deserialize)]
struct CapacitySource {
    source_credit_type: String,
    max_amount_per_transfer: i64,
}

#[derive(Deserialize)]
struct CapacityReply {
    sending_enabled: bool,
    sender_reconciled: bool,
    credential_can_transfer: bool,
    sends_remaining_in_window: i64,
    sources: Vec<CapacitySource>,
}

#[derive(Deserialize)]
struct HistoryCursorReply {
    before_time: DateTime<Utc>,
    before_id: Uuid,
}

#[derive(Deserialize)]
struct HistoryRecordReply {
    id: Uuid,
    sender_organization_id: Uuid,
    recipient_organization_id: Uuid,
    payout_program: Option<String>,
    amount: i64,
    recipient_cumulative: Option<i64>,
}

#[derive(Deserialize)]
struct HistoryReply {
    transfers: Vec<HistoryRecordReply>,
    next_cursor: Option<HistoryCursorReply>,
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct ErrorReply {
    error: ErrorBody,
}

#[cfg(test)]
mod tests {

    /// An unset credential id is logged as a fixed label, never a hash of
    /// nothing; a set one keeps its `sha256:` form.
    #[test]
    fn an_unset_credential_id_logs_as_unset() {
        assert_eq!(NearAiTransferClient::credential_label(None), "unset");
        let set = NearAiTransferClient::credential_label(Some(Uuid::from_u128(2)));
        assert!(set.starts_with("sha256:"), "{set}");
        assert_eq!(
            set,
            NearAiTransferClient::credential_id_hash(Uuid::from_u128(2))
        );
    }

    use super::*;

    fn credentials() -> NearAiTransferCredentials {
        NearAiTransferCredentials {
            token: SecretString::from(format!("oc_{}", "ab".repeat(32))),
            credential_id: Some(Uuid::from_u128(2)),
            sender_org: Uuid::from_u128(1),
            source: NearAiCreditSource::Grant,
        }
    }

    #[test]
    fn only_tls_or_loopback_bases_are_accepted() {
        for accepted in [
            "https://cloud-api.near.ai/v1",
            "http://127.0.0.1:8080/v1",
            "http://[::1]:9/v1",
        ] {
            assert!(base_is_tls_or_loopback(accepted), "{accepted}");
        }
        for refused in [
            "http://cloud-api.near.ai/v1",
            // A name goes through hosts and DNS; only an IP literal is
            // loopback by construction.
            "http://localhost:1/v1",
            "http://127.0.0.1@evil.example/v1",
            "https://user:pass@cloud-api.near.ai/v1",
            "http://10.0.0.1/v1",
            "https://cloud-api.near.ai/v1/",
            "https://cloud-api.near.ai/v1?x=1",
            "ftp://127.0.0.1/v1",
            "",
        ] {
            assert!(!base_is_tls_or_loopback(refused), "{refused}");
        }
    }

    #[test]
    fn the_test_constructor_takes_loopback_only() {
        let timeout = Duration::from_secs(1);
        assert!(
            NearAiTransferClient::at_loopback("http://127.0.0.1:1/v1", credentials(), timeout)
                .is_ok()
        );
        for refused in [
            "https://cloud-api.near.ai/v1",
            "https://127.0.0.1/v1",
            "http://example.invalid/v1",
            "http://localhost:1/v1",
            "http://127.0.0.1@example.invalid/v1",
        ] {
            assert_eq!(
                NearAiTransferClient::at_loopback(refused, credentials(), timeout).unwrap_err(),
                NearAiTransferError::ClientConfig,
                "{refused}"
            );
        }
        assert!(NearAiTransferClient::new(credentials()).is_ok());
    }

    #[test]
    fn every_source_parses_and_nothing_else_does() {
        for source in NearAiCreditSource::ALL {
            assert_eq!(NearAiCreditSource::parse(source.as_str()), Some(source));
        }
        assert_eq!(
            NearAiCreditSource::parse("payment"),
            Some(NearAiCreditSource::Payment)
        );
        assert_eq!(
            NearAiCreditSource::parse("staking_farm"),
            Some(NearAiCreditSource::StakingFarm)
        );
        for refused in ["", "Grant", "PAYMENT", "staking-farm", " grant", "transfer"] {
            assert_eq!(NearAiCreditSource::parse(refused), None, "{refused:?}");
        }
    }

    /// The capacity reply as staging serves it today, with the fields the
    /// rail does not read (`credits`, the constraints and budgets,
    /// `recipient_restrictions_apply`, `is_reservation`, and more) beside
    /// the ones it does. Synthetic values.
    #[test]
    fn a_full_capacity_reply_parses_and_picks_the_configured_source() {
        let body = serde_json::json!({
            "credits": [
                {"credit_type": "payment", "currency": "USD", "available": 5_000_000_000_i64},
                {"credit_type": "grant", "currency": "USD", "available": 0}
            ],
            "own_staking_earned_total": 0,
            "sender_reconciled": true,
            "sending_enabled": true,
            "credential_can_transfer": true,
            "organization_constraints": {"max_amount_per_transfer": null, "allowed_sources": ["payment", "staking_farm"]},
            "credential_constraints": {"max_amount_per_transfer": 1_000_000_000_i64},
            "organization_remaining_budget": 5_000_000_000_i64,
            "credential_remaining_budget": 5_000_000_000_i64,
            "sends_remaining_in_window": 60,
            "sources": [
                {"source_credit_type": "payment", "max_amount_per_transfer": 5_000_000_000_i64},
                {"source_credit_type": "staking_farm", "max_amount_per_transfer": 0},
                {"source_credit_type": "grant", "max_amount_per_transfer": 0}
            ],
            "recipient_restrictions_apply": true,
            "is_reservation": false,
            "a_field_added_later": {"nested": [1, 2, 3]}
        });
        let reply: CapacityReply = serde_json::from_value(body).unwrap();
        let max = |source: NearAiCreditSource| {
            reply
                .sources
                .iter()
                .find(|entry| entry.source_credit_type == source.as_str())
                .map(|entry| entry.max_amount_per_transfer)
        };
        assert_eq!(max(NearAiCreditSource::Payment), Some(5_000_000_000));
        assert_eq!(max(NearAiCreditSource::Grant), Some(0));
        assert_eq!(max(NearAiCreditSource::StakingFarm), Some(0));
        assert_eq!(reply.sends_remaining_in_window, 60);
    }

    #[test]
    fn a_malformed_credential_is_refused() {
        for token in [
            "",
            "oc_",
            "ab".repeat(33).as_str(),
            &format!("oc_{}", "g".repeat(64)),
        ] {
            let mut bad = credentials();
            bad.token = SecretString::from(token.to_string());
            assert_eq!(
                NearAiTransferClient::new(bad).unwrap_err(),
                NearAiTransferError::ClientConfig
            );
        }
        assert!(is_near_ai_org_credential(&format!(
            "oc_{}",
            "AB".repeat(32)
        )));
    }

    #[test]
    fn debug_prints_no_token_or_id() {
        let creds = credentials();
        let rendered = format!("{creds:?}");
        assert!(!rendered.contains("abab"), "{rendered}");
        assert!(
            !rendered.contains(&Uuid::from_u128(1).to_string()),
            "{rendered}"
        );
        assert!(
            !rendered.contains(&Uuid::from_u128(2).to_string()),
            "{rendered}"
        );
        let client = NearAiTransferClient::new(creds).unwrap();
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("abab"), "{rendered}");
        assert!(rendered.contains("sha256:"), "{rendered}");
    }

    #[test]
    fn error_labels_are_safe_identifiers() {
        for error in [
            NearAiTransferError::Refused {
                status: 400,
                code: NearAiErrorCode::Other,
            },
            NearAiTransferError::Redirect,
            NearAiTransferError::Timeout,
            NearAiTransferError::Unreachable,
            NearAiTransferError::ReplyUnreadable,
            NearAiTransferError::ReplyUnexpected,
            NearAiTransferError::ClientConfig,
        ] {
            let label = error.label();
            assert!(
                !label.is_empty()
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{error:?}"
            );
            assert_eq!(error.to_string(), error.label());
        }
    }
}

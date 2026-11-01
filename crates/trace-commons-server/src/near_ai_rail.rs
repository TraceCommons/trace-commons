// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The `near_ai` payout rail: [`PayoutRail`] over the #1271 transfer client
//! in cumulative mode (payout spec section 3, section 4 "The NEAR AI
//! client", "Program and link generation" and "`near_ai` refusal mapping").
//!
//! - `send` sends `recipient_cumulative = target_total` on the link's
//!   program and ignores `delta`: a repeated send for the same line finds
//!   the program at its target and answers `AlreadyAtOrAbove`.
//! - `read_total` is a `recipient_cumulative: 0` send, an exact read of the
//!   program total taken under cloud-api's org locks.
//! - `find_receipt` and `history` page outgoing history and filter on the
//!   client, because history has no program filter.
//! - `preflight` is `capacity`; `probe_recipient` is the zero send on the
//!   fixed program [`NEAR_AI_PROBE_PROGRAM`].
//!
//! Amounts are nano-USD (9 decimals). The wire is `i64`; a target above
//! `i64::MAX` is held as `payout_amount_out_of_range` before any call.
//! Every reply maps into a closed [`RailRefusal`]; the pass acts on the
//! variant only. A 401 or 403 on any call, read or send, is a fault in our
//! credential and stops the pass without touching an account.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use trace_commons_gate_api::pipeline::{
    AtomicUnits, InstrumentDescriptor, InstrumentId, InstrumentKind,
};
use trace_commons_gate_api::{
    PayoutRail, RailCapacity, RailDescriptor, RailHistoryEntry, RailId, RailLine, RailRefusal,
    ReceiptLookup, RecipientKind, RecipientRef, SendOutcome, async_trait,
};
use uuid::Uuid;

use crate::near_ai_transfer::{
    CumulativeReply, NEAR_AI_HISTORY_PAGE_LIMIT, NearAiErrorCode, NearAiHistoryCursor,
    NearAiTransferClient, NearAiTransferError, NearAiTransferRecord,
};

/// The rail's instrument: NEAR AI credit in nano-USD.
pub const NEAR_AI_INSTRUMENT_ID: &str = "near_ai_usd";
/// One atomic unit is one nano-USD.
pub const NEAR_AI_INSTRUMENT_DECIMALS: u8 = 9;
/// The fixed program of the link-time liveness probe. Outside the `tc1-`
/// namespace, so the drill never counts it; a zero target writes no row.
pub const NEAR_AI_PROBE_PROGRAM: &str = "tcprobe-v1";
/// The prefix of every link program.
pub const NEAR_AI_PROGRAM_PREFIX: &str = "tc1-";
/// The adapter label in hash-only reports.
pub const NEAR_AI_ADAPTER_IDENTITY: &str = "near_ai_transfer_cumulative_v1";
/// The most history pages one walk reads (100 records each). Past it the
/// pass stops rather than walk an unbounded history; spec section 12, open
/// item 4, asks NEAR AI for a program filter before that matters.
pub const NEAR_AI_MAX_HISTORY_PAGES: usize = 10_000;

/// The `near_ai` rail's descriptor: `near_ai_usd`, a credit account
/// instrument with 9 decimals, paid to a NEAR AI org, carried on an `i64`
/// wire.
pub fn near_ai_rail_descriptor() -> RailDescriptor {
    RailDescriptor {
        rail_id: RailId::near_ai(),
        instrument_id: InstrumentId::new(NEAR_AI_INSTRUMENT_ID)
            .expect("near_ai_usd is a safe instrument id"),
        instrument: InstrumentDescriptor {
            kind: InstrumentKind::CreditAccount,
            network: "near_ai".to_string(),
            contract: "usd".to_string(),
            decimals: NEAR_AI_INSTRUMENT_DECIMALS,
        },
        recipient_kind: RecipientKind::near_ai_org(),
        wire_max: Some(AtomicUnits::from_raw(i64::MAX as u128)),
    }
}

fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

/// A link's program: `tc1-` plus the first 32 hex of
/// `sha256("nearai-payout-program-v1:" || link_id)`. Opaque to the
/// recipient, 36 bytes, and new for every link generation.
pub fn near_ai_program_for_link(link_id: Uuid) -> String {
    let digest = sha256_hex(&format!("nearai-payout-program-v1:{link_id}"));
    format!("{NEAR_AI_PROGRAM_PREFIX}{}", &digest[..32])
}

/// The prefix of every link's referral program (referral rewards spec
/// section 5). Outside the `tc1-` namespace, so an earned-stream reading
/// never counts a referral record.
pub const NEAR_AI_REFERRAL_PROGRAM_PREFIX: &str = "tr1-";

/// A link's referral program: `tr1-` plus the first 32 hex of
/// `sha256("nearai-referral-program-v1:" || link_id)`. Like the earned
/// program it is opaque, 36 bytes, and new for every link generation, and
/// it is domain-separated from it, so the two never coincide.
pub fn near_ai_referral_program_for_link(link_id: Uuid) -> String {
    let digest = sha256_hex(&format!("nearai-referral-program-v1:{link_id}"));
    format!("{NEAR_AI_REFERRAL_PROGRAM_PREFIX}{}", &digest[..32])
}

/// The `metadata.tc` value of a send: `sha256:` of
/// `"nearai-payout-line-v1:" || line_ref_hash`. Metadata is visible to the
/// recipient, so it carries one domain-separated hash and nothing else.
pub fn near_ai_line_metadata_tag(line_ref_hash: &str) -> String {
    format!(
        "sha256:{}",
        sha256_hex(&format!("nearai-payout-line-v1:{line_ref_hash}"))
    )
}

/// A transfer's receipt hash: `sha256:` of `"nearai-transfer-v1:" || id`,
/// the id in its hyphenated lowercase form. `send`, `find_receipt` and
/// `history` all hash through here, so they agree.
pub fn near_ai_receipt_hash(transfer_id: Uuid) -> String {
    format!(
        "sha256:{}",
        sha256_hex(&format!("nearai-transfer-v1:{transfer_id}"))
    )
}

/// Which call a reply answers: the same status means different things on
/// a send to a recipient and on a read of our own org.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Call {
    /// A send, or a zero send (`read_total`) on a link's program.
    Send,
    /// The link-time zero send on [`NEAR_AI_PROBE_PROGRAM`].
    Probe,
    /// `capacity` or history: reads of our own org.
    Read,
}

/// The spec's refusal table, closed. Status first, then code; anything the
/// table does not name is never a guess: `Failed` on a send (the line's
/// request was refused for a reason we cannot place), `StopPass` on a read
/// or a probe, which have no line to fail.
fn refusal(call: Call, error: NearAiTransferError) -> RailRefusal {
    use NearAiErrorCode as Code;
    match error {
        NearAiTransferError::Timeout => RailRefusal::Retry("near_ai_timeout"),
        NearAiTransferError::Unreachable => RailRefusal::Retry("near_ai_unreachable"),
        // A 2xx we could not read may have moved money; the line stays
        // pending and recovery resolves it by reads.
        NearAiTransferError::ReplyUnreadable => RailRefusal::Retry("near_ai_reply_unreadable"),
        // `ReplyUnexpected` only ever follows a 2xx (the client turns every
        // other status into `Refused`). On a send a 2xx may have moved money
        // whatever its shape, so the line stays pending: the resend answers
        // `AlreadyAtOrAbove` and recovery finds the receipt. On a read or a
        // probe it says our model of #1271 is wrong: stop, change no row.
        NearAiTransferError::ReplyUnexpected => match call {
            Call::Send => RailRefusal::Retry("near_ai_reply_unexpected"),
            Call::Probe | Call::Read => RailRefusal::StopPass("near_ai_reply_unexpected"),
        },
        // The host is compiled in; a redirect from it is a fault in the
        // rail, not in a line. Stop and change no row.
        NearAiTransferError::Redirect => RailRefusal::StopPass("near_ai_redirect_refused"),
        NearAiTransferError::ClientConfig => RailRefusal::StopPass("near_ai_client_config_invalid"),
        NearAiTransferError::Refused { status, code } => match (status, code) {
            (401, _) => RailRefusal::StopPass("near_ai_credential_refused"),
            (403, Code::SenderNotEnabled) => RailRefusal::StopPass("near_ai_sender_not_enabled"),
            (403, _) => RailRefusal::StopPass("near_ai_credential_refused"),
            // Only `organization_unavailable` names an org. On a send or a
            // probe it is the recipient (#1271 raises it for the recipient
            // alone); on a read of our own org it is the sender. Any other
            // 404 -- a wrong route, a proxy page, a host without #1271 -- is
            // ours and must not mark a link.
            (404, Code::OrganizationUnavailable) if call == Call::Read => {
                RailRefusal::StopPass("near_ai_sender_unavailable")
            }
            (404, Code::OrganizationUnavailable) => RailRefusal::RecipientUnavailable,
            (404, _) => RailRefusal::StopPass("near_ai_route_unavailable"),
            (429, _) => RailRefusal::Retry("near_ai_transfer_rate_limited"),
            (503, Code::AccountingBusy) => RailRefusal::Retry("near_ai_accounting_busy"),
            (500..=599, _) => RailRefusal::Retry("near_ai_transfer_unavailable"),
            (409, Code::InsufficientPostedCapacity) => {
                RailRefusal::StopPass("near_ai_insufficient_posted_capacity")
            }
            (409, Code::SenderNotReconciled) => {
                RailRefusal::StopPass("near_ai_sender_not_reconciled")
            }
            // At the probe's zero target #1271 runs its policy checks
            // (source and `allowed_recipient_ids`, org then credential)
            // before the no-op return and checks no budget. The credential
            // code is the credential's closed recipient list (spec section
            // 2 puts the list there); the org code is our org's policy
            // refusing the source or the recipient, ours to fix.
            (409, Code::CredentialTransferLimitExceeded) if call == Call::Probe => {
                RailRefusal::HoldLine("near_ai_org_not_permitted")
            }
            (409, Code::OrganizationTransferLimitExceeded) if call == Call::Probe => {
                RailRefusal::StopPass("near_ai_organization_transfer_limit_exceeded")
            }
            // On a send (or the zero read, which is one) either code is a
            // limit on our own credential or org: its budget, its source,
            // or its recipient list, which the link probe already checked.
            // None of that is the recipient's standing, so it holds no
            // account: stop the rail and change no row, the pending line
            // included, and the next pass resends it once an operator has
            // raised the limit.
            (409, Code::CredentialTransferLimitExceeded) => {
                RailRefusal::StopPass("near_ai_credential_transfer_limit_exceeded")
            }
            (409, Code::OrganizationTransferLimitExceeded) => {
                RailRefusal::StopPass("near_ai_organization_transfer_limit_exceeded")
            }
            (409, Code::CapacityOverflow) => RailRefusal::HoldLine("near_ai_capacity_overflow"),
            (_, _) if call != Call::Send => RailRefusal::StopPass("near_ai_reply_unexpected"),
            (400, _) => RailRefusal::Failed("near_ai_request_refused"),
            _ => RailRefusal::Failed("near_ai_reply_unexpected"),
        },
    }
}

/// A recipient is an org UUID. Anything else is a link that will not open.
fn recipient_org(recipient: &RecipientRef) -> Result<Uuid, RailRefusal> {
    Uuid::parse_str(recipient.expose()).map_err(|_| RailRefusal::HoldLine("payout_link_unreadable"))
}

/// Narrows a target to the `i64` wire, or holds the line.
fn wire_target(target: AtomicUnits) -> Result<i64, RailRefusal> {
    i64::try_from(target.get()).map_err(|_| RailRefusal::HoldLine("payout_amount_out_of_range"))
}

fn units(value: i64) -> AtomicUnits {
    // Callers pass only values the client has checked to be non-negative.
    AtomicUnits::from_raw(u128::try_from(value).unwrap_or(0))
}

/// The `near_ai` rail.
pub struct NearAiRail {
    client: NearAiTransferClient,
    descriptor: RailDescriptor,
}

impl NearAiRail {
    pub fn new(client: NearAiTransferClient) -> Self {
        Self {
            client,
            descriptor: near_ai_rail_descriptor(),
        }
    }

    async fn zero_send(&self, call: Call, org: Uuid, program: &str) -> Result<i64, RailRefusal> {
        match self
            .client
            .send_cumulative(org, program, 0, Map::new())
            .await
            .map_err(|error| refusal(call, error))?
        {
            CumulativeReply::NoOp {
                recipient_cumulative,
            } => Ok(recipient_cumulative),
            // A zero target can never send; a 201 here is not #1271. Stop
            // and change no row: there is no line at fault.
            CumulativeReply::Committed { .. } => {
                Err(RailRefusal::StopPass("near_ai_reply_unexpected"))
            }
        }
    }

    /// Walks outgoing history newest first and returns the records to
    /// `org` on `program`, stopping early when `stop` matches one. Ends on
    /// an empty or short page even though a cursor is returned on the last
    /// page. The client refuses a page holding another sender's record, so
    /// a short page is short on the wire. #1271 orders by `(created_at,
    /// id)` descending and its cursor is the last row's, so each cursor
    /// must be strictly older than the one before; one that is not stops
    /// the pass rather than re-reading a page to the page cap.
    ///
    /// Every `find_receipt` and `history` call walks the org's whole
    /// outgoing history from the newest record (no program filter on the
    /// wire), so the drill costs links times history. Spec section 12,
    /// open item 4.
    async fn walk(
        &self,
        org: Uuid,
        program: &str,
        mut stop: impl FnMut(&NearAiTransferRecord) -> bool,
    ) -> Result<(Vec<NearAiTransferRecord>, bool), RailRefusal> {
        let mut cursor: Option<NearAiHistoryCursor> = None;
        let older = |next: &NearAiHistoryCursor, previous: &NearAiHistoryCursor| {
            (next.before_time, next.before_id) < (previous.before_time, previous.before_id)
        };
        let mut matched = Vec::new();
        for _ in 0..NEAR_AI_MAX_HISTORY_PAGES {
            let page = self
                .client
                .history_page(cursor, NEAR_AI_HISTORY_PAGE_LIMIT)
                .await
                .map_err(|error| refusal(Call::Read, error))?;
            let short = page.transfers.len() < usize::from(NEAR_AI_HISTORY_PAGE_LIMIT);
            for record in page.transfers {
                if record.recipient_organization_id == org
                    && record.payout_program.as_deref() == Some(program)
                {
                    if stop(&record) {
                        matched.push(record);
                        return Ok((matched, true));
                    }
                    matched.push(record);
                }
            }
            match page.next_cursor {
                Some(next) if !short => {
                    if cursor.is_some_and(|previous| !older(&next, &previous)) {
                        return Err(RailRefusal::StopPass("near_ai_history_cursor_stalled"));
                    }
                    cursor = Some(next);
                }
                _ => return Ok((matched, false)),
            }
        }
        Err(RailRefusal::StopPass("near_ai_history_too_long"))
    }
}

#[async_trait]
impl PayoutRail for NearAiRail {
    fn descriptor(&self) -> &RailDescriptor {
        &self.descriptor
    }

    fn adapter_identity(&self) -> &str {
        NEAR_AI_ADAPTER_IDENTITY
    }

    /// The real rail, outside a staging build. Readiness ANDs this with the
    /// rest of the rail's conditions (`live`, a rate epoch, the startup
    /// capacity probe, not halted).
    fn production_qualified(&self) -> bool {
        !crate::near_ai_api::near_ai_staging()
    }

    fn program_for_link(&self, link_id: Uuid) -> String {
        near_ai_program_for_link(link_id)
    }

    fn referral_program_for_link(&self, link_id: Uuid) -> Option<String> {
        Some(near_ai_referral_program_for_link(link_id))
    }

    /// `capacity`, settled into the generic shape. #1271 reports every
    /// source's maximum as 0 when no sends are left in the current minute,
    /// so a zero `per_send_max` beside `sends_remaining: Some(0)` means
    /// "wait for the window", not "nothing can be sent"; the pass waits on
    /// `sends_remaining`. A zero maximum with sends left (source not
    /// allowed, nothing available, or a budget spent) is not told apart
    /// from the reply; the pass sends nothing that round either way. A
    /// reply without the configured source is refused by the client.
    async fn preflight(&self) -> Result<RailCapacity, RailRefusal> {
        let capacity = self
            .client
            .capacity()
            .await
            .map_err(|error| refusal(Call::Read, error))?;
        let stop_label = if !capacity.sending_enabled {
            Some("near_ai_sending_disabled")
        } else if !capacity.sender_reconciled {
            Some("near_ai_sender_not_reconciled")
        } else if !capacity.credential_can_transfer {
            Some("near_ai_credential_cannot_transfer")
        } else {
            None
        };
        Ok(RailCapacity {
            sending_enabled: stop_label.is_none(),
            stop_label,
            per_send_max: units(capacity.per_send_max),
            sends_remaining: Some(
                u32::try_from(capacity.sends_remaining_in_window).unwrap_or(u32::MAX),
            ),
        })
    }

    async fn read_total(
        &self,
        recipient: &RecipientRef,
        program: &str,
    ) -> Result<AtomicUnits, RailRefusal> {
        let org = recipient_org(recipient)?;
        self.zero_send(Call::Send, org, program).await.map(units)
    }

    async fn send(&self, line: &RailLine<'_>) -> Result<SendOutcome, RailRefusal> {
        let org = recipient_org(line.recipient)?;
        let target = wire_target(line.target_total)?;
        let mut metadata = Map::new();
        metadata.insert(
            "tc".to_string(),
            Value::String(near_ai_line_metadata_tag(line.line_ref_hash)),
        );
        match self
            .client
            .send_cumulative(org, line.program, target, metadata)
            .await
            .map_err(|error| refusal(Call::Send, error))?
        {
            CumulativeReply::Committed {
                id,
                amount,
                recipient_cumulative,
            } => Ok(SendOutcome::Committed {
                receipt_hash: near_ai_receipt_hash(id),
                delta: units(amount),
                total_after: Some(units(recipient_cumulative)),
            }),
            CumulativeReply::NoOp {
                recipient_cumulative,
            } => Ok(SendOutcome::AlreadyAtOrAbove {
                total: units(recipient_cumulative),
            }),
        }
    }

    async fn find_receipt(
        &self,
        line: &RailLine<'_>,
        exclude: &[String],
    ) -> Result<ReceiptLookup, RailRefusal> {
        let org = recipient_org(line.recipient)?;
        let target = wire_target(line.target_total)?;
        let (matched, found) = self
            .walk(org, line.program, |record| {
                record.recipient_cumulative == Some(target)
                    && !exclude.contains(&near_ai_receipt_hash(record.id))
            })
            .await?;
        if !found {
            return Ok(ReceiptLookup::NotFound);
        }
        let record = matched.last().expect("a found walk ends on its match");
        Ok(ReceiptLookup::Found {
            receipt_hash: near_ai_receipt_hash(record.id),
            delta: units(record.amount),
        })
    }

    async fn history(
        &self,
        recipient: &RecipientRef,
        program: &str,
    ) -> Result<Vec<RailHistoryEntry>, RailRefusal> {
        let org = recipient_org(recipient)?;
        let (records, _) = self.walk(org, program, |_| false).await?;
        Ok(records
            .into_iter()
            .map(|record| RailHistoryEntry {
                receipt_hash: near_ai_receipt_hash(record.id),
                program: program.to_string(),
                delta: units(record.amount),
                total_after: record.recipient_cumulative.map(units),
            })
            .collect())
    }

    async fn probe_recipient(&self, recipient: &RecipientRef) -> Result<(), RailRefusal> {
        let org = recipient_org(recipient)?;
        self.zero_send(Call::Probe, org, NEAR_AI_PROBE_PROGRAM)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_is_derived_from_the_link_id_and_fits_100_bytes() {
        let link = Uuid::from_u128(0x1111_2222_3333_4444_5555_6666_7777_8888);
        let program = near_ai_program_for_link(link);
        let digest = hex::encode(Sha256::digest(
            format!("nearai-payout-program-v1:{link}").as_bytes(),
        ));
        assert_eq!(program, format!("tc1-{}", &digest[..32]));
        assert_eq!(program.len(), 36);
        assert!(program.len() <= 100);
        assert_eq!(
            program.trim(),
            program,
            "#1271 refuses surrounding whitespace"
        );
        assert!(!program.contains(&link.simple().to_string()[..8]));
        // Deterministic per link, distinct across links, never the probe.
        assert_eq!(program, near_ai_program_for_link(link));
        assert_ne!(program, near_ai_program_for_link(Uuid::from_u128(1)));
        assert_ne!(program, NEAR_AI_PROBE_PROGRAM);
        assert!(!NEAR_AI_PROBE_PROGRAM.starts_with(NEAR_AI_PROGRAM_PREFIX));
    }

    #[test]
    fn the_referral_program_is_tr1_plus_32_hex_and_never_the_earned_one() {
        let link = Uuid::from_u128(0x1111_2222_3333_4444_5555_6666_7777_8888);
        let program = near_ai_referral_program_for_link(link);
        let digest = hex::encode(Sha256::digest(
            format!("nearai-referral-program-v1:{link}").as_bytes(),
        ));
        assert_eq!(program, format!("tr1-{}", &digest[..32]));
        assert_eq!(program.len(), 36);
        assert!(program.starts_with(NEAR_AI_REFERRAL_PROGRAM_PREFIX));
        assert!(!program.starts_with(NEAR_AI_PROGRAM_PREFIX));
        // V142's link check: printable, no surrounding whitespace, at most
        // 100 bytes, and distinct from the link's earned program.
        assert!(
            program
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
        );
        assert_ne!(program, near_ai_program_for_link(link));
        assert_ne!(&program[4..], &near_ai_program_for_link(link)[4..]);
        assert_eq!(program, near_ai_referral_program_for_link(link));
        assert_ne!(
            program,
            near_ai_referral_program_for_link(Uuid::from_u128(1))
        );
        assert!(!program.contains(&link.simple().to_string()[..8]));
        assert_ne!(program, NEAR_AI_PROBE_PROGRAM);

        let client =
            NearAiTransferClient::new(crate::near_ai_transfer::NearAiTransferCredentials {
                token: secrecy::SecretString::from(format!("oc_{}", "ab".repeat(32))),
                credential_id: Some(Uuid::from_u128(2)),
                sender_org: Uuid::from_u128(1),
                source: crate::near_ai_transfer::NearAiCreditSource::Payment,
            })
            .unwrap();
        assert_eq!(
            NearAiRail::new(client).referral_program_for_link(link),
            Some(program)
        );
    }

    #[test]
    fn the_descriptor_is_nine_decimal_credit_on_an_i64_wire() {
        let descriptor = near_ai_rail_descriptor();
        descriptor.validate().unwrap();
        assert_eq!(descriptor.rail_id.as_str(), "near_ai");
        assert_ne!(descriptor.rail_id.as_str(), "near");
        assert_eq!(descriptor.instrument_id.as_str(), NEAR_AI_INSTRUMENT_ID);
        assert_eq!(descriptor.instrument.decimals, 9);
        assert_eq!(descriptor.instrument.kind, InstrumentKind::CreditAccount);
        assert_eq!(descriptor.recipient_kind.as_str(), "near_ai_org");
        assert_eq!(
            descriptor.wire_max,
            Some(AtomicUnits::from_raw(i64::MAX as u128))
        );
    }

    #[test]
    fn the_rail_is_qualified_only_outside_staging() {
        let client =
            NearAiTransferClient::new(crate::near_ai_transfer::NearAiTransferCredentials {
                token: secrecy::SecretString::from(format!("oc_{}", "ab".repeat(32))),
                credential_id: Some(Uuid::from_u128(2)),
                sender_org: Uuid::from_u128(1),
                source: crate::near_ai_transfer::NearAiCreditSource::Payment,
            })
            .unwrap();
        assert_eq!(
            NearAiRail::new(client).production_qualified(),
            !crate::near_ai_api::near_ai_staging()
        );
    }

    #[test]
    fn wire_narrowing_holds_above_i64() {
        assert_eq!(
            wire_target(AtomicUnits::from_raw(i64::MAX as u128)),
            Ok(i64::MAX)
        );
        assert_eq!(
            wire_target(AtomicUnits::from_raw(i64::MAX as u128 + 1)),
            Err(RailRefusal::HoldLine("payout_amount_out_of_range"))
        );
    }

    #[test]
    fn every_mapped_refusal_has_a_safe_label() {
        let codes = [
            NearAiErrorCode::TransferRateLimited,
            NearAiErrorCode::TransferUnavailable,
            NearAiErrorCode::AccountingBusy,
            NearAiErrorCode::InsufficientPostedCapacity,
            NearAiErrorCode::SenderNotReconciled,
            NearAiErrorCode::CredentialTransferLimitExceeded,
            NearAiErrorCode::OrganizationTransferLimitExceeded,
            NearAiErrorCode::CapacityOverflow,
            NearAiErrorCode::OrganizationUnavailable,
            NearAiErrorCode::CredentialRefused,
            NearAiErrorCode::SenderNotEnabled,
            NearAiErrorCode::Other,
        ];
        for call in [Call::Send, Call::Probe, Call::Read] {
            for status in [200, 302, 400, 401, 403, 404, 409, 418, 429, 500, 503] {
                for code in codes {
                    let mapped = refusal(call, NearAiTransferError::Refused { status, code });
                    assert!(mapped.label_is_safe(), "{call:?} {status} {code:?}");
                }
            }
            for error in [
                NearAiTransferError::Redirect,
                NearAiTransferError::Timeout,
                NearAiTransferError::Unreachable,
                NearAiTransferError::ReplyUnreadable,
                NearAiTransferError::ReplyUnexpected,
                NearAiTransferError::ClientConfig,
            ] {
                assert!(refusal(call, error).label_is_safe(), "{error:?}");
            }
        }
    }

    #[test]
    fn hashes_are_domain_separated() {
        let id = Uuid::from_u128(7);
        assert_ne!(
            near_ai_receipt_hash(id),
            near_ai_line_metadata_tag(&id.to_string())
        );
        assert!(near_ai_receipt_hash(id).starts_with("sha256:"));
        assert_eq!(near_ai_receipt_hash(id).len(), 7 + 64);
    }
}

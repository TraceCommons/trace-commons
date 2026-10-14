// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NEAR AI inference-credit rail against a fake of NEAR AI's admin API
//! that has no compare-and-swap, over real HTTP through the production
//! client. No network call leaves the loopback interface.

#[path = "support/fake_near_ai_admin.rs"]
mod fake;

use fake::{FakeState, Permission, Shared};
use secrecy::SecretString;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use trace_commons_server::near_ai_admin::HttpNearAiAdminApi;
use trace_commons_server::near_ai_inference_credit::{
    CHANGED_BY, NearAiInferenceCreditRail, history_row_ref, marker,
};
use trace_commons_server::settlement_rail::{
    PriorWrite, RailDelivery, RailErrorKind, RailVerification, RecipientRef, SettlementRail,
    delivery_hash,
};
use uuid::Uuid;

const UA: &str = "trace-commons-settlement/test";
const WRITE_KEY: &str = "adm_write";
const READ_KEY: &str = "adm_read";
const DOLLAR: u128 = 1_000_000_000;

struct Harness {
    state: Shared,
    base: String,
    org: Uuid,
}

async fn harness() -> Harness {
    let state: Shared = Arc::new(Mutex::new(FakeState::default()));
    let org = Uuid::new_v4();
    {
        let mut s = state.lock().unwrap();
        s.orgs.insert(org, true);
        s.keys
            .insert(WRITE_KEY.into(), (Permission::ReadWrite, Some(UA.into())));
        s.keys
            .insert(READ_KEY.into(), (Permission::ReadOnly, Some(UA.into())));
    }
    let base = fake::start(state.clone()).await;
    Harness { state, base, org }
}

impl Harness {
    fn rail_with(
        &self,
        key: &str,
        ua: &str,
        timeout: Duration,
        dry_run: bool,
    ) -> NearAiInferenceCreditRail {
        let api =
            HttpNearAiAdminApi::new(&self.base, SecretString::from(key.to_string()), ua, timeout)
                .unwrap();
        NearAiInferenceCreditRail::new(Arc::new(api), "grant", 100 * DOLLAR, dry_run)
    }

    fn rail(&self) -> NearAiInferenceCreditRail {
        self.rail_with(WRITE_KEY, UA, Duration::from_secs(5), false)
    }

    fn delivery(&self, units: u128, prior: Option<PriorWrite>) -> RailDelivery {
        RailDelivery {
            delivery_hash: delivery_hash(
                "near_ai_org_limits",
                "near_ai_credit_usd",
                Uuid::new_v4(),
            ),
            recipient: RecipientRef::new(self.org.to_string()),
            units,
            expected_prior: prior,
        }
    }

    fn active_amount(&self) -> Option<i64> {
        self.state
            .lock()
            .unwrap()
            .active(self.org, "grant")
            .map(|r| r.amount)
    }

    fn patch_calls(&self) -> u64 {
        self.state.lock().unwrap().patch_calls
    }
}

fn prior_of(outcome: &trace_commons_server::settlement_rail::RailOutcome) -> PriorWrite {
    PriorWrite {
        external_receipt_hash: outcome.receipt.external_receipt_hash.clone(),
        written_units: outcome.receipt.written_units,
    }
}

#[tokio::test]
async fn first_and_second_deliveries_add_to_the_absolute_ceiling() {
    let h = harness().await;
    let rail = h.rail();
    let first = rail.deliver(&h.delivery(5 * DOLLAR, None)).await.unwrap();
    assert_eq!(first.anomaly, None);
    assert_eq!(first.receipt.prior_units, 0);
    assert_eq!(first.receipt.written_units, 5 * DOLLAR);
    assert!(!first.receipt.replayed);
    assert_eq!(h.active_amount(), Some(5 * DOLLAR as i64));

    let second = rail
        .deliver(&h.delivery(3 * DOLLAR, Some(prior_of(&first))))
        .await
        .unwrap();
    assert_eq!(second.anomaly, None);
    assert_eq!(second.receipt.prior_units, 5 * DOLLAR);
    assert_eq!(second.receipt.written_units, 8 * DOLLAR);
    assert_eq!(h.active_amount(), Some(8 * DOLLAR as i64));

    let row = h.state.lock().unwrap().active(h.org, "grant").unwrap();
    assert_eq!(row.changed_by.as_deref(), Some(CHANGED_BY));
    assert_eq!(
        history_row_ref(&row.id.to_string()),
        second.receipt.external_receipt_hash
    );
    // The marker is a hash; no tenant id, outbox id or org id reaches NEAR AI
    // in the audit field.
    let reason = row.change_reason.unwrap();
    assert!(reason.starts_with("trace-commons:settle:v1:sha256:"));
    assert!(!reason.contains(&h.org.to_string()));
}

#[tokio::test]
async fn a_write_that_landed_behind_a_timeout_is_found_and_never_repeated() {
    let h = harness().await;
    let rail = h.rail_with(WRITE_KEY, UA, Duration::from_millis(1000), false);
    let delivery = h.delivery(7 * DOLLAR, None);
    h.state.lock().unwrap().stall_after_commit = Some(Duration::from_secs(4));
    let err = rail.deliver(&delivery).await.unwrap_err();
    assert_eq!(err.kind, RailErrorKind::Retry);
    assert_eq!(err.label, "near_ai_admin_ambiguous");
    assert_eq!(h.patch_calls(), 1);

    let again = rail.deliver(&delivery).await.unwrap();
    assert!(again.receipt.replayed);
    assert_eq!(again.anomaly, None);
    assert_eq!(again.receipt.written_units, 7 * DOLLAR);
    assert_eq!(h.patch_calls(), 1, "the retry must not PATCH again");
    assert_eq!(h.active_amount(), Some(7 * DOLLAR as i64));
    assert_eq!(
        h.state
            .lock()
            .unwrap()
            .rows_with_reason(h.org, &marker(&delivery.delivery_hash)),
        1
    );
}

#[tokio::test]
async fn a_pre_existing_foreign_grant_freezes_before_any_write() {
    let h = harness().await;
    h.state.lock().unwrap().write(
        h.org,
        "grant",
        Some("nearai"),
        (2 * DOLLAR) as i64,
        Some("ops"),
        Some("welcome"),
    );
    let err = h
        .rail()
        .deliver(&h.delivery(DOLLAR, None))
        .await
        .unwrap_err();
    assert_eq!(err.kind, RailErrorKind::Frozen);
    assert_eq!(err.label, "near_ai_grant_bucket_foreign_baseline");
    assert_eq!(h.patch_calls(), 0);
    assert_eq!(h.active_amount(), Some((2 * DOLLAR) as i64));
}

#[tokio::test]
async fn a_foreign_write_after_ours_replaces_it_and_is_detected() {
    let h = harness().await;
    let rail = h.rail();
    let d1 = h.delivery(4 * DOLLAR, None);
    let first = rail.deliver(&d1).await.unwrap();
    // NEAR AI replaces per credit type: a different `source` does not isolate.
    h.state.lock().unwrap().write(
        h.org,
        "grant",
        Some("hackathon"),
        DOLLAR as i64,
        Some("ops"),
        None,
    );
    assert_eq!(h.active_amount(), Some(DOLLAR as i64));

    let verify = rail.verify(&d1, &first.receipt).await.unwrap();
    assert_eq!(verify, RailVerification::Overwritten);

    let err = rail
        .deliver(&h.delivery(DOLLAR, Some(prior_of(&first))))
        .await
        .unwrap_err();
    assert_eq!(err.kind, RailErrorKind::Frozen);
    assert_eq!(err.label, "near_ai_grant_bucket_foreign_write");
    assert_eq!(h.patch_calls(), 1);
}

#[tokio::test]
async fn a_write_interleaved_between_our_read_and_our_patch_is_flagged() {
    let h = harness().await;
    let rail = h.rail();
    h.state.lock().unwrap().foreign_write_before_patch = Some((9 * DOLLAR) as i64);
    let outcome = rail.deliver(&h.delivery(2 * DOLLAR, None)).await.unwrap();
    // Our absolute write clobbered theirs; the rail cannot prevent that
    // without CAS, but it must notice.
    assert_eq!(outcome.anomaly, Some("near_ai_grant_interleaved_write"));
    assert_eq!(outcome.receipt.prior_units, 9 * DOLLAR);
    assert_eq!(h.active_amount(), Some((2 * DOLLAR) as i64));
}

#[tokio::test]
async fn verification_distinguishes_current_superseded_and_missing() {
    let h = harness().await;
    let rail = h.rail();
    let d1 = h.delivery(DOLLAR, None);
    let first = rail.deliver(&d1).await.unwrap();
    assert_eq!(
        rail.verify(&d1, &first.receipt).await.unwrap(),
        RailVerification::Current
    );
    let d2 = h.delivery(DOLLAR, Some(prior_of(&first)));
    let second = rail.deliver(&d2).await.unwrap();
    assert_eq!(
        rail.verify(&d1, &first.receipt).await.unwrap(),
        RailVerification::Superseded
    );
    assert_eq!(
        rail.verify(&d2, &second.receipt).await.unwrap(),
        RailVerification::Current
    );
    let mut forged = first.receipt.clone();
    forged.written_units += 1;
    assert_eq!(
        rail.verify(&d1, &forged).await.unwrap(),
        RailVerification::Missing
    );
}

#[tokio::test]
async fn a_read_only_key_cannot_write_and_freezes() {
    let h = harness().await;
    let rail = h.rail_with(READ_KEY, UA, Duration::from_secs(5), false);
    let err = rail.deliver(&h.delivery(DOLLAR, None)).await.unwrap_err();
    assert_eq!(err.kind, RailErrorKind::Frozen);
    assert_eq!(err.label, "near_ai_admin_read_only_key");
    assert_eq!(h.active_amount(), None);
}

#[tokio::test]
async fn a_key_used_from_another_user_agent_is_unauthorized_and_retried() {
    let h = harness().await;
    let rail = h.rail_with(WRITE_KEY, "Mozilla/5.0", Duration::from_secs(5), false);
    let err = rail.deliver(&h.delivery(DOLLAR, None)).await.unwrap_err();
    assert_eq!(err.kind, RailErrorKind::Retry);
    assert_eq!(err.label, "near_ai_admin_unauthorized");
    assert_eq!(h.patch_calls(), 0);
}

#[tokio::test]
async fn dry_run_reads_and_checks_but_never_writes() {
    let h = harness().await;
    let rail = h.rail_with(READ_KEY, UA, Duration::from_secs(5), true);
    let err = rail.deliver(&h.delivery(DOLLAR, None)).await.unwrap_err();
    assert_eq!(err.kind, RailErrorKind::DryRun);
    assert_eq!(h.patch_calls(), 0);
}

#[tokio::test]
async fn an_inactive_org_freezes_and_a_transient_failure_retries() {
    let h = harness().await;
    h.state.lock().unwrap().orgs.insert(h.org, false);
    let err = h
        .rail()
        .deliver(&h.delivery(DOLLAR, None))
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind, err.label),
        (RailErrorKind::Frozen, "near_ai_org_not_found")
    );

    h.state.lock().unwrap().orgs.insert(h.org, true);
    h.state.lock().unwrap().unavailable = 1;
    let err = h
        .rail()
        .deliver(&h.delivery(DOLLAR, None))
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind, err.label),
        (RailErrorKind::Retry, "near_ai_admin_unavailable")
    );
}

#[tokio::test]
async fn a_delivery_above_the_ceiling_or_zero_never_reaches_near_ai() {
    let h = harness().await;
    let err = h
        .rail()
        .deliver(&h.delivery(101 * DOLLAR, None))
        .await
        .unwrap_err();
    assert_eq!(err.label, "near_ai_delivery_above_ceiling");
    let err = h.rail().deliver(&h.delivery(0, None)).await.unwrap_err();
    assert_eq!(err.label, "near_ai_delivery_zero");
    assert_eq!(h.patch_calls(), 0);
}

#[tokio::test]
async fn history_is_read_across_pages() {
    let h = harness().await;
    let rail = h.rail();
    let mut prior = None;
    let mut first = None;
    // 120 deliveries span two history pages of 100.
    for _ in 0..120 {
        let d = h.delivery(1, prior.clone());
        let out = rail.deliver(&d).await.unwrap();
        assert_eq!(out.anomaly, None);
        prior = Some(prior_of(&out));
        first.get_or_insert(d);
    }
    assert_eq!(h.active_amount(), Some(120));
    // Re-delivering the oldest finds its marker on the second page.
    let replay = rail.deliver(&first.unwrap()).await.unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.receipt.written_units, 1);
    assert_eq!(replay.anomaly, None);
    assert_eq!(h.patch_calls(), 120);
    let state = h.state.lock().unwrap();
    assert_eq!(state.rows.get(&h.org).unwrap().len(), 120);
}

#[tokio::test]
async fn a_write_that_committed_behind_a_bad_gateway_is_not_repeated() {
    let h = harness().await;
    let rail = h.rail();
    let delivery = h.delivery(2 * DOLLAR, None);
    h.state.lock().unwrap().bad_gateway_after_commit = true;
    let err = rail.deliver(&delivery).await.unwrap_err();
    assert_eq!(
        (err.kind, err.label),
        (RailErrorKind::Retry, "near_ai_admin_unavailable")
    );
    let again = rail.deliver(&delivery).await.unwrap();
    assert!(again.receipt.replayed);
    assert_eq!(h.patch_calls(), 1);
    assert_eq!(h.active_amount(), Some((2 * DOLLAR) as i64));
}

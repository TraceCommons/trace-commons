// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NEAR AI inference-credit rails against a fake of the proposed
//! monotonic `trace-commons/credit` admin route, over real HTTP through the
//! production client. No network call leaves the loopback interface.

#[path = "support/fake_near_ai_admin.rs"]
mod fake;

use fake::{FakeState, Permission, Shared};
use secrecy::SecretString;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use trace_commons_server::near_ai_admin::{HttpNearAiAdminApi, NearAiOrgId};
use trace_commons_server::near_ai_inference_credit::{
    PullTraceCommonsCreditRail, PushTraceCommonsCreditRail, RAIL_ID,
};
use trace_commons_server::settlement_rail::{
    CumulativeStatement, Published, RailErrorKind, RecipientRef, SettlementRail, statement_ref,
};
use uuid::Uuid;

const UA: &str = "trace-commons-settlement/test";
const WRITE_KEY: &str = "adm_write";
const READ_KEY: &str = "adm_read";
const DOLLAR: u128 = 1_000_000_000;
const CEILING: u128 = 100 * DOLLAR;

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
    fn api(&self, key: &str, ua: &str, timeout: Duration) -> Arc<HttpNearAiAdminApi> {
        Arc::new(
            HttpNearAiAdminApi::new(&self.base, SecretString::from(key.to_string()), ua, timeout)
                .unwrap(),
        )
    }

    fn push(&self) -> PushTraceCommonsCreditRail {
        PushTraceCommonsCreditRail::new(
            self.api(WRITE_KEY, UA, Duration::from_secs(5)),
            CEILING,
            false,
        )
    }

    fn statement(&self, cumulative: u128) -> CumulativeStatement {
        let org_ref = NearAiOrgId::from_uuid(self.org).ref_hash();
        CumulativeStatement {
            recipient: RecipientRef::new(self.org.to_string()),
            statement_ref: statement_ref(RAIL_ID, &org_ref, cumulative),
            recipient_ref_hash: org_ref,
            cumulative_units: cumulative,
        }
    }

    fn current(&self) -> i64 {
        self.state.lock().unwrap().current(self.org)
    }

    fn puts(&self) -> u64 {
        self.state.lock().unwrap().put_calls
    }
}

#[tokio::test]
async fn publishing_cumulative_totals_raises_and_never_lowers() {
    let h = harness().await;
    let rail = h.push();
    for (total, applied, effective) in [
        (5 * DOLLAR, true, 5 * DOLLAR),
        // A repeat is a no-op.
        (5 * DOLLAR, false, 5 * DOLLAR),
        (8 * DOLLAR, true, 8 * DOLLAR),
        // A late, smaller total changes nothing.
        (6 * DOLLAR, false, 8 * DOLLAR),
    ] {
        assert_eq!(
            rail.publish(&h.statement(total)).await.unwrap(),
            Published::Recorded {
                effective_units: effective,
                applied
            },
            "{total}"
        );
    }
    assert_eq!(h.current(), (8 * DOLLAR) as i64);
    let raises = h.state.lock().unwrap().raises.clone();
    assert_eq!(raises.len(), 2);
    // The audit reference is a hash; no org id, tenant id or amount in clear.
    assert!(raises[1].2.starts_with("sha256:"));
    assert!(!raises[1].2.contains(&h.org.to_string()));
    assert_eq!(
        rail.observe(&RecipientRef::new(h.org.to_string()))
            .await
            .unwrap(),
        Some(8 * DOLLAR)
    );
}

#[tokio::test]
async fn a_publish_that_committed_behind_a_failure_is_simply_repeated() {
    let h = harness().await;
    let rail = h.push();
    h.state.lock().unwrap().bad_gateway_after_commit = true;
    let err = rail.publish(&h.statement(2 * DOLLAR)).await.unwrap_err();
    assert_eq!(
        (err.kind, err.label),
        (RailErrorKind::Retry, "near_ai_admin_unavailable")
    );
    let again = rail.publish(&h.statement(2 * DOLLAR)).await.unwrap();
    assert_eq!(
        again,
        Published::Recorded {
            effective_units: 2 * DOLLAR,
            applied: false
        }
    );
    assert_eq!(h.current(), (2 * DOLLAR) as i64);
    assert_eq!(h.state.lock().unwrap().raises.len(), 1, "credited once");
}

#[tokio::test]
async fn a_publish_that_timed_out_after_commit_is_ambiguous_and_safe_to_repeat() {
    let h = harness().await;
    let rail = PushTraceCommonsCreditRail::new(
        h.api(WRITE_KEY, UA, Duration::from_millis(1000)),
        CEILING,
        false,
    );
    h.state.lock().unwrap().stall_after_commit = Some(Duration::from_secs(4));
    let err = rail.publish(&h.statement(DOLLAR)).await.unwrap_err();
    assert_eq!(
        (err.kind, err.label),
        (RailErrorKind::Retry, "near_ai_admin_ambiguous")
    );
    rail.publish(&h.statement(DOLLAR)).await.unwrap();
    assert_eq!(h.state.lock().unwrap().raises.len(), 1);
}

#[tokio::test]
async fn a_read_only_key_cannot_publish_and_freezes() {
    let h = harness().await;
    let rail = PushTraceCommonsCreditRail::new(
        h.api(READ_KEY, UA, Duration::from_secs(5)),
        CEILING,
        false,
    );
    let err = rail.publish(&h.statement(DOLLAR)).await.unwrap_err();
    assert_eq!(
        (err.kind, err.label),
        (RailErrorKind::Frozen, "near_ai_admin_read_only_key")
    );
    assert_eq!(h.current(), 0);
}

#[tokio::test]
async fn a_key_used_from_another_user_agent_is_unauthorized_and_retried() {
    let h = harness().await;
    let rail = PushTraceCommonsCreditRail::new(
        h.api(WRITE_KEY, "Mozilla/5.0", Duration::from_secs(5)),
        CEILING,
        false,
    );
    let err = rail.publish(&h.statement(DOLLAR)).await.unwrap_err();
    assert_eq!(
        (err.kind, err.label),
        (RailErrorKind::Retry, "near_ai_admin_unauthorized")
    );
    assert_eq!(h.puts(), 0);
}

#[tokio::test]
async fn dry_run_reads_but_never_writes() {
    let h = harness().await;
    let rail =
        PushTraceCommonsCreditRail::new(h.api(READ_KEY, UA, Duration::from_secs(5)), CEILING, true);
    let err = rail.publish(&h.statement(DOLLAR)).await.unwrap_err();
    assert_eq!(err.kind, RailErrorKind::DryRun);
    assert_eq!(h.puts(), 0);
}

#[tokio::test]
async fn an_inactive_org_freezes_and_a_transient_failure_retries() {
    let h = harness().await;
    h.state.lock().unwrap().orgs.insert(h.org, false);
    let err = h.push().publish(&h.statement(DOLLAR)).await.unwrap_err();
    assert_eq!(
        (err.kind, err.label),
        (RailErrorKind::Frozen, "near_ai_org_not_found")
    );
    h.state.lock().unwrap().orgs.insert(h.org, true);
    h.state.lock().unwrap().unavailable = 1;
    let err = h.push().publish(&h.statement(DOLLAR)).await.unwrap_err();
    assert_eq!(
        (err.kind, err.label),
        (RailErrorKind::Retry, "near_ai_admin_unavailable")
    );
}

#[tokio::test]
async fn a_total_above_the_ceiling_never_reaches_near_ai() {
    let h = harness().await;
    let err = h
        .push()
        .publish(&h.statement(CEILING + 1))
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind, err.label),
        (RailErrorKind::Frozen, "near_ai_cumulative_above_ceiling")
    );
    assert_eq!(h.puts(), 0);
}

#[tokio::test]
async fn pull_defers_without_any_admin_call_and_observes_only_with_a_read_key() {
    let h = harness().await;
    let blind = PullTraceCommonsCreditRail::new(None, CEILING);
    assert_eq!(
        blind.publish(&h.statement(DOLLAR)).await.unwrap(),
        Published::Deferred
    );
    assert_eq!(
        blind
            .observe(&RecipientRef::new(h.org.to_string()))
            .await
            .unwrap(),
        None
    );
    let observing =
        PullTraceCommonsCreditRail::new(Some(h.api(READ_KEY, UA, Duration::from_secs(5))), CEILING);
    assert_eq!(
        observing
            .observe(&RecipientRef::new(h.org.to_string()))
            .await
            .unwrap(),
        Some(0)
    );
    assert_eq!(h.puts(), 0);
    assert_eq!(
        blind
            .publish(&h.statement(CEILING + 1))
            .await
            .unwrap_err()
            .label,
        "near_ai_cumulative_above_ceiling"
    );
}

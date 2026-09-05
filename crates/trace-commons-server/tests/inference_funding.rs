// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::{Value, json};
use trace_commons_server::inference_funding::{
    BalanceDecodeError, MAX_BALANCE_BYTES, ObservationFreshness, OrganizationRemaining,
    UsdNanoDollars, decode_organization_balance,
};
use uuid::Uuid;

fn org() -> Uuid {
    Uuid::from_u128(1)
}

fn observed_at() -> DateTime<Utc> {
    "2026-09-04T12:00:00Z".parse().unwrap()
}

// Synthetic contract fixture, never an account response or credential.
fn fixture() -> Value {
    json!({
        "organization_id": org(),
        "total_spent": 1000000001_i64,
        "total_spent_display": "not used for arithmetic",
        "spend_limit": 2000000000_i64,
        "remaining": 999999999_i64,
        "updated_at": "2026-09-01T10:00:00Z",
        "credit_limits": [{"type": "payment", "amount": 2000000000_i64, "currency": "USD"}],
        "total_requests": 2, "total_tokens": 10
    })
}

fn decode(
    value: Value,
) -> Result<
    trace_commons_server::inference_funding::OrganizationBalanceObservation,
    BalanceDecodeError,
> {
    decode_organization_balance(&serde_json::to_vec(&value).unwrap(), org(), observed_at())
}

#[test]
fn preserves_provider_integer_precision_without_display_arithmetic() {
    let result = decode(fixture()).unwrap();
    assert_eq!(result.total_spent, UsdNanoDollars(1000000001));
    assert_eq!(
        result.remaining,
        OrganizationRemaining::Reported {
            spend_limit: UsdNanoDollars(2000000000),
            remaining: UsdNanoDollars(999999999),
        }
    );
    let mut large = fixture();
    large["total_spent"] = json!(9007199254740993_i64);
    large["spend_limit"] = json!(9007199254740994_i64);
    large["remaining"] = json!(1);
    assert_eq!(
        decode(large).unwrap().total_spent,
        UsdNanoDollars(9007199254740993)
    );
}

#[test]
fn unknown_zero_and_negative_are_distinct() {
    let mut missing = fixture();
    missing.as_object_mut().unwrap().remove("spend_limit");
    missing.as_object_mut().unwrap().remove("remaining");
    assert_eq!(
        decode(missing).unwrap().remaining,
        OrganizationRemaining::Unspecified
    );
    let mut nulls = fixture();
    nulls["spend_limit"] = Value::Null;
    nulls["remaining"] = Value::Null;
    assert_eq!(
        decode(nulls).unwrap().remaining,
        OrganizationRemaining::Unspecified
    );
    for remaining in [0, -1] {
        let mut value = fixture();
        value["spend_limit"] = json!(1000000001_i64 + remaining);
        value["remaining"] = json!(remaining);
        assert!(matches!(decode(value).unwrap().remaining,
            OrganizationRemaining::Reported { remaining: UsdNanoDollars(actual), .. } if actual == remaining));
    }
}

#[test]
fn rejects_wrong_organization_and_keeps_errors_identity_free() {
    let mut value = fixture();
    value["organization_id"] = json!(Uuid::from_u128(2));
    let error = decode(value).unwrap_err();
    assert_eq!(error, BalanceDecodeError::OrganizationMismatch);
    assert_eq!(error.to_string(), "inference_balance_organization_mismatch");
    assert!(!format!("{error:?}").contains(&Uuid::from_u128(2).to_string()));
}

#[test]
fn rejects_currency_ambiguity_and_inconsistent_or_overflowing_snapshots() {
    let mut currency = fixture();
    currency["credit_limits"][0]["currency"] = json!("EUR");
    assert_eq!(
        decode(currency),
        Err(BalanceDecodeError::CurrencyUnsupported)
    );
    for (limit, remaining) in [
        (Value::Null, json!(1)),
        (json!(2), Value::Null),
        (json!(2), json!(5)),
        (json!(i64::MIN), json!(0)),
    ] {
        let mut value = fixture();
        value["spend_limit"] = limit;
        value["remaining"] = remaining;
        assert_eq!(decode(value), Err(BalanceDecodeError::SnapshotInconsistent));
    }
}

#[test]
fn malformed_numbers_timestamps_and_duplicate_fields_fail_without_raw_errors() {
    for (field, value) in [
        ("remaining", json!(0.5)),
        ("total_spent", json!("1")),
        ("updated_at", json!("secret-provider-error")),
        ("credit_limits", Value::Null),
    ] {
        let mut body = fixture();
        body[field] = value;
        assert_eq!(decode(body), Err(BalanceDecodeError::Malformed));
    }
    let mut missing = fixture();
    missing.as_object_mut().unwrap().remove("total_spent");
    assert_eq!(decode(missing), Err(BalanceDecodeError::Malformed));
    let body = serde_json::to_string(&fixture())
        .unwrap()
        .replacen('{', "{\"total_spent\":1,", 1);
    assert_eq!(
        decode_organization_balance(body.as_bytes(), org(), observed_at()),
        Err(BalanceDecodeError::Malformed)
    );
    assert_eq!(
        decode_organization_balance(&vec![b' '; MAX_BALANCE_BYTES + 1], org(), observed_at()),
        Err(BalanceDecodeError::ResponseTooLarge)
    );
}

#[test]
fn freshness_tracks_local_observation_and_does_not_mistake_idle_usage_for_staleness() {
    let result = decode(fixture()).unwrap();
    let ttl = TimeDelta::minutes(5);
    assert_eq!(
        result.freshness(observed_at(), ttl),
        Ok(ObservationFreshness::Fresh)
    );
    assert_eq!(
        result.freshness(observed_at() + ttl, ttl),
        Ok(ObservationFreshness::Fresh)
    );
    assert_eq!(
        result.freshness(observed_at() + ttl + TimeDelta::seconds(1), ttl),
        Ok(ObservationFreshness::Stale)
    );
    assert_eq!(
        result.freshness(observed_at() - TimeDelta::seconds(1), ttl),
        Err(BalanceDecodeError::InvalidObservationTime)
    );
    assert_eq!(
        result.freshness(observed_at(), TimeDelta::seconds(-1)),
        Err(BalanceDecodeError::InvalidObservationTime)
    );
}

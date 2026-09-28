//! What leaves this machine, to whom, and what this client checked: the
//! facts behind the disclosure screens (K11 of the connect-and-forget
//! consent design, "The disclosure").
//!
//! # Facts only
//!
//! This module decides nothing a contributor reads. It reports what the
//! configuration in force says -- which route a session takes, the witness
//! and its pins, where that witness came from, which privacy filter this
//! machine attaches, and whether inference receipts are fetched -- and
//! `consent_copy::route_disclosure_copy` turns that into sentences. A shell
//! renders both and branches on neither.
//!
//! # What this client does not know, and so does not report
//!
//! - **The witness's classifier.** Which privacy filter the witness calls is
//!   part of the configuration its measurement covers. This client compares
//!   the measurement with its pins; it does not read that configuration, and
//!   it never sees the classifier's attestation. There is no field for it,
//!   rather than a guessed one.
//! - **Whether a receipt signer is one the witness pins.** That is the
//!   witness's configuration too.
//!
//! The copy says so in words; this struct has no field that could imply
//! otherwise.

use serde::{Deserialize, Serialize};

use crate::config::{ContributorConfig, WitnessOriginView};
use crate::witness::status::{WitnessTrustState, witness_status};

/// Which way a session leaves this machine, if at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// A pinned witness: each session is sent, unredacted, to the witness,
    /// which redacts it inside its enclave.
    Witness,
    /// A witness is configured and refusing (nothing pinned, or a pin that
    /// does not parse): no session is sent at all.
    WitnessRefusing,
    /// No witness: the session is redacted on this machine, and the
    /// unredacted session does not leave it.
    Local,
    /// This device is not enrolled.
    NotEnrolled,
    /// The configuration could not be read, so nothing is sent.
    SettingsUnreadable,
}

/// The privacy filter this machine attaches on the local route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalFilter {
    /// None: only the fixed patterns run.
    None,
    /// NEAR AI's hosted privacy filter, from `pii_filter = "near-ai"` or the
    /// environment. The text the patterns leave is sent to it.
    NearAi,
    /// A privacy-filter endpoint named in the environment.
    SelfHosted,
    /// A local program named in the environment.
    Sidecar,
    /// The filter setting is not one this client can build, so the
    /// redactor refuses and nothing is sent.
    Invalid,
}

/// The configured witness, as a disclosure screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessFacts {
    /// The same state the witness settings card shows.
    pub state: WitnessTrustState,
    pub url: String,
    pub signing_address: String,
    /// Verbatim, in stored order, as `witness_status` returns them.
    pub pinned_measurements: Vec<String>,
    /// Where it came from; `not_recorded` when this client cannot say.
    pub origin: WitnessOriginView,
}

/// Inference receipts, which only a witnessed submission fetches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptFacts {
    /// `inference_receipt_endpoint` is set.
    pub endpoint_configured: bool,
    /// `inference_receipt_check_attestation` is on.
    pub check_attestation: bool,
}

/// Everything the route disclosure says, as facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDisclosure {
    pub route: Route,
    /// Present whenever a witness is configured, pinned or not.
    pub witness: Option<WitnessFacts>,
    /// Present only on [`Route::Local`]: on the witness route this
    /// machine's filter does not run, and on the others nothing is sent.
    pub local_filter: Option<LocalFilter>,
    pub receipts: ReceiptFacts,
    /// `ironwire_attested_bodies`: a witnessed submission also carries the
    /// final inference call's verbatim request and response to the witness.
    pub attested_bodies: bool,
}

/// The environment's privacy filter, as `TRACE_PRIVACY_FILTER_BACKEND`
/// names it: the protocol crate's backend label, or [`ENV_FILTER_INVALID`]
/// when the variable is set to something that does not build.
pub type EnvFilter = &'static str;

/// [`env_filter`]'s answer for an environment filter that does not build.
pub const ENV_FILTER_INVALID: &str = "invalid";

/// Read the environment's privacy filter for [`route_disclosure`].
#[must_use]
pub fn env_filter() -> EnvFilter {
    trace_commons_protocol::trace_contribution::privacy_filter_backend_from_env()
        .map_or(ENV_FILTER_INVALID, |tag| tag.label())
}

/// The facts, from the configuration in force.
///
/// `config` is the result of loading the contributor config: `Ok(None)` is
/// not enrolled, and `Err(())` is unreadable. `env` is [`env_filter`], taken
/// as a parameter so a test does not mutate the process environment.
#[must_use]
pub fn route_disclosure(
    config: Result<Option<&ContributorConfig>, ()>,
    attested_bodies: bool,
    env: EnvFilter,
) -> RouteDisclosure {
    let cfg = match config {
        Ok(Some(cfg)) => cfg,
        Ok(None) | Err(()) => {
            return RouteDisclosure {
                route: if config.is_err() {
                    Route::SettingsUnreadable
                } else {
                    Route::NotEnrolled
                },
                witness: None,
                local_filter: None,
                receipts: ReceiptFacts {
                    endpoint_configured: false,
                    check_attestation: false,
                },
                attested_bodies,
            };
        }
    };
    let status = witness_status(cfg);
    let route = match status.state {
        WitnessTrustState::Pinned => Route::Witness,
        WitnessTrustState::Absent => Route::Local,
        WitnessTrustState::NotEnrolled => Route::NotEnrolled,
        WitnessTrustState::SettingsUnreadable => Route::SettingsUnreadable,
        WitnessTrustState::RefusingUnpinned
        | WitnessTrustState::RefusingPinMalformed
        | WitnessTrustState::RefusingInferenceReceiptsMissing => Route::WitnessRefusing,
    };
    let witness = match (
        status.url,
        status.signing_address,
        cfg.witness_origin_view(),
    ) {
        (Some(url), Some(signing_address), Some(origin)) => Some(WitnessFacts {
            state: status.state,
            url,
            signing_address,
            pinned_measurements: status.pinned_measurements,
            origin,
        }),
        _ => None,
    };
    let local_filter = (route == Route::Local).then(|| local_filter(cfg, env));
    RouteDisclosure {
        route,
        witness,
        local_filter,
        receipts: ReceiptFacts {
            endpoint_configured: cfg.inference_receipt_endpoint.is_some(),
            check_attestation: cfg.inference_receipt_check_attestation,
        },
        attested_bodies,
    }
}

/// The filter `envelope::build_redactor` would attach: the config's
/// `pii_filter` when set, and otherwise whatever the environment attaches.
fn local_filter(cfg: &ContributorConfig, env: EnvFilter) -> LocalFilter {
    match cfg.pii_filter.as_deref() {
        Some("near-ai") => LocalFilter::NearAi,
        // `build_redactor_with` refuses any other value.
        Some(_) => LocalFilter::Invalid,
        None => match env {
            "none" => LocalFilter::None,
            "near_ai" => LocalFilter::NearAi,
            "self_hosted" => LocalFilter::SelfHosted,
            "sidecar" => LocalFilter::Sidecar,
            // `ENV_FILTER_INVALID`, or a backend label this build does not
            // know: neither is "no filter".
            _ => LocalFilter::Invalid,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{WitnessOrigin, WitnessSettings};

    fn cfg() -> ContributorConfig {
        serde_json::from_value(serde_json::json!({
            "schema_version": crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
            "issuer_url": "https://issuer.invalid",
            "ingest_url": "https://ingest.invalid",
            "audience": "aud",
            "tenant_id": "tenant-1",
            "instance_id": "instance-1",
            "user_subject": "alice",
            "device_key_id": "sha256:aa",
            "consent_scopes": ["debugging_evaluation"],
            "pii_filter": null,
            "allowed_hosts": null,
        }))
        .expect("a contributor config")
    }

    fn witness(pins: &[&str]) -> WitnessSettings {
        WitnessSettings {
            admission_evidence: false,
            url: "https://witness.invalid".into(),
            signing_address: "0xab".into(),
            expected_measurements: pins.iter().map(|p| (*p).to_string()).collect(),
        }
    }

    fn pin() -> String {
        format!("mrtd={}", "ab".repeat(48))
    }

    #[test]
    fn no_enrollment_and_an_unreadable_config_send_nothing_and_say_why() {
        let none = route_disclosure(Ok(None), false, "none");
        assert_eq!(none.route, Route::NotEnrolled);
        assert!(none.witness.is_none() && none.local_filter.is_none());
        let unreadable = route_disclosure(Err(()), false, "none");
        assert_eq!(unreadable.route, Route::SettingsUnreadable);
        assert!(unreadable.witness.is_none() && unreadable.local_filter.is_none());
    }

    #[test]
    fn a_pinned_witness_is_the_raw_send_route_with_its_pins_and_origin() {
        let mut c = cfg();
        c.set_witness(witness(&[&pin()]), WitnessOrigin::PublishedAtJoin);
        let facts = route_disclosure(Ok(Some(&c)), true, "near_ai");
        assert_eq!(facts.route, Route::Witness);
        let w = facts.witness.expect("the witness");
        assert_eq!(w.state, WitnessTrustState::Pinned);
        assert_eq!(w.url, "https://witness.invalid");
        assert_eq!(w.signing_address, "0xab");
        assert_eq!(w.pinned_measurements, vec![pin()]);
        assert_eq!(
            w.origin,
            WitnessOriginView::Recorded(WitnessOrigin::PublishedAtJoin)
        );
        // This machine's filter does not run on the witness route, whatever
        // the environment says.
        assert_eq!(facts.local_filter, None);
        assert!(facts.attested_bodies);
    }

    #[test]
    fn an_unpinned_or_malformed_witness_refuses_and_still_shows_itself() {
        for pins in [vec![], vec!["not a pin".to_string()]] {
            let mut c = cfg();
            let pins: Vec<&str> = pins.iter().map(String::as_str).collect();
            c.witness = Some(witness(&pins));
            let facts = route_disclosure(Ok(Some(&c)), false, "none");
            assert_eq!(facts.route, Route::WitnessRefusing);
            let w = facts.witness.expect("shown even while refusing");
            assert_eq!(w.origin, WitnessOriginView::NotRecorded);
            assert_eq!(facts.local_filter, None);
        }
    }

    #[test]
    fn the_local_route_names_the_filter_this_machine_attaches() {
        let mut c = cfg();
        assert_eq!(
            route_disclosure(Ok(Some(&c)), false, "none").local_filter,
            Some(LocalFilter::None)
        );
        for (env, expected) in [
            ("near_ai", LocalFilter::NearAi),
            ("self_hosted", LocalFilter::SelfHosted),
            ("sidecar", LocalFilter::Sidecar),
            ("from_the_future", LocalFilter::Invalid),
            (ENV_FILTER_INVALID, LocalFilter::Invalid),
        ] {
            let facts = route_disclosure(Ok(Some(&c)), false, env);
            assert_eq!(facts.route, Route::Local);
            assert_eq!(facts.local_filter, Some(expected), "{env:?}");
            assert!(facts.witness.is_none());
        }
        // The config's own selection is what the redactor builds from.
        c.pii_filter = Some("near-ai".into());
        assert_eq!(
            route_disclosure(Ok(Some(&c)), false, "none").local_filter,
            Some(LocalFilter::NearAi)
        );
        c.pii_filter = Some("something-else".into());
        assert_eq!(
            route_disclosure(Ok(Some(&c)), false, "none").local_filter,
            Some(LocalFilter::Invalid)
        );
    }

    #[test]
    fn receipts_are_reported_as_configured() {
        let mut c = cfg();
        let off = route_disclosure(Ok(Some(&c)), false, "none").receipts;
        assert!(!off.endpoint_configured && !off.check_attestation);
        c.inference_receipt_endpoint = Some("https://receipts.invalid/v1".into());
        c.inference_receipt_check_attestation = true;
        let on = route_disclosure(Ok(Some(&c)), false, "none").receipts;
        assert!(on.endpoint_configured && on.check_attestation);
    }

    /// The wire form round-trips, so a shell can hand the daemon's answer to
    /// `consent_copy::route_disclosure_copy` unchanged.
    #[test]
    fn the_facts_round_trip_through_the_wire() {
        let mut c = cfg();
        c.set_witness(witness(&[&pin()]), WitnessOrigin::ConnectedInference);
        let facts = route_disclosure(Ok(Some(&c)), false, "none");
        let wire = serde_json::to_value(&facts).unwrap();
        assert_eq!(wire["route"], "witness");
        assert_eq!(wire["witness"]["origin"], "connected_inference");
        assert_eq!(wire["witness"]["state"], "pinned");
        let back: RouteDisclosure = serde_json::from_value(wire).unwrap();
        assert_eq!(back, facts);
    }
}

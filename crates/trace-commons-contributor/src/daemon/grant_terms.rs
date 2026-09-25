//! The terms a standing `auto_upload` grant was given under, and when a
//! change to them voids it.
//!
//! R6 of the connect-and-forget design. Arming a folder is consent to send
//! its sessions unattended *to particular parties, carrying particular
//! content*. When either grows -- a new recipient sees them, or more leaves
//! the machine -- the grant no longer covers what would happen, so it is
//! voided and the folder asks again instead of carrying on under terms nobody
//! agreed to.
//!
//! Not `preview::input_fingerprint`. That hashes the crate version and
//! `REDACTION_RULESET_VERSION`, so reusing it would void every grant on every
//! release, and a hash cannot say which way a setting moved -- which the rule
//! below needs, because narrowing must not void.
//!
//! # What voids, and what does not
//!
//! Widening is defined by who sees content and what leaves, not by which
//! direction a setting moved:
//!
//! - **Voids:** a different destination (ingest, issuer, audience, host
//!   allowlist) or identity (tenant, instance, subject, device); scopes
//!   gaining any entry; any change to the privacy filter or its host or model,
//!   including one being *added* (prose goes to a new party) or *removed*
//!   (less is scrubbed); a receipt endpoint added or changed (the fetch tells
//!   the provider the exchange is being contributed); any change to the
//!   witness URL or signing address, or a measurement being admitted; attested
//!   bodies turning on.
//! - **Does not void:** scopes narrowing, attested bodies turning off, a
//!   receipt endpoint removed, a witness measurement retired. Each means fewer
//!   parties or less content, which the grant already covered.
//!
//! The NEAR AI API key is deliberately absent, as it is from
//! `input_fingerprint`: rotating a credential changes nothing about what
//! leaves the machine.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::config::ContributorConfig;
use crate::envelope::NearAiSettings;

/// Everything whose change could put a standing grant's sessions in front of
/// someone new, or send more of them.
/// The environment-attached privacy-filter backend, as a label.
pub fn env_filter_backend() -> String {
    match trace_commons_protocol::trace_contribution::privacy_filter_backend_from_env() {
        Ok(tag) => tag.label().to_string(),
        Err(_) => "invalid".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantTerms {
    pub ingest_url: String,
    pub issuer_url: String,
    pub audience: String,
    pub allowed_hosts: Option<String>,
    pub tenant_id: String,
    pub instance_id: String,
    pub user_subject: String,
    pub device_key_id: String,
    pub consent_scopes: BTreeSet<String>,
    pub pii_filter: Option<String>,
    /// The privacy-filter backend the environment attaches on its own
    /// (`TRACE_PRIVACY_FILTER_BACKEND`), resolved as the redactor resolves it:
    /// `DeterministicTraceRedactor::new` adds that filter whatever
    /// `pii_filter` says, so the config field alone misses it. `"invalid"`
    /// for a backend named without its credentials.
    pub env_filter_backend: String,
    pub classifier_base_url: Option<String>,
    pub classifier_model: Option<String>,
    pub classifier_present: bool,
    pub receipt_endpoint: Option<String>,
    pub witness_url: Option<String>,
    pub witness_signing_address: Option<String>,
    pub witness_measurements: BTreeSet<String>,
    pub attested_bodies: bool,
}

pub const VOID_DESTINATION: &str = "destination-changed";
pub const VOID_IDENTITY: &str = "identity-changed";
pub const VOID_SCOPES_WIDENED: &str = "scopes-widened";
pub const VOID_FILTER: &str = "privacy-filter-changed";
pub const VOID_RECEIPT_ENDPOINT: &str = "receipt-endpoint-changed";
pub const VOID_WITNESS: &str = "witness-changed";
pub const VOID_MEASUREMENT_ADMITTED: &str = "witness-measurement-admitted";
pub const VOID_ATTESTED_BODIES: &str = "attested-bodies-on";

impl GrantTerms {
    /// The terms in force now, read from the daemon's config and settings.
    /// `None` when there is no config to read, in which case nothing can be
    /// sent at all.
    pub fn in_force(shared: &super::ipc::DaemonShared) -> Option<Self> {
        let cfg = shared.store.load_config().ok().flatten()?;
        let (near_ai, attested_bodies) = {
            let s = shared.settings.lock().expect("settings lock");
            (s.near_ai.clone(), s.ironwire_attested_bodies)
        };
        Some(Self::current(
            &cfg,
            near_ai.as_ref(),
            attested_bodies,
            &env_filter_backend(),
        ))
    }

    /// The terms in force now.
    pub fn current(
        cfg: &ContributorConfig,
        near_ai: Option<&NearAiSettings>,
        attested_bodies: bool,
        env_filter_backend: &str,
    ) -> Self {
        let witness = cfg.witness.as_ref();
        Self {
            ingest_url: cfg.ingest_url.clone(),
            issuer_url: cfg.issuer_url.clone(),
            audience: cfg.audience.clone(),
            allowed_hosts: cfg.allowed_hosts.clone(),
            tenant_id: cfg.tenant_id.clone(),
            instance_id: cfg.instance_id.clone(),
            user_subject: cfg.user_subject.clone(),
            device_key_id: cfg.device_key_id.clone(),
            consent_scopes: cfg.consent_scopes.iter().cloned().collect(),
            pii_filter: cfg.pii_filter.clone(),
            env_filter_backend: env_filter_backend.to_string(),
            classifier_present: near_ai.is_some(),
            classifier_base_url: near_ai.and_then(|n| n.base_url.clone()),
            classifier_model: near_ai.and_then(|n| n.model.clone()),
            receipt_endpoint: cfg.inference_receipt_endpoint.clone(),
            witness_url: witness.map(|w| w.url.clone()),
            witness_signing_address: witness.map(|w| w.signing_address.clone()),
            witness_measurements: witness
                .map(|w| w.expected_measurements.iter().cloned().collect())
                .unwrap_or_default(),
            attested_bodies,
        }
    }

    /// Every way these terms widen `granted`, as labels. Empty means the
    /// grant still covers them.
    pub fn widening_from(&self, granted: &GrantTerms) -> Vec<&'static str> {
        let mut reasons = Vec::new();
        if self.ingest_url != granted.ingest_url
            || self.issuer_url != granted.issuer_url
            || self.audience != granted.audience
            || self.allowed_hosts != granted.allowed_hosts
        {
            reasons.push(VOID_DESTINATION);
        }
        if self.tenant_id != granted.tenant_id
            || self.instance_id != granted.instance_id
            || self.user_subject != granted.user_subject
            || self.device_key_id != granted.device_key_id
        {
            reasons.push(VOID_IDENTITY);
        }
        if !self.consent_scopes.is_subset(&granted.consent_scopes) {
            reasons.push(VOID_SCOPES_WIDENED);
        }
        // Any change at all, in either direction: added sends prose to a new
        // party, removed scrubs less, and a different host or model is a
        // different operator reading it.
        if self.pii_filter != granted.pii_filter
            || self.env_filter_backend != granted.env_filter_backend
            || self.classifier_present != granted.classifier_present
            || self.classifier_base_url != granted.classifier_base_url
            || self.classifier_model != granted.classifier_model
        {
            reasons.push(VOID_FILTER);
        }
        // Added or changed widens; removed means one fewer party told.
        if self.receipt_endpoint.is_some() && self.receipt_endpoint != granted.receipt_endpoint {
            reasons.push(VOID_RECEIPT_ENDPOINT);
        }
        if self.witness_url != granted.witness_url
            || self.witness_signing_address != granted.witness_signing_address
        {
            reasons.push(VOID_WITNESS);
        } else if !self
            .witness_measurements
            .is_subset(&granted.witness_measurements)
        {
            // Same witness, a newly admitted build of it. Retiring one is not
            // widening; admitting one lets different code see the session.
            reasons.push(VOID_MEASUREMENT_ADMITTED);
        }
        if self.attested_bodies && !granted.attested_bodies {
            reasons.push(VOID_ATTESTED_BODIES);
        }
        reasons
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> GrantTerms {
        GrantTerms {
            ingest_url: "https://ingest.example".into(),
            issuer_url: "https://issuer.example".into(),
            audience: "aud".into(),
            allowed_hosts: None,
            tenant_id: "tenant-1".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: "sha256:aa".into(),
            consent_scopes: ["debugging_evaluation", "benchmark_only"]
                .into_iter()
                .map(String::from)
                .collect(),
            pii_filter: None,
            env_filter_backend: "none".into(),
            classifier_present: false,
            classifier_base_url: None,
            classifier_model: None,
            receipt_endpoint: None,
            witness_url: Some("https://witness.example".into()),
            witness_signing_address: Some("0xab".into()),
            witness_measurements: ["mrtd=aa".to_string()].into_iter().collect(),
            attested_bodies: false,
        }
    }

    fn widening(change: impl FnOnce(&mut GrantTerms)) -> Vec<&'static str> {
        let granted = base();
        let mut now = base();
        change(&mut now);
        now.widening_from(&granted)
    }

    /// Every widening that voids a grant also changes `input_fingerprint`,
    /// so an entry approved under the old terms is re-offered rather than
    /// sent under the new ones. Driven from the inputs both are derived from,
    /// not from `GrantTerms`, so a term added to one and not the other fails
    /// here. The environment's filter backend is left out: varying it means
    /// mutating the process environment, and both read it from the same
    /// `env_filter_backend`.
    #[test]
    fn every_widening_also_changes_the_input_fingerprint() {
        use crate::daemon::preview::input_fingerprint;
        let cfg = ContributorConfig {
            inference_receipt_endpoint: None,
            inference_receipt_check_attestation: false,
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
            issuer_url: "https://issuer.invalid".to_string(),
            ingest_url: "https://ingest.invalid".to_string(),
            audience: "aud".to_string(),
            tenant_id: "tenant-1".to_string(),
            instance_id: "instance-1".to_string(),
            user_subject: "alice".to_string(),
            device_key_id: "sha256:aa".to_string(),
            consent_scopes: vec!["debugging_evaluation".to_string()],
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness: None,
        };
        let classifier = NearAiSettings {
            api_key: "k".to_string(),
            base_url: Some("https://classifier.invalid".to_string()),
            model: Some("m".to_string()),
        };
        let witness = crate::config::WitnessSettings {
            admission_evidence: false,
            url: "https://witness.invalid".to_string(),
            signing_address: "0x0000000000000000000000000000000000000001".to_string(),
            expected_measurements: Vec::new(),
        };
        type Inputs = (ContributorConfig, Option<NearAiSettings>, bool);
        let changes: Vec<(&str, Box<dyn Fn(&mut Inputs)>)> = vec![
            (
                "destination",
                Box::new(|i| i.0.ingest_url = "https://other.invalid".into()),
            ),
            ("identity", Box::new(|i| i.0.tenant_id = "tenant-2".into())),
            (
                "scopes",
                Box::new(|i| i.0.consent_scopes.push("model_training".into())),
            ),
            (
                "pii filter",
                Box::new(|i| i.0.pii_filter = Some("other".into())),
            ),
            (
                "receipt endpoint",
                Box::new(|i| i.0.inference_receipt_endpoint = Some("https://r.invalid".into())),
            ),
            ("witness", {
                let witness = witness.clone();
                Box::new(move |i| i.0.witness = Some(witness.clone()))
            }),
            ("classifier", {
                let classifier = classifier.clone();
                Box::new(move |i| i.1 = Some(classifier.clone()))
            }),
            ("attested bodies", Box::new(|i| i.2 = true)),
        ];
        let terms = |i: &Inputs| GrantTerms::current(&i.0, i.1.as_ref(), i.2, "none");
        let fingerprint = |i: &Inputs| input_fingerprint(&i.0, i.1.as_ref(), i.2);
        let base: Inputs = (cfg, None, false);
        for (name, change) in &changes {
            let mut now = base.clone();
            change(&mut now);
            assert!(
                !terms(&now).widening_from(&terms(&base)).is_empty(),
                "{name}: expected a widening"
            );
            assert_ne!(fingerprint(&now), fingerprint(&base), "{name}");
        }
    }

    #[test]
    fn unchanged_terms_do_not_void() {
        assert!(widening(|_| {}).is_empty());
    }

    #[test]
    fn a_new_recipient_voids() {
        assert_eq!(
            widening(|t| t.ingest_url = "https://other".into()),
            [VOID_DESTINATION]
        );
        assert_eq!(
            widening(|t| t.audience = "other".into()),
            [VOID_DESTINATION]
        );
        assert_eq!(
            widening(|t| t.tenant_id = "tenant-2".into()),
            [VOID_IDENTITY]
        );
        assert_eq!(
            widening(|t| t.device_key_id = "sha256:bb".into()),
            [VOID_IDENTITY]
        );
    }

    #[test]
    fn scopes_void_only_when_they_widen() {
        assert_eq!(
            widening(|t| {
                t.consent_scopes.insert("model_training".into());
            }),
            [VOID_SCOPES_WIDENED]
        );
        assert!(
            widening(|t| {
                t.consent_scopes.remove("benchmark_only");
            })
            .is_empty(),
            "narrowing is covered by the grant"
        );
    }

    /// Adding a filter sends prose to a new party; removing one scrubs less.
    /// Both void, and so does moving it to a different host or model.
    #[test]
    fn any_change_to_the_privacy_filter_voids() {
        assert_eq!(
            widening(|t| t.pii_filter = Some("near-ai".into())),
            [VOID_FILTER]
        );
        assert_eq!(widening(|t| t.classifier_present = true), [VOID_FILTER]);
        let mut granted = base();
        granted.classifier_present = true;
        granted.classifier_model = Some("model-a".into());
        let mut now = granted.clone();
        now.classifier_model = Some("model-b".into());
        assert_eq!(now.widening_from(&granted), [VOID_FILTER]);
        let mut removed = granted.clone();
        removed.classifier_present = false;
        removed.classifier_model = None;
        assert_eq!(removed.widening_from(&granted), [VOID_FILTER]);
    }

    /// Reviewed on #1024: a filter attached from the environment is a filter
    /// all the same, and so is one taken away by config alone.
    #[test]
    fn an_environment_filter_and_a_removed_config_filter_both_void() {
        assert_eq!(
            widening(|t| t.env_filter_backend = "sidecar".into()),
            [VOID_FILTER]
        );
        let mut granted = base();
        granted.pii_filter = Some("near-ai".into());
        let mut removed = granted.clone();
        removed.pii_filter = None;
        assert_eq!(removed.widening_from(&granted), [VOID_FILTER]);
    }

    #[test]
    fn a_receipt_endpoint_voids_when_added_or_changed_and_not_when_removed() {
        assert_eq!(
            widening(|t| t.receipt_endpoint = Some("https://receipts".into())),
            [VOID_RECEIPT_ENDPOINT]
        );
        let mut granted = base();
        granted.receipt_endpoint = Some("https://receipts".into());
        let mut removed = granted.clone();
        removed.receipt_endpoint = None;
        assert!(removed.widening_from(&granted).is_empty());
    }

    #[test]
    fn a_different_witness_voids_and_a_retired_measurement_does_not() {
        assert_eq!(
            widening(|t| t.witness_url = Some("https://w2".into())),
            [VOID_WITNESS]
        );
        assert_eq!(
            widening(|t| t.witness_signing_address = Some("0xcd".into())),
            [VOID_WITNESS]
        );
        assert_eq!(
            widening(|t| t.witness_url = None),
            [VOID_WITNESS],
            "removing it scrubs less"
        );
        assert_eq!(
            widening(|t| {
                t.witness_measurements.insert("mrtd=bb".into());
            }),
            [VOID_MEASUREMENT_ADMITTED]
        );
        assert!(
            widening(|t| {
                t.witness_measurements.clear();
            })
            .is_empty(),
            "retiring a build admits nothing new"
        );
    }

    #[test]
    fn attested_bodies_void_only_when_turned_on() {
        assert_eq!(
            widening(|t| t.attested_bodies = true),
            [VOID_ATTESTED_BODIES]
        );
        let mut granted = base();
        granted.attested_bodies = true;
        let mut off = granted.clone();
        off.attested_bodies = false;
        assert!(off.widening_from(&granted).is_empty());
    }
}

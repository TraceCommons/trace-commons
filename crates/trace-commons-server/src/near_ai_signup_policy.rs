// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The policy template a NEAR AI sign-up's contributor grant is written from.
//!
//! Invite enrollment writes a contributor tenant access grant for the device
//! principal, with its scopes taken from the instance's policy template, and
//! the upload-claim issuer reads that grant as the ceiling for the device's
//! claims (`resolve_device_scope_ceiling`). A NEAR AI sign-up has no instance
//! and wrote no grant, so its claims always got the hard-coded floor
//! (debugging, evaluation, aggregate analytics) whatever the contributor
//! consented to in the app. Owner decision 2026-10-24 ("Honor app consent"):
//! NEAR AI sign-up writes the same grant, from this deployment's template.
//!
//! The template is one environment variable,
//! [`NEAR_AI_SIGNUP_POLICY_TEMPLATE_ENV`], read by both processes that write
//! the grant: ingest, at provisioning, bind and device enrollment, and the
//! issuer, which writes a missing grant at claim time for a device NEAR AI
//! provisioned before this existed. Both parse it here, so they cannot read
//! one value two ways. Its JSON shape is the allowlist's instance
//! `policy_template`.
//!
//! Fail-closed: unset or empty means no template, so no grant is written and
//! the floor stays. A value that does not parse, names an unknown scope or
//! use, or leaves either list empty refuses the process at boot, rather than
//! storing a grant the issuer would refuse on every later claim.

use serde::Deserialize;
use trace_commons_protocol::trace_contribution::{ConsentScope, TraceAllowedUse};

/// The variable both ingest and the issuer read the template from.
pub const NEAR_AI_SIGNUP_POLICY_TEMPLATE_ENV: &str = "TRACE_COMMONS_NEAR_AI_SIGNUP_POLICY_TEMPLATE";

/// A validated template: wire names, de-duplicated, in the order given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearAiSignupPolicyTemplate {
    policy_version: String,
    allowed_consent_scopes: Vec<String>,
    allowed_uses: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTemplate {
    policy_version: String,
    allowed_consent_scopes: Vec<ConsentScope>,
    allowed_uses: Vec<TraceAllowedUse>,
}

/// Why a template was refused. Static labels only: the value is operator
/// configuration, not a secret, but nothing here needs to echo it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NearAiSignupPolicyTemplateError {
    /// Not JSON of the template's shape, or an unknown scope or use.
    Malformed,
    /// `policy_version` is empty.
    MissingPolicyVersion,
    /// `allowed_consent_scopes` is empty.
    NoConsentScopes,
    /// `allowed_uses` is empty.
    NoAllowedUses,
}

impl std::fmt::Display for NearAiSignupPolicyTemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Malformed => "not a policy template, or names an unknown scope or use",
            Self::MissingPolicyVersion => "policy_version is empty",
            Self::NoConsentScopes => "allowed_consent_scopes is empty",
            Self::NoAllowedUses => "allowed_uses is empty",
        })
    }
}

impl std::error::Error for NearAiSignupPolicyTemplateError {}

fn wire_name<T: serde::Serialize>(value: &T) -> Result<String, NearAiSignupPolicyTemplateError> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => Ok(name),
        _ => Err(NearAiSignupPolicyTemplateError::Malformed),
    }
}

fn wire_names<T: serde::Serialize>(
    values: &[T],
) -> Result<Vec<String>, NearAiSignupPolicyTemplateError> {
    let mut names: Vec<String> = Vec::with_capacity(values.len());
    for value in values {
        let name = wire_name(value)?;
        if !names.contains(&name) {
            names.push(name);
        }
    }
    Ok(names)
}

impl NearAiSignupPolicyTemplate {
    /// Parse and validate a template.
    pub fn parse(value: &str) -> Result<Self, NearAiSignupPolicyTemplateError> {
        let raw: RawTemplate =
            serde_json::from_str(value).map_err(|_| NearAiSignupPolicyTemplateError::Malformed)?;
        let policy_version = raw.policy_version.trim().to_string();
        if policy_version.is_empty() {
            return Err(NearAiSignupPolicyTemplateError::MissingPolicyVersion);
        }
        if raw.allowed_consent_scopes.is_empty() {
            return Err(NearAiSignupPolicyTemplateError::NoConsentScopes);
        }
        if raw.allowed_uses.is_empty() {
            return Err(NearAiSignupPolicyTemplateError::NoAllowedUses);
        }
        Ok(Self {
            policy_version,
            allowed_consent_scopes: wire_names(&raw.allowed_consent_scopes)?,
            allowed_uses: wire_names(&raw.allowed_uses)?,
        })
    }

    /// Read [`NEAR_AI_SIGNUP_POLICY_TEMPLATE_ENV`]. Unset or blank is `None`
    /// (no grant, the floor); anything else must parse or the caller's boot
    /// fails.
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        Self::from_env_value(
            std::env::var(NEAR_AI_SIGNUP_POLICY_TEMPLATE_ENV)
                .ok()
                .as_deref(),
        )
    }

    fn from_env_value(value: Option<&str>) -> anyhow::Result<Option<Self>> {
        match value.map(str::trim) {
            None | Some("") => Ok(None),
            Some(value) => Self::parse(value).map(Some).map_err(|error| {
                anyhow::anyhow!("{NEAR_AI_SIGNUP_POLICY_TEMPLATE_ENV} is invalid: {error}")
            }),
        }
    }

    pub fn policy_version(&self) -> &str {
        &self.policy_version
    }

    /// Wire names, as `trace_tenant_access_grants.allowed_consent_scopes`
    /// stores them.
    pub fn allowed_consent_scopes(&self) -> &[String] {
        &self.allowed_consent_scopes
    }

    /// Wire names, as `trace_tenant_access_grants.allowed_uses` stores them.
    pub fn allowed_uses(&self) -> &[String] {
        &self.allowed_uses
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{"policy_version":"staging-v1",
        "allowed_consent_scopes":["debugging_evaluation","benchmark_only","ranking_training","model_training","public_attribution"],
        "allowed_uses":["debugging","evaluation","benchmark_generation","ranking_model_training","model_training","aggregate_analytics"]}"#;

    #[test]
    fn a_template_keeps_its_wire_names_in_order() {
        let template = NearAiSignupPolicyTemplate::parse(FULL).unwrap();
        assert_eq!(template.policy_version(), "staging-v1");
        assert_eq!(
            template.allowed_consent_scopes(),
            [
                "debugging_evaluation",
                "benchmark_only",
                "ranking_training",
                "model_training",
                "public_attribution"
            ]
        );
        assert_eq!(
            template.allowed_uses(),
            [
                "debugging",
                "evaluation",
                "benchmark_generation",
                "ranking_model_training",
                "model_training",
                "aggregate_analytics"
            ]
        );
    }

    #[test]
    fn duplicates_collapse() {
        let template = NearAiSignupPolicyTemplate::parse(
            r#"{"policy_version":"v","allowed_consent_scopes":["model_training","model_training"],"allowed_uses":["model_training","model_training"]}"#,
        )
        .unwrap();
        assert_eq!(template.allowed_consent_scopes(), ["model_training"]);
        assert_eq!(template.allowed_uses(), ["model_training"]);
    }

    #[test]
    fn every_malformed_template_is_refused_by_name() {
        use NearAiSignupPolicyTemplateError::*;
        for (value, expected) in [
            ("not json", Malformed),
            (
                r#"{"policy_version":"v","allowed_consent_scopes":["everything"],"allowed_uses":["debugging"]}"#,
                Malformed,
            ),
            (
                r#"{"policy_version":"v","allowed_consent_scopes":["model_training"],"allowed_uses":["resale"]}"#,
                Malformed,
            ),
            (
                r#"{"policy_version":"v","allowed_consent_scopes":["model_training"],"allowed_uses":["debugging"],"extra":1}"#,
                Malformed,
            ),
            (
                r#"{"allowed_consent_scopes":["model_training"],"allowed_uses":["debugging"]}"#,
                Malformed,
            ),
            (
                r#"{"policy_version":"  ","allowed_consent_scopes":["model_training"],"allowed_uses":["debugging"]}"#,
                MissingPolicyVersion,
            ),
            (
                r#"{"policy_version":"v","allowed_consent_scopes":[],"allowed_uses":["debugging"]}"#,
                NoConsentScopes,
            ),
            (
                r#"{"policy_version":"v","allowed_consent_scopes":["model_training"],"allowed_uses":[]}"#,
                NoAllowedUses,
            ),
        ] {
            assert_eq!(
                NearAiSignupPolicyTemplate::parse(value),
                Err(expected),
                "{value}"
            );
        }
    }

    /// The staging env templates carry the value systemd hands both
    /// processes (single quotes stripped). It must parse, and the two must
    /// agree, or staging boots refuse / drift.
    #[test]
    fn the_staging_templates_carry_one_valid_template() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/staging-gcp");
        let value_in = |file: &str| {
            let text = std::fs::read_to_string(root.join(file)).unwrap();
            let line = text
                .lines()
                .find(|line| line.starts_with(&format!("{NEAR_AI_SIGNUP_POLICY_TEMPLATE_ENV}=")))
                .unwrap_or_else(|| panic!("{file} sets {NEAR_AI_SIGNUP_POLICY_TEMPLATE_ENV}"));
            let value = &line[NEAR_AI_SIGNUP_POLICY_TEMPLATE_ENV.len() + 1..];
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
                .expect("single-quoted")
                .to_string()
        };
        let ingest = value_in("ingest.env.template");
        assert_eq!(ingest, value_in("issuer.env.template"));
        let template = NearAiSignupPolicyTemplate::parse(&ingest).unwrap();
        assert!(
            template
                .allowed_uses()
                .iter()
                .any(|u| u == "model_training")
        );
    }

    #[test]
    fn unset_or_blank_is_no_template_and_anything_else_must_parse() {
        assert_eq!(
            NearAiSignupPolicyTemplate::from_env_value(None).unwrap(),
            None
        );
        assert_eq!(
            NearAiSignupPolicyTemplate::from_env_value(Some("  ")).unwrap(),
            None
        );
        assert!(
            NearAiSignupPolicyTemplate::from_env_value(Some(FULL))
                .unwrap()
                .is_some()
        );
        let error = NearAiSignupPolicyTemplate::from_env_value(Some("{}"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(NEAR_AI_SIGNUP_POLICY_TEMPLATE_ENV),
            "{error}"
        );
    }
}

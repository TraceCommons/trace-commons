// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shared consent and allowed-use authority checks.

use std::collections::BTreeSet;

use trace_commons_protocol::trace_contribution::{ConsentScope, TraceAllowedUse};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubmissionAllowlists {
    pub allowed_consent_scopes: BTreeSet<ConsentScope>,
    pub allowed_uses: BTreeSet<TraceAllowedUse>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionAuthority {
    pub tenant: SubmissionAllowlists,
    pub policy: Option<SubmissionAllowlists>,
    pub require_policy: bool,
}

impl SubmissionAuthority {
    pub fn permits(&self, source_scopes: &[ConsentScope], source_uses: &[TraceAllowedUse]) -> bool {
        if !source_matches_consent_allowlist(source_scopes, &self.tenant.allowed_consent_scopes)
            || !source_matches_allowed_use_allowlist(source_uses, &self.tenant.allowed_uses)
        {
            return false;
        }

        match self.policy.as_ref() {
            Some(policy) => {
                source_matches_consent_allowlist(source_scopes, &policy.allowed_consent_scopes)
                    && source_matches_allowed_use_allowlist(source_uses, &policy.allowed_uses)
            }
            None => !self.require_policy,
        }
    }
}

pub fn source_matches_consent_allowlist(
    source_scopes: &[ConsentScope],
    allowlist: &BTreeSet<ConsentScope>,
) -> bool {
    allowlist.is_empty() || source_scopes.iter().any(|scope| allowlist.contains(scope))
}

pub fn source_matches_allowed_use_allowlist(
    source_uses: &[TraceAllowedUse],
    allowlist: &BTreeSet<TraceAllowedUse>,
) -> bool {
    allowlist.is_empty()
        || source_uses
            .iter()
            .any(|allowed_use| allowlist.contains(allowed_use))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_and_policy_allowlists_both_apply() {
        let authority = SubmissionAuthority {
            tenant: SubmissionAllowlists {
                allowed_consent_scopes: BTreeSet::from([ConsentScope::DebuggingEvaluation]),
                allowed_uses: BTreeSet::from([TraceAllowedUse::Debugging]),
            },
            policy: Some(SubmissionAllowlists {
                allowed_consent_scopes: BTreeSet::from([ConsentScope::DebuggingEvaluation]),
                allowed_uses: BTreeSet::from([TraceAllowedUse::Evaluation]),
            }),
            require_policy: true,
        };
        assert!(!authority.permits(
            &[ConsentScope::DebuggingEvaluation],
            &[TraceAllowedUse::Debugging]
        ));
        assert!(authority.permits(
            &[ConsentScope::DebuggingEvaluation],
            &[TraceAllowedUse::Debugging, TraceAllowedUse::Evaluation]
        ));
    }

    #[test]
    fn required_missing_policy_fails_closed() {
        let authority = SubmissionAuthority {
            tenant: SubmissionAllowlists::default(),
            policy: None,
            require_policy: true,
        };
        assert!(!authority.permits(
            &[ConsentScope::DebuggingEvaluation],
            &[TraceAllowedUse::Evaluation]
        ));
    }
}

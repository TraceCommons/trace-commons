//! Pins the consent surface to the published policy version.
//!
//! <https://tracecommons.ai/legal/> is the single source of truth for the
//! terms, the data policy and the consent scopes, and every envelope records
//! the `policy_version` its contributor consented under. That only means
//! something if the two move together: a scope added, removed or renamed
//! without a policy-version bump leaves published text describing a consent
//! surface that no longer exists, and leaves already-submitted envelopes
//! pointing at a version whose text has quietly changed underneath them.
//!
//! So this test does not check that the code is correct. It checks that a
//! change to the code was accompanied by a decision about the document. If it
//! fails, the fix is: update <https://tracecommons.ai/legal/> (the page and its
//! permalink live in the `trace-commons-community` repo), bump
//! `TRACE_CONTRIBUTION_POLICY_VERSION` and `src/policy.ts` there to match, then
//! update the table below.

use trace_commons_protocol::trace_contribution::{ConsentScope, TRACE_CONTRIBUTION_POLICY_VERSION};

/// Every scope, with the wire string it serialises to. The wire string is what
/// the envelope carries and what part C of the published document names in its
/// headings, so a rename here silently changes what an already-submitted
/// envelope means: the stored value no longer matches any scope the policy
/// version it cites defines.
const PINNED_SCOPES: &[(ConsentScope, &str)] = &[
    (ConsentScope::DebuggingEvaluation, "debugging_evaluation"),
    (ConsentScope::BenchmarkOnly, "benchmark_only"),
    (ConsentScope::RankingTraining, "ranking_training"),
    (ConsentScope::ModelTraining, "model_training"),
    (ConsentScope::PublicAttribution, "public_attribution"),
];

/// The version the published document currently describes.
const PINNED_POLICY_VERSION: &str = "2026-04-24";

#[test]
fn policy_version_matches_the_published_document() {
    assert_eq!(
        TRACE_CONTRIBUTION_POLICY_VERSION, PINNED_POLICY_VERSION,
        "the policy version changed. Publish the new version of \
         https://tracecommons.ai/legal/ , bump src/policy.ts in the community \
         repo to match, then update PINNED_POLICY_VERSION here",
    );
}

#[test]
fn every_consent_scope_is_described_by_the_published_document() {
    for (scope, wire) in PINNED_SCOPES {
        let encoded = serde_json::to_string(scope).expect("scope serialises");
        assert_eq!(
            encoded,
            format!("\"{wire}\""),
            "consent scope {scope:?} no longer serialises as `{wire}`. Part C \
             of https://tracecommons.ai/legal/ defines `{wire}`, and envelopes \
             already submitted carry it -- renaming the variant leaves those \
             envelopes citing a scope their policy version does not define",
        );

        let decoded: ConsentScope = serde_json::from_str(&encoded).expect("scope round-trips");
        assert_eq!(&decoded, scope);
    }
}

/// Catches the addition or removal of a variant, which the loop above cannot:
/// a new scope simply would not appear in `PINNED_SCOPES`.
///
/// This is a match rather than a count so the compiler names the offending
/// variant. Adding a scope stops the build here until someone has written the
/// paragraph in part C that says what the new scope permits and, crucially,
/// what it does not.
#[test]
fn no_consent_scope_exists_that_the_document_does_not_describe() {
    fn assert_described(scope: ConsentScope) -> &'static str {
        match scope {
            ConsentScope::DebuggingEvaluation => "debugging_evaluation",
            ConsentScope::BenchmarkOnly => "benchmark_only",
            ConsentScope::RankingTraining => "ranking_training",
            ConsentScope::ModelTraining => "model_training",
            ConsentScope::PublicAttribution => "public_attribution",
        }
    }

    for (scope, wire) in PINNED_SCOPES {
        assert_eq!(assert_described(*scope), *wire);
    }
}

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

use trace_commons_protocol::trace_contribution::{
    ConsentMetadata, ConsentScope, TRACE_CONTRIBUTION_POLICY_VERSION,
};

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

// --- content flags ---------------------------------------------------------
//
// The pin above covers the scopes and the version. It did not cover the
// content flags, which are the other half of the consent surface an envelope
// carries: `message_text_included` and `tool_payloads_included` had no
// reference here at all, so a new content class could be added and this test
// would still pass.
//
// The flags are not permissions -- authorization lives in `scopes` -- but they
// are what the published document's redaction promises are ABOUT, and they are
// what the protective controls read: `residual_risk` floors a declaring
// envelope at Medium, and the ingest PII-backstop hold enrols on any of them.
// A content class the document does not mention is a class the page describes
// wrongly by omission.
//
// `correction_included` is the live example. A correction is stored as
// written -- the semantic redaction passes are skipped, deliberately -- while
// the published page says redaction is applied locally and re-applied
// server-side. That page is currently wrong about corrections. The disclosure
// that reaches a contributor in the meantime is the caption on the correction
// control itself, at the moment of writing; the published clause
// (`docs/legal-correction-clause-draft.md`) is a follow-up.

/// Every content flag, with the wire key it serialises to and what a
/// contributor is told about it. The wire key is what the envelope carries, so
/// a rename changes what an already-submitted envelope declares.
const PINNED_CONTENT_FLAGS: &[(&str, &str)] = &[
    (
        "message_text_included",
        "raw redacted message text from the session is present",
    ),
    (
        "tool_payloads_included",
        "raw redacted tool arguments and results are present",
    ),
    (
        "correction_included",
        "a contributor-authored correction is present, and unlike the two \
         above it is stored AS WRITTEN -- only secret detection runs over it",
    ),
    (
        "routing_metadata_included",
        "routing and cost metadata about the inference hops that produced the \
         session is present -- a backend id, a rung, a token count, a price. \
         It is numbers and labels, not prose from the session, so unlike the \
         three above it does not enrol the trace in the PII backstop hold and \
         does not floor residual risk",
    ),
];

/// A `ConsentMetadata` with every content flag set, so the serialised form
/// names them all.
fn all_flags_declared() -> ConsentMetadata {
    ConsentMetadata {
        policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
        scopes: vec![ConsentScope::DebuggingEvaluation],
        message_text_included: true,
        tool_payloads_included: true,
        correction_included: true,
        routing_metadata_included: true,
        revocable: true,
    }
}

/// Catches the addition or removal of a content flag, which a key-by-key
/// check cannot: a new flag simply would not appear in `PINNED_CONTENT_FLAGS`.
///
/// The destructure below names every field with no `..`, so adding a fourth
/// stops the build here until someone has decided what the published document
/// says about it -- in particular whether the new class is redacted, since the
/// page's redaction promise is stated for the envelope as a whole.
#[test]
fn no_content_flag_exists_that_the_document_does_not_describe() {
    // The exhaustive struct pattern is the guard. It is written as a `let`
    // rather than the `match` the scope guard above uses only because clippy
    // rejects an infallible match; the compiler error on a new field is the
    // same either way (E0027, "pattern does not mention field ..."), because
    // it comes from the pattern and not from the match.
    fn declared_flags(consent: &ConsentMetadata) -> Vec<&'static str> {
        let ConsentMetadata {
            policy_version: _,
            scopes: _,
            message_text_included,
            tool_payloads_included,
            correction_included,
            routing_metadata_included,
            revocable: _,
        } = consent;

        let mut declared = Vec::new();
        if *message_text_included {
            declared.push("message_text_included");
        }
        if *tool_payloads_included {
            declared.push("tool_payloads_included");
        }
        if *correction_included {
            declared.push("correction_included");
        }
        if *routing_metadata_included {
            declared.push("routing_metadata_included");
        }
        declared
    }

    let described: Vec<&str> = PINNED_CONTENT_FLAGS.iter().map(|(key, _)| *key).collect();
    assert_eq!(
        declared_flags(&all_flags_declared()),
        described,
        "a content flag exists that PINNED_CONTENT_FLAGS does not describe. \
         Decide what https://tracecommons.ai/legal/ says about the new content \
         class -- above all whether it is redacted -- then add it here",
    );
}

/// The wire keys themselves. A rename is invisible to the exhaustiveness check
/// above (which works on field identifiers), and it is the wire key that an
/// already-submitted envelope carries.
#[test]
fn every_content_flag_serialises_under_its_pinned_wire_key() {
    let encoded = serde_json::to_value(all_flags_declared()).expect("consent metadata serialises");
    let object = encoded.as_object().expect("consent metadata is an object");

    for (wire, meaning) in PINNED_CONTENT_FLAGS {
        assert_eq!(
            object.get(*wire).and_then(serde_json::Value::as_bool),
            Some(true),
            "content flag `{wire}` no longer serialises under that key. It \
             means: {meaning}. Envelopes already submitted carry the old key",
        );
    }
}

/// An envelope submitted before a content flag existed omits its key, and that
/// silence must read as `false`. It is the honest reading -- nothing could
/// have set a flag that did not exist -- and it is also the safe direction:
/// `reconcile_consent_declarations` corrects a false flag upward when the
/// payload proves otherwise, and never the reverse.
#[test]
fn an_omitted_content_flag_deserialises_as_false() {
    let legacy = serde_json::json!({
        "policy_version": TRACE_CONTRIBUTION_POLICY_VERSION,
        "scopes": ["debugging_evaluation"],
        "message_text_included": true,
        "tool_payloads_included": false,
        "revocable": true,
    });

    let decoded: ConsentMetadata =
        serde_json::from_value(legacy).expect("an envelope predating the flag still decodes");

    assert!(decoded.message_text_included);
    assert!(!decoded.correction_included);
}

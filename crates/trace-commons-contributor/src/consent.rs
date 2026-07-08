//! Consent-scope validation and scope-to-allowed-use mapping for the
//! contributor CLI's upload-claim requests.
//!
//! `debugging_evaluation` is the always-on floor scope: every claim request
//! carries it regardless of what the operator configured, matching the
//! issuer's own floor behavior.

use anyhow::{Result, bail};

/// The full set of consent scopes the contributor CLI understands, in the
/// canonical wire order used for validation, dedup, and mapping output.
pub const VALID_SCOPES: [&str; 5] = [
    "debugging_evaluation",
    "benchmark_only",
    "ranking_training",
    "model_training",
    "public_attribution",
];

const FLOOR_SCOPE: &str = "debugging_evaluation";

/// Validate a list of wire-name consent scopes against [`VALID_SCOPES`].
///
/// Unknown names produce an error listing the valid set (with a hint to
/// re-run login to fix the stored config). The result is deduped, ordered to
/// match [`VALID_SCOPES`], and always includes the floor scope
/// `debugging_evaluation` even if it was omitted from `names`.
pub fn validate_scopes(names: &[String]) -> Result<Vec<String>> {
    for name in names {
        if !VALID_SCOPES.contains(&name.as_str()) {
            bail!(
                "unknown consent scope \"{name}\" in stored config; valid scopes are {:?} \
                 (re-run login to fix the stored config)",
                VALID_SCOPES
            );
        }
    }
    Ok(VALID_SCOPES
        .iter()
        .filter(|scope| **scope == FLOOR_SCOPE || names.iter().any(|n| n == *scope))
        .map(|s| s.to_string())
        .collect())
}

/// Map validated consent scopes to the wire-name allowed-uses the issuer
/// grants for them, always including `aggregate_analytics`, deduped, in
/// stable order.
pub fn scopes_to_allowed_uses(scopes: &[String]) -> Vec<String> {
    let mut uses: Vec<String> = Vec::new();
    for scope in VALID_SCOPES
        .iter()
        .filter(|s| scopes.iter().any(|x| x == *s))
    {
        let mapped: &[&str] = match *scope {
            "debugging_evaluation" => &["debugging", "evaluation"],
            "benchmark_only" => &["benchmark_generation"],
            "ranking_training" => &["ranking_model_training"],
            "model_training" => &["model_training"],
            "public_attribution" => &[],
            _ => &[],
        };
        for u in mapped {
            if !uses.iter().any(|existing| existing == u) {
                uses.push(u.to_string());
            }
        }
    }
    uses.push("aggregate_analytics".to_string());
    uses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_scopes_dedups_orders_and_always_includes_floor() {
        let got = validate_scopes(&["model_training".into(), "model_training".into()]).unwrap();
        assert_eq!(
            got,
            vec![
                "debugging_evaluation".to_string(),
                "model_training".to_string()
            ]
        );
        let err = validate_scopes(&["training".into()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("training") && err.contains("model_training"));
    }

    #[test]
    fn scope_to_use_mapping_matches_spec() {
        let got = scopes_to_allowed_uses(&["debugging_evaluation".into(), "model_training".into()]);
        assert_eq!(
            got,
            vec![
                "debugging".to_string(),
                "evaluation".to_string(),
                "model_training".to_string(),
                "aggregate_analytics".to_string()
            ]
        );
        let attribution_only = scopes_to_allowed_uses(&["public_attribution".into()]);
        assert_eq!(attribution_only, vec!["aggregate_analytics".to_string()]);
    }
}

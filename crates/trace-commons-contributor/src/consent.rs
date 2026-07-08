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

/// Interactive consent answers gathered at login. `debugging_evaluation` is
/// always on and is not represented here; the other four fields correspond
/// 1:1 to the optional scopes in [`VALID_SCOPES`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ConsentAnswers {
    pub benchmark: bool,
    pub ranking: bool,
    pub training: bool,
    pub attribution: bool,
}

/// Pure mapping: answers -> validated wire-name scope list. Always includes
/// the `debugging_evaluation` floor; an all-false `ConsentAnswers` yields
/// just the floor scope.
pub fn scopes_from_answers(a: ConsentAnswers) -> Vec<String> {
    let mut names = Vec::new();
    if a.benchmark {
        names.push("benchmark_only".to_string());
    }
    if a.ranking {
        names.push("ranking_training".to_string());
    }
    if a.training {
        names.push("model_training".to_string());
    }
    if a.attribution {
        names.push("public_attribution".to_string());
    }
    // validate_scopes always adds the floor and orders per VALID_SCOPES;
    // these names are already known-valid, so this cannot fail.
    validate_scopes(&names).expect("scope names derived from ConsentAnswers are always valid")
}

/// Print the plain-language consent menu and read one y/N answer per
/// optional scope from `input`. `debugging_evaluation` is always on and is
/// not prompted for.
pub fn prompt_consent_answers(
    input: &mut impl std::io::BufRead,
    output: &mut impl std::io::Write,
) -> Result<ConsentAnswers> {
    writeln!(
        output,
        "How may your submitted traces be used? (you can revoke submitted traces later)"
    )?;
    writeln!(
        output,
        "  Debugging and evaluation                 [always on]"
    )?;
    writeln!(output, "  Benchmark generation                     [y/N]")?;
    let benchmark = read_yes_no(input)?;
    writeln!(output, "  Ranking-model training                   [y/N]")?;
    let ranking = read_yes_no(input)?;
    writeln!(output, "  Model training                           [y/N]")?;
    let training = read_yes_no(input)?;
    writeln!(output, "  Public attribution of your handle        [y/N]")?;
    let attribution = read_yes_no(input)?;

    Ok(ConsentAnswers {
        benchmark,
        ranking,
        training,
        attribution,
    })
}

/// Read one line from `input` and interpret it as a yes/no answer: `y`, `Y`,
/// or `yes` (case-insensitive) is true; anything else (including an empty
/// line) is false.
fn read_yes_no(input: &mut impl std::io::BufRead) -> Result<bool> {
    let mut line = String::new();
    input.read_line(&mut line)?;
    let trimmed = line.trim();
    Ok(trimmed.eq_ignore_ascii_case("y") || trimmed.eq_ignore_ascii_case("yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_map_to_scopes() {
        assert_eq!(
            scopes_from_answers(ConsentAnswers::default()),
            vec!["debugging_evaluation".to_string()]
        );
        let all = ConsentAnswers {
            benchmark: true,
            ranking: true,
            training: true,
            attribution: true,
        };
        assert_eq!(
            scopes_from_answers(all),
            vec![
                "debugging_evaluation".to_string(),
                "benchmark_only".to_string(),
                "ranking_training".to_string(),
                "model_training".to_string(),
                "public_attribution".to_string()
            ]
        );
    }

    #[test]
    fn prompt_reads_yes_no_answers_without_tty() {
        let mut input = std::io::Cursor::new(b"y\nn\nyes\n\n".to_vec());
        let mut output = Vec::new();
        let a = prompt_consent_answers(&mut input, &mut output).unwrap();
        assert!(a.benchmark && !a.ranking && a.training && !a.attribution);
        let printed = String::from_utf8(output).unwrap();
        assert!(printed.contains("How may your submitted traces be used?"));
        assert!(printed.contains("Model training"));
    }

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

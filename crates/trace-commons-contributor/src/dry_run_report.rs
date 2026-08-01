//! Human-readable dry-run formatting. The report receives only envelope
//! metadata: tier, warnings, labels, and counts. Trace content never enters
//! this module.

use trace_commons_protocol::trace_contribution::ResidualPiiRisk;

use crate::submit::SubmitOutcome;

pub fn session(outcome: &SubmitOutcome) -> Option<String> {
    let SubmitOutcome::DryRun {
        submission_id,
        bytes,
        risk,
        warnings,
        redaction_counts,
        pii_labels_present,
    } = outcome
    else {
        return None;
    };

    let mut lines = vec![format!(
        "dry-run: submission_id={submission_id} bytes={bytes} risk={}",
        risk.as_str()
    )];

    if warnings.is_empty() {
        lines.push("  warnings: none".to_string());
    } else {
        lines.extend(
            warnings
                .iter()
                .map(|warning| format!("  warning: {warning}")),
        );
    }

    if redaction_counts.is_empty() {
        lines.push("  redaction-counts: none".to_string());
    } else {
        lines.extend(
            redaction_counts
                .iter()
                .map(|(label, count)| format!("  redaction-count: {label}={count}")),
        );
    }

    if pii_labels_present.is_empty() {
        lines.push("  pii-labels: none".to_string());
    } else {
        lines.extend(
            pii_labels_present
                .iter()
                .map(|label| format!("  pii-label: {label}")),
        );
    }

    lines.push(storage_line(*risk).to_string());
    lines.push(
        "  server re-scrub: The server can raise this risk and only lowers it after a proven-complete re-scrub."
            .to_string(),
    );
    Some(lines.join("\n"))
}

fn storage_line(risk: ResidualPiiRisk) -> &'static str {
    match risk {
        ResidualPiiRisk::Low => {
            "  storage: Low risk does not trigger risk-based quarantine; server checks can still hold the trace before corpus storage."
        }
        ResidualPiiRisk::Medium => {
            "  storage: Medium risk is quarantined unless the server accepts medium-risk submissions; other server checks can still hold it."
        }
        ResidualPiiRisk::High => "  storage: High risk is quarantined for privacy review.",
    }
}

pub fn summary(outcomes: &[SubmitOutcome]) -> String {
    let mut low = 0usize;
    let mut medium = 0usize;
    let mut high = 0usize;
    let mut refused = 0usize;
    let mut skipped = 0usize;
    let mut already_submitted = 0usize;
    let mut submitted = 0usize;
    let mut failed = 0usize;

    for outcome in outcomes {
        match outcome {
            SubmitOutcome::DryRun { risk, .. } => match risk {
                ResidualPiiRisk::Low => low += 1,
                ResidualPiiRisk::Medium => medium += 1,
                ResidualPiiRisk::High => high += 1,
            },
            SubmitOutcome::AlreadySubmitted { .. } => already_submitted += 1,
            SubmitOutcome::SkippedParseFailure { .. } => skipped += 1,
            SubmitOutcome::Refused { .. } => refused += 1,
            SubmitOutcome::Failed { .. } => failed += 1,
            SubmitOutcome::Submitted { .. } => submitted += 1,
        }
    }

    format!(
        "dry-run summary: low={low} medium={medium} high={high} refused={refused} \
         skipped={skipped} already-submitted={already_submitted} submitted={submitted} \
         failed={failed} total={}",
        outcomes.len()
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use uuid::Uuid;

    use super::*;

    fn dry_run(risk: ResidualPiiRisk) -> SubmitOutcome {
        SubmitOutcome::DryRun {
            submission_id: Uuid::nil(),
            bytes: 321,
            risk,
            warnings: vec!["Canonical envelope warning.".to_string()],
            redaction_counts: BTreeMap::from([
                ("private_email".to_string(), 2),
                ("secret:api_key".to_string(), 1),
            ]),
            pii_labels_present: vec!["private_email".to_string()],
        }
    }

    #[test]
    fn session_formats_only_safe_envelope_evidence() {
        let rendered = session(&dry_run(ResidualPiiRisk::High)).unwrap();
        assert_eq!(
            rendered,
            concat!(
                "dry-run: submission_id=00000000-0000-0000-0000-000000000000 bytes=321 risk=high\n",
                "  warning: Canonical envelope warning.\n",
                "  redaction-count: private_email=2\n",
                "  redaction-count: secret:api_key=1\n",
                "  pii-label: private_email\n",
                "  storage: High risk is quarantined for privacy review.\n",
                "  server re-scrub: The server can raise this risk and only lowers it after a proven-complete re-scrub."
            )
        );
    }

    #[test]
    fn session_formats_empty_evidence_without_inventing_findings() {
        let outcome = SubmitOutcome::DryRun {
            submission_id: Uuid::nil(),
            bytes: 0,
            risk: ResidualPiiRisk::Low,
            warnings: Vec::new(),
            redaction_counts: BTreeMap::new(),
            pii_labels_present: Vec::new(),
        };
        let rendered = session(&outcome).unwrap();
        assert!(rendered.contains("risk=low"));
        assert!(rendered.contains("  warnings: none"));
        assert!(rendered.contains("  redaction-counts: none"));
        assert!(rendered.contains("  pii-labels: none"));
        assert!(rendered.contains("Low risk does not trigger risk-based quarantine"));
    }

    #[test]
    fn summary_counts_every_dry_run_outcome_with_a_denominator() {
        let outcomes = vec![
            dry_run(ResidualPiiRisk::Low),
            dry_run(ResidualPiiRisk::Medium),
            dry_run(ResidualPiiRisk::High),
            SubmitOutcome::Refused {
                reason_label: "secret-leak-detected".to_string(),
            },
            SubmitOutcome::SkippedParseFailure {
                reason_label: "parse-failed".to_string(),
            },
            SubmitOutcome::AlreadySubmitted {
                submission_id: Uuid::nil(),
                prior_status: "accepted".to_string(),
            },
            SubmitOutcome::Failed {
                reason_label: "claim-mint-failed".to_string(),
            },
            SubmitOutcome::Submitted {
                submission_id: Uuid::nil(),
                status: "accepted".to_string(),
            },
        ];

        assert_eq!(
            summary(&outcomes),
            "dry-run summary: low=1 medium=1 high=1 refused=1 skipped=1 already-submitted=1 submitted=1 failed=1 total=8"
        );
    }
}

# Validation evidence

Structural run finished UTC: `2026-11-06T23:26:58.557479+00:00`.

Quint `0.33.0`, Node `v26.13.0`, Python `3.12.8`.

**14 models typechecked; 155 frame/control projections matched; 1,792 sampled traces and 180,992 sampled states passed view validation.**

Random simulation is structural validation, not a safety proof. The [complete induction results](proofs/results.md) separately establish all 13 selected model hypotheses. [validation.json](validation.json) records model/view/frame hashes, exact commands and unobserved actions/frames.

| Model | Traces | Sampled states | Observed / declared frames | Unobserved actions |
| --- | ---: | ---: | ---: | ---: |
| client_consent | 128 | 12,928 | 4 / 4 | 4 |
| client_contribution | 128 | 12,928 | 11 / 16 | 31 |
| client_learning | 128 | 12,928 | 14 / 18 | 19 |
| client_providers | 128 | 12,928 | 8 / 8 | 12 |
| client_services | 128 | 12,928 | 16 / 19 | 18 |
| client_shell | 128 | 12,928 | 18 / 18 | 13 |
| end_to_end | 128 | 12,928 | 9 / 12 | 48 |
| platform_clients | 128 | 12,928 | 15 / 15 | 4 |
| server_identity | 128 | 12,928 | 7 / 8 | 7 |
| server_interfaces | 128 | 12,928 | 8 / 9 | 9 |
| server_lifecycle | 128 | 12,928 | 8 / 8 | 0 |
| server_pipeline | 128 | 12,928 | 5 / 9 | 15 |
| server_receipt | 128 | 12,928 | 5 / 5 | 4 |
| server_settlement | 128 | 12,928 | 5 / 6 | 19 |

## Source and native validation

The original model seed and initial correspondence review are preserved in the JSON. Current repaired native source hashes, complete native-suite results, proof certificates and source regressions are recorded in [the evidence manifest](proofs/evidence/manifest.json). This is a source-review argument plus tests, not executable-source equivalence. The source inventory retains explicit gaps.

The native regression suite has 24 deterministic tests. The original freshness-guard bypass produced 55 assertion failures; the initial-owner regression separately failed before its fix. Restored focused tests passed. Callback tasks are awaited directly, without a drain sleep. A separate reviewer approved the final native repair.

During integration, these failures were diagnosed and resolved:

- `ContributionAccountTests.testResponseForOldLifecycleIsDiscardedDespiteSameEnrollment`: its fixture parked every RPC, including new background grant/history reads, starving the cooperative pool. The fixture now parks only its account operation, returns valid empty background frames, and bounds its wait. All four account tests pass.
- `FirstRunDaemonAdaptorTests.test_anUnconfirmedGrantCarriesALabelTheCoreKnows`: supplied a valid owner-status fixture so the test reaches its intended grant refusal; the precise refusal-label assertion is preserved.
- `MonitorNavigationTests.test_theStatusReadRecordsItsFailure`: replaced the obsolete implementation-string assertion with an actual current status failure/recovery regression.
- `CredentialRenewalTests.testExpiredSessionRetainsKeyAndOffersSignInWithoutAChooser` and `InferenceDuringOnboardingTests.testSignInRendersOnInferenceWhileOnboardingIsRequired`: failed in the restricted rendering runtime. Both unchanged image/OCR tests passed with normal approved AppKit/Vision access. Final native-suite status uses that environment.

Inventory self-tests/comparison, ITF decoder controls, proof-runner negative controls and generated JavaScript parsing are separate checks. Historical counterexample charts are retained under `proofs/historical`; they are not displayed as current-model statecharts. HTML frames do not establish native visual fidelity.

No application dependency or Rust production source changed. The exact current FFI build and four license-boundary tests passed. Merge, release and deployment remain separate operations.

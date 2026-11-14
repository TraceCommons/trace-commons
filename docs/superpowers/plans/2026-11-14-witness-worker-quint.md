# Witness Worker Quint Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans. The user explicitly requested this extension to the approved witness retry policy; no new design approval is needed.

**Goal:** Model the actual background witness queue and check retry/circuit/scheduling safety, with reproducible regression evidence.

**Architecture:** Add a focused companion under `docs/flows/trace-commons/worker/`, linked from the existing contribution model. Keep the existing UI-flow models and P1–P13 proof artifacts intact. Four trace identities, two background slots, and a three-entry queue exercise the three-failure threshold plus healthy/displaced work. Queue capacity is an explicit abstraction of 64, not a production constant change.

**Tech Stack:** Installed Quint 0.33.0, Python standard library, Apalache 0.62.1/Java 21.

**Spec:** `docs/superpowers/specs/2026-11-14-witness-retry-fairness-design.md` and source at `fb16129e425752565fbfb43ed4ddd38a3cd27959`.

## Constraints and review focus

- Model source behavior: terms equality is not a generation token; automatic reapproval may recreate the same identity.
- Include the existing watchdog's ineffective first-attempt deferral, rather than invent its intended fix.
- Keep snapshot refresh, capacity overlap, probe ownership, source/consent gates, and durable retry state distinct.
- Priority safety is not unconditional progress. No liveness claim without explicit environmental assumptions.
- Use observation-only invariants and deliberate negative controls. Do not weaken the model step to make checks pass.
- No product code, dependency, UI text, or previous promise-selection changes.

## Tasks

- [x] Write deterministic positive and negative regression traces; implement the source-grounded worker model and observation predicates.
- [x] Add a reproducible validation runner with version/hash/log evidence and honest bounded/sampled outcomes.
- [x] Run typecheck, model tests, sampled safety checks, bounded symbolic checks, and existing model-tool unit tests. Resolve model defects against source.
- [x] Independently review source correspondence and evidence, document bounds/results, commit and update draft PR #2138.

## Review and validation

The first review verified all retained hashes and the induction certificate, but found the worker projection overallowed automatic reapproval after a trust hold. The model now retains that hold and requires manual approval; an additional negative control rejects bypassing it. Successful pinning now clears witness retry state exactly as the queue does. Final regression coverage is 35 cases, including six negative controls.

Full-step induction supplements sampled/bounded checking. It checks original initialization, preservation by the complete original step, implication to the original safety predicate, and an expected counterexample to false for nonvacuity. It uses exact map domains and unrestricted scalar types; no extra numerical clipping or omitted transition. Earlier bounded searches timed out and remain explicitly incomplete.

After the review fixes, all 35 model regressions, 1,000 sampled traces, and four full-step induction obligations passed. Updated arbitrary-state coverage includes 151 scalar leaves and all four exact-key maps.

Focused independent review verified the corrected model and all current evidence hashes, confirmed all findings closed, and found no new blockers within the documented model-safety scope.

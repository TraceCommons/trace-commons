# Witness retry fairness

Implement the policy approved in conversation on 2026-11-14. One failed trace must not stop unrelated approved traces, and nothing may upload without its own verified witness review.

## Policy

- Keep per-trace approval and retries indefinitely. Witness transient failures back off at nominal 1, 2, 5, 15, then 60 minutes, with equal jitter (half to full delay). Existing non-witness retry policies stay separate.
- Scheduling priority: explicit user approvals, then automatic traces not waiting on a retry, then automatic retries. A retry is an attempted entry that still holds a retry deadline; `attempts` never resets, so a reapproved or pinned trace counts as fresh work again (amended 2026-11-14, after #2138). Order retries fairly using their ready time before source age. Apply this to the worker's already queued jobs as well as each producer pass.
- Correlated service failure means three distinct trace failures in one run: no successful review between them, and each failing request dispatched no more than five minutes after the previous failure ended. Measuring the gap to the next dispatch, not to its failure, lets a witness that hangs every request past the 570-second review timeout still open the circuit (amended 2026-11-14, after #2138; the original rule compared failure end times to a five-minute window and could never trip on timeouts). `witness_redaction_failed` does not count toward the circuit: it can be caused by one trace's content. It is still retried per trace. One trace repeated never trips the shared circuit. The circuit pauses for 30, 60, 120, 240, then 300 seconds on successive failed probes. After each pause allow one probe, preferring a different already-approved trace. A successful certified review clears the circuit and permits the existing two concurrent jobs.
- Witness saturation is separate: honor the server Retry-After plus the existing per-entry jitter, independent of that trace’s earlier attempts, then resume with one job. Failure outcomes and pre-existing in-flight completions cannot erase an active server delay.
- Preserve classifier-unavailable and redaction-failed response labels separately from connection failures, timeouts, capacity, and permanent refusal. Preserve current health copy; this work adds no notices.
- Permanent oversized/unclassifiable/trust refusals stay permanent or review-required according to existing rules. No automatic approval, relaxed attestation, changed grants, or bypassed privacy controls.
- In-memory circuit state resets on app restart; persisted entry retry deadlines remain effective. Wall-clock observations after sleep must not move backward.

## Approach

Use a small deterministic retry/circuit policy module, integrated into the existing witness worker and queue ordering. This retains the current two-slot worker, existing queue persistence, and upload gates. A global pause after every failure caused the observed starvation; removing all shared pacing would overload a genuinely unavailable service. Correlated failures plus a single recovery probe provide both progress and bounded outage traffic.

## Verification

Regression tests cover isolated repeated failure with healthy work behind it, distinct-failure threshold/window, one half-open probe and a different trace when available, successful recovery, server delay overlap, retry fairness including late manual approvals, persisted deadline/restart behavior, precise transport labels, and permanent refusals. Run the contributor/protocol suites, targeted server licensing boundary, strict clippy and formatting; rebuild the FFI and run the native shell suite before completion.

## Formal-model coverage

The existing `docs/flows/trace-commons/models/client_contribution.qnt` covers the bounded contribution lifecycle for two traces. The new companion `docs/flows/trace-commons/worker/witness_worker.qnt` adds four trace identities, independent entry deadlines, saturation/circuit state, captured approvals and overlapping completions. Its own regression tests, sampled checks and full-step model safety induction do not change the lifecycle induction proofs and do not establish unconditional scheduler fairness. See the companion README for source correspondence, known watchdog behavior, approval-identity limits and exact validation bounds.

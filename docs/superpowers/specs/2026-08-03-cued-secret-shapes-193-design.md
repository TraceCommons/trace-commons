# Cued-secret shape decisions (#193)

**Date:** 2026-08-03  
**Status:** implemented  
**Issue:** [#193](https://github.com/TraceCommons/trace-commons-server/issues/193)

Decision matrix for the residual shapes #187 explicitly deferred. House rule from `docs/superpowers/plans/2026-07-09-secret-redaction-hardening.md`: over-redaction is acceptable, under-redaction is a defect; when entropy is ambiguous, redact.

## Decisions

| # | Shape | Decision | Rationale |
|---|---|---|---|
| 1 | Zero-separator glue (`BearerSECRET`) | **Accept, documented** | No separator → no boundary without splitting inside identifiers. FP cost too high. |
| 2 | Sub-16-character cued opaque | **Redact** | Cue + opaque value is strong enough; floor lowered 16→8. Below 8 still too noisy. |
| 3 | UUID-shaped even when cued | **Keep allowlisted** | ~105k structural IDs vs ~20 real secrets in the prototype scan. |
| 4 | Lowercase hex ≥32 when cued | **Redact** | Allowlist narrowed to the uncued case. HMAC/AES material has this shape; real hashes are rarely cue-adjacent. Short pure-hex 7–8 (git short SHA) stays allowlisted even when cued. |
| 5 | Sub-threshold entropy / spaced padding | **Redact** (windowed entropy from #187) | Already covered; not re-litigated here. |
| 6 | k8s/env `{name,value}` array | **Redact** | Cue lives in the sibling `name` string; key-name and text cue-window never see it. |
| 7 | JSON key-name cue | **Redact** | Already handled by `redact_sensitive_json`; fixtures pin it. |

## FP budget

Rows 2 and 4 loosen controls that exist specifically to prevent over-redaction. Fixtures in `contextual_entropy_fp_budget_for_cued_shape_changes` pin the cost:

- Uncued content hashes / shas survive (row 4 only removes the *cued* hex allowlist).
- UUID and prefixed structural IDs survive even when cued.
- Git short SHAs (7–8 hex) survive even when cued.
- Low-entropy short values after a cue survive (length floor moved; entropy floor did not).
- Sub-8 tokens and uncued short opaque tokens survive.

If a future change makes any of those fire, the FP cost has moved and needs an explicit decision — do not delete the fixture to go green.

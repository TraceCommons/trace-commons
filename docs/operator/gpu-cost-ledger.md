# GPU Cost Ledger

Append-only running ledger of GPU instance spend for trace-commons-server
bake-offs and corpus rebuilds. Each row records one provisioned
instance's run. Do not back-edit existing rows — append a new row
if a prior estimate needs correction.

Referenced from
[`./a26-bakeoff-result-handler.md`](./a26-bakeoff-result-handler.md)
(cost recording step) and
[`./hf-dataset-cache-hygiene.md`](./hf-dataset-cache-hygiene.md)
(cost note).

## Columns

`date | run-id | hardware | hours | cost | purpose`

- `date` — UTC date the instance was provisioned (YYYY-MM-DD).
- `run-id` — short identifier (Axx slice id or descriptive label).
- `hardware` — provider + instance shape.
- `hours` — billed wall-clock hours, rounded to nearest 0.5.
- `cost` — USD, rounded to nearest dollar. `TBD` until teardown.
- `purpose` — one-line description (no operator secrets).

## Ledger

| date | run-id | hardware | hours | cost | purpose |
|---|---|---|---|---|---|
| 2026-05-13 | A2.3c | Lambda H100 SXM5 80GB | ~5 | ~$25 | model bake-off (boilerplate duplicate slice) |
| 2026-05-13 | A2.4 | Lambda H100 SXM5 80GB | ~5 | ~$25 | model bake-off (Wikipedia duplicate slice) |
| 2026-05-14 | A2.6 | Lambda H100 SXM5 80GB | 8.65 | ~$37 | model bake-off (agent-traces novel slice, 4 candidates; final) |
| 2026-05-14 to 2026-05-15 | A2.6 instance idle | Lambda H100 SXM5 80GB | ~20 | ~$86 | unintended: instance not torn down at bakeoff_complete; verified-and-terminated 2026-05-15 ~07:15 UTC. Lesson: always confirm termination via `curl /api/v1/instances` rather than trusting a verbal "torn down" |
| 2026-05-14 | A2.7 attempt 1 | Lambda H100 SXM5 80GB | ~1 | ~$5 | aborted: build-flag mistake (CPU-only build); see reports/2026-05-14-pr95-multimodal-hang-regression.md retraction |
| 2026-05-15 | A2.7 attempt 2 | Lambda H100 SXM5 80GB | ~6.2 | ~$27 | re-bake-off (Qwen 3.6 27B Dense only); AUC 0.9363 bit-identical to A2.6, per_trace_scores captured, calibration yielded floor=6246774 micros |

## Hash-only / no-secrets reminder

Instance IDs, provider account references, IP addresses, and
billing-portal links are operator-secret and must not appear in
this ledger. `purpose` is label-only; cost is a rounded scalar.

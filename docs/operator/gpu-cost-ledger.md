# GPU Cost Ledger

Append-only running ledger of GPU instance spend for tracedao-server
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
| 2026-05-14 | A2.6 | Lambda H100 SXM5 80GB | ~5 | ~$22 | model bake-off (agent-traces novel slice) |

## Hash-only / no-secrets reminder

Instance IDs, provider account references, IP addresses, and
billing-portal links are operator-secret and must not appear in
this ledger. `purpose` is label-only; cost is a rounded scalar.

# Local Ingest Verification — Operator Runbook

How to prove a locally-built `trace-commons-ingest` actually accepts,
redacts, stores, and routes envelopes — offline, with no PostgreSQL, no
GPU, no HuggingFace reachability, and no contributor enrollment.

This is the loop to run after a toolchain bump, a protocol-envelope
change, or any edit to the submit/quarantine path, before touching a
deployment. For the post-deploy checklist against a real environment see
[`smoke-test.md`](smoke-test.md); for the load-generation runbook see
[`pilot-bootstrap.md`](pilot-bootstrap.md).

## Why not just run `pilot-bootstrap-smoke.sh`

`scripts/operator/pilot-bootstrap-smoke.sh` binds one mock server that
plays **both** the HuggingFace Hub API **and** `/v1/traces`. It validates
the harness — wire protocol, sidecar accounting, idempotency math — and
never exercises a real ingest process. Passing that smoke says nothing
about whether the server accepts the envelope.

The harness reads the ingest base URL from `--target` and the Hub
endpoint from `HF_ENDPOINT`. Because those are independent, the mock can
be demoted to Hub-only duty on a second port while `--target` points at a
real server. That substitution is the whole technique below.

## Why the contributor CLI cannot do this

`trace-commons-contributor` only uploads with a short-lived issuer-minted
upload claim. Minting one against your own issuer requires a device-key
registry: `issue_claim_for_device_key` fails closed with
`device_key_registry_not_configured` without one
(`crates/trace-commons-server/src/trace_upload_claim_issuer.rs:1757`), and
`connect_from_config` only ever builds `PgBackend`
(`crates/trace-commons-server/src/db/mod.rs:1590`) — there is no SQLite
fallback in this crate.

So a local-only loop needs PostgreSQL plus an issuer plus an allowlist
instance entry. The pilot-bootstrap harness sidesteps all of it by
authenticating with a static tenant token, which is why it is the right
tool for verifying the server rather than the contributor experience.

## Prereqs

- Rust toolchain satisfying the workspace `rust-version` (1.92 or newer).
- `python3` and `curl` on PATH.
- No process already bound on the two loopback ports used below.

## Steps

### 1. Build

```bash
cargo build --bin trace-commons-ingest
cargo build --release --bin trace-commons-pilot-bootstrap
```

### 2. Start ingest with contributor and admin tokens

Zero-credit calibration semantics are mandatory for a verification run:
the harness is not a credit-issuing path.

```bash
TRACE_COMMONS_TENANT_TOKENS='bootstrap-tenant:contributor:dev-contrib-token;expires_at=2027-01-01T00:00:00Z,bootstrap-tenant:admin:dev-admin-token;expires_at=2027-01-01T00:00:00Z' \
TRACE_COMMONS_BIND='127.0.0.1:3907' \
TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=0 \
RUST_LOG=info \
  ./target/debug/trace-commons-ingest
```

Token entries are comma-separated; each is `tenant:role:token` with an
optional `;expires_at=<RFC3339>`. Roles parse per `TokenRole::parse` —
`contributor`, `admin`, `reviewer`, and the worker variants.

Confirm the listener:

```bash
curl -sS http://127.0.0.1:3907/health
```

Expect `status: ok` plus `schema_version`, `build_commit`, and
`build_version`.

### 3. Prime the hf-hub cache from checked-in fixtures

One `.jsonl` file is one session is one trace. The fixtures live in the
repo, so this step needs no network.

```bash
SOURCE='jedisct1/agent-traces-swival'
HF_CACHE=/tmp/tc-verify-hf-cache
COMMIT=smokecommit0000000000000000000000000001
SNAP="$HF_CACHE/datasets--${SOURCE//\//--}/snapshots/$COMMIT"
mkdir -p "$SNAP" "$HF_CACHE/datasets--${SOURCE//\//--}/refs"
printf '%s' "$COMMIT" > "$HF_CACHE/datasets--${SOURCE//\//--}/refs/main"
cp -f scripts/operator/fixtures/swival-smoke/*.jsonl "$SNAP"/
```

### 4. Start the mock as Hub-only, on a different port

Port 3908 here, so 3907 stays with the real server. Every fixture must be
advertised as a sibling or the harness enumerates nothing.

In `zsh`, build the sibling flags as an array — unquoted parameter
expansion does **not** word-split, and passing them as one string makes
the mock exit 2 with `unrecognized arguments`.

```bash
sibs=()
for f in "$SNAP"/*.jsonl; do sibs+=(--hf-sibling "${f:t}"); done
python3 scripts/operator/pilot-bootstrap-mock-server.py \
  --host 127.0.0.1 --port 3908 \
  --hf-dataset "$SOURCE" --hf-commit "$COMMIT" \
  "${sibs[@]}" &
```

### 5. Submit against the real server

```bash
TRACE_COMMONS_PILOT_TENANT_TOKEN='dev-contrib-token' \
HF_ENDPOINT='http://127.0.0.1:3908' \
  ./target/release/trace-commons-pilot-bootstrap \
    --source "$SOURCE" \
    --count 5 \
    --target http://127.0.0.1:3907 \
    --rate 5 \
    --sidecar /tmp/tc-verify-sidecar.jsonl \
    --cache-dir "$HF_CACHE"
```

### 6. Re-run unchanged to prove idempotency

Run step 5 a second time with no edits. The sidecar is append-only, so
rows double while distinct submission ids must not:

```bash
python3 -c "
import json
ids=[json.loads(l)['submission_id'] for l in open('/tmp/tc-verify-sidecar.jsonl')]
print('rows', len(ids), 'distinct', len(set(ids)))
"
```

### 7. Confirm server-side state

```bash
curl -sS -H 'Authorization: Bearer dev-admin-token' \
  'http://127.0.0.1:3907/v1/review/quarantine?limit=10'
```

The entry count is the authoritative stored-submission count for this
run. Note the route is `/v1/review/quarantine` — there is no
`/v1/review/queue`.

### 8. Optional: admin drills

Drills are `POST` with a JSON content type. Omitting the header returns
`415`, not a failure of the drill itself.

```bash
for D in audit-chain key-rotation retention-dry-run; do
  curl -sS -o /dev/null -w "$D %{http_code}\n" -X POST \
    -H 'Authorization: Bearer dev-admin-token' \
    -H 'Content-Type: application/json' -d '{}' \
    "http://127.0.0.1:3907/v1/admin/$D-drill"
done
```

## Passing criteria

| Check | Expected |
|---|---|
| `/health` | `status: ok`, `build_commit` matches your checkout |
| HTTP status per submission | `200` |
| Gate decision | `quarantined` under default dev config — see below |
| Sidecar after two runs of N | `2N` rows, `N` distinct submission ids |
| Quarantine queue entries | exactly `N` |
| `audit-chain` / `key-rotation` / `retention-dry-run` drills | `200` with a coherent evidence body |

## Reading the outcome

`quarantined` is the **correct** default result, not a defect. The
harness builds envelopes carrying message text, and the dev profile does
not set `TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS`, so Medium residual
risk routes to privacy review. A hosted environment with that flag
enabled returns `accepted` for the same envelope. Treat a run that
reports `accepted` locally as a signal that the flag leaked into your dev
environment.

Known-good divergences that are not bugs:

- `vector-index` drill returns `503 requires configured DB mirror`.
  Expected: no PostgreSQL, so no mirror.
- `canary-read` drill returns `422 missing field submission_id`. It needs
  a target submission in the body.
- Contributor `GET /v1/traces` may list fewer rows than the quarantine
  queue. The listing applies its own filtering and pagination; use the
  admin quarantine count as the authoritative figure.

## Cleanup

```bash
pkill -f pilot-bootstrap-mock-server.py
kill "$(pgrep -f 'trace-commons-ingest')"
rm -rf /tmp/tc-verify-hf-cache /tmp/tc-verify-sidecar.jsonl
```

Submitted envelopes persist in the dev artifact store under
`~/.ironclaw/trace_commons_ingest/tenants/`. Remove that tree to reset
between verification runs; keep it if you intend to exercise the review
or revocation paths against the same submissions.

# trace-commons-contributor

The contributor-facing CLI for submitting local coding-agent traces to a
Trace Commons instance. It runs entirely on the contributor's own machine:
it discovers local Claude Code and Codex session files, redacts them
locally, and only then uploads the redacted envelope to the instance the
contributor enrolled with. Nothing leaves the machine until the contributor
explicitly runs `submit`.

## Install

For now, build from source:

```bash
cargo build --release -p trace-commons-contributor
./target/release/trace-commons-contributor --help
```

Prebuilt GitHub Releases binaries are a follow-up; not available yet.

## Quickstart

1. Run `login` with no `--grant` to print this device's key id:

   ```bash
   trace-commons-contributor login
   # device_key_id: <hex>
   # give this to your instance to mint an enrollment grant, then re-run `login --grant <grant>`
   ```

2. Give that `device_key_id` to whoever operates your Trace Commons
   instance. They mint an enrollment grant (see "mint-grant" below) and
   hand you back a base64 blob.

3. Enroll with the grant:

   ```bash
   trace-commons-contributor login --grant <base64-grant>
   ```

   This saves your instance's `issuer_url`, `ingest_url`, `tenant_id`, and
   `device_key_id` to local config. Pass `--allowed-hosts <csv>` to pin
   which hosts this device will ever talk to; it persists into the config
   so every later command enforces it too.

4. See what would be submitted, then submit:

   ```bash
   trace-commons-contributor list
   trace-commons-contributor submit --dry-run --since 7d
   trace-commons-contributor submit --since 7d
   ```

## Consent model (v1 scope)

- Every device-key claim issued in v1 is server-capped to the
  `debugging_evaluation` consent scope. Envelopes this CLI produces carry
  `debugging_evaluation` and only the `debugging` / `evaluation` allowed-use
  labels — nothing broader.
- Broader consent scopes (e.g. `model_training`) are explicit server-side
  follow-up work; this CLI does not attempt to request them, and the issuer
  will not grant them to a device-key claim today.
- Local secret redaction (deterministic, via the shared protocol crate) runs
  on every session before it ever reaches the network. It replaces secrets
  and file paths with stable placeholders; it never sends the raw content
  out for scrubbing.
- An optional second pass, `--pii-filter near-ai`, sends the
  already-locally-redacted text through a NEAR AI Cloud (TEE-hosted) PII
  filter for a second opinion. It requires `TRACE_NEAR_AI_PRIVACY_API_KEY`
  to be set; `TRACE_NEAR_AI_PRIVACY_BASE_URL` and
  `TRACE_NEAR_AI_PRIVACY_MODEL` are optional overrides. This path is
  fail-closed: if the filter is requested but unreachable or misconfigured,
  or if an unknown `--pii-filter` value is given, the batch is refused
  rather than silently uploaded unfiltered.
- Once per batch, a synthetic privacy-filter canary is run through the
  active redactor before any real session is uploaded. If the canary
  values survive redaction, the whole batch aborts — this catches a broken
  or disabled filter before it can leak anything.
- The server applies its own rescrub pass on top of whatever the client
  sends; local redaction is a first line of defense, not the only one.

## Local state

All local state lives under one directory (default:
`$XDG_CONFIG_HOME/trace-commons`, i.e. `~/.config/trace-commons` on Linux
and the platform config dir elsewhere; override with
`TRACE_COMMONS_CONTRIBUTOR_DIR` or `--config-dir`). The directory is
created mode `0700` on unix; every file in it is `0600`:

- `contributor.json` — issuer/ingest URLs, tenant id, device key id, consent
  scopes, PII filter choice, allowed-hosts pin. No secrets.
- `device.pk8` — this device's Ed25519 keypair, PKCS#8 DER. Never leaves the
  machine; only its public key id is ever sent to the server.
- `receipts.jsonl` — one hash-only line per submission: submission id,
  session hash, source, timestamp, status. Never a path or trace content.

`logout` deletes all three files and sweeps any orphaned atomic-write temp
files left behind by a crash mid-write.

## Sources

- **Claude Code** — `~/.claude/projects/**/*.jsonl`.
- **Codex** — `~/.codex/sessions/**/rollout-*.jsonl`.

Both readers drop `thinking`/`reasoning` content entirely; unknown record
types are kept only as a record-type-only marker (no payload). Full local
file paths are never included in an uploaded envelope — only what the
redactor and mapper produce from message content.

## Subcommands

| Command | What it does |
|---|---|
| `login [--grant <b64>] [--allowed-hosts <csv>]` | Without `--grant`, prints this device's key id to hand to an instance operator. With `--grant`, redeems an enrollment grant and saves local config. |
| `list` | Lists discoverable local sessions from all sources (no network). |
| `submit [--all] [--since <dur>] [--project <path>] [--source claude-code\|codex] [--yes] [--dry-run] [--pii-filter near-ai]` | Redacts and uploads selected sessions. `--dry-run` runs the full pipeline (parse, redact, canary check, sizing) without uploading. `--yes` skips the interactive picker confirmation. |
| `status` | Shows server-side status of previously submitted sessions from the local receipts log. |
| `whoami` | Prints local identity (instance id, tenant id, device key id, hashed user subject, config dir). No network call; never prints the raw subject. |
| `logout` | Deletes local config, device key, and receipts, plus orphaned temp files. |
| `mint-grant --instance-key-pem <path> --instance-id <id> --user-subject <subject> --audience <aud> --issuer-url <url> [--device-key-id <id>] [--ttl-seconds <secs>]` | Operator/dogfood tool: signs an enrollment grant with an instance private key (PEM) and prints it base64 to stdout for a contributor to redeem with `login --grant`. |

## Operator flow: `mint-grant`

`mint-grant` is how an instance operator (or a solo dogfooder acting as
their own operator) issues enrollment grants without standing up a full
enrollment UI. It signs a short-lived (`--ttl-seconds`, default 300)
attestation binding a `user_subject` and `instance_id` to a device key,
using the instance's own Ed25519 private key (PEM, PKCS8). The output is a
base64 blob the contributor redeems with `login --grant`. If
`--device-key-id` is omitted, it binds to the local device key of whoever
ran `mint-grant` — useful for dogfooding where operator and contributor are
the same person on the same machine.

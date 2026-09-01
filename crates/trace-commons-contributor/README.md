# trace-commons-contributor

The contributor-facing CLI for submitting local coding-agent traces to a
Trace Commons instance. It runs entirely on the contributor's own machine:
it discovers local Claude Code and Codex session files, redacts them
locally, and only then uploads the redacted envelope to the instance the
contributor enrolled with. Nothing leaves the machine until the contributor
explicitly runs `submit`.

## Install

Signed binaries are published per release. The shell installer covers macOS
and Linux, the PowerShell installer covers Windows, and there is a Homebrew tap
for macOS:

```bash
# macOS, Linux
curl -fsSL https://raw.githubusercontent.com/TraceCommons/trace-commons/main/scripts/install.sh -o install.sh
sh install.sh

# macOS, via the tap
brew tap TraceCommons/tap
brew trust tracecommons/tap
brew install trace-commons-contributor
```

```powershell
# Windows
irm https://raw.githubusercontent.com/TraceCommons/trace-commons/main/scripts/install.ps1 -OutFile install.ps1
.\install.ps1
```

Neither installer will place a binary it cannot verify: the published checksum
must match, and where the platform carries a signature — Developer ID on macOS,
Authenticode on Windows — it must be valid *and* name our identity. There is no
flag to skip either check. The Linux binary is unsigned, so the checksum is the
only check available for it. https://tracecommons.ai/install/ and the
[root README](../../README.md#contributor-cli) carry the rest — the per-release
download table, checking a signature by hand, and the desktop app. A winget
package is generated on each release but is not published in `winget-pkgs`
yet, so `winget install` does not work today. To build from source instead:

```bash
cargo build --release -p trace-commons-contributor
./target/release/trace-commons-contributor --help
```

## Uninstall

`logout` first — it stops a running daemon before wiping the credentials that
daemon uploads with — then remove the binary the way you installed it, and the
now-empty state directory. Withdraw anything you want withdrawn *before*
logging out: uninstalling removes local state, not submitted traces, and
`daemon withdraw` needs the account session that `logout` deletes. The
[root README](../../README.md#uninstalling) has the per-platform commands,
including the autostart registrations (systemd user unit, macOS login item,
Windows startup task or Run key) that removing the program does not take with
it.

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

## Consent model

- The instance's onboarding policy template sets a **ceiling**: it lists the
  consent scopes device-key claims from that instance are allowed to carry.
  Nothing this CLI requests can exceed that ceiling; the issuer enforces it
  server-side regardless of what the contributor picks locally.
- At `login`, the contributor picks which of those scopes to grant, up to
  the instance ceiling:
  - Interactively, when no `--scopes` flag is given and stdin is a
    terminal: `login` prints a plain-language menu (`Debugging and
    evaluation` is always on; `Benchmark generation`, `Ranking-model
    training`, `Model training`, and `Public attribution of your handle`
    are each a y/N prompt) and stores the resulting scope set.
  - Non-interactively, via `login --scopes debugging_evaluation,model_training`
    (a CSV of wire-name scopes). Unknown scope names are rejected before any
    network call, naming the valid set.
  - With no `--scopes` and no terminal (e.g. CI), `login` falls back to the
    `debugging_evaluation` floor only.
  - With `login --default`, the menu is skipped and its default answer (no)
    is taken for every optional scope, leaving the same
    `debugging_evaluation` floor. This is for agents and other scripted
    callers, which usually *do* have a terminal on stdin and so would
    otherwise sit at the prompt forever. To grant more than the floor
    non-interactively, use `--scopes` instead; the two flags conflict.
- The scopes actually written to `contributor.json` are only ever what the
  contributor chose; the envelopes this CLI produces carry whatever the
  server granted for that device-key claim (the intersection of the
  contributor's choice and the instance ceiling), and `status` shows the
  per-trace granted scopes for each submission.
- Local secret redaction (deterministic, via the shared protocol crate) runs
  on every session before it ever reaches the network, and covers
  *everything* in the envelope: message text, tool-call and tool-result
  content, and structured tool payloads alike. It never sends the raw
  content out for scrubbing. It is layered:
  1. **Known-pattern scrubbing.** Named regexes for the common secret
     shapes: OpenAI/Anthropic `sk-`, GitHub `ghp_`/`gho_`/`ghu_`/`ghs_`,
     AWS `AKIA...`, Slack/GitLab/Stripe-style provider tokens (`xoxb-`,
     `glpat`, `rk`/`pk`), three-segment JWTs (`eyJ...eyJ...`), npm `npm_`
     tokens, Google `AIza...` API keys, and whole PEM private-key blocks
     (`-----BEGIN ... PRIVATE KEY-----` through the trailing body, even
     when the `-----END` marker is missing or truncated). Matches are
     replaced with stable placeholders (`[REDACTED]`,
     `<REDACTED_PRIVATE_KEY>`).
  2. **Cue-gated high-entropy catch-all.** Secret formats change faster
     than any fixed pattern list, so a second pass looks for opaque,
     high-Shannon-entropy tokens (>=16 chars, >=3.2 bits/char) that sit
     immediately after a secret-shaped cue word (`api_key:`, `Bearer `,
     `password=`, `token:`, ...). This is deliberately *cue-gated*, not a
     blanket entropy scan: an ungated scan over real transcripts flags on
     the order of 100k+ tokens (message ids, base64 blobs, UUIDs, content
     hashes) for every real secret it catches, which makes plain entropy
     scanning useless in practice. Structural identifiers (UUIDs, known ID
     prefixes like `msg_`/`req_`/`toolu_`, content hashes) are allowlisted
     so they are never mistaken for opaque secrets.
  3. **Per-session fail-closed leaked-token guard.** After the two passes
     above, the redactor re-scans its own output for anything
     key-shaped. Redaction is fail-closed: if a secret is still detected
     in the finished envelope, that session is refused rather than
     uploaded — the CLI does not silently ship a partially-redacted
     session.
  4. **NEAR AI (optional add-on).** A separate, optional pass for prose
     PII (not secrets) — see below.

  This layering is validated against every real local Claude Code session
  transcript, 992 of them at last run, by a developer-only `#[ignore]`d
  harness in `tests/local_redaction_audit.rs` (never run in CI). Pattern-shaped
  secrets (API keys, GitHub/npm/Google/provider tokens, JWTs, PEM key blocks)
  and cue-gated high-entropy candidates all show zero survivors in the
  post-redaction envelope: 1003 cue-gated tokens, 243 `sk-` keys, and 154 PEM
  private keys found and removed on the most recent run.

  **Known gap: opaque bearer tokens (narrowed by #193).** Production covers
  `Bearer ` values through the cue-gated entropy pass. After #193 the pass
  redacts cued short opaque tokens (8–15 chars) and cued lowercase-hex ≥32
  (HMAC/AES-shaped material that the old content-hash allowlist spared).
  Two deliberate survivals remain: UUID-shaped tokens stay allowlisted even
  when cued (~105k structural IDs vs ~20 real secrets in the prototype
  scan), and zero-separator glue (`BearerSECRET` with no space) is accepted
  as unfixable without splitting inside arbitrary identifiers. Low-entropy
  static credentials still fall under the 3.2 bits/char floor. The audit
  reports surviving bearer values as advisories so the residual stays
  visible. If a session pasted a bearer token of a surviving shape, assume
  it is still present and do not submit that session until reviewed.
- An optional second pass, `--pii-filter near-ai`, sends the
  already-locally-redacted **message text only** (`content`/
  `human_correction` fields — not structured tool payloads) through a NEAR AI
  Cloud (TEE-hosted) PII filter for a second opinion. Structured tool
  payloads are covered solely by the deterministic pass above, never by the
  NEAR AI pass. This path requires `TRACE_NEAR_AI_PRIVACY_API_KEY` to be
  set; `TRACE_NEAR_AI_PRIVACY_BASE_URL` and `TRACE_NEAR_AI_PRIVACY_MODEL`
  are optional overrides. It is fail-closed: if the filter is requested but
  unreachable or misconfigured, or if an unknown `--pii-filter` value is
  given, the batch is refused rather than silently uploaded unfiltered. The
  API key is read from the environment only; it is never written to
  `contributor.json` or any other local state file (no key at rest).
- The first time a batch runs with `--pii-filter near-ai` (or a saved config
  with that filter selected), the CLI prints a one-time notice that
  redacted-but-unscrubbed message text will be sent to NEAR AI under your
  API key, then records that the notice has been shown (a marker file in
  the local state directory) so later runs stay quiet.
- Once per batch, a synthetic privacy-filter canary is run through the
  active redactor before any real session is uploaded — including, when one
  is attached, the NEAR AI (or other) privacy-filter backend itself, not
  just the deterministic pass. If the canary values survive redaction
  through either stage, the whole batch aborts — this catches a broken,
  disabled, or no-op filter before it can leak anything.
- The server applies its own rescrub pass on top of whatever the client
  sends; that rescrub is deterministic (the same class of secret/path
  redaction as the client's first pass), not a NEAR AI-style PII pass. Local
  redaction is a first line of defense, not the only one.

## Local state

All local state lives under one directory (override with
`TRACE_COMMONS_CONTRIBUTOR_DIR` or `--config-dir`; otherwise
`$XDG_CONFIG_HOME/trace-commons`, i.e. `~/.config/trace-commons`, on Linux,
`~/Library/Application Support/trace-commons` on macOS, and
`%LOCALAPPDATA%\trace-commons` on Windows). The directory is created mode
`0700` on unix; every file in it is `0600`:

- `contributor.json` — issuer/ingest URLs, tenant id, device key id, consent
  scopes, PII filter choice, allowed-hosts pin. No secrets.
- `device.pk8` — this device's Ed25519 keypair, PKCS#8 DER. Never leaves the
  machine; only its public key id is ever sent to the server.
- `receipts.jsonl` — one hash-only line per submission: submission id,
  session hash, source, timestamp, status. Never a path or trace content.

The daemon keeps its settings, queue, projects, history, audit log, and the
account session beside those, in the same directory.

On Windows this is LocalAppData rather than the roaming AppData
`dirs::config_dir()` would give: roaming profiles copy that directory between
machines, and a device key is bound to one machine. An enrollment left in the
old roaming location by an earlier CLI is moved across on the next run.

`logout` deletes all of the above and sweeps any orphaned atomic-write temp
files left behind by a crash mid-write. It leaves the directory itself, so
remove that too if you are uninstalling.

## Sources

- **Claude Code** — `~/.claude/projects/<project>/*.jsonl` for ordinary
  sessions, plus `~/.claude/projects/<project>/<session-id>/subagents/*.jsonl`
  for subagent transcripts. Each subagent transcript is offered as its own
  session rather than folded into its parent. Only those two layouts are
  walked; other nested directories are ignored.
- **Codex** — `~/.codex/sessions/**/rollout-*.jsonl`.
- **Trajectory** — a [Letta Trajectory](https://github.com/letta-ai/trajectory)
  v1 file or directory of them, named explicitly with `--trajectory`. Covers
  any harness Letta adapts (Hermes, Letta Code, OpenClaw, OpenHands, Pi, Deep
  Agents). A path that does not exist is an error rather than an empty
  result. Without `--trajectory`, discovery is limited to two places: files
  named `*.trajectory.json`/`*.trajectory.jsonl` in the working directory,
  and any `.json`/`.jsonl` in the trajectory staging folder inside the
  contributor state directory — where `import-antigravity` writes. Anything
  else is invisible to `list` and `submit` until `--trajectory` names it.
- **Antigravity** — imported with `trace-commons-contributor import-antigravity`,
  not watched like the sources above. The Antigravity IDE must be running:
  its conversations are only readable through the local API it serves, not
  by reading its on-disk store directly. Only conversations the running
  instance has actually loaded are reachable this way, so open the relevant
  project in Antigravity before importing. `--project <path>` scopes the
  import to that project (default: the current directory); `--all` takes
  every conversation the running instance exposes. Conversations created
  before Antigravity's storage-format change are not listed by the API and
  are therefore not imported, even though the IDE can still open them.
  Imported conversations are staged, not submitted — run `submit`
  afterwards to redact and upload them. Staged files are discovered without
  `--trajectory`, so a later bare `submit` offers them: an import is not
  inert, and a run that fails partway still reports what it staged.

`trace-commons-contributor daemon` — the CLI's watcher — watches Claude Code,
Codex and Gemini CLI's conventional stores by default, with no declaration
step: it asks for `SourceRoots::conventional()` explicitly on startup. This
is different from the desktop shells (macOS, Windows, Linux), which watch
nothing for a source until the contributor has declared it on the roots
screen.

All readers capture model reasoning (Claude Code `thinking` blocks, Codex
`reasoning` items, Gemini CLI `thought` records, Letta Trajectory
`reasoning` records) as a distinct event type, redacted through the same
client-side pipeline as every other event. Pass
`--no-reasoning` to exclude it from a submission. Reasoning is the least
sanitized part of a transcript — it routinely quotes file contents verbatim
and restates values the model just read — so review what you are contributing
if the session touched sensitive material.

Unknown record types are kept only as a record-type-only marker (no payload).
Full local file paths are never included in an uploaded envelope — only what
the redactor and mapper produce from message content.

## Subcommands

| Command | What it does |
|---|---|
| `login [--grant <b64>] [--allowed-hosts <csv>]` | Without `--grant`, prints this device's key id to hand to an instance operator. With `--grant`, redeems an enrollment grant and saves local config. |
| `list [--trajectory <path>]` | Lists discoverable local sessions from all sources (no network). Trajectory sessions appear when `--trajectory` names a file or directory, and — without it — from the working directory's `*.trajectory.json(l)` files and the trajectory staging folder that `import-antigravity` writes to. |
| `submit [--all] [--since <dur>] [--project <path>] [--source claude-code\|codex\|trajectory] [--trajectory <path>] [--no-reasoning] [--remediate-quarantined] [--yes] [--dry-run] [--pii-filter near-ai]` | Redacts and uploads selected sessions. `--trajectory` names a Letta Trajectory v1 file or directory. `--no-reasoning` excludes model reasoning, which is otherwise included. `--remediate-quarantined` re-uploads sessions whose local receipt is `quarantined`, keeping the same `submission_id` so the server can supersede the stored envelope. `--dry-run` runs the full pipeline (parse, redact, canary check, sizing) without uploading. `--yes` skips the interactive picker confirmation. |
| `import-antigravity [--project <path>\|--all]` | Imports conversations from a running Antigravity IDE instance and stages them for `submit`. Requires Antigravity to be running, since its conversations are only readable through the local API it serves. `--project` scopes to one workspace (default: the current directory); `--all` takes every conversation the running instance exposes. |
| `status` | Shows server-side status of previously submitted sessions from the local receipts log. |
| `whoami` | Prints local identity (instance id, tenant id, device key id, hashed user subject, config dir). No network call; never prints the raw subject. |
| `logout` | Stops a running daemon, then deletes local config, device key, receipts, daemon state (settings, queue, projects, history, audit), the account session, and orphaned temp files. Leaves the state directory itself. |
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

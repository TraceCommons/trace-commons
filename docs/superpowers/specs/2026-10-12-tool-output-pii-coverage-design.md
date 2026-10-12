# Privacy coverage for tool traffic

Status: proposed, 2026-10-12. Issue #1362. The first slice (direction 1,
part A) is draft PR #1366.
Scope: `trace-commons-protocol` (the redactor, which the client and the
server share), `trace-commons-server` (the re-scrub and classifier
backstop), the contributor daemon and shells (copy, and direction 4).
Out of scope: the prose path, which keeps per-text classification, and the
release config (`TRACE_COMMONS_PII_CLASSIFY_POLICY=prose-only`).

## Problem

A coding-agent trace is mostly tool traffic: `ToolCall`, `ToolResult`,
`RoutingDecision`, `HttpExchange` events and their `structured_payload`.
`policy_examines_event` sends these to the prose-PII classifier only under
`all-events`. The pilot runs `prose-only`, so the deterministic pass is
the whole of the coverage tool events get. That function's comment names the
gap: "Unpatterned PII arriving through tool output is the accepted,
documented gap."

Tool output is where machine-local identifiers turn up: file contents,
`git log` and `git blame` authors, environment and config dumps, shell
prompts, host names, addresses, log lines and API responses. This spec
extends coverage to that traffic.

What the pilot measured on 2026-10-11 (#1362):

- **Latency.** About 1 s per classifier request. The median trace has 91
  events, the p90 2,019, and the largest have tens of thousands. Classifying
  every event one text at a time does not fit the privacy-pass budget.
- **Packing is unsafe.** Packing several texts into one request changed and
  fragmented spans and missed some real URLs, so #1359 shipped it opt-in
  only.
- **The classifier is noisy on code and logs.** It tags path fragments and
  command lines as URLs and splits timestamps into many spans. It is trained
  on prose.

## What exists today

The deterministic pass is `DeterministicTraceRedactor::redact_text_with_state`.
`redact_trace` (the client) and `rescrub_trace_envelope_with` (the server)
both run it, and so does the trailing sweep in
`rescrub_envelope_prose_pii_with`. In order, it runs private emails, generic
local paths, known path prefixes, PEM blocks, then named secret patterns,
the cue-gated contextual-entropy sweep and the split-literal pass. Emails and
paths become numbered typed placeholders (`<PRIVATE_EMAIL_1>`) from one
`PlaceholderMap` per trace. Secrets become `[REDACTED]`.

Other controls this design builds on:

- **Tool payload profiles.** `redact_tool_specific_payload` applies a
  `ToolPayloadProfile` per tool name. An unknown or missing name gets
  `Unrecognized`, which fails closed: a string leaf longer than
  `UNRECOGNIZED_FREE_TEXT_LIMIT` (160 codepoints) becomes
  `[REDACTED:unrecognized_tool_free_text]`.
- **Residual scan.** `residual_envelope_scan` runs the deterministic
  detectors over every string leaf and key of the finished envelope,
  detection only. A secret that survives forces High (`ResidualSurvivor`).
  Any other residual count blocks a downgrade on the classifier backstop
  path (`PostScrubAssessment::residual_clean`).
- **Severity.** `residual_risk` puts any found-and-removed label at Medium
  or above, except labels in `NON_SEVERITY_REDACTION_LABELS` (`local_path`,
  #219a: present in 93% of sessions, so it cannot tell sessions apart). A set
  consent content flag also puts a trace at Medium, so in practice severity
  decides the tier only for traces without content, and decides the
  `FoundAndRemoved` basis label for every trace.
- **Credit.** `compute_value_scorecard` multiplies the quality terms by
  `privacy_gate(risk)` (Low 1.0, Medium 0.5, High 0.0). High earns
  nothing. `quality` scales with `content_share`, and `replayability` with
  `replay_sufficiency`, which reads `event_carries_result` and
  `event_carries_arguments`.
- **Contributor surfaces.** The daemon reports `redactions` (occurrences) and
  `redactions_distinct` (placeholder values). `redaction_summary::describe`
  gives each family a sentence; unknown families, `private_email` among them,
  get `REDACTION_CATEGORY_UNKNOWN`. `daemon::unsure_spans` hints at
  email-, phone- and key-shaped text the scrubber did not mark, and never
  redacts anything.
- **Envelope pins.** `daemon::preview::tests::a_known_envelope_pins_a_known_digest`
  pins the sha256 of a fully built fixture envelope. Any change to envelope
  content for that fixture moves it, including a redaction that newly
  fires. `DETERMINISTIC_REDACTION_PIPELINE_VERSION` is stamped into each
  envelope and is part of `FULL_REDACTION_PIPELINE_VERSIONS`, which witness
  certificates are checked against.

## Directions

All four are meant to be combined. They are listed in the order they can
ship.

### D1. Deterministic typed placeholders for tool-output shapes

Patterns that are shape-anchored, fast, testable, and identical on the client
and the server, because both run the same function.

**Part A, in #1366:**

| Label | Shapes | Severity |
|---|---|---|
| `git_author` | `Author:`/`Commit:`/`Committer:` and person trailers followed by `<addr@...>`; `git blame` lines | bears |
| `ip_address` | public, RFC 1918 and 100.64/10 IPv4; 2000::/3 and fc00::/7 IPv6 | none |
| `private_hostname` | `.local`, `.localdomain`, `.internal`, `.intranet`, `.lan`, `.corp`, `.home.arpa`; the host part of a prompt or ssh target | none |
| `os_username` | prompt and ssh user, `id`, `whoami`, `USER=`/`LOGNAME=`/`USERNAME=`, `ls -l` owner | none |
| `phone_number` | E.164, NANP written forms, UK mobile | bears |

The split follows the `local_path` precedent. Machine identifiers appear in
any terminal session and would bring back Medium for everyone. Person
identifiers bear severity like `private_email`. The PR lists what each
pattern deliberately leaves alone (version strings, OIDs, CIDR networks,
hashes, `std::io::Error`, member accesses such as `self.local`, package
references such as `lodash@latest`, timestamps). It also gives false-positive
counts over the fixture corpora and over this repository's own source,
standing in for file-read output.

**Part B, follow-ups in the same style:**

- `git shortlog -sn`, `git blame --porcelain` (`author ...` and
  `author-mail`), `git log --format` with custom fields. These need the
  command, so they belong with D2's per-tool context.
- Prompts, `whoami` and `ls -l` inside an escaped JSON string. Their line
  anchors only accept a literal `\n` for git lines today.
- MAC addresses, Windows profile paths (`C:\Users\<name>\`), and
  `/proc/*/environ`-style dumps.
- Personal-value config keys (`user.name`/`user.email` in `git config -l`,
  `npmrc` `email=`, `.netrc` `login`), keyed on the key the way
  `redact_sensitive_json` keys on secret names.
- Handles in URLs (`github.com/<user>`) only where the host is a known
  forge. A general "handle" detector is not pattern-safe.
- Using the contributor's own username, which the client knows from the home
  directory, to confirm a shape-anchored candidate. Never as a bare word
  match: `dev`, `admin` and `runner` would fire everywhere. The server cannot
  know it, so this breaks parity and needs an owner decision (Q3).

**Envelope:** content only, no schema or serde change. It moves the golden
digest pin whenever the fixture session contains a new shape (#1366's did
not). It should bump `DETERMINISTIC_REDACTION_PIPELINE_VERSION` to v4 so a
stored stamp says which shapes were covered, but that moves
`FULL_REDACTION_PIPELINE_VERSIONS` and the witness's accepted set together
(Q1).

### D2. Per-tool structural policy

Replace high-risk, low-value tool results with a placeholder and metadata,
and keep the call:

```
[tool output omitted: 214 lines, environment]
```

Candidates are environment dumps (`env`, `printenv`, `set`), file reads
outside the session's workspace root, raw HTTP response bodies above a size,
`~/.ssh`, `~/.aws` and keychain reads, browser-profile reads, and `ps`/`top`
full command lines.

This is an extension of `ToolPayloadProfile`, not a new mechanism. Add a
`kind` judgement per (tool, arguments) pair, alongside the per-field
`ToolRedactionAction`. The unknown-tool rule stays as it is: an
unrecognised tool still fails closed. D2 adds coarser actions for
recognised tools whose output is known to be low-value. Classifying
arguments (`cat ~/.zshrc`) needs the shell command, so it applies to shell
tools first and is conservative: an unparsable command keeps today's
behaviour and is not omitted.

**Interactions:**

- **Credit.** An omitted result fails `event_carries_result`, which lowers
  `replay_sufficiency` and `content_share`, and so `replayability` and
  `quality`. That is the intended direction for an environment dump. It is
  not intended for a file read the task needed. The omission marker should
  therefore count as "result present, withheld" for replay, but not as
  content. That needs a new `REPLAY_RESULT_KEYS`-style rule and a pinned
  test.
- **Risk.** An omission is found-and-removed: a new
  `tool_output_omitted:{kind}` count family, with the same severity as the
  shape it stands in for. An environment dump bears severity; an
  out-of-workspace read does not.
- **Envelope.** A marker string in `redacted_content` is content only and
  needs no schema change. A typed `omitted: {lines, kind}` field on the
  event is a schema change. That means a `TRACE_CONTRIBUTION_SCHEMA_VERSION`
  decision, server deserialisation that accepts older clients, and the
  golden pin. Prefer the marker string, matching `[REDACTED:...]`.
- **Parity.** The client must decide, because the server never sees the
  raw text. The server can only check that a marker is well formed.
- **Consent.** `tool_payloads_included` stays true when results are
  omitted: the calls and the other results are still payloads.
  `reconcile_consent_declarations` needs no change.

### D3. A classifier suited to tool output, with per-text batching

The difference from packing: put many texts into one GPU forward pass as
separate sequences in a batch, each with its own attention mask and its own
span decoding. Packing concatenated texts into one sequence; batching keeps
them independent, so spans cannot cross texts or move. Two candidates:

- **The self-hosted `openai/privacy-filter` on a GPU**, batched. It is
  already a backend (`privacy_filter_self_hosted.rs`,
  `PRIVACY_FILTER_SELF_HOSTED_PIPELINE_SUFFIX`) and has the same taxonomy.
  Batching alone does not fix its noise on code.
- **A code- and log-aware NER model** (secrets plus PII on source and logs),
  behind the same `PrivacyFilterAdapter` trait, so it substitutes at the
  existing seam. It would have its own backend tag and pipeline suffix.

The policy gains a third value, `prose-and-tool-batched`: tool events go to
a batch adapter with a per-trace budget. If the budget is exhausted, the
pass sets `coverage_incomplete` (forcing High), as the structured-payload
budget does, rather than skipping text silently.
`SafePrivacyFilterSummary.events_examined` and `events_skipped_by_policy`
already record what was examined.

**Envelope:** no schema change. The redaction pipeline version gains a
suffix, the backend tag changes, and witness acceptance needs the new string
(Q1 again). **Parity:** server only, unless D4 lands. **Cost:** a GPU
host. The pilot is a CPU-only C3 instance; the spot L4 batch path measured
for the embedder is the reference.

### D4. On-device classification in the contributor client

Classify on the contributor's machine, so unscrubbed tool output never
leaves it. This is the strongest privacy position and the hardest to deliver:

- **Model size.** `openai/privacy-filter` is 1.5B parameters (50M active).
  The weights are a multi-gigabyte download for a desktop app, which the
  macOS lead client would have to justify in onboarding copy. A small
  token-classification model distilled for code and logs is the realistic
  target.
- **Attestation.** The server cannot verify what a client ran. The witness
  path certifies a server-side classifier by measurement. A client-side
  verdict is self-reported and must not satisfy
  `is_full_redaction_pipeline_version`. The server keeps its own pass.
- **Parity.** Client output becomes stricter than the server's. That is
  safe, since the server pass is idempotent over placeholders (#1366 tests
  this), but residual-scan behaviour must not depend on which side ran the
  model.

**Envelope:** no schema change. It adds a client pipeline suffix.

## Evaluation plan

Choose using measurements. Run the evaluation before D3 or D4 is built, and
keep running it after each D1 slice ships.

1. **Labelled set.** A few hundred tool-output spans from consenting
   contributors' traces, stratified by tool (shell, file read, git, HTTP,
   search) and by trace size (median, p90, largest). Each span is labelled
   with the five D1 labels plus the classifier's eight categories. Labelling
   happens on the pilot host under the same handling as the 2026-10-11
   comparison harness. Data is destroyed after the run, and only counts
   leave the host.
2. **Metrics per direction and label:** recall, precision, and
   placeholder-only share (the fraction of a tool result's bytes that end up
   as placeholders, a proxy for lost value). Also wall time per trace at
   median, p90 and the largest.
3. **Counts-only harness.** The comparison harness from 2026-10-11,
   extended to read the labelled set and emit only `label, tp, fp, fn`
   rows and timings, no text. Reuse #1366's measurement method: run the
   redactor over corpora, count per label, look at hits locally, and record
   counts in the PR.
4. **Prevalence on the pilot**, counts only: the share of accepted traces
   carrying each new label. This confirms or revisits the non-severity set,
   as 93% did for `local_path`.
5. **Gate:** a direction ships when it adds recall on the labelled set
   without dropping any label's precision below the deterministic pass's,
   and fits the privacy-pass budget at p90.

## Risk tiers and credit

- D1 machine labels cannot move a trace's tier on their own (pinned by
  test). Person labels can move a no-content trace from Low to Medium, as
  `private_email` already does.
- **The residual scan is the trap.** It runs every deterministic detector
  over every typed leaf. A detector that fires on a typed value (a harness
  version, a model name, a timestamp) leaves a permanent residual count.
  That blocks every classifier downgrade for the trace, and the rewriting
  pass may also alter the typed field. #1366 found and fixed exactly this
  for four-part versions. Every new D1 detector needs the
  envelope-level residual test.
- **Value loss.** A tool result that is mostly placeholders is worth less
  for training. `quality` and `content_share` do not see placeholders today.
  Proposal: measure placeholder-only share in the evaluation (step 2) before
  touching the scorecard. If it matters, add a placeholder-density factor to
  `quality` behind a pinned test, and never apply it to the privacy gate:
  that would be the double penalty the 2026-08-07 spec removed.
- D2 omissions lower `replayability` unless the replay rule above lands, so
  D2 should ship with it.

## Client and server parity

- D1: identical by construction (one function). Known-username matching
  would break this, so it is not in D1 part A.
- D2: client decides, server validates the marker shape.
- D3: server only. The client's preview cannot show what the server
  classifier will remove. The contributor copy must say that tool output
  is checked again after sending.
- D4: client stricter than server; server unchanged and idempotent.
- Pipeline version stamps must name what ran on each side, so a stored
  envelope says which coverage it had.

## Contributor copy

Following `docs/ui-copy-guide.md` (context beside the action, separate
titles and accessible names, macOS first as the parity target):

- Add `REDACTION_CATEGORY_*` sentences for the new families. Today they,
  and `private_email`, fall back to the unknown-category sentence. Drafts:
  - git author: "Names from git history."
  - IP address: "Network addresses."
  - private host name: "Computer and network names."
  - account name: "Your account name on this computer."
  - phone number: "Phone numbers."
  - email: "Email addresses."
- Display names: "IP address", not the "ip address" that `display()`
  produces today. That is one exception, like "URL path".
- D2 omissions, on the review sheet: "Left out: environment variables (214
  lines)". The tool call stays visible. No separate button: omission is
  automatic, and the existing per-session review is where it can be
  questioned.
- Do not claim more coverage than exists. The scrubbing caveat beside named
  marks stays; a region with no mark is not a region with nothing in it.
- `unsure_spans` keeps its phone hint as a backstop for shapes D1 skips.
  Once D1's phone detector removes a number, that number gets no hint, which
  holds the existing "a matched value yields no hint" property.

## Rollout

1. **D1 part A** (#1366): deterministic, shared, no schema change. Ships in
   the next release. Measure prevalence on the pilot (counts only) and
   confirm the severity split.
2. **Copy** for the new families, macOS first, then GTK, Windows and Tauri.
3. **Evaluation set and harness** (steps 1-3), before any model work.
4. **D2 for shell tools**, with the replay rule and a marker-string
   envelope. Then extend to file reads outside the workspace.
5. **D1 part B**, in order of the evaluation's false-negative counts.
6. **D3** if the evaluation shows tool-output recall that D1 and D2 cannot
   reach. Only with a GPU host and a budget that sets
   `coverage_incomplete` when it runs out.
7. **D4** last, once a small enough model exists. It is additive to the
   server pass, never a replacement for it.

## Open questions for the owner

1. **Pipeline version.** Bump `DETERMINISTIC_REDACTION_PIPELINE_VERSION`
   to v4 for D1? This needs witness acceptance of the new
   `FULL_REDACTION_PIPELINE_VERSIONS` strings to be released together.
2. **Private ranges.** D1 redacts RFC 1918, 100.64/10 and ULA addresses as
   well as public ones. Keep that, or keep private ranges for debugging
   value?
3. **Known username.** May the client confirm candidates with the account
   name it knows, at the cost of client/server parity?
4. **Bot identities.** Redact `Co-Authored-By: <bot> <noreply@...>` like a
   person (today's behaviour), or allowlist known bot addresses?
5. **Severity set.** Accept the person/machine split until pilot prevalence
   is measured?
6. **D2 scope.** Is "file read outside the workspace" an omission, or only
   a D1 pass? It is high-value for some tasks.
7. **Value.** Should placeholder density reduce `quality`, or is the gate
   enough?
8. **D3 hosting.** Is a GPU host for a batched tool-output classifier in
   budget, and would its verdicts need witness attestation like the prose
   classifier?
9. **D4 model.** Is a multi-gigabyte on-device model acceptable for the
   macOS client, or should D4 wait for a small distilled model?

# Native network Zaki lane execution and scope matrix

**Goal:** Implement #1173's authorized Zaki lane behind the additive C3
contracts in [daemon IPC](../../contributor-daemon-ipc-v1_1.md#native-network-additions-c3-1173).
**Architecture:** Keep secrets, consent and network validation in the permissive
Rust contributor core. Swift runs the platform passkey UI and consumes the same
contracts; Ron owns screens and Kristi owns local matching/data.
**Baseline:** `ba8a87777`; C3 lands without Rust or Swift behavior.

**Mission mechanism source:** [Michael's original mechanism draft](https://docs.google.com/document/d/1PFdbc91S3_aTUFO4LoJCBhW0KB8jiZscAp_ee6HD06I/edit?tab=t.0)
proposes daily missions, monthly Trace Activity, levels, streaks and badges;
it leaves one daily mission versus activity-based assignment open. User decision:
**“Build configurable server; rewards off.”** This authorizes the configurable
server while daily economics remain unspecified. Matching stays on the Mac,
missions grant no sending consent, and any future credit stays pending until
commons settlement. The commons pending-credit ledger decision is settled;
the credit-to-inference bridge remains unspecified.

| Work | Deliverable / evidence boundary |
|---|---|
| C3 | Contract names, DTOs, SAMPLE JSON and support status before behavior |
| Z1 | Bounded anonymizer and actual local log/refusal projections. The pre-existing fake-local fixture is synthetic. Successful invite, summary, HTTP credit/settlement and mission publication captures remain operational gates. |
| Z2 | Bounded IronWire summary read preserving real upstream groups and totals |
| Z3 | Register proof lookup; return stored label, no fabricated check details |
| Z4 | Authoritative NEAR AI organization-wide per-model billing, with source/window and exact nano-USD amounts; unavailable remains unknown and never becomes local-device spend |
| Z5 | Extend existing map additively with owned hub/route facts; no invented provider or remote dev-server discovery; coordinate K14 |
| Z6 | Core disclosure plus read/write wrappers around existing hosting lifecycle |
| Z7 | Build the configurable missions server with rewards off. Commons pending-credit ledger is settled (#1118); daily economics and credit-to-inference bridge remain unspecified. Preserve #1174 M1–M4; no reward semantics invented. |
| Z8 | Separate skill-evaluation and trace-activity catalogue/status IPC; anonymous catalogue reads and local matching send no profile/results |
| Z9 | Existing `history_rollup.community` is authoritative; preserve rank and absence behavior |
| Z10 | Existing non-redeeming issuer lookup over IPC; full invite URL, real unit, pending conditions |
| Z11 | Approved native passkey names, daemon ceremonies/bind/session lifecycle and Swift platform adapter; no screen redesign |
| Z12 | Renewed approved profile and native Associated Domains entitlement with release validation. Signed observed Apple origin and AASA qualification remain operational gates |
| Z13 | Extract existing browser sign-in/status into IPC; preserve enrollment prerequisite and support pre-enrollment status/sign-out |
| Z14 | Existing Sparkle release path retained; current 0.12.6 build 4301 enclosure signature verified against the installed signed app key. No installation/update or new release performed |
| Z15 | Review consent boundaries in affected core/adapter work; every future screen PR still needs its own consent review |

- [x] Commit C3 documentation before behavior and provide implementers the exact interfaces.
- [x] Implement network, identity, Swift adapter, catalogue and release support in isolated branches; preserve existing licensing and no-new-dependency rule.
- [x] Integrate focused commits with tests for unknown/error/empty states, invalid/replayed inputs, authority changes and consent separation.
- [x] Run contributor/FFI checks with warnings denied, contract tests, relevant clippy, format, license boundary and Swift tests against freshly built FFI. Release script tests validate profile/domain/update gates without claiming signing or deployment.
- [x] Review the integrated diff and report implemented, pre-existing and unresolved/external work separately.

## Integration verification, 2026-10-02

Core implementation checkpoint: `2ef56c9cf`, incorporating main `4339c53cb`.
The separate C1 follow-up is `3f142ad53`, based on Kristi's open #1175 head
`9a49e658c`; it changes adapters and contract fixtures, not screens. Combined
verification checkpoint `e0f4fc8b7` includes both branches. These are local
commits, not merged or deployed changes.

- Contributor library: 3,063 passed, 11 ignored with normal macOS access.
- Protocol: 379 passed. Release pipeline: 53 passed. License boundary: 4 passed;
  new AGPL file headers verified, dependency manifests unchanged.
- Missions: three HTTP tests and one restricted-role PostgreSQL test passed
  against an isolated real database after the main merge; no self-skips.
- FFI: 207 passed across the complete serial suite. A parallel run had one
  `tc_unsubscribe_refuses_a_freed_handle` failure; its isolated rerun passed.
  Existing FFI docs explicitly describe same-address allocation reuse as a
  limitation of the pointer registry. That is consistent with the failure,
  but reuse was not instrumented. No FFI implementation or ABI tests were changed.
- Combined Swift: 1,155 XCTest cases, one skipped, zero failures, plus eight
  Swift Testing cases passed against freshly built FFI with normal macOS access.
  The earlier sandbox UI failures did not occur in this run.
- Server default, `near-ai-scorer`, and `local-gpu-models` compile checks passed;
  server test targets compiled. Server and client Clippy used the repository
  allowlist and denied warnings. One imported main Boolean predicate was
  simplified equivalently to satisfy Clippy; no admission behavior changed.
- Native entitlement fixtures passed, including system Bash 3.2; 8 metadata
  and 14 anonymizer tests passed. Signed Sparkle enclosure verification passed,
  including a corrupted-signature negative check, without installation.
- Independent reviews covered identity, missions server and IPC, billing,
  network, Swift, C1, release tooling, and the integrated method/auth boundaries.
  The final registry contains 116 methods with no advertisement/dispatch gaps.

## Remaining operational and screen gates

The user-supplied renewed profile includes Associated Domains and is integrated.
This does not qualify a signed Apple create/login ceremony: AASA checks have
not passed, and observed native `clientDataJSON.origin` still needs signed
staging verification against the server allowlist. No deployment, notarization,
installation, new release, or economic activation occurred.

Successful approved invite, current summary, HTTP credit/settlement, and mission
publication recordings remain unavailable; committed refusal projections and
synthetic samples are explicitly labeled. Mission policies still require operator
configuration, and rewards remain off regardless of configuration.

Z15 covers the available PR heads, not future screens. At #1182 head
`aa64a1cc6`, the monitor still maps a missing/null Private AI setting to off.
That screen-owner correction remains required. The C1 follow-up resolves the
provisional mission/invite contract and sample issues within its scope; future
Missions screens must consume Rust disclosure and preserve unknowns and consent
gates. K4's existing withdrawal-copy exception and future M4 screen checks remain
separate work. macOS 14/26 design qualification remains Ron's release gate.

All credit remains conditional/pending, proof means only verified model proof,
registry prices never mean billed spend, unknown never means zero/off, and
sample data never becomes a release fallback. No deployment, messages to
collaborators, production capture consent or proposed screen-copy approval
follows from these contracts. Native tokens remain daemon-side; sign-out/wipe
invalidates stale ceremony/account-owned work. The existing source-offer and
permissive-to-AGPL dependency boundary remain load-bearing.

## Continuation and published review branches, 2026-10-02

This section supersedes the operational status above; the earlier counts remain
historical evidence for their named checkpoints. Core implementation checkpoint
`41449b646` includes main `67403a212` and is published in draft PR #1187. C1
`3f142ad53` is draft PR #1188, stacked on Kristi's #1175. The two-file Monitor
unknown-state correction is draft PR #1189 (`4b1bdf955`), stacked on Ron's #1182
base `336d7c139`. None of these three implementation PRs has been merged.

Fresh integration evidence after the main update:

- Contributor library repeat: 3,069 passed, 11 ignored. The first full run hit
  the existing five-second wallet-listener rebind timeout under load; both its
  isolated rerun and the full repeat passed. No timeout or runtime change was
  made; the intermittent cause is not proven.
- FFI: 216 passed with 16 test threads. The stale-pointer probes now preserve
  their original assertions in isolated subprocesses so other tests cannot
  reuse a freed address during the probe. Production registry and ABI unchanged.
- Combined core/C1 Swift against freshly rebuilt FFI: 1,158 XCTest cases, one
  skipped, zero failures, plus eight Swift Testing cases passed.
- The activity-missions PostgreSQL test is now explicitly wired into CI with
  its own database and the existing transaction guard. Its independent local
  run passed without skips, with 403 committed transactions.
- Deployment inventory explicitly classifies the new mission routes; 50 tooling
  tests pass. Unknown interfaces remain rejected.
- C1 has 36 passing PR checks at its exact published head. Duplicate branch-push
  checks were intentionally cancelled. Core and Monitor follow-up CI remain
  pending; local results are not a claim that their complete remote checks pass.
- Monitor correction: five focused tests pass on the updated R5 base, covering
  missing settings, missing/null Private AI, explicit false, and explicit true.

The user separately authorized merging and deploying community PR #46. It
merged as `0ed30579e1693272839edeecd2bb2b65b76e8c65`; both community and docs
deployment jobs succeeded. The live AASA origin returns HTTP 200 JSON with
`KXSWJN7WY8.ai.tracecommons.shell`, matching the qualified artifact. Apple CDN
reads currently vary between that valid response and an older cached 404;
propagation remains an open qualification gate.

The existing documented `ingest.tracecommons.ai` backend is the live pilot,
not an established separate staging deployment. Using a new qualification
account there awaits the user's answer. Native create/sign-in has not been
performed. The platform adapter is implemented, but the current application
has no production ceremony call site; the R12 screen or a dedicated signed
qualification harness is still needed for runtime qualification. No new native
release, notarization, installation, or economic activation is claimed.

Z15 was refreshed against R6 `5befed586` and R7 `639f9a6d8`. Permissions and
Review controls remain in the Trace Tree, contrary to the supplied Inspector
placement feedback. R7 Contribute checks preview/copy availability but does not
yet consume the existing enrollment and eligibility fields. These are screen
acceptance gaps, not evidence of bypassing daemon admission. The bounded remote
branch/PR query found no published R8 source; it does not establish whether Ron
has local R8 work. The Monitor unknown-dot fix addresses only its narrow state
projection, not those later placement or action-gating gaps.

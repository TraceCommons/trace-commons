# Claude Desktop Code validation — 2026-11-05

## Installed client with a local synthetic provider

Claude Desktop 4.16336.0 used a new Local saved environment named Private AI
protocol validation, scoped to /private/tmp/tc-desktop-validation. Its original
account, terminal settings and other environments were not changed. The preset
included the five Vertex variables, an absolute 100000-token auto-compact window,
an 8192 output cap and server-classifier disable flag. Its capability belonged
only to the disposable fixture, not the NEAR credential.

Observed in the real Desktop UI:

- A first streaming answer displayed READY.
- A subsequent turn executed the actual Read tool against proof.txt and resumed
  to another READY answer.
- /context completed with a 100k window despite terminal 400 count-token responses.
- /compact reported completion, and the next ordinary turn again displayed READY.

Fixture logs retained only request kind, body length, tool count and whether
credential headers existed. No Authorization/API-key header was observed.
Counting calls occurred in batches for tool/schema inspection; no client hang
was observed. These are synthetic usage figures and responses, not NEAR billing,
model-quality, automatic threshold-compaction or enclave-proof evidence.
The local fixture is stopped after the rehearsal; its saved environment is not
an active NEAR connection.

## New bound gateway with live NEAR inference

A separate temporary native harness ran the pinned IronWire gateway on an
isolated ephemeral home/port. The existing credential-owning local proxy was
used as its upstream through a temporary loopback bridge, with an explicit
NEAR route. The harness held only a placeholder key; no credential was exported
from the running application or copied into configuration, arguments or logs.
The public model catalogue was fetched with system TLS verification intact.

Final result:

- Selected model Qwen/Qwen3.8-27B; returned model Qwen/Qwen3.8-27B.
- HTTP 200; answer contained READY.
- Wrong project capability: 403. Other inference facade: 404.
- Listener rejected connections after full shutdown.

The first version of this live check exposed translated responses retaining the
requested Claude alias. The bound-only translation fix now propagates provider
model identity; cross-wire streaming/nonstreaming regressions also cover a
provider reporting a different canonical ID. Generic translation is unchanged.

These two rehearsals are separate evidence: the installed Desktop was tested
against a synthetic provider, and the new bound gateway against live NEAR via
the existing proxy. A single Desktop-to-live-NEAR session, automatic compaction
at a genuinely full context, and Desktop-specific stop-button cancellation are
not claimed. Lifecycle, cancellation, credential changes, restart, corrupted
state and privacy/context bounds are covered by deterministic regression tests.
Native setup visual inspection and final suite results are recorded in the PR.

## Independent review and local checks

Fresh review identified two issues, now repaired with regressions: unavailable
setup could prevent a user selecting another model, and Desktop withdrawal
could wait behind an indefinitely draining managed CLI stream. Known unavailable
status now permits retry/reset while unknown states still refuse mutations.
Desktop revocation runs before CLI drain; the real held-stream regression failed
under the previous order and passed under the corrected order.

The native setup was rendered in regular/narrow widths and light/dark appearance
(16 PNGs); representative rows and sheets were visually inspected for clipping
and access to actions. Environment wrapping does not change the copied preset.
Final suite counts and hosted check status are recorded in the linked draft PRs.
The existing macOS compact-unwind linker warning remains.

Final local checks passed: contributor 5,517 passed/10 ignored; FFI 293 passed;
contributor/FFI clippy; fresh FFI build followed by the full Swift suite; Rust
formatting and diff checks. The final upstream all-feature run passed with
1,575 passed/two ignored and the independent Vertex rerun passed 13 tests.
All four license configurations, four license-boundary tests and three locked
metadata reads passed. No merge, deployment or release was performed.

## PR #2003 feature flag integration

The Desktop feature is now a live, default-false environment flag:
`claude_desktop_private_ai`, requiring `claude_desktop_private_ai_v1`. Registry
and all profile versions are 2, and example manifests remain false. Native
setup and daemon configure/export/restore fail closed without an effective
enable. Disable/expiry closes the listener and clears native preset state,
while accepted streams drain and the saved binding remains for re-enable.
Account, credential and Private AI policy revocations still remove the binding.
No live configuration was published.

Local combined checks passed: contributor 5,546 passed/10 ignored; protocol 676
passed; FFI 295 passed; license boundary four passed; profile/publisher/real-CLI
checks eight passed; three-crate lint; formatting; fresh FFI build followed by
the full Swift suite (3,542 XCTest cases executed, nine skipped, zero failures;
eight Swift Testing cases passed).

Windows CI previously failed four new Desktop tests. The withdrawal regression
now measures the actual listener-close acknowledgement within one second and
separately checks TCP refusal within five seconds, preserving the held CLI
stream assertion. Three other failures returned unavailable during setup;
bounded test-only stage/error labels were added to identify the cause on the
hosted Windows rerun. These failures are not declared fixed by local macOS tests.

Final review also found a flag-off versus final-publication race. Startup now
holds a synchronous feature snapshot guard through metadata save and route
publication, in settings -> feature policy -> Desktop lock order. A controlled
interleaving regression verifies that off acceptance cannot overtake the commit.

The first hosted follow-up run caught old production profile digests in the two
plist golden fixtures. Only those expected identity values were refreshed for
the new capability/profile version. The exact macOS channel-plist check passed
locally afterward; no bundle identity or feed behavior changed.

## Windows follow-up repairs

The first diagnostic run did not reproduce the three earlier route-start
failures. It exposed a live-disable test connecting after its aggregate state
check, plus two logout cleanup failures. Pending listener barriers now remain
shared across all concurrent stops and cancelled reconfiguration; the live
disable test waits the exact listener acknowledgement and retains a bounded
old-capability HTTP probe plus fixture-call counter on unexpected TCP success.

Ordinary credential cleanup tests now use production's 15-second OS budget,
which includes thread scheduling and durable journal writes. Only deliberate
blocked-store tests request a 500 ms budget for the operation under test. A
600 ms successful-delete regression requires both secret and journal cleanup,
while the blocked-delete test still requires the failed reference to remain
journaled. Production's budget did not change.

Isolated upstream Windows controls showed a Tokio connect could exceed a
two-second observation window without succeeding; the actual embedded held-SSE
barrier test passed, including unchanged upstream-call count. A separate TDD
regression proved an accepted connection could dispatch after the shared stop
flag; the bound-only shutdown guard now returns fixed 403 before accepting a
new inference request. Existing streams and generic routing remain unchanged.
The final upstream pin is `eeab5a61ae658842946f03c857f1f50345dd9487` (1,579
all-feature tests passed, two ignored; lint and formatting passed). Hosted
results for the final combined revision are recorded in the PR.

The next combined Windows run passed every other check and failed only the
live-disable TCP-refusal assertion. Its same-socket HTTP probe observed a read
failure, no HTTP response and zero additional fixture completions after exact
listener acknowledgement. The test now requires concrete request refusal:
TCP refusal, EOF/known closed-transport error before any response bytes, or a
complete valid HTTP 403. Timeouts, generic errors, partial/malformed headers and
all other statuses remain failures; the fixture count must still stay one.
A live-gateway positive control proves that the same raw request produces HTTP
200 and one fixture completion before stop. Local full contributor tests passed
5,546/10 ignored, followed by focused final probe tests, lint and formatting.
Production code was unchanged by this final assertion repair.

The corrected live-disable scenario and probe controls passed on Windows. A
second TCP-only assertion remained in the daemon-shutdown test; all five
Desktop post-stop checks now use the same bounded request-refusal probe. They
retain exact listener acknowledgements, fixture-call counts, the shutdown and
withdrawal deadlines, saved metadata assertions and the held CLI stream. The
final local targeted run passed all 26 Desktop tests plus the withdrawal test,
with lint and formatting clean. No production code changed in these test-only
consistency repairs. Final hosted results are recorded in the PR.

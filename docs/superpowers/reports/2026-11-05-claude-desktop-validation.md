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

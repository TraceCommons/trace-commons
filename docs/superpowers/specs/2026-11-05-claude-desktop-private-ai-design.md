# Claude Desktop Code tab Private AI foundation

The owner approved starting implementation after the 2026-11-05 investigation of
`desktop-code-tab-private-ai-method-2026-11-04.md` (tested 2026-11-04).

## Initial foundation scope

Allow Claude Desktop Code sessions to reach Private AI through a saved local
environment. Start with a reviewable protocol foundation, not a claim that
Desktop is connected or that its sessions are managed. This draft embeds the
Vertex facade, tests it through a NEAR-only managed route, and records the
remaining product work. No user settings, private Desktop stores, or credentials
are edited by this slice.

## Approaches

The selected first approach is a saved environment with Vertex gateway variables:
it scopes inference routing to Code sessions started from that environment.
IronWire normalizes the Vertex envelope and uses its existing Messages pipeline.
The alternative is Desktop's documented third-party inference gateway mode,
which reuses Messages directly but changes the app's inference mode across Chat,
Cowork and Code. Validate it separately before substituting it for this approach.
A standalone Python forwarding service is useful evidence but is not the shipped
architecture: it duplicates streaming, error handling and observation.

## Foundation contract

- Accept Vertex rawPredict and streamRawPredict paths, deriving model and stream
  from the path; remove the Vertex-specific anthropic_version envelope field.
- Reuse Messages routing, eligibility, privacy, credentials, receipts, streaming,
  cancellation and ledger observation. Existing Messages requests keep their
  current native-lane byte contract.
- The original requested model remains observable separately from served model.
  Do not hardcode model IDs from the investigation's catalogue snapshot.
- Reject caller route overrides on the Vertex facade. On an unbound facade, project and region are not credentials or permission
  to choose an account; the bound embedding policy below uses an opaque project
  capability for its dedicated listener.
- Count-tokens returns an explicit terminal unsupported response, never a
  fabricated count. Confirm client fallback on a real Desktop session before
  treating this as usable context accounting.
- Record canonical logical paths, never raw project/region values, including on
  malformed, oversized, cancelled and panicking requests.
- No dependencies are added. Preserve permissive client licensing. Use
  RUSTFLAGS="-D warnings" and CARGO_INCREMENTAL=0 on every cargo invocation.

## Product requirements identified by the investigation

A Desktop connection needs a durable revocable route binding, selected main and
background models, context/capability configuration, daemon restart semantics,
and observed status. Existing two-minute launch tickets and heartbeat-based CLI
ownership do not provide that lifecycle. Prefer native client session headers
for attribution when observed; do not infer sessions from cwd or newest files.

Generate the five baseline Vertex variables plus model/context controls as a preset for manual entry in Claude's saved environment
UI, with an explicit folder. Never write encrypted internal stores or globally
reroute terminal Claude sessions. Report configured status until calls arrive;
use proxy/receipt evidence for the served model, not the model's self-report.
Raw trace collection remains a separate consent decision.

Before promotion, validate real Desktop tool calls, side calls, subsequent turns,
compaction, cancellation, disconnect and restart. Confirm no subscription
fallback, credential forwarding or count retry storm. Measure the large Desktop
prompt baseline and cache behavior. This is Code inference routing, not a claim
that all Desktop network activity is private.

## Product follow-up authorized 2026-11-05

The owner requested completing the recorded follow-up work. Extend the same
saved-environment architecture and draft PRs. Native whole-app gateway mode
remains an alternative, not an implicit global change.

A separate Desktop connection is owned by the daemon, not a CLI session. Persist
one private connection record with route UUID, stable port, random opaque
`nearai-<32hex>` project capability, main/background model IDs, and SHA256 of the
nonexpiring NEAR inference credential. Never put the upstream credential into
the preset, route config, logs or IPC. Restore the same port/capability only
while the same credential and Private AI switch remain authorized. Missing or
changed credentials, switch-off, dry-run, shutdown, corrupt state and occupied
ports fail closed. Reconfiguration rotates the capability; disconnect withdraws
the listener before acknowledging and prevents stale pending starts publishing.

The embedded VertexGateway authenticates the capability before parsing/counting,
forces the configured NEAR backend/main or Haiku background model, and disables
other inference facades on that dedicated listener. Caller headers, mutable
control pins and failed backends cannot move the request to another account or
model. Requested/served model attribution and normal capability checks survive.
Background selection follows the existing privacy-tier ceiling. Keep Desktop
route bodies/token capture off: setup never grants trace collection consent.

Expose four local IPC actions: claude_desktop_status, claude_desktop_configure,
claude_desktop_preset and claude_desktop_disconnect. Safe status states are
disconnected, ready, observed, unavailable, sign_in_required and disabled. Only
explicit configure/preset actions export environment text; observed status
requires successful completion evidence rather than a bound listener or token
count. A local source ledger is retained without treating Desktop as a managed
transcript session.

Add one ordinary tools row and setup sheet inside the existing Private AI tools
panel: model choices, Save, Copy and Disconnect. All copy comes from the shared
daemon table. No new notification, banner, card, global settings rewrite or
private encrypted-store edit. Environment remains sheet-local and clears on
dismiss, edits, disconnect or status invalidation. Clipboard writes happen only
on explicit Copy. A saved Local environment must include a chosen folder.

Use catalogue-derived safe input/output budgets and the explicit absolute
CLAUDE_CODE_AUTO_COMPACT_WINDOW: known Claude picker IDs may ignore generic
MAX_CONTEXT. Claude currently clamps this override to at least 100,000 tokens,
so reject unproven or smaller safe budgets. Token-count HTTP 400 remains terminal;
validate the installed Desktop client fallback and tool loop with a synthetic
local server first, keeping that evidence distinct from real NEAR inference.

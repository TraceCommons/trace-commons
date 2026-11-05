# Claude Desktop Code tab Private AI foundation

The owner approved starting implementation after the 2026-11-05 investigation of
`desktop-code-tab-private-ai-method-2026-11-04.md` (tested 2026-11-04).

## Intent and scope

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
- Reject caller route overrides on the Vertex facade. Project and region are
  not credentials, ownership proof, or permission to choose another account.
- Count-tokens returns an explicit terminal unsupported response, never a
  fabricated count. Confirm client fallback on a real Desktop session before
  treating this as usable context accounting.
- Record canonical logical paths, never raw project/region values, including on
  malformed, oversized, cancelled and panicking requests.
- No dependencies are added. Preserve permissive client licensing. Use
  RUSTFLAGS="-D warnings" and CARGO_INCREMENTAL=0 on every cargo invocation.

## Remaining product work

A Desktop connection needs a durable revocable route binding, selected main and
background models, context/capability configuration, daemon restart semantics,
and observed status. Existing two-minute launch tickets and heartbeat-based CLI
ownership do not provide that lifecycle. Prefer native client session headers
for attribution when observed; do not infer sessions from cwd or newest files.

Generate a five-variable preset for manual entry in Claude's saved environment
UI, with an explicit folder. Never write encrypted internal stores or globally
reroute terminal Claude sessions. Report configured status until calls arrive;
use proxy/receipt evidence for the served model, not the model's self-report.
Raw trace collection remains a separate consent decision.

Before promotion, validate real Desktop tool calls, side calls, subsequent turns,
compaction, cancellation, disconnect and restart. Confirm no subscription
fallback, credential forwarding or count retry storm. Measure the large Desktop
prompt baseline and cache behavior. This is Code inference routing, not a claim
that all Desktop network activity is private.

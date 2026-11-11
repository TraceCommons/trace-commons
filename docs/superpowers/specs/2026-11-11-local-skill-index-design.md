# Local skill inventory and usage

Status: approved discovery scope and implementation workflow. See the implementation plan for delivered coverage and validation.

## Intent and scope

Help people find the skills installed on this machine and understand how often
their agents use them. Put Skills within the Index surface. Discover known agent
locations and project skill directories within already indexed projects. Keep
inventory and analysis local. Do not crawl the entire disk.

The user selected known agent locations after considering whole-machine search.
Known locations provide predictable coverage and fewer archived duplicates;
whole-machine search provides broader coverage at greater cost and ambiguity.
Trace-only discovery was also rejected as insufficient: unused installed skills
would never appear.

No automatic installation, updates, deletion, skill generation, effectiveness
scoring, public catalog, or model-service dependency is included.

## Discovery

Use provider adapters for Codex, Claude Code, OpenCode, and shared agent skill
directories. Resolve configured roots before defaults, including alternate home
directories and active plugin manifests. Verify each adapter against installed
configuration and fixtures during implementation; do not infer agent support
from a directory name alone. Unsupported providers can be added independently.

Discover SKILL.md files beneath those bounded roots and recognized project skill
directories in projects already included in Index. Never expand project scope
from a transcript path alone. Follow symlinks only after checking the canonical
target against permissions and exclusions; break cycles and deduplicate targets.
Never-folder rules apply to both aliases and targets. Indexing permission is
separate from contribution/upload permission.

Read only bounded metadata and definitions. Never execute a skill, its scripts,
or commands embedded in configuration. Parse frontmatter defensively; preserve
malformed entries as unreadable/invalid instead of dropping them silently.
Distinguish available, disabled, cached, missing, and unknown availability.
Installed files alone do not prove an agent can activate a skill.

Run a bounded initial scan, incremental refresh after changes, periodic repair
scans, and a manual Refresh. Coalesce jobs and cancel safely on settings changes.
Expose last scan time and incomplete/inaccessible roots in the view's coverage
details, without adding a notification or banner.

## Identity and storage

Separate a skill identity, an installation, and observed usage. Identity uses
verified provider/package/repository-relative identity where available, with a
local opaque identifier otherwise. Installation records contain agent, scope,
canonical location, aliases, availability, metadata, and a content fingerprint.
Group proven copies while retaining each location. Same names or identical text
alone do not establish common provenance. Different contents stay distinguishable.
Historical events must not be reassigned when a path changes to a different skill.

Store inventory and usage in bounded daemon-local storage under existing local
protection conventions. Do not put raw paths, definitions, or trace excerpts into
audit logs, telemetry, server payloads, or the counter store's label-only rows.
Resolve definition previews from permitted local files on demand. Session evidence
links use opaque local IDs and existing session navigation.

## Usage semantics

Use deterministic provider-specific extraction from authorized local transcripts:

- Invoked: an explicit, attributable skill invocation event.
- Loaded: an observed successful read/load of the resolved skill definition.
- Mentioned or available: catalog listings, prompt inventories, prose references,
  and failed read attempts; these do not count as usage.

Show invocation counts and load counts separately. Count distinct sessions with
either observed invocation or load as Sessions used, with that definition in
coverage details. Do not imply that loading proves successful application.
Use stable session/event IDs to deduplicate overlapping exports and rereads.
Replace a session's derived events atomically on reanalysis; do not append totals.
Ambiguous names remain unattributed instead of being assigned to every match.
When invocation and load describe one activity, keep both evidence types but
count the session once. Child sessions retain separate IDs and a parent link;
the displayed session count counts each transcript once, never parent rollups
plus their child events.

Show 7/30/90-day windows, last observed use, and a daily usage trend. Bucket by
event time and local display timezone, retaining UTC timestamps. Missing event
time is undated evidence and excluded from time-window totals. The default is
30 days. Coverage reports supported agents, analyzed/partial sessions, earliest
retained evidence, and last analysis time. A 90-day window does not promise 90
days of retained history.

Use No observed usage only when a supported analysis has completed over available
history. Use Usage unavailable when analysis is disabled or the format lacks
supported evidence. Pending and partial coverage remain visible, even alongside
nonzero counts. Never label a skill unused on the basis of missing evidence.

## Index experience

Add a Skills destination inside Index without replacing existing session browsing.
Use the native macOS client as the first implementation and parity reference.
The inspected checkout currently routes history through MonitorWindowView's Home
history page and monitor views; it does not declare an Index tab. Implementation
must reconcile the intended Index surface with the current target branch before
wiring navigation, rather than silently creating a competing top-level tab.

The main table shows name, short description, agents, availability, sessions used,
invocations, and last used. Search name/description; filter by agent, availability,
and usage window. Default to alphabetical order; allow sorting by usage and recency.
A detail inspector shows definition, installation locations, separate invocation
and load totals, daily trend, evidence sessions, and coverage. Preserve selection
and filters while refreshing. Use shared copy keys and the repository copy guide.

Inventory remains useful when analysis is off. The existing Local trace analysis
settings route enables usage analysis; opening Skills must not silently grant
permission. Empty inventory, scanning, no matching results, unreadable roots,
analysis off, and partial history are distinct inline states. No new notices.

## Integration and lifecycle

Place parsing/aggregation in the permissive contributor-insights and sources
layers; keep filesystem scheduling, permissions, and store ownership in the
daemon. Extend typed IPC/FFI models for inventory, usage, coverage, and paginated
evidence. SwiftUI renders these models without doing its own filesystem scan.
No permissive crate may depend on an AGPL crate.

Reuse the bounded background scheduling principles in daemon/insights_week.rs:
coalesced jobs, streaming session reads, parser-version invalidation, cancellation,
and checking permission again before committing results. Do not perform scans on
the UI thread or reuse existing counter rows to retain sensitive definitions.
Disabling local analysis stops extraction; clearing local analysis deletes derived
usage and invalidates pending writes. Inventory refresh follows its own indexing
permission. Excluding a folder removes associated inventory/evidence and derived
counts. Removed installations keep only permitted historical identity needed to
explain retained events and are marked missing. Retention expiry removes events
and recomputes counts. Reenable performs a bounded backfill.

## Validation and delivery

Test known/configured roots, project scope, duplicate aliases, symlink escapes,
cycles, inactive plugins, malformed definitions, and same-name distinct skills.
Use fixtures from each supported trace format to prove invocations versus loads,
catalog exclusion, ambiguous attribution, failed reads, duplicate imports,
incremental replacement, time boundaries, and child-session accounting.
Exercise disabling, clearing, exclusion, and retention while a scan is in flight.
Verify counts and evidence navigation with an authorized local corpus and report
unsupported formats explicitly. Check large inventories for bounded memory and
responsive navigation; verify keyboard navigation and accessibility.

Deliver inventory first, then extraction/aggregation, then usage UI and evidence.
Run required repository checks after integration, including license boundaries;
rebuild FFI before native client tests. No new dependencies are assumed.

Implementation choices delegated to the builder: concrete scan budgets, storage
schema, watcher strategy, and pagination sizes, justified by existing conventions
and fixture measurements. Navigation integration must be verified on the target
branch. This document is a proposal for approval, not a claim of shipped behavior.

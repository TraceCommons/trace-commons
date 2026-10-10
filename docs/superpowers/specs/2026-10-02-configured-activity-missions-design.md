# Configured activity missions with rewards disabled

The user selected a configurable server with rewards off. This implements #1173
Z7 and #1174 M1–M3 without choosing economic rates or product thresholds.

This domain is `trace_activity`; the existing `/v1/missions` skill-evaluation
catalogue and its reward programs remain separate. The public
`GET /v1/activity-missions` publishes one complete, bounded policy for all
accounts. No profile, matching result, mission selection or completion assertion
is accepted. The authenticated `GET /v1/account/activity-missions/status` derives
progress from the account's active principals under tenant RLS.

`TRACE_COMMONS_ACTIVITY_MISSIONS_POLICY_JSON` optionally configures a schema-v1
policy at startup. Absence is explicitly `unconfigured`; malformed policy fails
startup. The policy has an explicit UTC date interval (maximum 366 days), a
catalogue of count-based missions, an optional global daily selection rule
(fixed mission or ordered rotation from the policy start), optional monthly
contribution-count level thresholds and optional badge thresholds. No default
selection, level, badge or reward threshold exists. Operators must preserve each
policy JSON and digest with release configuration; changing a policy recomputes
a new projection, not a durable historical award. UTC and the bounded interval
are explicit schema-v1 technical constraints, not configurable reward mechanics.

A policy explicitly chooses `accepted` or `received_or_accepted` contribution
qualification. Both operate on actual stored trace submissions, excluding
withdrawals, revocations, purges and elapsed retention. A unit is a distinct
submission ID, not a claimed local session or a quality score. Received time
chooses its UTC day; current authoritative status determines whether it still
qualifies. No tool/category inference is implemented server-side: mission
targets are contribution counts. Matching remains local.

## Mission predicates

A mission may carry an optional, versioned `predicate` block saying what local
work it asks for. The server only validates and publishes it: it never
evaluates a predicate, never infers one, and accepts no profile, matching
result or completion assertion. Progress stays count-based and ignores it.

Version 1:

```json
{"version":1,"tools":["claude-code"],"tool_families":["anthropic"],"languages":["rust"],"min_sessions":2}
```

- `tools` are session sources as the contributor client names them,
  `tool_families` are protocol families (`anthropic`, `openai`, `google`),
  `languages` are folder languages the client reads from marker files at a
  folder's root. Each list is "any of"; an empty or absent list does not
  restrict; the lists combine with AND per session. A mission fits when at
  least `min_sessions` readable sessions satisfy every non-empty list.
- Bounds, equal to what the client catalogue accepts: at most 32 values a
  list, each 1-64 bytes of lowercase ASCII letters, digits, `-` or `_`, no
  duplicates within a list; `min_sessions` 1-1000; at least one non-empty list,
  so a predicate never fits everything; a mission carrying one has a title of
  at most 200 characters. Unknown fields in a version-1 block are refused.
- `TRACE_COMMONS_ACTIVITY_MISSIONS_POLICY_JSON` accepts only version 1. A
  malformed block, or any other version, fails startup like the rest of the
  policy.
- The block is digest-covered. Every predicate block serializes as key-sorted
  JSON, and a mission without one serializes exactly as before the field
  existed, so no predicate-free policy digest moved.
- A reader that meets a later version keeps the block verbatim (key-sorted, so
  the digest still verifies) and treats the mission as having no predicate; it
  never half-reads one. A later version must therefore also be emitted
  key-sorted, and may hold no float and no integer outside i64/u64: a reader
  parses numbers with serde_json's default parser, which does not keep those
  exactly, so its re-serialized bytes, and the digest, would differ. A
  later-version block holding one is malformed and refuses the catalogue. The
  block is the policy's one versioned extension point: every
  other policy object stays strict.

Rollout order matters. Mission objects are strict, so a contributor build
from before this field refuses any catalogue in which a mission carries a
`predicate` key at all: its whole activity-missions surface reads unavailable,
not just mission fit. A policy without predicates is unaffected. Operators
should configure predicates only once the client release that reads them is
what contributors run.

The contributor daemon fetches the catalogue on its own schedule, the same
anonymous request for every contributor, and feeds only missions with a
version-1 predicate to its local matcher. Missions without one are excluded
from mission fit, so "fits a mission" never means "fits everything".

Progress is a single-snapshot database aggregate of those submissions by day,
scoped to auth-derived tenant and active principals. No new tables or migrations
are needed because assignment and completion are recomputable read-only views.
The response carries observation time, policy digest, source/window and source
predicate; retractions can reduce past progress. Monthly activity resets on the
UTC calendar month and is clipped to policy coverage, which is reported.
Current streak counts consecutive assigned/completed days ending today, or
ending yesterday when today's mission remains incomplete. It is clipped to
policy start. Badges describe current qualified progress within policy coverage;
they are not irrevocable grants. No configured daily rule means daily/streak
results are null. No configured level/badge thresholds means those results are
null. DB errors or timeouts are unavailable, never a fabricated zero.

Rewards are hard-disabled in the implementation. Policy accepts no reward amount
or enable flag. `credit_points_pending` is null with
`mission_credit_ledger_unavailable`, even for completed missions: this code has
no approved mission-credit ledger adapter. Existing corpus credit is neither
multiplied nor used as mission credit. No settlement, standing, funding, grant,
project mode, consent, approval or upload is changed by these GETs.

Implementation sequence: protocol DTO/validation and deterministic evaluator;
tenant-scoped read source; public/authenticated routes with startup config and
unbound-route classification; tests for policy validation, calendar boundaries,
withdrawal/ownership, absent policy/source and reward fail-closed behavior.
Native IPC integration is a follow-up adapter owned by the root/contributor lane;
it must fetch the common catalogue independent of local matching and preserve
null/unavailable states and the trace_activity namespace.

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
qualifies. No tool/category inference or vendor predicate is implemented:
mission targets are contribution counts. Broader matching metadata can be added
only with a source-backed, versioned predicate contract. Matching remains local.

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

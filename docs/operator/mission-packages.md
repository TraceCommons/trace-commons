# Published mission packages

Publishing a mission package makes a public task and skill artifact discoverable under an existing account-bound mission reward offer. The contributor evaluator and local attempt journal support execution; evidence submission and Cloud credit awards require further integration.

Run the production migration chain through `V72__published_mission_packages.sql` and follow [participant reward provisioning](participant-rewards.md) for the existing issuer and account runtime roles. The public catalog uses the account runtime's `trace_reward_participant_runtime` membership to execute bounded read functions without an account session or participant enrollment. Keep direct table writes and issuer membership outside the HTTP runtime.

## Publish and suspend

First use `offer-publish` to publish a readable `mission_completion` offer with `account_bound` identity mode; operator-asserted programs cannot acquire public mission packages. Supply the existing local issuer database connection through `TRACE_COMMONS_REWARDS_DATABASE_URL`, then invoke `mission-publish` with the authorized `--tenant`, the offer's `--program` UUID and the reviewed JSON file's `--package` path.

The CLI reads at most 64 KiB and validates the shared [package contract](../../crates/trace-commons-protocol/src/mission_evaluation.rs) before publication. PostgreSQL independently enforces the structural and task-control rules required for safe storage and anonymous projection, including rejecting C0/C1 controls except newline and tab. Rust remains authoritative for canonical skill rendering, renderer/hash agreement and privacy screening; the SQL boundary deliberately does not duplicate that evolving classifier. The JSON contains:

| Field | Required value or binding |
| --- | --- |
| `schema_version` | `1` |
| `mission_id`, `program_id` | Non-nil UUIDs; the program matches the command and offer |
| `offer_version_hash` | The offer's exact `sha256:`-prefixed version |
| `task` | Public task text, bounded to 8,000 UTF-8 bytes |
| `skill` | Strict `name`, `description` and `procedure` fields |
| `skill_sha256` | Bare lowercase SHA-256 of the shared renderer's exact skill bytes |
| `evaluator_id` | `skill-evaluation-v1` |
| `evaluation_contract_hash` | Bare hash returned by the intended contributor build's `evaluation_contract_hash()` |
| `execution` | The fixed policy in the shared contract |

The fixed policy requires a model owned by `nearai`, 24 requests, at most 900 output tokens per request, a 90-second request timeout and concurrency of two. The evaluator selects held-out repair-plan and applicability fixtures. It does not execute generated code. Its contract hash conservatively binds exact production evaluator, fixture, scoring, transport and renderer sources while excluding sibling test files; a production-source change requires republishing, but test-only maintenance does not invalidate packages. Structural server validation cannot establish evaluator competence or provider availability.

The issuer can publish one immutable package per program; identical retries return the original bytes, digest and timestamp, including after suspension or closure. Changed bytes conflict, and database guards refuse modification, deletion and truncation of published rows.

Use a new reviewed offer for changed terms or package content, preserving the existing work-duplication policy.

`offer-suspend --suspended true` hides its mission from public list/detail reads, and resuming restores visibility only while the offer remains open. Discovery creates no reservation.

## Public reads

| Request | Response |
| --- | --- |
| `GET /v1/missions?limit=20&before={mission_id}` | Available mission summaries and an exclusive continuation cursor |
| `GET /v1/missions/{mission_id}` | Exact canonical `package_json`, `package_sha256`, schema and publication timestamp |

List limits range from one through 50. Omit `before` for the first page and stop when `next_cursor` is null. Each page reflects current availability, so suspension or closure can remove entries between requests. Summary fields are public text; render task previews as plain text.

The [contributor client](../../crates/trace-commons-contributor/src/mission_catalog.rs) validates the configured source, host allowlist, response size and package binding, sends no account credentials and follows no redirects. All anonymous reward offer, mission list and mission detail reads share one public-read permit while also holding one permit from the unchanged two-permit reward database budget. This prevents anonymous discovery from occupying both reward database slots; account operations retain the existing tenant and global budgets. Responses return `no-store` and `nosniff` headers. Missing or unavailable missions return 404; invalid requests return 400; exhausted request admission returns 429.

## Local attempt records

[MissionAttemptStore](../../crates/trace-commons-contributor/src/mission_attempt.rs) retains separate practice and account scopes. It binds a stable request ID, spend-approval ID, package and evaluator hashes, selected model and all expected trial keys before execution. In mission reports, the legacy `review_id` field carries this approval ID; consumers must not interpret it as a separate review record. A matching retry returns the existing attempt. Partial trial results survive failure; completion requires the complete expected set. Cancellation first records the request and only reaches a terminal state after execution drains. A closed cancellation channel whose last value is false means no cancellation, rather than an implicit request.

The private, bounded journal uses atomic replacement and can recover running or cancel-requested attempts to an interrupted state at startup. Terminal content revocation removes raw trials while retaining the receipt. If abort draining encounters both a provider or cancellation failure and a trial persistence failure, `skill-evaluation-persistence-unavailable` takes precedence so lost durable evidence is never reported as provider-only. Daemon lifecycle and native controls must connect these library operations before a participant can use the workflow.

## Verification and activation

Use a fresh disposable loopback PostgreSQL database named `reward_test_*` through `TRACE_COMMONS_REWARDS_PG_TEST_URL`. The selected tests refuse a missing test URL.

```sh
cargo test -p trace-commons-server --test mission_publication_pg -- --ignored --test-threads=1
cargo test -p trace-commons-server --bin trace-commons-ingest \
  public_mission_catalog_http_is_bounded_anonymous_and_side_effect_free -- --ignored --test-threads=1
cargo test -p trace-commons-contributor --lib mission_catalog::tests
cargo test -p trace-commons-contributor --lib mission_attempt
```

The PostgreSQL tests exercise issuer authority, immutable replay, cross-tenant refusal, concurrent publication, database-clock availability and the real operator CLI. The HTTP test uses the application router and participant database role. Evaluator tests use controlled transports; a live provider run, independent review, purpose-specific evidence consent and funded Cloud-credit delivery remain separate activation evidence.

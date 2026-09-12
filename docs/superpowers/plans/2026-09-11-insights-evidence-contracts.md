# Local model and outcome evidence contracts

These permissive contributor APIs support the next evidence-linkage slice of
the [Insights program](2026-09-11-trace-insights-program.md). They add no provider
network call, storage schema, automatic attribution, or comparative ranking.

`models::extract_model_observations(source_format, bytes)` reads bounded declared
metadata from a selected Codex or trajectory file. Its result binds to a source
SHA-256 and names the coordinate system for record references. It preserves
missing, invalid, and omitted declarations, with at most 32 labels and 256
references. Labels describe declarations, not verified serving identities.
The caller must validate the source with its adapter before attaching the result,
and compare the result's source format and digest when reading a saved cache.

Version 2 observations from new Codex imports treat only official
`turn_context.model` fields as declaration candidates. A Codex source with no
turn contexts therefore has zero candidates rather than missing model metadata;
its model answer remains unavailable. Session metadata and assistant messages
do not declare a model under this version. Trajectory imports and previously
saved version 1 observations keep their existing meaning. Reads and unrelated
mutations do not reinterpret version 1 evidence; an explicit reimport writes
version 2. Store version 6 is unchanged. Older clients that only validate nested
model-observation version 1 may refuse a snapshot after that reimport, and no
automatic downgrade is provided.

`outcomes::inspect_git_commit(repository, object_id)` inspects an exact canonical
lowercase 40- or 64-hex commit ID in an explicitly selected repository. It records
only repository-path digest, object/tree/parent identifiers, inspection time,
and `inspected_local_object` provenance. Working trees, linked working trees,
and bare repositories are supported; parent discovery is refused. Git settings
that could redirect inspection or fetch missing objects are disabled. The
process/output are bounded. Commit existence says nothing about acceptance,
merge status, reversion, or which model produced the change.

`outcomes::parse_test_report(bytes)` and `import_test_report(path)` accept a
strict report of at most 64 KiB. A report includes schema version 1, a bounded
runner label, unsigned `passed`, `failed`, and `skipped` counts, `observed_at`,
and optional `commit_id`. Counts must add without overflow; timestamps before
the epoch or more than five minutes in the future are rejected. Reports retain
artifact digest and `imported_report` provenance. Importing neither executes
tests nor verifies a claimed revision or result.

Both outcome kinds expose `validate()` and `identity_digest()`. The latter
excludes import/inspection time so an explicit repeated association can be
idempotent. The consuming store must bind links to exact snapshot source digests,
retain user-link authority, enforce link limits, and invalidate associations on
source replacement or deletion. Those mutations and CLI/service entry points
are the following stacked PR.

Synthetic tests cover mixed/missing/invalid declarations, coordinate and
truncation rules, ignored model-like prose, Git object formats and repository
scope, environment/replacement overrides, deadlines, malformed reports, and
imported authority. No dependencies are added. Local fixtures do not qualify
model comparisons or independent test-execution claims.

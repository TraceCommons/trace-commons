# Skills catalog server: implementation plan

Spec: `docs/superpowers/specs/2026-11-15-skills-catalog-server.md`.

Executed under CLAUDE.md "Executing a multi-task plan": each task writes its
tests (TDD: tests first), runs at most a plain `cargo check -p <crate>
--all-targets`, and reports its tests as written, not run. The full
verification runs once after task 9.

## Task 1: protocol document module

`crates/trace-commons-protocol/src/skills_catalog/{mod.rs,tests.rs}`, exported
from `lib.rs`.

- Constants: `CATALOG_SIGNING_DOMAIN`, `MAX_CATALOG_BYTES` (4 MiB),
  `MAX_PUBLISHERS` (1000), `MAX_SKILLS` (5000), `MAX_VALIDITY_SECONDS`
  (30 days), `MAX_CLOCK_SKEW_SECONDS`, `MAX_SIGNATURES`, field bounds,
  `PATTERN_LABELS`, `CHECKS_VERSION`.
- Types (deny unknown fields): `CatalogDocument`, `CatalogPublisher`,
  `RepositorySignals`, `CatalogSkill`, `License`, `SkillChecks`,
  `BundledFiles`, `SignedCatalog`, `CatalogSignature`, `SkillSuggestion`,
  `Environment` reused from `feature_flags`.
- Functions: `signing_input`, `open_signed_catalog(bytes, keys)`,
  `decode_payload(bytes, environment, now)`, `CatalogDocument::validate`,
  URL builders, `normalize_repository`, `safe_skill_path`,
  `SkillSuggestion::validate`, `assemble_signed_catalog`.
- Tests: every rule in the spec's "Rules a reader enforces" list, signature
  domain separation against a feature-manifest signature, suggestion
  validation.

## Task 2: server checks and frontmatter

`crates/trace-commons-server/src/skills_catalog/{mod.rs,checks.rs}` (AGPL).

- `parse_frontmatter(bytes) -> Result<(name, description, license), label>`.
- `hidden_text`, `instruction_patterns`, `bundled_files(tree entries)`,
  `license_fact`, `inactive`, `blocking_labels`.
- Tests: one fixture per outcome.

## Task 3: fetcher trait, sources, crawler

`skills_catalog/{fetch.rs,sources.rs,crawl.rs}`.

- `SkillsSourceFetcher` trait (`repository`, `head`, `tree`, `participation`,
  `raw`), `HttpFetcher` (reqwest, no redirects, bounded reads, optional
  token, configurable API/raw bases), `FixtureFetcher` for tests.
- `SourcesFile` parsing; Claude marketplace parsing with the source forms in
  the spec; awesome-list link extraction.
- `crawl_repository(fetcher, target) -> RepositoryCrawl` producing entries
  with checks; caps.
- Tests: fixtures for seed, marketplace (relative/github/git-subdir/url;
  npm/command skipped), archived, truncated tree, 202 participation, private;
  HTTP fetcher against a 127.0.0.1 stub refusing a redirect and bounding a
  body.

## Task 4: V160 migration and store

`migrations/V160__skills_catalog.sql`, registered in `db/postgres.rs` with a
static pin test; `skills_catalog/store.rs` (guard role, upsert repository,
upsert skills preserving `first_seen_at`, previous hashes, mark absent on a
complete crawl, take/mark suggestions, load for export);
`Database::record_skills_catalog_suggestion` (default error, PG impl through
the guard-owned function); `tests/skills_catalog_pg.rs` own `[[test]]`.

## Task 5: export and operator binary

`skills_catalog/export.rs` (deterministic payload, suppressions, validate via
protocol) and `src/bin/trace_commons_skills_catalog.rs` (`crawl`, `export`).
`crates/trace-commons-server/skills-catalog/{sources.json,suppressions.json}`. Tests: export
determinism, suppression, blocked skills absent, sources file parses.

## Task 6: ingest suggestion route

`trace_commons_ingest_internal/skills_catalog_suggestions.rs` and the route in
the unauthenticated router. Tests: parse labels, 503 with no database.

## Task 7: contributor flags, pinned keys, CLI

- Registry v3 with `skills_server_catalog` and `skills_catalog_suggestions`;
  three profiles; `scripts/feature-flags` validator passes.
- `skills-catalog/<env>.json` pinned keys (empty), `channel::skills_catalog_url`.
- `near-ai skills-catalog signing-input|assemble|verify`.
- Tests: CLI round trip with an in-test key by injecting keys.

## Task 8: daemon server catalog and suggestions

`daemon/insights_skills/server_catalog.rs`, `daemon/insights_skills/suggestions.rs`,
snapshot fields, `insights_skills_suggestions` IPC method, copy strings.
Tests as listed in the spec.

## Task 9: Swift decoding, publish script, docs

`SkillsModels.swift` additive fields and `SkillsWireTests`;
`scripts/skills-catalog/{publish.py,test_publish.py}`;
`docs/operator/skills-catalog.md` listed in `docs/operator/README.md`;
the module doc in `insights_skills.rs` and `docs/operator/feature-flags.md`
updated for the reporting exception.

## Final verification (once)

Builds through `~/.claude/turn-locks/turn.sh build`, heavy tests through
`turn.sh test`, never both at once:

- CLAUDE.md "Local development" commands (server and ingest), clippy with
  the allow-list, `cargo fmt --all -- --check`.
- `cargo test -p trace-commons-protocol --features near-ai-privacy-filter`.
- `cargo test -p trace-commons-contributor`, `-p trace-commons-contributor-insights`.
- `skills_catalog_pg` and the migration pin test on a fresh
  `admission_test_*` database; ingest `--lib` filter for the new module.
- `license_boundary` and `unsafe_boundary`; `cargo deny check`.
- `python3 -m unittest` for the feature-flag and skills-catalog scripts,
  `scripts/ci/test-feature-profiles.sh`.
- FFI build, then `cd macos && swift test`.

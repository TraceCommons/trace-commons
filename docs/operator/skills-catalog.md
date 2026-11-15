# Skills catalog

The public Skills catalog is one signed document that every contributor
daemon downloads whole and anonymously. The operator crawls public skill
sources into PostgreSQL, exports an unsigned payload, and the owner signs it
with a KMS key and publishes it to a bucket. Design:
[`../superpowers/specs/2026-11-15-skills-catalog-server.md`](../superpowers/specs/2026-11-15-skills-catalog-server.md).

**Status (2026-11-15): not deployed.** No signing key exists, no build pins
one (`crates/trace-commons-contributor/skills-catalog/<env>.json` ship with
`signing_keys: []`), and nothing has been crawled from GitHub or uploaded.
Daemons therefore refuse every catalog and keep fetching the four fixed
publishers from GitHub.

## Pieces

| Piece | Where |
|---|---|
| Source list (seeds, repositories, marketplaces, lists) | `crates/trace-commons-server/skills-catalog/sources.json` |
| Suppression list (skill ids, publisher ids, repositories) | `crates/trace-commons-server/skills-catalog/suppressions.json` |
| Crawler and exporter | `trace-commons-skills-catalog crawl` / `export` |
| Database | V160: `trace_skills_catalog_{repositories,skills,suggestions}`, role `trace_skills_catalog_guard` |
| App suggestions | `POST /v1/public/skills-catalog/suggestions` on ingest (anonymous) |
| Signing and upload | `scripts/skills-catalog/publish.py` with `near-ai skills-catalog` |
| Pinned keys (clients) | `crates/trace-commons-contributor/skills-catalog/<env>.json` |
| Served object | `skills-catalog/<env>.json` in the feature-flag buckets |

Both lists are reviewed in Git. Suppressing a skill, publisher or repository
is a pull request to `suppressions.json`, then an export and a publish.

## One-time setup

1. Grant the crawler's PostgreSQL login the guard role (the migrator never
   keeps it):

   ```sql
   GRANT trace_skills_catalog_guard TO <crawler login>;
   ```

   The crawler refuses to start (`skills-catalog-guard-unavailable`) if it
   cannot take the role. The ingest runtime needs nothing: V160 grants it
   EXECUTE on `trace_skills_catalog_suggest` only.
2. Create one Cloud KMS key per environment, algorithm `ec-sign-ed25519`,
   held by the owner only, as for feature manifests
   ([`feature-flags.md`](./feature-flags.md), "The signing key"). Suggested
   key ring `tc-skills-catalog`, keys `skills-catalog-<environment>`. Do not
   reuse the feature-flag or witness keys.
3. Export the public key and print its raw hex:

   ```sh
   gcloud kms keys versions get-public-key 1 --key <KEY> --keyring <KEYRING> \
     --location <LOCATION> --project <PROJECT> --public-key-format pem \
     --output-file skills-catalog-<environment>-v1.pem
   near-ai feature-flags public-key --pem skills-catalog-<environment>-v1.pem
   ```

   Add `public_key_hex` to that environment's
   `crates/trace-commons-contributor/skills-catalog/<env>.json`, and ship that
   build before publishing. This does not change the feature profile digest.

## Each publication

```sh
export TRACE_COMMONS_SKILLS_CATALOG_DATABASE_URL=postgres://...
# Optional: raises GitHub's rate limit. Read from the environment, never logged.
export TRACE_COMMONS_SKILLS_CATALOG_GITHUB_TOKEN=...
trace-commons-skills-catalog crawl \
  --sources crates/trace-commons-server/skills-catalog/sources.json
trace-commons-skills-catalog export --environment staging \
  --suppressions crates/trace-commons-server/skills-catalog/suppressions.json \
  --output skills-catalog-staging.json
python3 scripts/skills-catalog/publish.py --environment staging \
  --payload skills-catalog-staging.json --cli target/debug/near-ai
```

`crawl` prints counts and outcome labels only. It stops at the first GitHub
rate limit and keeps what it recorded; run it again later. A repository whose
crawl was incomplete keeps its skills; only a complete crawl marks a skill
absent.

`export` prints the revision (`issued_at` in Unix seconds), counts of
published, blocked, suppressed, absent and over-budget skills, and the
payload's SHA-256. Validity defaults to 14 days (`--validity-days`, at most
30). It refuses an empty catalog.

The publish dry run prints the target, revision, size, SHA-256, current
object generation, and the added, removed and changed skill ids. Add
`--publish` with `--kms-location --kms-keyring --kms-key --public-key-pem`
(and `--kms-project`, `--account` as needed) to sign and upload. It refuses a
revision not above the current object's, and an absent object unless
`--initial` (first publish only). It uploads with `Cache-Control: public,
max-age=300` and a generation precondition.

Publish again before `expires_at`: an expired catalog is not used, and
daemons fall back to GitHub.

## Suggestions

Apps may report a public skill the catalog lacks: off by default, behind the
`skills_catalog_suggestions` flag and the user's own opt-in. The route keeps
no address and logs labels only. Each crawl takes up to 50 repositories with
pending suggestions. A suggested-only repository's skills are published only
when it has at least 10 stars and is at least 30 days old.

To read the queue, as the guard:

```sql
SET ROLE trace_skills_catalog_guard;
SELECT status, count(*), sum(report_count) FROM trace_skills_catalog_suggestions GROUP BY status;
```

## Not yet built

- A drill with hash-only evidence for the rollout smoke (spec, "Drill").
- Codex and Cursor marketplace parsing (no documented format confirmed).

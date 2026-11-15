# Server-side Skills catalog

Status: draft for owner review, 2026-11-15. Branch `skills-catalog-server`.

## Problem

Today every contributor daemon fetches four fixed publishers straight from
GitHub (`crates/trace-commons-contributor/src/daemon/insights_skills.rs`):
a ref lookup, a recursive tree and one raw read per `SKILL.md`, per
publisher, every six hours while a publisher is followed or Skills is open.
That has three costs. The catalog can never grow past a hand-edited list in
a client release. Every install spends GitHub's anonymous rate limit
(60 requests an hour per address) on the same public bytes. And the
daemon can only describe a skill by its name and hash: it has no licence,
no repository signals, and no record of what the skill bundles.

This slice moves catalog work to the server. A crawler finds skills on a
bounded set of public sources, checks and enriches them, and the owner
publishes one signed document that every daemon downloads whole,
anonymously, on its own schedule.

## Owner decisions this spec implements

1. Sources: (a) plugin marketplaces, (b) a maintained list of known
   repositories and awesome-lists seeded with today's four publishers,
   (c) skills that apps report. No GitHub-wide code search.
2. Curation is auto-publish with checks, plus an operator suppression list.
   Check results are facts the app can show, never a "safe" verdict.
3. Delivery is one signed static document in a bucket, fetched anonymously,
   verified against a pinned key, bounded in size, and never revealing which
   skills a user views or has installed.
4. Apps may report skills the catalog lacks: opt-in, off by default, behind
   its own flag, limited to publicly resolvable sources, without identity if
   possible, abuse-limited on the server.

## Architecture

```
 sources.json ─┐
 marketplaces ─┼─> trace-commons-skills-catalog crawl ──> PostgreSQL (V160)
 suggestions ──┘        (operator binary)                     │
        ^                                                     v
        │                trace-commons-skills-catalog export ─> payload.json (unsigned)
        │                                                     │
 POST /v1/public/skills-catalog/suggestions                   v
 (ingest, anonymous)          scripts/skills-catalog/publish.py
        ^                       near-ai skills-catalog signing-input
        │                       gcloud kms asymmetric-sign   (owner key)
        │                       near-ai skills-catalog assemble / verify
        │                       gcloud storage cp --if-generation-match
        │                                                     │
        │                                                     v
 daemon (opt-in reports)  <──── gs://<bucket>/skills-catalog/<environment>.json
                                daemon fetch: anonymous, pinned key, bounded
```

### Crawler: an operator binary, not a worker route

Choice: the crawler is a new operator binary, `trace-commons-skills-catalog`,
in `crates/trace-commons-server` (AGPL), run by the operator on a schedule
(systemd timer or by hand). It is not an ingest worker route.

Reasons. Publication needs the owner's KMS signature anyway, so a worker
route could only ever stage an unsigned payload for a human. The crawler
makes hundreds of outbound GitHub requests per run; keeping that out of the
ingest process keeps ingest's egress, timeouts and failure modes unchanged.
And it needs no new scoped bearer token (the repo's rule is one credential
per worker scope; a binary that talks to PostgreSQL directly needs none).
If the owner later wants unattended publication with a service-held key, the
crawl and export subcommands become the body of a worker without changes.

Subcommands:

- `crawl --sources <file>`: reads the source list, expands marketplaces one
  hop, takes pending suggestions from PostgreSQL, fetches each repository at
  its default-branch head, runs every check, and upserts repositories and
  skills. Bounded per run (`--max-repositories`, default 200; per-repository
  caps below). Optional GitHub token from
  `TRACE_COMMONS_SKILLS_CATALOG_GITHUB_TOKEN` (raises GitHub's rate limit;
  the crawler never creates one). The database URL comes from
  `TRACE_COMMONS_SKILLS_CATALOG_DATABASE_URL`.
- `export --environment <env> --suppressions <file> --output <file>
  [--validity-days N]`: builds the unsigned payload from the database,
  deterministically (sorted, no wall-clock fields other than
  `issued_at`/`expires_at`), applies the suppression list, and validates it
  with the same protocol code a client runs.

All GitHub access goes through one trait (`SkillsSourceFetcher`) with an
HTTP implementation and a fixture implementation. Tests never reach the
network: they use the fixture or a local stub server bound to 127.0.0.1.

### Sources

`crates/trace-commons-server/skills-catalog/sources.json`, reviewed in Git:

```json
{
  "schema_version": 1,
  "repositories": [
    {"repository": "obra/superpowers", "publisher_id": "obra",
     "publisher_name": "Obra", "skills_root": "skills",
     "instructions_url": "https://github.com/obra/superpowers#installation"}
  ],
  "marketplaces": [{"repository": "anthropics/claude-plugins-official"}],
  "lists": []
}
```

- **Seeded repositories** carry today's publishers' fixed ids (`poteto`,
  `obra`, `slack`), so existing follows, feed history and app cover art
  (seeded from catalog id and publisher id) do not change.
- **Rex is not a seed and cannot be crawled.** Today's fourth publisher,
  `rex`, is documentation only (empty repository in `insights_skills.rs`,
  status `unavailable`). Its skill ships inside the Rex app: `rex skill
  setup` (or "Install Rex for Agents") writes a small stub `SKILL.md` into
  each agent's skills directory, and the stub tells the agent to run
  `rex __internal skill`, which prints the guide from the installed CLI.
  Rex's documentation
  (superlogical.com/rex/docs/automate/agent-skill, read 2026-11-15) names
  no public repository, so there is no commit and no file for the crawler
  to fetch, hash or check. The daemon keeps `rex` as a fixed client-side
  publisher with its existing id, alongside the server catalog's
  publishers, so follows and history are unchanged. If Rex later publishes
  its skill in a public GitHub repository, it can be added to
  `sources.json` with `publisher_id: "rex"`.
- **Plugin marketplaces.** Claude Code: `.claude-plugin/marketplace.json`,
  verified against the published marketplace reference
  (code.claude.com/docs/en/plugins/marketplace-reference). Plugin entries are
  resolved only for these `source` forms: a relative path (`./...`, or a bare
  name under `metadata.pluginRoot`) inside the marketplace repository;
  `{"source":"github","repo":"o/r"}`; `{"source":"git-subdir","url",
  "path"}` and `{"source":"url","url"}` only when the URL is github.com.
  `npm`, `archive` and `command` sources are skipped: they are not GitHub
  content at a commit, and `command` runs code on the installer's machine.
  Skills are taken from `<plugin>/skills/*/SKILL.md`. One hop only: a
  marketplace's plugins are crawled, a plugin's own marketplace is not.
  At most 200 plugin entries per marketplace.
  Codex: `.agents/plugins/marketplace.json` appears to share the same
  `plugins[].source` shape, but the only sources found were third-party
  write-ups, not OpenAI's documentation. Cursor: no documented marketplace
  file was found. Neither is parsed in this slice; their repositories can be
  listed directly in `repositories` (cursor/plugins already is). See
  "Unverified".
- **Lists** (awesome-lists): a repository plus a file path; the crawler takes
  `https://github.com/<owner>/<repo>` links from that one Markdown file (at
  most 500), and treats each as a repository with `skills_root` unset
  (`**/SKILL.md`, depth-bounded). This is the "maintained list" source; it is
  empty in the shipped file and the owner adds lists by review.
- **Suggestions** (decision 4): pending rows from V160, crawled like a
  repository with no root, and marked `crawled` or `refused`.

A repository found by several sources keeps the most trusted kind, in order
`seed`, `list`, `marketplace`, `suggested`.

### Per-repository fetch

1. `GET /repos/{o}/{r}`: default branch, `archived`, `stargazers_count`,
   `pushed_at`, `created_at`, `private` (must be false), licence SPDX id.
2. `GET /repos/{o}/{r}/git/ref/heads/{default_branch}`: the head commit
   SHA (the ref must name a commit object), which pins everything below.
3. `GET /repos/{o}/{r}/git/trees/{sha}?recursive=1`: refuse a truncated tree.
4. `GET /repos/{o}/{r}/stats/participation`: commit counts for the last 1
   and 4 weeks. A 202 (GitHub is computing) records "unknown", never zero.
5. `raw.githubusercontent.com/{o}/{r}/{sha}/{path}` for each `SKILL.md`.
   A `LICENSE*` file in the skill's directory is detected from the tree
   listing; its content is not fetched.

Caps: 256 skills per repository, 256 KiB per `SKILL.md`, 8 MiB of tree JSON,
20 s per request, no redirects (the client refuses them), responses read
with a byte bound. A 429, or a 403 that GitHub marks as a spent rate limit
(`x-ratelimit-remaining: 0` or `retry-after`), stops the run; any other 403
is about that one repository and is recorded as `http_error`.

Order: seeds first, then repositories never crawled, then the least recently
crawled, so a run that stops at its bound or a rate limit reaches different
repositories next time.

## Checks

Every check is a fact. Blocking checks keep a skill out of the document;
annotating checks are published with it. The app shows facts; nothing in the
document says "safe", "verified" or "passed".

| Check | Fact published | Blocks publication |
|---|---|---|
| Pinned content | `revision` (40-hex commit) and `sha256` of the exact `SKILL.md` bytes at that commit | yes, when either is missing |
| Frontmatter | `frontmatter: valid` | yes, when not valid: missing `---` block, missing or empty `name`/`description`, a repeated key, a flow collection as a value, control characters, or a name over 128 bytes. A description over 1024 bytes is cut at a character boundary and marked with an ellipsis, not blocked |
| Size | `document_bytes`, `directory_bytes`, `directory_files` | yes, over 256 KiB / 4 MiB / 500 files |
| Hidden text | label `hidden_text` | yes: bidirectional embeddings, overrides and isolates (U+202A-202E, U+2066-2069), the zero-width space, invisible operators (U+2060-2064), a BOM after the start, Unicode tag characters outside an emoji flag sequence, or an HTML comment over 200 bytes. The zero-width joiner and non-joiner and the LRM/RLM marks are not hidden text: emoji, Persian, Indic and mixed-direction writing need them |
| Repository archived | `archived: true` | yes |
| Repository private / unreachable anonymously | none | yes |
| Suppressed | none | yes (skill id, publisher id, or repository) |
| Suggestion provenance | `source_kind: suggested` | yes for suggested-only repositories under 10 stars or created under 30 days ago |
| Bundled files | counts of `scripts` (by extension: sh, bash, zsh, py, js, mjs, ts, rb, pl, ps1, bat, cmd), `executables` (Git mode 100755) and `binaries` (known binary extensions) in the skill's directory | no |
| Licence | `license: {status: declared/absent/unknown, spdx}` from frontmatter `license`, a `LICENSE*` file in the skill directory, then the repository's licence | no |
| Activity | `days_since_push`, `commits_7d`, `commits_28d`; `inactive` label when no push for 365 days | no |
| Download and execute | label `pipe_to_shell` (curl/wget output piped to a shell or Python) or `remote_script_exec` (`bash <(curl ...)`, `eval`/`sh -c "$(curl ...)"`, PowerShell `irm ... \| iex` and `DownloadString ... iex`) | yes (owner decision, 2026-11-15) |
| Instruction patterns | labels, each a substring family: `credential_paths` (~/.ssh, ~/.aws, .netrc, keychain), `exfiltration_endpoint` (webhook.site, requestbin, ngrok, pastebin-style hosts), `encoded_blob` (base64 run over 200 chars), `instruction_override` ("ignore previous instructions" family), `sudo` | no |

Blocking instruction patterns are `hidden_text` and the two
download-and-execute patterns. Hidden text exists to show a reviewer
something other than what the agent reads. Download and execute hands the
agent code that no check here can see (owner decision, 2026-11-15). Both
are common enough in honest installation steps that the cost is real: a
sweep of 1,096 local `SKILL.md` files on 2026-11-15 blocks 6, all
installer one-liners (5 `pipe_to_shell`, one of them also
`remote_script_exec`; 1 `remote_script_exec` alone, the Homebrew
`bash -c "$(curl ...)"` installer). The remaining patterns are shown as
facts.

Every check is a pure function of fetched bytes and metadata, unit-tested
with fixtures, and versioned (`checks_version` in the document, now 2: the
download-and-execute patterns moved from annotating to blocking, so a
published skill never carries them). A change to any rule bumps it, and the
exporter also drops a stored row whose patterns block under the current
rules even if it was crawled under an older version.

## Storage: V160

V159 is taken by open PR #2075; this slice uses V160 (the runner applies
unrecorded versions in order and allows gaps).

No tenant: catalog data is public and belongs to no account. Following
V152/V153: forced RLS, PUBLIC has nothing, and one NOLOGIN NOSUPERUSER
NOBYPASSRLS role, `trace_skills_catalog_guard`, holds every privilege and
policy.

- `trace_skills_catalog_repositories`: repository (lowercase `owner/repo`),
  source kind, publisher id and name, skills root, instructions URL, head
  revision, GitHub signals (stars, pushed_at, created_at, archived, licence
  SPDX, commits 7d/28d, signals_checked_at), last crawl time and a crawl
  outcome label.
- `trace_skills_catalog_skills`: skill id, publisher id, repository, path,
  revision, sha256, name, description, licence, `checks` (JSONB in the
  protocol's shape), blocking labels, up to 8 previous sha256 values,
  first_seen_at, last_changed_at (sha256 changed), last_seen_at, present.
  A skill missing from a complete crawl of its repository is marked not
  present; an incomplete crawl never removes anything.
- `trace_skills_catalog_suggestions`: repository, path, last reported
  revision, first and last received *day* (truncated by the database clock),
  report count (capped at 1,000,000), status (`pending`, `crawled`,
  `refused`). No address, no time finer than a day, no request id: a row
  joins to nothing, and two reports of the same skill are one row.

The crawler connects with an operator login that the operator has granted
the guard (`GRANT trace_skills_catalog_guard TO <crawler login>`) and runs
`SET ROLE trace_skills_catalog_guard` on each connection; it refuses to run
if that fails. The ingest runtime gets EXECUTE on one SECURITY DEFINER
function, `trace_skills_catalog_suggest(repository, path, revision)`, owned
by the guard. It validates the shape again, upserts the one row, and refuses
with `skills_catalog_suggestions_full` when 10,000 rows are pending.

## Document schema (version 1)

Protocol module `trace_commons_protocol::skills_catalog` (MIT OR Apache-2.0),
shared by the crawler, the CLI and the daemon. Served object:

```json
{"payload": "<the catalog JSON text>",
 "signatures": [{"key_id": "<16 hex>", "signature": "<128 hex>"}]}
```

Ed25519 over `trace-commons.skills-catalog.v1\n` followed by the payload
bytes. Its own domain string, so neither a feature-manifest signature nor
any other document signature verifies as a catalog. Its own key per
environment; the feature-flag keys are not reused. At most four signatures,
for rotation. The payload is read only after a signature verifies.

Payload:

```json
{
  "schema_version": 1,
  "checks_version": 2,
  "environment": "staging",
  "revision": 1763251200,
  "issued_at": "2026-11-16T00:00:00Z",
  "expires_at": "2026-11-30T00:00:00Z",
  "publishers": [{
    "id": "obra", "name": "Obra", "repository": "obra/superpowers",
    "url": "https://github.com/obra/superpowers",
    "instructions_url": "https://github.com/obra/superpowers#installation",
    "skills_root": "skills",
    "source_kind": "seed",
    "signals": {"stars": 1234, "pushed_at": "...", "created_at": "...",
                "archived": false, "commits_7d": 3, "commits_28d": 12,
                "checked_at": "..."}
  }],
  "skills": [{
    "id": "obra:skills/brainstorming/SKILL.md",
    "publisher_id": "obra",
    "name": "brainstorming", "description": "...",
    "repository": "obra/superpowers",
    "path": "skills/brainstorming/SKILL.md",
    "revision": "<40 hex>", "sha256": "<64 hex>",
    "previous_sha256": ["<64 hex>"],
    "source_url": "https://github.com/obra/superpowers/blob/<rev>/skills/brainstorming/SKILL.md",
    "license": {"status": "declared", "spdx": "MIT"},
    "first_seen_at": "...", "last_changed_at": "...",
    "checks": {
      "frontmatter": "valid",
      "document_bytes": 2048, "directory_bytes": 4096, "directory_files": 2,
      "bundled": {"scripts": 0, "executables": 0, "binaries": 0},
      "patterns": ["sudo"],
      "inactive": false
    }
  }]
}
```

Rules a reader enforces (the same function on server and client):

- `schema_version` 1; an unknown field anywhere is refused (deny unknown
  fields), so a later field means a new schema version.
- `environment` equals the reader's; `revision` > 0 and equals
  `issued_at` as Unix seconds (monotonic by construction, so no operator
  picks a revision); validity positive and at most 30 days; `issued_at` at
  most five minutes ahead of the reader's clock.
- Bounds: signed document at most 4 MiB; at most 1,000 publishers and
  5,000 skills; name 128 bytes, description 1,024, at most 8 previous hashes,
  at most 16 pattern labels from the fixed set.
- Ids: publisher id `^[a-z0-9][a-z0-9_-]{0,63}$` (never starting `gh.`) for seeds, else
  `gh.<owner>.<repo>` lowercase; skill id is `<publisher_id>:<path>`; both
  unique; skills sorted by id; every skill's publisher present and its
  repository equal to the publisher's.
- Repository is a lowercase `owner/repo`; path a safe relative path ending
  `/SKILL.md` (or `SKILL.md`) with no `.`/`..`/empty segments, at most 512
  bytes; revision 40 lowercase hex; sha256 64 lowercase hex; `source_url`
  equals the URL built from repository, revision and path, and `url` the
  one built from the repository, so no URL in the document is free text.
- Seed publishers keep `instructions_url` from the source list; other
  publishers' `instructions_url` is their repository URL.

Versioning: a breaking change is `schema_version: 2`, served at a new object
path (`skills-catalog/v2/<env>.json`) while v1 keeps being published until
the oldest supported client reads v2. `checks_version` moves independently
and only changes what the facts mean, never their shape.

## Signing and publishing

As for feature flags (`docs/operator/feature-flags.md`), at first:

- One Cloud KMS key per environment, `ec-sign-ed25519`, held by the owner
  only (`roles/cloudkms.signerVerifier`), never by a service account, with
  KMS data-access audit logs on. Suggested names: key ring
  `tc-skills-catalog`, keys `skills-catalog-<environment>`. **No key exists
  yet**; this slice creates none.
- Pinned keys live in `crates/trace-commons-contributor/skills-catalog/
  <environment>.json` (`signing_keys`, raw hex). They ship empty: a build
  with no pinned key refuses every catalog and keeps the direct GitHub path.
  Pinning does not touch the feature-flag profile, so its digest and cached
  flag decisions are unaffected.
- `scripts/skills-catalog/publish.py --environment <env> --payload <file>
  --cli <near-ai>` previews (revision, counts, size, SHA-256, destination,
  current generation, added/removed/changed skills) and writes nothing.
  `--publish` also needs `--kms-location/--kms-keyring/--kms-key/
  --public-key-pem`; it has the CLI write the signing input
  (`near-ai skills-catalog signing-input`), signs with `gcloud kms
  asymmetric-sign`, has the CLI check the signature against a key pinned for
  that environment (`assemble`), verifies the document exactly as a client
  would (`verify`), and uploads with `--if-generation-match` and
  `Cache-Control: public, max-age=300`, `Content-Type: application/json`.
  It refuses a revision not above the current object's, and an absent object
  unless `--initial`.
- Destination: `skills-catalog/<environment>.json` in the same buckets as
  feature manifests (`tracecommons-flatpak` for production,
  `tc-staging-updates-20261022` for staging and testing), so no new bucket
  or ACL. As for flags, bucket writers can withhold or delete the object
  (clients fall back) but cannot change a skill.

## Daemon

### Fetch

New flag `skills_server_catalog` (product, default false, live, capability
`skills_server_catalog_v1`, remote-enable permitted in all three profiles).
When it and `insights_skills` are on and Local trace analysis is on (today's
gate for catalog requests):

- `GET <fixed bucket URL>` with no credentials, no cookies, no proxy, no
  redirects, HTTPS only, a 20 s timeout, `Accept: application/json`, a
  JSON content type required, at most 4 MiB streamed. The URL is fixed per
  build channel and environment (`channel::skills_catalog_url`). The request
  carries nothing derived from inventory, follows or views; every install
  downloads the same whole object. That is the whole of decision 3's privacy
  property: the server learns only that an address fetched the catalog.
- On the existing refresh schedule, unchanged: at most every six hours in
  the background while a publisher is followed, and when Skills is opened
  or refreshed (30 s debounce). With no key pinned no catalog request is
  made at all.
- Verified with the pinned keys, then `decode_payload` with the build's
  environment and the current time. A revision lower than the highest ever
  accepted is refused (`skills-catalog-rollback`); the watermark persists
  separately from the cache, like the feature-flag watermark.
- The verified signed bytes are cached (`insights-skills-catalog-<env>-v1
  .json`, 0600, atomic) and re-verified on load.

### Migration path and fallback

`PublicState` (the existing cache) keeps its exact format, because its
loader denies unknown fields and an older daemon must still read it.

When a valid, unexpired server catalog is held and `skills_server_catalog`
is on, a refresh does not contact GitHub. Instead, for each seeded publisher
the daemon converts that publisher's server skills into today's
`CatalogSkill` rows (id `<seed>:<path>`, the same source URL and
instructions URL) and feeds them through the existing `accept_fetched`.
Follows, the update feed and hash history therefore continue unchanged for
the four publishers. Skills from every other publisher are read from the
server catalog cache only, never persisted into `PublicState`.

When the flag is off, no valid catalog is held, the catalog is expired,
no key is pinned, or verification fails, the daemon does exactly what it
does today (direct GitHub fetch of the four publishers). Removing that path
is a later change, after a published catalog has been live for one release.

### Snapshot

The Skills snapshot is bounded by the 1 MiB IPC frame. Today the catalog
rows are capped at 180 KiB. With a server catalog the daemon keeps that cap
and orders rows: catalog skills matching an installed skill, then seed and
followed publishers' skills, then the rest by repository stars; `partial`
is true when rows were cut. New fields, all optional and additive:

- `catalog.source`: `"server"` or `"github"`.
- `catalog.revision`, `catalog.issued_at`, `catalog.expires_at`: server
  catalog identity, absent for `github`.
- `catalog.total_skills`: skills in the whole catalog.
- `catalog.publishers[]`: `url`, `source_kind`.
- `catalog.publisher_states[].signals`: `stars`, `pushed_at`,
  `repository_checked_at`, `commits_7d`, `commits_28d`,
  `activity_checked_at`), filled from the server catalog and decoded in
  Swift as `SkillsRepositorySignals`.
- `catalog.skills[]`: `license`, `first_seen_at`, `last_changed_at`,
  `checks` (the document's object), `checks_version`.
- `suggestions`: `{available, opted_in, pending, sent_total, last_sent_at}`
  (below).

Updates: an installed skill matching a non-seed server skill gets freshness
from the server's `previous_sha256` exactly as the four publishers do from
local history.

## Reporting missing skills (decision 4)

### What is sent

A suggestion is `{repository, path, revision}`: a public GitHub repository,
a path to a `SKILL.md`, and a 40-hex commit. Nothing else.

The daemon derives candidates only from installed skills whose source came
from a git checkout (`git_source`: the only provenance that carries a
commit, and only when the local bytes equal the tracked blob), whose remote
normalises to a github.com `owner/repo`, and which are not already in the
catalog (by repository and path). Lock-file and plugin-manifest sources have
no commit and are never eligible. Local-only skills have no source.

Before sending, the daemon proves publicness **without our server**: an
anonymous GET of `https://raw.githubusercontent.com/<repo>/<rev>/<path>`
(no credentials, no redirects, 256 KiB bound) must return 200 and bytes
whose SHA-256 equals the installed skill's. A private repository, an
unpushed commit or a local edit fails this, and that skill is never sent;
the failure is remembered by a keyed local hash for 30 days. GitHub sees an
anonymous request for a repository it hosts; our server sees nothing.

### When

Only when all of these hold: `insights_skills` on, `skills_catalog_suggestions`
flag on, the user's own opt-in (stored in
`insights-skills-suggestions-v1.json`, default false), and Local trace
analysis on. At most one suggestion per request, at most four requests per
day, each after an independent random delay of 1-30 minutes, to a different
host from the catalog fetch, and each candidate at most once per 30 days.
Candidates come only from the inventory a Skills view already scanned; the
reporter never starts a scan of its own.
One per request means a server log line could never hold the set of skills
one install has.

### Flag class

The owner asked for "its own product flag". The registry rule says anything
that changes what leaves the machine is `privacy_sensitive`. But `evaluate()`
blocks every flag that is not in a profile's `remote_enable`, and the
validator refuses `privacy_sensitive` in `remote_enable`, so a
`privacy_sensitive` flag can never be on today. This slice follows the
owner's wording: `skills_catalog_suggestions` is `product`, default false,
and it only makes the toggle available; nothing is sent until the user opts
in. Listed for confirmation.

Production waits for copy approval (owner decision, 2026-11-15): only the
staging and testing profiles declare `skills_catalog_suggestions_v1` and
permit its remote enable. A production build cannot enable the flag by
manifest or local override; adding it to the production profile, after the
consent copy is approved, ships through a release.

### Anonymous vs authenticated

| Option | Abuse control | Privacy | Cost |
|---|---|---|---|
| A. Anonymous POST, per-address and global rate limits in memory, server re-derives everything from GitHub | weak per sender, strong in aggregate (global caps, dedup, provenance check before publication) | the address only, in memory, never stored or logged | small; same shape as `/v1/usage-events` |
| B. Contributor credential | strong (per account) | links an account to the skills it has installed: breaks the existing guarantee | small |
| C. Anonymous rate-limit tokens (blind-signed per enrolled install, Privacy Pass style) | strong per sender | unlinkable | new cryptography, issuance route, a dependency; weeks |
| D. A over an oblivious relay (OHTTP or the witness) | as A | hides the address too | a relay we do not run yet |

Recommendation and implementation: **A**. What is sent is public by
construction and is only a candidate: the server never fetches a submitted
URL as given, re-normalises the repository strictly, crawls it itself, and
publishes nothing that fails the checks, including the suggestion provenance
threshold (10 stars, 30 days). The worst an abuser can do is fill the
pending queue (capped at 10,000 rows) or spend crawl budget (capped per
run). C is the upgrade if abuse appears; D if the owner wants the address
hidden from us as well.

### Server route

`POST /v1/public/skills-catalog/suggestions`, unauthenticated, outside tenant
context, registered beside `/v1/usage-events`. Body at most 2 KiB:
`{"repository": "...", "path": "...", "revision": "..."}`, deny unknown
fields, validated with the protocol's `SkillSuggestion::validate`. Rate
limits: 10 per minute per address and 600 per minute globally
(`ACCOUNT_RATE_LIMITER`). Answers 202 when stored, 400 with a fixed label,
429, or 503 when the database is missing or the queue is full. Logs carry
fixed labels only, never the repository, path or address.

### Consent copy (needs owner approval)

Added to `skills_copy.rs` and listed in the report, not wired to any view:

- `skills_suggestions_toggle`: "Suggest public skills for the catalog"
- `skills_suggestions_detail`: "Sends the GitHub address of public skills
  you use that the catalog doesn't list yet. Never sends private or local
  skills, their contents, or who you are."

## Failure behaviour

- Crawler: a repository that fails any step records a label and keeps its
  previous rows; an incomplete tree or a failed `SKILL.md` read never marks
  skills absent. A rate limit (above) stops the run with the
  `rate_limited` label; rows already written stay. A database error aborts
  the run.
- Export: drops the lowest-ranked skills (suggested before marketplace
  before list before seed, then fewest stars) past 5,000 skills or a 3 MiB
  payload and counts them as `over_budget`; refuses (non-zero, label) if
  nothing is publishable or the result fails `decode_payload`.
- Publish: refuses before upload on any check, as for feature flags.
- Daemon fetch: a refused or unreachable document is silent and falls back
  to the verified cache while it is unexpired, then to GitHub. Only the
  GitHub path's own failures set the existing `error` label; nothing is a
  notice.
- Suggestions: only definitive outcomes are remembered for 30 days (the
  server took it, the server refused it with 400, or GitHub does not serve
  those bytes anonymously). A timeout or a 5xx is tried again at a later
  attempt, after its random delay and within the daily cap.
- Ingest route: database absent or write failure answers 503, which the
  daemon treats as no answer.

## Licensing and boundaries

- Protocol module and daemon/CLI changes: MIT OR Apache-2.0.
- Crawler, storage, ingest route: AGPL, with headers.
- No permissive crate gains an AGPL dependency; no `unsafe`; no new
  dependencies (reqwest, serde_json, ring, sha2, tokio-postgres, clap are
  already present in the crates that use them).

## Test plan

- Protocol: sign/verify round trip with an in-test keypair; wrong domain,
  feature-manifest signature, unknown key, five signatures, oversize
  document refused; every payload rule (environment, revision/issued_at,
  validity, skew, bounds, ids, URLs, ordering, unknown field); suggestion
  validation.
- Server checks: one fixture per check outcome; frontmatter edge cases;
  hidden text; licence precedence; bundled-file counts.
- Crawler: fixture fetcher for a seed repository, a Claude marketplace with
  relative, github and git-subdir sources plus skipped npm/command, an
  archived repository, a truncated tree, a 202 participation; an HTTP
  fetcher test against a 127.0.0.1 stub that refuses a redirect.
- Export: deterministic bytes; suppression by skill, publisher and
  repository; blocked skills absent; result passes `decode_payload`.
- PostgreSQL (`skills_catalog_pg`): migration applies as a non-superuser
  migrator; the runtime can only call the suggest function; the guard can
  upsert/read; first_seen is kept and last_changed moves only with sha256;
  pending cap; the static migration-pin test in `postgres.rs`.
- Ingest: body parsing labels; unauthenticated route registered; 503 with no
  database.
- Daemon: server catalog accepted only with a pinned key and current
  environment; rollback refused; expired falls back; seeds flow into
  `PublicState` with unchanged ids; GitHub not contacted while a server
  catalog is held (the fetch closure records calls); snapshot fields and
  ordering; suggestions: eligibility, publicness check (stub), opt-in and
  flag gates, one per request, no auth header or cookie sent.
- CLI: `skills-catalog signing-input/assemble/verify` round trip.
- Publish script: dry run, revision floor, `--initial`, generation
  precondition, signer required (fakes, as `test_publish.py`).
- Swift: decode a snapshot with every new field and one without any.

### Drill (follow-up, not in this slice)

`trace-commons-skills-catalog drill` against a fixture source: crawl, export,
sign with a throwaway key, verify with that key pinned, and record hash-only
evidence (payload SHA-256, counts, check-label histogram) for the rollout
smoke. Wiring it into `operator-smoke` evidence is listed as follow-up.

## Choices for the owner to confirm

1. Crawler as an operator binary rather than an ingest worker route.
2. Blocking vs annotating split in the checks table. Decided 2026-11-15:
   `pipe_to_shell` and `remote_script_exec` block; the other instruction
   patterns annotate; `hidden_text` and `archived` block.
3. The suggestion provenance threshold (10 stars, 30 days) for
   suggested-only repositories.
4. Suppressions as a reviewed file (`crates/trace-commons-server/skills-catalog/suppressions.json`)
   passed to `export`, rather than a database table.
5. `skills_catalog_suggestions` as a `product` flag (your wording) rather
   than `privacy_sensitive` (the registry rule, which today cannot be
   enabled at all).
6. Anonymous submission (option A) with one suggestion per request.
7. Decided 2026-11-15: catalog validity 14 days by default, 30 at most
   (feature manifests: 7); revision equal to `issued_at`.
8. Same buckets as feature manifests, under `skills-catalog/`.
9. Codex and Cursor marketplaces not parsed until their formats are
   documented.
10. Follows stay limited to the four seeded publishers in this slice.
11. Decided 2026-11-15: the production profile does not declare
    `skills_catalog_suggestions_v1`; staging and testing do. Production
    waits for copy approval.
12. A description over 1024 bytes is shortened with an ellipsis rather
    than blocking the skill (3 of 1,096 local skills swept on 2026-11-15
    would otherwise have been dropped).

## Unverified

- No signing key exists; nothing has been signed with a real key, uploaded,
  or fetched from a bucket.
- The crawler has run only against fixtures and a local stub, never GitHub.
- Codex's `.agents/plugins/marketplace.json` shape (third-party sources only)
  and any Cursor marketplace format.
- The consent copy is not approved.

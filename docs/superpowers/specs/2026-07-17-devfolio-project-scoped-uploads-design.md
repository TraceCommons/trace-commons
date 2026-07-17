# Devfolio project-scoped uploads — design

Date: 2026-07-17
Status: Approved (brainstorming), pending implementation plan

## Problem

The contributor CLI (`crates/trace-commons-contributor`) treats a contributor's
**entire local corpus** as the candidate set for upload: it scans both hardcoded
roots (`~/.claude/projects/**` and `~/.codex/sessions/**`), one `.jsonl` = one
session = one submission, and subsets at submit time via an interactive picker
or the `--project` / `--source` / `--since` filters.

The devfolio workflow needs two things this does not cleanly support:

1. **User-controlled, project-scoped upload.** A hackathon participant wants to
   upload only the traces they produced while building *this hackathon project*,
   not their whole machine's history. Today's `--project` filter matches on the
   **cwd basename** (`SessionRef.project`), which is unreliable
   (hyphen-ambiguous decoding — see the caveat around `claude_code.rs:58-72`).

2. **Envelope → devfolio submission linkage.** Uploaded envelopes must carry the
   devfolio submission they belong to, so devfolio can associate traces with a
   hackathon submission for judging/credit.

## Decisions (brainstorming outcome)

- **Boundary is bound to a devfolio submission**, and users must still control
  what is uploaded. Both requirements are in scope.
- **Submission link is self-asserted.** The participant supplies a submission id;
  it rides on the envelope as attribution-only metadata. The server does **not**
  verify it. Devfolio cross-checks out of band against its own records. This is
  the same trust class as `tenant_scope_ref` / pseudonymous contributor id —
  "attribution only, never an authorization input" (`docs/trace-commons.md:139`).
- **Server stores it opaquely only.** No dedicated column, no index, no query
  route, no migration. Whatever already persists the envelope keeps it; devfolio
  owns the reverse mapping.
- **Primary scope UX is "point at the project directory."** Deterministic,
  1:1 with a repo. The interactive picker remains an optional refinement.
- **Envelope representation is a blessed `feature_flags` key** (Approach A),
  not a typed protocol field — zero protocol/schema/server change.

Explicitly **out of scope**: devfolio-signed submission attestation; typed
protocol field; server-side submission column/index; judging or query route;
any server-side verification of the submission id.

## Command shape

```
trace-commons-contributor submit \
  --project ~/code/my-hackathon-repo \
  --submission <devfolio-submission-id>
```

- `--project <path>` bounds the candidate set to sessions whose real working
  directory is under `<path>`.
- `--submission <id>` stamps every envelope in the batch with the self-asserted
  devfolio submission id. May also be set in the contributor config file so a
  participant configures it once.
- The interactive numbered picker still runs (unless `--all`/`--yes`), so the
  participant retains final control over the exact selection.

## Changes

### 1. Scope control — sharpen `--project` to match the real session cwd

**Current:** `discover_filtered()` (`commands.rs:189-222`) applies the `--project`
filter by matching `SessionRef.project` (a cwd **basename**) or a path prefix of
the session file (`commands.rs:206-214`). The basename is derived from the
encoded-cwd directory name and is unreliable to decode
(`claude_code.rs:58-72`).

**Change:** Match `--project` against a **path-prefix of the decoded session
working directory**. The true cwd is already recovered during `load()`
(`claude_code.rs:199-204`); the design surfaces that decoded cwd on the discovered
`SessionRef` (or equivalent) so `discover_filtered()` can do a reliable
path-prefix comparison at discovery time rather than relying on the basename.

- `--project ~/code/my-hack` selects exactly the sessions whose working dir is
  under that path.
- Keep the existing basename/path behavior available as a fallback only if the
  decoded cwd is unavailable, so the change does not regress non-devfolio use.

### 2. Submission flag — thread `--submission` through the submit path

- Add `submission: Option<String>` to the `Submit` clap args
  (`bin/trace-commons-contributor.rs:39-52`).
- Optionally read a default from the contributor config file so it can be set
  once per participant. CLI flag overrides config.
- Thread it through `SubmitSelection` (`commands.rs:325-333`), `submit()`
  (`commands.rs:338-426`), into envelope construction so every envelope in the
  batch carries the same id.

### 3. Envelope representation — blessed `feature_flags` key

- In `build_raw_contribution` (`envelope.rs:254-327`), where
  `feature_flags["project"]` is set today (`envelope.rs:267-270`), also set
  `feature_flags["devfolio_submission_id"] = <id>` when a submission id is
  present. Omit the key entirely when no submission id is supplied (non-devfolio
  uploads are unchanged).
- No change to `ContributorMetadata` / `IronclawTraceMetadata` /
  `RawTraceContributionOptions`. No protocol schema change. No migration. No
  server route. The id persists inside the stored envelope exactly as `project`
  does today.

## Data flow

```
participant runs: submit --project <repo> --submission <id>
   -> discover_filtered() scans corpus roots, filters by decoded cwd prefix (<repo>)
   -> interactive picker (optional) narrows selection
   -> for each selected session:
        build_raw_contribution() sets feature_flags["devfolio_submission_id"] = <id>
   -> envelope uploaded and persisted as-is (server stores opaquely)
devfolio later: reads submission id from stored envelope metadata,
                cross-checks against its own submission records (out of band)
```

## Conventions honored

- **Attribution only / never authorization.** The submission id is
  self-asserted and never gates a read/write path; tenant + actor context still
  drive all scoping.
- **Hash-only audit.** The submission id is devfolio-issued, non-secret
  attribution metadata carried in the envelope (like `project`); no raw secret,
  URL, token, or contributor identity is introduced into audit rows or logs.
- **Fail-closed unchanged.** No new gate; when `--submission` is absent the key
  is simply omitted and behavior is identical to today.
- **No new dependencies.** Reuses the existing clap surface, config loader, and
  `feature_flags` map.

## Testing

- `--project <path>` selects only sessions whose decoded cwd is under `<path>`
  (positive + negative: a session in a sibling repo is excluded; a hyphenated
  repo name that previously mis-decoded is now matched correctly).
- `--project` still works when the decoded cwd is unavailable (fallback path).
- `--submission <id>` sets `feature_flags["devfolio_submission_id"]` on every
  envelope in a multi-session batch.
- Omitting `--submission` leaves the key absent (non-devfolio path unchanged).
- Config-file default is applied and overridden by the CLI flag.

## Future (not now)

If devfolio ever needs the server to key on the submission id (query "all traces
for submission X" server-side), promote the blessed `feature_flags` key to a
typed field on `ContributorMetadata` plus a tenant-scoped stored column and a
scoped, bearer-gated worker route with hash-only audit. Deferred until a
concrete need exists.

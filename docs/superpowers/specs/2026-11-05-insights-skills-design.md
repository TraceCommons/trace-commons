# Skills in Insights

Status: approved product scope in conversation; implementation authorized by “start building”.

## Intent

Help people understand the agent skills they already have and discover public
skills and updates from creators and software vendors. Skills belongs in the
native macOS Insights surface. It works without NEAR AI or Judgements.

## Approved v1

- Detect installed skills across agents, global and known project scopes.
- Proactively collect observed skill invocation evidence through the existing
  local Insights counter pass. Distinguish invocations from loads and unknown
  coverage; never infer effectiveness or absence from a missing event.
- Maintain a curated public catalog and followable publisher feeds, including
  verified sources for Poteto, Obra/Superpowers, Slack, and a Rex vendor entry.
- Detect changes to upstream skills, distinguish local modifications and stale
  checks, and direct users to publisher instructions. Do not apply updates.

Revisions, editing, installation, generated skills, model effectiveness
judgements, remote trace processing, and company-private publishing are outside
v1. No new third-party dependencies. No banners, toasts or OS notifications.

## Architecture and approaches

The chosen approach combines local deterministic inventory/usage with a curated
public metadata catalog. Reviewing every session with a model and corpus-driven
recommendations were considered; both introduce inference dependencies outside
the approved scope. A curated registry with automatic source refresh provides
attribution without an unrestricted crawler.

Rust owns inventory, source identity, public catalog cache, following, freshness,
usage evidence and shared copy. Native SwiftUI reads a typed local daemon API.
The existing counter pass owns trace reads, its bounds and cancellation, its
Insights-off behavior, Never-folder exclusions and session references.

## Identity and freshness

Match source repository and repository-relative SKILL.md path, never just the
display name. Hash exact entrypoint bytes. A matching latest hash is current;
an installed hash matching a known earlier upstream entrypoint is an available
update; a mismatch without historical evidence is locally modified or unknown,
not proof of an older version. Missing/stale/failed checks cannot establish
current status. Bundle scripts and effective CLI-provided instructions are not
covered by an entrypoint hash. Rex delegates its guide to the installed CLI;
its documentation entry must not imply verified skill freshness.

Inventory records source, agent, scope, symbolic location and bounded metadata.
Scan only standard global skill roots, configured roots and known permitted
projects; support ordinary skill symlinks with explicit bounds. Unreadable or
truncated scans carry partial coverage. Do not recursively search arbitrary
user folders or execute skill code.

## Catalog and feeds

Verified sources on 2026-11-05:

| Publisher | Repository | Root | Instructions |
| --- | --- | --- | --- |
| Poteto | cursor/plugins | pstack/skills | https://github.com/cursor/plugins/blob/main/pstack/README.md |
| Obra | obra/superpowers | skills | https://github.com/obra/superpowers#installation |
| Slack | slackapi/slack-skills-plugin | skills | https://github.com/slackapi/slack-skills-plugin#installation |
| Rex | documentation only | none | https://www.superlogical.com/rex/docs/automate/agent-skill |

Resolve immutable upstream revisions before fetching bounded data. Use only
fixed HTTPS public source endpoints, no ambient credentials or publisher code
execution. Preserve cached catalog data when offline and expose its freshness.
Background checks are throttled. Feed events record when an addition or change
was observed, rather than fabricating a publication date. Following is a local
preference; it does not send trace or installed-skill content to publishers.

## Presentation

Add Skills to the existing Insights tabs, with installed inventory, usage,
publisher following, catalog and feed sections. Show update state beside each
installed skill and publisher instructions beside catalog entries. Keep
loading, unavailable, partial and genuinely empty states distinct. Preserve
last-known information on refresh failure. Use shared Rust copy and existing
glass components, accessible action names, and no mutation controls for skills.

## Validation

Exercise filesystem bounds and symlinks, frontmatter, exact provenance matching,
freshness ambiguity, stale/offline catalog retention, following persistence,
explicit tool event extraction and deduplication, user-text false positives,
Never/off/clear behavior, backward-compatible Swift decoding, refresh/follow
races and stale response handling. Run client Rust suites, license boundary,
clippy and formatting; rebuild FFI before Swift tests. Inspect real local data
without printing trace contents or credentials. Report coverage and runtime
limitations separately from test results.

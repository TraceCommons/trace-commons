# Local refactor comparison context v1

Status: implementation contract; adapter and pilot qualification remain required.
Parent: [personal refactor comparison delivery plan](../plans/2026-09-12-personal-refactor-comparison.md).

This contract defines which user-reviewed task settings can be compared. It
does not establish model ownership, causal effects, or source qualification.
Draft creation and outcome capture remain available when context is incomplete.

## Identity and authority

`project_id` is a canonical lowercase UUID generated locally and explicitly
selected by the user. Reusing that ID groups tasks from different worktrees of
the same project. Neither a filesystem path hash, remote URL, branch name, nor
an imported trace label establishes project identity. Import does not silently
assign a project. A user selecting the wrong project remains a documented
limitation of user-confirmed context.

Task context has user-confirmed provenance. Harness metadata may suggest a
value only when a named source adapter qualifies its meaning. The user reviews
that suggestion before saving. A user-entered harness version is a context
label, not a verified producer version and not authority to admit source data.

## Comparable fields

Exact strata contain project ID, category, language, and the configuration
fingerprint below. The first qualified category is `refactor`. Task date is a
user-confirmed calendar date used for the specification's inclusive date
window; different task dates inside that window do not split a stratum.

The configuration allowlist is exactly:

| Field | Meaning |
|---|---|
| Harness ID | Stable adapter/product identifier, not a model or provider label |
| Harness version | Exact user-reviewed version label, without inferred alias expansion |
| Reasoning effort | `none`, `minimal`, `low`, `medium`, `high`, or `xhigh` |
| Tool-policy profile ID | Versioned policy identity describing the allowed tools and their authority |
| Tool-policy profile version | Exact version of that policy; the same name with different permissions needs a different version |
| Prompt-template digest | SHA-256 of the exact UTF-8 template bytes used for task instructions, before task-specific substitution |

Every field, and language, has an explicit unknown state. Unknown is distinct
from the known reasoning effort `none`. Unsupported reasoning settings stay
unknown until a later schema supports them. Missing comparable fields suppress
comparison; they do not prevent creating, viewing, annotating, or deleting a
task. Never fill a missing setting from another task in the cohort.

Identifiers and version labels are bounded structured strings, with fixed
validation errors. They are case-sensitive after validation; do not silently
lowercase, trim, alias, or parse versions. Language uses a bounded canonical
identifier, not a free-text description. No paths, trace bodies, arbitrary
environment objects, or test output belong in context. The implementation must
publish its accepted character sets and maximum lengths alongside the DTO.

Models are deliberately absent from the configuration fingerprint: they define
the cohorts being compared. Nested model settings must not enter it indirectly.
Approvals, sandbox rules, and tool permissions belong to the reviewed
tool-policy identity/version, not an arbitrary serialized environment blob.
An adapter must qualify that mapping before treating an observed policy as a
known comparable profile.

## Canonical encoding

The implementation must pin a versioned canonical encoding with golden bytes
and digest tests before persisting fingerprints. Encode fields in the table's
order with explicit unknown/known tags and unambiguous string lengths; use
UTF-8, SHA-256, and a domain separator specific to comparison configuration v1.
Do not hash generic map iteration, display copy, debug formatting, or raw source
JSON. Publish the exact separator, length encoding, and golden example with
the implementation. A schema change cannot silently reuse v1 fingerprints.

The material-evidence digest separately includes context and frozen bindings.
Its revision is separate from the task's concurrency revision. Changing a
comparable field, project, date, category, checkout provenance, or bound
evidence stales the previous outcome binding and independence confirmation.
Outcome-only edits and explicit reconfirmation leave material evidence intact.

## Checkout provenance

Existing repository-path digests identify local checkouts; they are never a
substitute for `project_id`. Preserve them only as qualified provenance.

A checkout/source-tree digest must carry a named construction scheme. V1 does
not invent a universal construction from branch names, a Git commit alone, or
an arbitrary supplied hash. Until an adapter supplies a qualified construction
that states its treatment of dirty, untracked, and ignored files, tree identity
is explicitly unavailable. This optional provenance does not suppress outcome
comparison. Existing Git evidence can still be inspected with its existing
limits. Different source trees do not split an otherwise exact stratum.

## Required behavior examples

| Change | Matching and review effect |
|---|---|
| Same project ID across different checkout paths | Same project; no path-based split |
| Different project ID with identical code | Different stratum |
| Same context with another declared model | Different cohort in the same stratum, subject to attribution qualification |
| Different language or known configuration field | Different stratum; prior task review becomes stale |
| Unknown required setting | Incomplete draft; comparison unavailable with a field-specific reason |
| Task date changes within selected window | Same stratum; prior task review becomes stale |
| Source tree changes | Provenance/material evidence changes; same stratum |
| Outcome changes from pending to accepted | New CAS revision and invalidated result; independence remains current |

Qualification must test these cases through storage and service operations,
including legacy drafts and canonical serialization. A complete context alone
never grants model-comparison eligibility: source attribution, exact current
bindings, independent task review, and overlap checks remain separate gates.

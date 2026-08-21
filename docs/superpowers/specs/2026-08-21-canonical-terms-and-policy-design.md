# Canonical terms, data policy and consent document

Status: design approved 2026-08-21. Drafting not started.

## Why

A partner asked for "a single source of truth for the terms and conditions,
data policy and user consents that we can use universally across all our
communications". Today there is no single anywhere:

| Subject | Where it lives now |
| --- | --- |
| Terms and conditions | Nowhere. No page, no file, no clause in any README. |
| Data policy | `src/pages/about/data-policy.astro` (108 lines) and `src/pages/about/privacy.astro` (62 lines) in `trace-commons-community`, whose "Source-of-truth links" section defers to four markdown files in this repo. |
| Consent scopes | Authoritative as the `ConsentScope` enum in `crates/trace-commons-protocol/src/trace_contribution.rs`, then restated by hand in the data-policy page, `crates/trace-commons-contributor/src/commands.rs`, `macos/Sources/TraceCommonsApp/AppModel.swift`, `windows/src/TraceCommons.App/ViewModels/OnboardingViewModel.cs`, `crates/trace-commons-contributor-gtk/src/copy.rs`, and both READMEs. |

Six hand-maintained restatements of a five-variant enum is the drift the
request is reacting to.

## What gets built

One page at `https://tracecommons.ai/legal/`, in the `trace-commons-community`
repo, in four parts:

- **A. Terms of Service.** Iqlusion Inc as offering entity, eligibility,
  enrolment and account, acceptable use, licence grant, credits, disclaimers
  and limitation of liability, termination, California governing law, how
  changes take effect.
- **B. Data Policy.** What a submission contains, the redaction pipeline,
  third-party processors, retention, security posture, what published
  aggregates do and do not protect.
- **C. Consent scopes.** All five, each stating what it permits *and* what it
  does not.
- **D. Withdrawal and revocation.** What withdrawal removes, on what deadline,
  and what survives it.

One page rather than four: "single source of truth" means one URL a partner can
paste, and consent scopes are not interpretable without the data policy around
them.

### Presentation: human-first, binding text beneath

Each section leads with a plain-language statement and carries the binding
clause in a disclosure beneath it. Both registers are authored in the same file
and rendered together, so they cannot drift apart the way two documents would.
The page states that the plain-language layer is a summary and the binding
clause governs where they differ — which makes an inaccurate summary a defect in
the document, to be fixed in the clause rather than papered over.

The plain-language layer is also the supply for the consent surfaces: the CLI
prompt, the three app onboarding screens, and the invite email quote from it
with a `/legal/<version>#scope-<name>` anchor, instead of each inventing its own
wording as they do today.

### Versioning is what makes it a source of truth

Every envelope already carries `ConsentMetadata.policy_version`, today
`"2026-04-24"` (`TRACE_CONTRIBUTION_POLICY_VERSION`,
`crates/trace-commons-protocol/src/trace_contribution.rs:29`). The document
declares the same identifier. Each revision is permalinked at
`/legal/<version>/`; `/legal/` redirects to current.

This makes "which terms did this contributor agree to?" answerable from the
envelope alone, which is the question that matters if a consent is ever
disputed. It costs nothing: the field exists and is already populated on every
submission.

Guard: a test in this repo pins the `ConsentScope` variants and
`TRACE_CONTRIBUTION_POLICY_VERSION` to a checked-in table, failing CI when a
scope is added, removed or renamed without a policy-version bump. Adding a scope
and publishing the terms that describe it become one action.

### The existing pages are absorbed

`about/data-policy.astro` and `about/privacy.astro` become sections of the
canonical page; the old URLs redirect. Leaving them live recreates the reported
problem. Their content is largely accurate and is carried over, not rewritten.

## Constraints on the drafting

**Nothing is published until counsel has reviewed it.** The deliverable is the
page plus a preview URL for a lawyer, with a review checklist naming every
clause where a judgment call was made rather than a fact transcribed.

**Every factual claim about system behaviour cites the code that enforces it.**
Counsel should be editing law, not guessing at mechanics. Claims that cannot be
traced to code or to operator configuration do not go in.

**Revocability states its limits.** The licence is scope-limited and revocable,
which is what the code enforces: `delete_withdrawn_trace_objects`
(`crates/trace-commons-server/src/bin/trace-commons-ingest.rs:14829`) deletes
the file-side submission record, the `trace_object_refs` rows and every
status-derived envelope path, and errors propagate so withdrawal cannot report
success while content survives. Community snapshots refuse to serve when older
than `COMMUNITY_SNAPSHOT_MAX_AGE` (900 seconds), which is what makes the
≤15-minute removal bound real rather than aspirational.

But a trained model and a published statistic cannot be un-made. The clause
grants revocation over the trace and its derivatives in storage, and says
plainly that published aggregates and previously-trained artifacts are not
retracted. A revocation clause that promises more than
`delete_withdrawn_trace_objects` delivers is worse than no clause.

**Credits confer no present entitlement.** Credit quality scoring, duplicate
penalties and per-contributor caps all run in shadow mode; settlement to
on-chain credits is an intention, not a shipped feature. The terms say so.

## Decisions taken, with their basis

| Decision | Value | Basis |
| --- | --- | --- |
| Offering entity | Iqlusion Inc | What ships: binaries are signed `CN=Iqlusion Inc, L=Santa Clara, S=California`, and the winget publisher is `Iqlusion Inc`. |
| Governing law | California | User's call, 2026-08-21. |
| Licence grant | Non-exclusive, scope-limited, revocable | User's call; matches the consent-scope machinery already enforced. |
| Repo | `trace-commons-community` | One build, no cross-repo sync step. The drift guard above covers what proximity to the code would have bought. |
| Minimum age | 18, marked counsel-confirmable | Nothing in the repo specifies one. 18 avoids COPPA and GDPR child-consent machinery entirely; lowering it is a legal decision, not an engineering one. |

## Facts to confirm before the prose is written

These are load-bearing for clauses in parts B and D and none of them should be
asserted from memory:

1. **Soft-delete retention.** `docs/operator/backup-restore.md` records GCS
   object versioning plus soft-delete with a configurable retention window.
   Withdrawn bytes may therefore persist in prior generations after
   `delete_withdrawn_trace_objects` returns. The withdrawal clause must state
   the real window, so the live bucket configuration has to be read.
2. **PII backstop state.** `TRACE_COMMONS_PII_BACKSTOP_ENABLED` gates the
   asynchronous re-redaction pass. Whether it is on for the pilot determines
   whether the data policy may describe it as part of the pipeline.
3. **NEAR AI as a processor.** The privacy filter and the scorer both send
   content to a third-party TEE-hosted service. If that is live, NEAR AI is a
   sub-processor and must be named in part B, with what is sent and what is not.
4. **Retention schedule.** A retention scheduler and a retention dry-run drill
   exist; the actual pruning schedule and its bounds need reading before any
   retention period is stated.
5. **Aggregate protection.** `COMMUNITY_MIN_CELL_COUNT_FLOOR` is 2 and the noise
   seed is the placeholder `v1:no_noise_yet`, refused on both recompute and
   serve. Part B must describe minimum-cell suppression without implying a
   calibrated noise mechanism that does not exist yet.

## Non-goals

- Generating app and CLI copy from the document automatically. The anchors and
  quoted sentences are a convention here, not a build step. If drift returns
  after the surfaces are aligned once, that is the moment to automate.
- A cookie or tracking policy. Nothing in the community site sets tracking
  cookies today; adding a policy for behaviour that does not exist invites the
  behaviour.
- Per-tenant or enterprise terms. This document covers individual contributors.
  Instance operators onboarding under an agreement are a separate slice.

## Sequence

1. Confirm the five facts above against live configuration and code.
2. Draft the page, both registers, with clause-level source anchors.
3. Self-review for clause/gloss disagreement.
4. Counsel review, via preview URL, with the judgment-call checklist.
5. Publish, redirect the two old pages, bump `policy_version` if the scope
   semantics moved, align the six restating surfaces to quote the anchors.

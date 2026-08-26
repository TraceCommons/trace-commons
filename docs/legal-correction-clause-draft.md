# Draft policy text: contributor corrections

Proposed addition to <https://tracecommons.ai/legal/> (source:
`src/components/LegalBody.astro` in `trace-commons-community`) for the
`correction_included` content flag.

**This is a draft by a non-lawyer and needs counsel review before the banner
comes off.** It follows the structure of
[`legal-counsel-review-checklist.md`](legal-counsel-review-checklist.md):
proposed clause text first, then the judgment calls a lawyer should confirm or
overrule, then the facts each clause asserts and what verifies them.

Adding this changes the consent surface, so
`TRACE_CONTRIBUTION_POLICY_VERSION` must be bumped in the same change and
`crates/trace-commons-protocol/tests/consent_policy_pin.rs` updated to match.
The code must not ship before the text is published.

---

## Proposed clause: B.x -- Corrections you write

> When you mark a session as failed or partly successful, you may add a short
> written correction saying what went wrong. Writing one is optional. Nothing
> in the service requires it, and declining costs you nothing.
>
> **A correction is stored exactly as you write it.** This is the one
> exception to the redaction described above. Everywhere else, file paths,
> email addresses and similar identifiers in your session are replaced with
> placeholders before upload. A correction is not: replacing the details in
> it would remove the explanation it exists to give.
>
> One protection does still apply. If a correction appears to contain a
> credential -- an API key, an access token, a private key -- the submission
> is refused and you are asked to remove it. It is not quietly masked and
> sent anyway.
>
> Because a correction is kept as written, treat it as public writing about
> your own work. Do not put anything in it you would not want in the corpus:
> not someone else's personal information, not confidential material
> belonging to an employer or client, and not anything you are not free to
> share.
>
> Every envelope records whether it carries a correction. A correction is
> covered by the same consent scopes as the rest of the trace it belongs to;
> it does not grant any additional use. You can withdraw a trace, and its
> correction goes with it.
>
> Corrections do not currently affect credits.

---

## Judgment calls a lawyer should confirm or overrule

| Clause | The call I made | Why it might be wrong |
| --- | --- | --- |
| B.x Storage as written | A correction is exempt from the redaction that covers everything else | This is the substantive change. Everywhere else the document promises identifiers are replaced before upload; here they are not. Confirm the carve-out is stated clearly enough that a contributor cannot reasonably believe the general promise still covers it. |
| B.x Credential refusal | Refuse the submission rather than mask the credential | Chosen because a masked credential has still been typed and transmitted, and the contributor should know to rotate it. Confirm refusing is preferable to accepting-and-masking from a liability standpoint. |
| B.x Third-party content warning | Prose warning only, no technical control | Nothing stops a contributor pasting another person's data into a correction. The clause asks them not to. Confirm a warning is sufficient, or whether the correction needs the same PII classifier the rest of the trace gets. |
| B.x Scope inheritance | A correction grants no additional use beyond the trace's existing scopes | Drafted narrowly on purpose. Confirm this is broad enough for the corpus's actual purpose, given a correction is the highest-value label in a trace. |
| B.x Credits | "Do not currently affect credits" | True today (shadow mode). Confirm the wording does not foreclose crediting them later, and does not read as a promise that they never will. |

---

## Facts each clause asserts, and what verifies them

| Statement | Verified against |
| --- | --- |
| A correction is optional and declining costs nothing | The control is skippable; an absent correction leaves the flag false and behaves as 0.5.0 |
| Offered only on a failed or partly-successful verdict | Collection is gated on the verdict, spec S5 "Collection" |
| Stored as written; semantic redaction passes skipped | Spec S5 "A correction is not scrubbed" |
| Credential detection still runs and refuses the submission | `blocked_secret_detected` on a High or Critical match |
| Every envelope records whether it carries one | `ConsentMetadata.correction_included` |
| Withdrawal removes it with the trace | `delete_withdrawn_trace_objects` removes the artifact; the correction is inside it |
| Corrections do not affect credits | Shadow mode: the value is computed and stored, and nothing is credited |

---

## OPEN QUESTION -- this clause cannot be published until it is settled

Writing this draft exposed a decision the design did not make.

A correction is rewritten by **two** passes today, not one
(`trace_contribution.rs:3671` and `:3673`):

1. `redact_text_with_state` -- the deterministic passes: secrets, paths,
   emails, the cue-gated entropy sweep.
2. `apply_privacy_filter_to_text` -- the NEAR AI prose-PII filter, which
   rewrites the text it is given.

The decision recorded so far was "skip the semantic passes, keep secret
detection". That addressed only the first. If the second still runs, a
correction naming a person is still rewritten, and **"stored exactly as you
write it" is not true** -- the clause above would be misleading as drafted.

Two coherent positions, and the clause text differs between them:

- **Neither pass rewrites.** The correction reaches the corpus as typed, with
  credential detection the only control. Maximum fidelity, and the clause
  above stands as written. It also means contributor prose is the least
  reviewed content in the corpus, which is a strange place to end up.
- **The prose filter still runs.** Personal information is removed, paths and
  identifiers survive. The clause must then say the correction may be
  modified to remove personal information, not that it is kept as written.

An implementation note that bears on the choice: secret detection is inside
`redact_text_with_state` along with paths and emails. "Keep secrets, drop the
rest" is therefore not a skip but a decomposition of that function, and
whichever position is taken should be implemented as an explicit correction
path rather than by threading flags through the general one.

## What is NOT claimed, deliberately

The clause as drafted does not say a correction is reviewed for personal
information before it reaches the corpus. Whether that is the right thing to
say depends on the open question above.

Separately, and true under either answer: the asynchronous PII backstop --
which re-examines a held trace before it reaches the corpus -- ships disabled,
and on this deployment a held trace releases to a quarantine queue that is not
currently worked. So no post-hoc review of corrections happens either way
today. If counsel wants an assurance of review in the text, the backstop has
to be enabled and that queue owned first.

The clause also does not promise a correction will ever earn credit, and does
not promise it will not.

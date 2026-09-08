# Contribution eligibility surface

Issue: #736. Status: design, not implemented.

## The problem

`handle_list_pending` returns the whole queue with no eligibility filter, and a
queue entry carries nothing that could support one:

```
entry_id  session_hash  session_path  source  declared_source
project_id  project_label  project_path
```

Eligibility is decided per submission, at submit time, in
`submit::admission_profile_for_request(enabled, request_body)` — from whether
that session's attested final call carries
`trace_commons_protocol::admission::REQUEST_METADATA_KEY`.

So a contributor admitted on evidence rather than an invite sees every session
on their computer, picks one, submits it, and only then learns whether it was
admissible. The list looks like a menu and is not one.

An invited contributor never sees this, because everything in their queue is
contributable. That is why it has stayed invisible.

## Why this is worth a slice

It is the same shape as #728: a surface offering an action the transport cannot
perform, discovered on the press. It is worse in one respect — pressing Cancel
did nothing, whereas pressing this sends a submission to the server and has it
refused.

It also lands on the population least able to interpret it. The comment on
`admission_profile_for_request` names them:

> The signup flag enables account-bound evidence, but **existing unbound
> history** still needs an ordinary signed review for the server-controlled
> window.

"Existing unbound history" is everything recorded before the contributor
started routing through attested inference. For someone who arrives without an
invite, that is most of what is on their disk, and precisely the part they
cannot use.

## What decides eligibility today

Three things, in order:

1. `settings.admission_evidence` — the signup flag. Off for invited
   contributors, who need none of this.
2. `include_inference_bodies` — the consent setting. Without it `submit` passes
   `None` for the attested call and the profile is `false`.
3. `routing::attested::attested_final_call(rows, bodies_dir)` succeeding, and
   its request metadata containing `REQUEST_METADATA_KEY`.

## The load-bearing constraint: two costs, not one

`attested_final_call` is not uniformly cheap, and the split falls exactly along
the line this surface needs.

**Answerable from ledger metadata alone, no disk read:**

| variant | means |
|---|---|
| `NoCall` | the session joined no inference hops |
| `CaptureOff` | `capture.bodies` was off, or the body was not held whole |
| `DigestAbsent` | a restarted, cancelled or truncated stream |
| `UpstreamIdAbsent` | no provider identifier, so no receipt is reachable |

**Requires reading bodies off disk and hashing them:**

`ReferenceMalformed`, `BodiesUnreadable`, `BodyNotUtf8`, `BodyTooLarge`,
`DigestMismatch`.

A list view cannot run the second group. Hashing every captured body for every
pending session, on every list, is not a cost this surface may impose — the
daemon already competes for a contributor's machine.

**So the list answers the cheap group and says the expensive group is checked at
submit.** That is not a hedge; it is the honest shape. The cheap group contains
the case that actually matters — `NoCall`, the session that predates attested
routing — and that is a permanent answer, not a provisional one.

## The states

Four, and the distinction that matters is not eligible/ineligible but
**permanent/provisional**.

- **`eligible`** — cheap checks pass and the metadata key is present. The
  expensive checks still run at submit; this is a well-founded expectation, not
  a guarantee, and the copy must not promise more.
- **`ineligible_permanent`** — `NoCall`, `DigestAbsent`, `UpstreamIdAbsent`,
  `DigestMismatch`. Nothing the contributor does will change this session.
  Retrying is wasted work and the surface should say so.
- **`ineligible_configuration`** — `CaptureOff`. This session stays ineligible,
  but a setting governs whether *future* sessions will be. The only state where
  telling the contributor what to change is useful.
- **`unknown`** — the daemon could not evaluate it. Distinct from ineligible,
  for the reason `credential_unreported` is distinct from `credential_absent`:
  degrading "could not tell" into "no" invites a contributor to conclude
  something false about their own work.

`unknown` must not be a silent default. If evaluation is skipped for cost, that
is `unknown` and says so.

## What the shells do

**Show every session. Offer only the eligible ones.**

Not hiding the ineligible: hiding a contributor's own work is its own
dishonesty, and makes the app look as though it had not noticed files the
contributor knows it can see. The rule the credential surface already follows —
show what is true, offer what works — applies unchanged.

An ineligible row is present, not offered, and carries its reason. A permanent
one says so plainly enough that nobody retries it. `ineligible_configuration`
is the only one that names a setting, because it is the only one where changing
a setting helps.

This is a **daemon decision rendered by shells**, exactly as the credential
state table is. The eligibility label, its sentence, and whether a contribute
control is offered all come from the shared crate. No shell branches on a
variant name.

## Contract

`list_pending` entries gain:

| field | meaning |
|---|---|
| `eligibility` | `eligible` \| `ineligible_permanent` \| `ineligible_configuration` \| `unknown` |
| `eligibility_reason` | a stable label naming the `Unattestable` variant, or absent |

Copy in `private_inference_copy.rs`'s swept region — one sentence per state,
plus one per reason label — with the pinned-count discipline and the four-place
copy rule (Rust, Swift `CodingKeys`, C# record and `Sentences`, IPC doc
inventory).

**`eligibility` is absent, not `unknown`, when `settings.admission_evidence` is
off.** An invited contributor has no eligibility question, and a field answering
one they do not have would invite three shells to render an answer to it.

## Decisions

Ruled 2026-09-08 so implementation is not blocked on them. Each is reversible;
the reasoning is recorded so a reversal is an argument rather than a rediscovery.

**1. A submit-time failure writes its reason back into the row.**

A row that goes on claiming `eligible` after a submission proved otherwise is
the exact defect this surface exists to remove, reproduced one layer up. The
cost — a list that can change while a contributor reads it — is real and is
also just what happened; a surface that hides a change to stay still is lying
to look calm.

**2. `unknown` is in the contract, and the first slice never emits it.**

The daemon evaluates the cheap group eagerly, so every row gets a real answer.
But the variant ships in the contract and every shell renders it from day one,
because lazy evaluation is the obvious later optimisation and a shell that has
never seen `unknown` will render it wrong on the day it first arrives. Each
shell carries a test that an unrecognised or unevaluated state borrows no other
state's sentence — the same guard the credential surface uses.

**3. Only `ineligible_configuration` names a setting.**

`CaptureOff` has a true, actionable answer about future sessions. `NoCall` has
none for the session the row is about, and advice about the *next* session is
guidance rather than status — it does not belong on a row describing this one.
Rows stay about their own session.

**4. The daemon reports eligibility on a row and enforces it only where a
shell cannot.**

Ruled after the Windows shell found the hole. A group-level submit --
`approve {project_id}`, and `approve {all}`, which is simply the largest group
there is -- means **all eligible**, never all. The per-row gate cannot reach
it: a per-project approve has no row to check, so three shells would each have
to enumerate and classify rows themselves, which is three implementations of
one filter and exactly what produced #728.

A single `entry_id` is **not** filtered. Naming one entry is an explicit act
about a session the contributor is looking at, the shell's per-row gate
already covers it, and the server decides admission either way. A daemon that
refused a named entry would be enforcing an expectation as though it were the
answer.

That is the boundary, and it is written here because it is the kind of line
somebody later tidies up by filtering everywhere: **the daemon reports
eligibility on a row and enforces it only where a shell cannot.**

The same rule applies to the group's own control. A header offering "Submit
all" on a group where nothing is eligible is a press with no visible
consequence -- the row-level rule ("shown, not offered") one level up -- so
the group control is offered only when something in the group can be sent.
`list_projects` carries the subtotal beside the total so a shell can answer
that before the press rather than after it.

## Not in scope

Changing what the server admits. This surface reports the existing rule
earlier; it does not alter it. If the client's cheap check and the server's
decision can disagree, the server is right and the client's optimism was a bug.

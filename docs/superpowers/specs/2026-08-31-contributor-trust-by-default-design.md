# Trust by default: acting on the contributor-app UX review

Status: design, not yet implemented.

Source: `poldsam/trace-commons-ux-review`, a single README written by a
contributor who used the app wanting to contribute regularly without
managing it. Its thesis: the app asks the user to make too many decisions,
and the preferred shape is "connect, consent once, contribute
automatically, earn credits", with exclusion available as the exception
rather than approval required as the rule.

This document sorts that review into three piles -- what is a real gap,
what is a considered decision the review is asking us to reverse, and what
rests on a premise that is not yet true -- and proposes an order to work
in.

## Pile 1: a real gap, and it is the whole complaint

`ProjectMode::AutoUpload` exists
(`crates/trace-commons-contributor/src/daemon/policy.rs:35`). It does
exactly what the review asks for: upload without asking. The daemon
accepts it, `set_project_mode` sets it, and the uploader honours it.

**On macOS it cannot be reached from the app at all.** Onboarding screen 5
withholds it by design, and Settings withholds it too --
`macos/Sources/TraceCommonsApp/Views/SettingsView.swift:649` says so
outright: "arming `auto_upload` outside a deliberate confirmation flow is
still not built." The only way a macOS contributor arms a project is to
open a terminal and run the CLI.

GTK and Windows are ahead here. Both offer the mode from settings:
`crates/trace-commons-contributor-gtk/src/ui/settings.rs:736` pushes
"Contribute automatically" into the picker, and
`windows/src/TraceCommons.Interop/UnresolvedBucketCopy.cs:64`
(`OfferableModes`) returns `ask, auto_upload, ignore` for an ordinary row.

So a substantial part of this review is not a disagreement about product
philosophy. It is a macOS shell that is missing a control its two sibling
shells already ship, on the platform the review was almost certainly
written against. **Build the macOS arming flow.** The existing comment
already specifies the constraint: it must consult `ProjectRow.canBeArmed`
rather than enumerating `ProjectMode`, because the daemon refuses
`auto_upload` for the unresolvable-cwd bucket
(`policy.rs:98` and `policy.rs:125`, two independent places) and a control
offering it there would be a choice that cannot be delivered.

This is the single highest-value item in the review and it changes no
policy, no default, and nothing about what leaves the machine unless a
contributor asks for it.

## Pile 2: a considered decision the review asks us to reverse

The review's headline ask is that automatic contribution be the *default*,
not an option. Today `ProjectPolicy::resolve` returns `NotifyOnly` for any
project not explicitly configured (`policy.rs:97`), and onboarding screen
5 deliberately does not offer arming. The reason is written down in
`macos/Sources/TraceCommonsApp/Views/OnboardingProjectsView.swift:15`:

> `Ignore` is offered here and `auto_upload` is deliberately not: excluding
> a client repo is a live thought at this exact moment and never returns,
> whereas arming automation before the contributor has seen a single
> preview asks for trust they have no basis to give yet.

That reasoning survives the review. Someone on their first run has not yet
seen what a redacted trace looks like, so consent to automatic
contribution at that moment is consent to something they have not been
shown. The review's author is not a first-run user -- they had already
formed a view of the privacy model -- which is exactly the population for
whom arming is the right answer and the population currently forced into a
terminal to get it.

The proposal, then, is not to flip the default at onboarding. It is:

- Onboarding still lands ask-first, and still offers `Ignore`.
- The Done screen, and the first digest, say plainly that automatic
  contribution exists and where to turn it on. Today nothing tells a
  contributor the mode is available.
- After the contributor has actually reviewed some traces, offer arming in
  context -- at the point they approve, not before it. "You've contributed
  from this project 5 times. Contribute from it automatically?" is a
  question backed by evidence they now have.

That gets the review to its stated goal ("I finish setup thinking: it is
running, and I don't need to think about it again") by a route that does
not require trusting the product before seeing it work.

## Pile 3: the premise that is not yet true

The review's argument rests on one sentence:

> content is scrubbed locally before it is sent

and concludes that this "should be a major part of the product promise."

That claim is stronger than what the code does.
`crates/trace-commons-protocol/src/redaction.rs` redacts by **key name**:
it walks structured fields and blanks the values of keys whose names
tokenize to things like `api_key`, `access_token`, `authorization`,
`session_token`. It does not classify prose. Content typed into the body
of a conversation -- a name, a customer, an internal hostname, a
credential pasted inline -- is not in scope for the local pass. The prose
classifier is a *server*-side control, applied after upload.

That is defensible under ask-first, where a person reads a preview before
anything leaves. It is the load-bearing control under auto-upload, where
nobody does. So the honest position is:

- Do not market "we remove private information before anything leaves your
  machine" as the reason automatic contribution is safe. It overstates the
  local pass.
- Any per-project arming flow (pile 1) is fine as-is, because the
  contributor arming it has seen previews from that project and is making
  an informed judgement about that project's content.
- A *global* automatic default (pile 2's stronger form) should wait on a
  local content pass, not just a key-name pass. Until then the product
  promise for auto-armed projects is "you decided this project is safe",
  not "we guarantee it is."

Related prior work: `docs/superpowers/specs/` already carries the
server-side privacy-filter design. What is missing is a local content
scan, and that is a separate slice with its own cost.

## Pile 4: uncontroversial UX work

These come straight out of the review and cost little:

1. **Onboarding weight.** macOS is six screens with four decisions
   (`OnboardingCoordinatorView.swift:67`); GTK mirrors it. The privacy-scan
   screen already self-skips when the operator has not configured the
   second scanner. The projects screen could self-skip when the daemon has
   discovered no projects yet, which is the common first-run case, and move
   to a settings-time prompt instead.
2. **A high-control mode.** The review concedes that review-everything
   should remain available -- but framed as a choice, not as the floor.
   Once arming exists on all three shells, a single "review everything"
   switch that pins every project to ask-first makes the current behaviour
   an explicit stance rather than an absence.
3. **Digest and credit copy.** The review's example -- "12 conversations
   contributed. Privacy filtering complete. Credits pending." -- is a
   summary the daemon already has the data for. Saying what accrued is
   cheaper than asking about each trace and does more for the value
   exchange.
4. **Pause is built and under-advertised.** `daemon/ipc.rs:249` and
   `daemon/mod.rs:434` implement pause, including a timed pause that
   auto-clears. The review asks for it as if it were missing.

## Proposed order

1. macOS per-project arming flow, to shell parity with GTK and Windows.
   Pure gap-closing.
2. Digest and credit summary copy (pile 4.3).
3. Tell contributors arming exists -- Done screen and first digest
   (pile 2).
4. Contextual arming offer after N approvals from a project (pile 2).
5. "Review everything" switch (pile 4.2).
6. Local content scan, as its own slice, before any conversation about a
   global automatic default (pile 3).

Items 1-5 do not change what leaves any contributor's machine without
that contributor asking. Item 6 is the prerequisite for the review's
strongest ask, and should not be skipped to get there faster.

## What this review does not cover

The reviewer did not exercise revocation (`withdraw.rs`), the mark/
attestation surface, or multi-source capture. Their reaction is to the
first-run and steady-state contribution loop only. Nothing here should be
read as a finding about the rest of the app.

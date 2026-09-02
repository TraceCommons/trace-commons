# IronWire in the contributor apps

The user-facing half of IronWire: how a contributor declares a routing proxy,
how they can tell it is working, and what they are told about what leaves their
machine.

PR #513 builds the client-library half -- a ledger client, an `ironwire`
settings key, and envelope stamping. Its plan excludes UI in as many words:
*"Any UI or consent copy. The declaration is settings-file only in this plan.
Naming `cost_usd` on the consent card is a follow-up."* This spec is that
follow-up, and it is larger than the deferral implies.

## What #513 leaves, verified against its patch set

- `IronWireDeclaration { Watch { port: u16 } | Off }`, settings key `ironwire`.
  `None` means **off**, not "never asked" -- deliberately unlike
  `SourceDeclaration`, because probing `127.0.0.1:8463` unasked would be probing
  a service the contributor never mentioned. That semantic is right and this
  spec preserves it.
- The port is a free-form `u16`: no default, no range check, no reachability
  probe.
- `ironwire_ledger_for()` returns `None` when `$IRONWIRE_HOME/control.token` is
  unreadable, so **a missing token is indistinguishable from off**.
- `IronWireLedger::refresh` returns `()`. Unreachable, 401 and malformed body
  all log at debug and keep the previous snapshot.
- The ledger is constructed **once** in `DaemonShared::load`, so a settings edit
  takes effect only after a daemon restart.
- No shell has any status surface. `has_rows()` is `pub` on the ledger but
  reaches no IPC method, no health label, and no FFI.

`routing/mod.rs` states the governing rule: *"Absence and failure are the same
state... Nothing here can fail a submission."*

## The problem this spec exists to solve

**That rule is correct for the submission path and unacceptable for a setting.**

A contributor can declare `ironwire` with the wrong port, receive no error, see
no indicator, and have every subsequent trace silently carry no routing data,
with nothing anywhere telling them. They would reasonably conclude the feature
works.

The resolution is not to weaken the submission rule. It is to recognise that
**declaring** and **submitting** are different moments with opposite
requirements:

- **Submission must never fail on routing.** A proxy that went away must not
  cost a contributor a trace. Absence and failure staying indistinguishable
  *there* is a deliberate protection and stays exactly as #513 wrote it.
- **Declaration must be falsifiable.** When a contributor types a port and
  presses save, they are asking a question, and the app must answer it. Silence
  is a wrong answer.

Everything below follows from that split.

## What actually leaves the machine

The disclosure copy has to describe the **leaving** set, not the parsed set.
Overstating it is a lie; understating it is worse. Written by
`raw_routing_event_for` in `crates/trace-commons-contributor/src/envelope.rs`,
of `RoutedExchange`'s sixteen parsed fields:

**Nine leave verbatim in `structured_payload`:** `backend`, `facade`, `rung`,
`attempts`, `requested_model`, `served_model`, `cache_read_tokens`,
`cache_write_tokens`, `status`.

**Four leave as typed fields:** `timestamp` (from `started_at`), `latency_ms`
(from `total_ms`), `token_counts` (`input_tokens` + `output_tokens`, emitted
only if **both** are present -- a fabricated zero would understate consumption),
and `cost_usd`.

**`content` is always `None`.** No prompt or response text is in a routing event.

**Two never leave:** `id` (a paging cursor) and `client_session_id` (a local
join key).

Nothing is bucketed or coarsened; values leave at full precision. The only lossy
paths are a saturating `u32::MAX` clamp on token counts and the both-or-nothing
token rule.

**`cost_usd` is priced, not billed.** `routing/mod.rs` states it: work served on
a subscription is priced at what it *would* have cost on the meter, and **no
surface may render it as money the contributor spent.** Any copy naming a dollar
figure has to carry that distinction, which rules out the obvious phrasings --
"what you spent", "your cost", a running total that reads like a bill.

**So the honest one-line description is: which model was asked, which answered,
how long it took, how many tokens, and a priced estimate of what that work would
cost on the meter. No prompts, no responses.**

The same file adds a constraint worth repeating on any surface that shows these
numbers: they are **attribution only**, and must never reach a gate, a scoring
input, or a credit computation, because they come from a proxy the contributor
can patch. A UI that presents routing data beside credit figures invites exactly
that conflation.

## Design

### 1. The declaration surface

Follow `SourceDeclaration`'s established shape rather than inventing one. It is
a tri-state in each shell today (`SourceChoice` on macOS, `SourceDecisionKind`
on Windows, built inline in GTK's `ui/roots.rs`), and routing is a two-state
plus a port.

The control is: off by default, a toggle, and a port field enabled only when on.
Default the field to IronWire's conventional port so a contributor is not asked
to know it, but **write nothing until they act** -- the displayed default must
not become a declaration, because `None` means off and that distinction is the
one thing #513 got exactly right.

### 2. Declaration-time validation, which is the new thing

On save, the daemon probes the declared port and returns a result the shell
renders. Three outcomes, each with distinct copy:

- **Reachable, token readable** -- confirmed, and say what was found.
- **Reachable, token missing or unreadable** -- name the file
  (`$IRONWIRE_HOME/control.token`), because this is the failure a contributor
  can actually fix, and today it is silently identical to off.
- **Not reachable** -- name the port that was tried.

This requires a new IPC method. It does **not** change the submission path: the
probe runs when a human asks, never on the trace path.

### 3. The `IRONWIRE_HOME` gap

The token is read from `$IRONWIRE_HOME/control.token` -- stated in
`routing/ironwire.rs`'s module doc, and read at call time, never copied into our
settings or logged. That is the right handling of a credential. It also creates
a failure this design has to name.

**A GUI-launched application does not inherit the user's shell environment.** On
macOS an app started from Finder or Dock gets `launchd`'s environment, not the
one in a shell profile; the same is broadly true of desktop-launched apps on
Linux. So `$IRONWIRE_HOME` will be unset for the daemon our app starts, whatever
the contributor has configured in their shell. If their IronWire lives anywhere
but the default location, the token read fails -- and today that is
indistinguishable from "off".

This is the same silent-failure class as the port, arriving by a different
route, and it is worse because the contributor has no reason to suspect it: they
set the variable, their shell honours it, and the app quietly does not see it.

Three things follow:

- The declaration probe (above) must report **which file it looked for**, by
  absolute path, not merely that a token was unreadable. That single string
  turns an invisible failure into a fixable one.
- A path field belongs beside the port, defaulted to the conventional location
  and only needed by contributors whose install is elsewhere.
- **Confirm the fallback behaviour before implementing.** I could not verify
  from the patch set what happens when `$IRONWIRE_HOME` is unset -- whether it
  falls back to a conventional directory or simply fails. The implementer must
  read `ironwire_ledger_for()` and design to what it does, not to this
  paragraph.

### 4. A status surface

`has_rows()` already exists and is `pub`; it reaches nothing. Expose routing
state through the existing daemon health/status IPC -- declared or not, last
successful refresh, and rows seen -- and render one line in each shell's
settings screen.

This is what converts "absence and failure are the same state" from a trap into
a defensible design: the submission path still cannot fail, and the contributor
can still see that nothing is arriving.

### 5. Restart semantics, stated rather than hidden

The ledger is built once at daemon construction, so a declaration change takes
effect on restart. **This is not a macOS quirk; it is true everywhere**, and it
happens to match macOS, which has no runtime settings write at all.

Two honest options, and this spec picks the first:

- **Say so.** The declaration UI states that routing changes apply after a
  restart, and offers the restart. Cheap, truthful, consistent across shells.
- Make the ledger rebuildable on settings change. More code in the daemon, and
  it still cannot help macOS.

### 6. Consent, which is the largest piece

Two distinct surfaces, and #513's deferral names only the smaller one.

**The preview sheet is data-driven.** `PreviewSummary`
(`daemon/preview.rs`, mirrored field-for-field in `PreviewSummary.cs`,
`PreviewSheet.swift`, `gtk/src/ui/preview.rs`) carries `would_send_bytes`,
`raw_session_bytes`, `event_count`, `opening_prompt`, `redactions`,
`pii_labels_present`, `consent_scopes`, `residual_risk`, `enrolled`.

Routing events currently appear there **only** as growth in `event_count` and
`would_send_bytes`. `redactions` and `pii_labels_present` never fire on them
because `content` is `None`. **`cost_usd` appears nowhere on any shell** -- not
as a field, not as a derived line. It reaches the envelope and no user-facing
surface names it.

So "name `cost_usd` on the consent card" is not a copy change. It needs a new
`PreviewSummary` field before any shell can render anything, plus the mirrored
type in all three.

There is precedent for exactly this: `PreviewSummary` already carries
`subagent_count`, a per-category count added when subagent events became a
thing a contributor should see named. Routing is the same shape of problem and
takes the same shape of answer -- a `routing` block carrying exchange count,
distinct models, and total `cost_usd`, so the preview answers "what routing data
is in this trace" the way it already answers it for redactions and subagents.

**The consent flag is legally pinned, and this is the constraint to respect.**
`ConsentMetadata` gains `routing_metadata_included`, and
`crates/trace-commons-protocol/tests/consent_policy_pin.rs` requires an entry in
`PINNED_CONTENT_FLAGS` whose text **is the published wording** at
`https://tracecommons.ai/legal/`, whose page and `src/policy.ts` live in the
**trace-commons-community repo**, not this one.

`TRACE_CONTRIBUTION_POLICY_VERSION` is pinned at `2026-04-24`, and #513 added
the flag and its pin entry **without bumping it** -- the test enforces bumps for
scope changes, not flag additions. Whether shipping a new content category to
contributors under an unchanged policy version is acceptable is a question for
whoever owns the legal page. **It is not a decision for this spec or its
implementer**, and it is a cross-repo dependency that must land before any shell
shows consent copy naming routing.

### 7. Onboarding

Onboarding is written three times with no shared scaffolding -- macOS
`Onboarding*View.swift` under `OnboardingCoordinatorView`, Windows
`OnboardingWindow.xaml` + `OnboardingViewModel`, GTK `ui/onboarding.rs`. The
only shared artefact is the settings JSON shape.

**Routing does not get its own onboarding step -- it gets detected.**

The data to collect is unusually small: a toggle, a port, and (per the
`IRONWIRE_HOME` gap above) possibly a path. No account, no endpoint, no
credential -- the token is read from disk, never typed. So the usual argument
against a new step, that it costs three implementations to gather a lot from a
few people, only half applies: it would cost three implementations to gather
very little.

What still holds is that a new contributor almost certainly does not have
IronWire, and onboarding should not ask about a proxy before it has asked about
traces.

The resolution is to **detect rather than ask**: if the daemon finds a readable
`control.token` at startup, the settings screen surfaces routing as an offer;
if it does not, routing stays a setting a contributor can find. Nobody is
interrogated about software they do not run, and nobody who runs it has to know
the feature exists to find it.

That reuses machinery this spec already needs -- the probe from section 2 and
the status block from section 4 -- so it costs little beyond the offer copy.
Revisit a dedicated step only if IronWire ships bundled, where the assumption
inverts.

## Per-shell notes

- **macOS is the constrained one.** It never calls `set_settings` at runtime --
  its only IPC methods are `set_project_mode`, `set_consent_scopes`,
  `set_public_profile`. Declarations enter at daemon start through
  `tc_daemon_start_with_settings`. So the declaration UI writes settings and
  restarts the daemon, which the restart-semantics decision above already makes
  the universal behaviour. Copy is scattered across `Views/*.swift`.
- **Windows** writes settings live: `SettingsProtocol.cs` serialises exactly one
  `set_settings` key per edit, from `ContributorSettingsViewModel`. Copy is
  spread across `Interop/*Copy.cs` and XAML literals.
- **Linux GTK** sends `set_settings` from `ui/settings.rs`, which rejects
  unknown keys. It is the only shell with a centralised copy module
  (`src/copy.rs`), so it is the cheapest place to draft wording first.

The FFI header already exposes a generic `set_settings`, so the declaration
needs no ABI change. The probe and status additions do.

## Open items

- **Does the policy version need bumping for a new content flag?** Cross-repo,
  and it gates the consent copy. Owner: whoever owns the legal page.
- **What is IronWire's conventional port?** #513 hardcodes none. The default
  shown in the field should be right, and this spec does not assert a value.
- **Should a declared-but-silent proxy nag?** A contributor who declares routing
  and then sees nothing arrive for a week is in a failure state the status line
  reports only if they look. A notification is the obvious answer and also the
  obvious way to become annoying. Not decided here.
- **Does `$IRONWIRE_HOME` have a fallback?** Unverified from #513's patch set.
  It decides whether the environment gap is a nuisance for unusual installs or
  a hard failure for every GUI-launched daemon. Read `ironwire_ledger_for()`
  before designing the copy.
- **What does the probe report to a contributor who has no IronWire at all?**
  The offer only appears on detection, so this should be unreachable -- but a
  contributor who types a port by hand can reach it, and "not reachable" must
  not read as an error they caused.
- **`has_rows` is a poor status signal.** It says data exists, not that the
  proxy is healthy now. A last-successful-refresh timestamp is the better
  primitive and may need adding to the ledger.

## Sequencing

1. Land #513. Nothing here can start until the settings key exists.
2. Resolve the policy-version question. It gates step 5 and has a lead time
   nothing else here has.
3. Daemon: the probe IPC method and a routing block on the status IPC.
4. `PreviewSummary` gains its routing block, plus the three mirrored types.
5. GTK first -- centralised copy makes it the cheapest place to settle wording.
6. Windows, then macOS. macOS last because the restart path is its own work.

# Contributor shell — shared design

Date: 2026-08-08
Status: approved for planning
Scope: sub-project 2. Platform-agnostic UX, copy, and behaviour for all three shells.
Companions: `-macos-`, `-linux-`, `-windows-` platform specs.
Depends on: daemon core v1.1, trace withdrawal.

## What this document is

Everything the three shells share: the flows, the screens, the exact words.
Platform specs cover only mechanics that genuinely differ. Divergence beyond
what those documents name is cost with no user benefit.

## The premise the interface has to survive

This app asks a developer to upload transcripts of their real work — possibly
their employer's or a client's — automatically, in the background, for credit
that is explicitly not money. No amount of reassuring tone makes that
acceptable. The interface has to be demonstrably more careful than the user
is, and it has to be careful in ways they can *see*.

Four properties carry that weight. Every screen below serves at least one.

1. **The first thing the app ever does is nothing.** Onboarding ends with
   "Nothing has been sent." Inaction is established as the default before
   anything else happens.
2. **The user watches the system refuse.** When a session grows between
   approval and upload, the daemon declines to send it. That event is
   surfaced loudly rather than being a silent state change; one of them buys
   more trust than any privacy copy.
3. **"Not sent" is always distinguishable from "sent."** `pending`, `refused`,
   `expired`, and `superseded` all mean nothing left the machine, and the
   queue view says so in words.
4. **The app can always explain itself.** Why a session was not offered, why
   something is waiting, what a status means.

### Uninstall triggers, to design against explicitly

- An upload action reachable from a notification. **There is none, ever.**
  Notification actions are `Review` and `Not now`.
- Discovering something was uploaded they did not decide on — the auto-upload
  path, which is why unattended uploads owe a receipt the user cannot miss.
- A trace stuck in quarantine for months with no explanation and no way to
  take it back.
- The app being unable to say why a session they know exists was not offered.
- Being asked for credentials for something called a "privacy filter" they
  never asked for.

## Onboarding

Six screens, one decision each.

### 1. What this is

> **Trace Commons**
>
> Coding agents get better when there are real transcripts to learn from.
> Almost all of that data is locked inside companies. Trace Commons is a
> shared pool that isn't.
>
> This app watches for finished Claude Code and Codex sessions on this machine
> and shows them to you.
>
> Before anything leaves this machine it is scrubbed locally for secrets,
> keys, and tokens. That scrubbing is good and it is not perfect — which is
> why you get to look first.
>
> **You decide what gets contributed. Nothing is sent unless you say so.**
>
> [ Get started ]   [ What gets removed? ]

"Good and not perfect" is load-bearing. A developer knows automatic redaction
is imperfect; conceding it first is what makes the rest credible.

Order revised 2026-08-19, after the GTK client became the first shell anyone
had seen rendered. The promise was inline in paragraph 2 and the screen ended
on scrubbing; photographed, it read as the fourth sentence of a stack nobody
finishes. It is now a separate terminal block, given the same standing
treatment the roots screen gives its caveat, and the page reads as an argument
in sequence: here is what this machine does mechanically, here is the limit of
it, therefore you are the one who decides. All three shells end this screen on
the promise -- do not restore the inline form in one client alone.

### 2. Connect

Invite paste field plus a `tracecommons://enroll?...` deep-link handler, so an
invite clicked in mail lands here. Resolve and show the instance before
committing. A dead invite says *"This invite link is no longer valid. Ask
whoever sent it for a new one."* — never the underlying HTTP condition.

### 3. Consent scopes

The most consequential screen in the product. Two visually distinct groups,
because they are two different kinds of decision. Scope list and descriptions
come from `consent_options`, never hardcoded per shell.

> **How may your traces be used?**
> You can change this later. It applies to traces you send from now on.
>
> **Always included**
>
> ☑︎ **Finding bugs and measuring agents**  `always on`
> Researchers read traces to find where coding agents fail, and score agents
> against each other. This is the baseline — it's what the commons is for.
>
> **Optional — each one lets your traces do more**
>
> ☐ **Turn my traces into test cases**
> Parts of your sessions may become benchmark problems. Benchmarks are usually
> published, so redacted excerpts of your work could appear in one.
>
> ☐ **Train models that judge agent output**
> Used to train models that rank or grade what an agent produced. Not models
> that write code.
>
> ☐ **Train coding models directly**
> Your traces become training data for models that write code — potentially
> including models built by other organizations, and commercial ones. This is
> the broadest permission here. If your sessions touch client or employer
> work, this is the box to think hardest about.
>
> **Credit**
>
> ☐ **List my handle publicly as a contributor**
> Affects your name only. Does not change how any trace is used.
>
> To pull a trace back later, use History → Withdraw.
>
> [ Continue with 1 permission ]   ← count updates live

Three rules:

- **The first four words of each bold label carry the distinction**, because
  that is all most people read. "Train models that **judge**" versus "Train
  coding models **directly**". Neither label may begin with "Training".
- **Nothing optional is pre-checked.** A pre-ticked `model_training` is a
  screenshot on Hacker News. Accept the lower opt-in rate; voluntariness is
  the premise.
- **`public_attribution` is visually separated**, because it grants no data
  use at all (`consent.rs:59` maps it to an empty set). Listing it beside four
  real scopes misleads in both directions.

### 4. Extra privacy scan (only if the operator offers it)

Never headline the words "PII filter" or "NEAR AI".

> **Extra scrub before sending? (optional)**
>
> Local scrubbing removes secrets, keys, tokens and credentials by pattern
> before anything leaves this machine. It runs either way.
>
> You can additionally send the *message text* of each trace — not tool
> output, not file contents — through a second scanner run by **NEAR AI**, a
> third party, to catch personal information the patterns miss: names,
> addresses, that kind of thing.
>
> This means your message text is transmitted to NEAR AI before it reaches
> Trace Commons. If that scanner is unreachable, **nothing is sent at all** —
> traces wait rather than going out unscanned.
>
> ( ) Local scrubbing only
> ( ) Local scrubbing + NEAR AI scan
>
> [ Continue ]

Choosing the scan calls `acknowledge_near_ai_notice`. Without that call the
daemon refuses the filter forever and the user experiences unexplained
paralysis.

### 5. What to watch

Lists discovered projects, all set to ask-first. **`Ignore` is offered here
and `auto_upload` is not**: excluding the client repo is a live thought at
this moment and never returns, whereas arming automation before seeing a
single preview is asking for trust not yet earned. Sessions with no resolvable
project get a permanent plain-English note that they can never be armed.

The screen's words, added 2026-08-19. This section previously specified only
behaviour and gave the screen no copy, so it shipped as a bare title over an
unlabelled list -- on the one screen deciding which of a contributor's
repositories are eligible to leave the machine. All three shells transcribe
these; do not reword them in one client alone.

> **What to watch**
>
> Every project starts at ask-first: you see each session before anything is
> sent. Ignore a project to leave it out entirely.
>
> `PROJECTS` (section eyebrow)
>
> Per row: the project name, with its mode beneath as **Ask me first** or
> **Ignored**. `Ask me first` is the vocabulary Settings already uses for this
> mode -- two screens setting one field must not name it two ways, and
> `Ignored` echoes the button that produced it rather than introducing a third
> name.
>
> Empty: **No projects yet. Sessions you run later will appear here, and in
> Settings.**

The subtitle states the default before the exception on purpose: the default
is what happens to a contributor who reads nothing and clicks Continue.

### 6. Done

> **You're set up. Nothing has been sent.**
>
> Trace Commons lives in your menu bar. When a session finishes and goes quiet
> for 30 minutes, it'll show up there. You'll get at most one notification
> every 4 hours, and none at all if there's nothing waiting.

## Credit, framed honestly

Shown on first run and again in History:

> **About credit.** Contributions earn credit points, scored on how novel and
> information-rich a trace is. Today credit is a **record**, not a currency:
> there is no payout, no token, no exchange rate, and no date. The intent is
> that credit eventually settles to something real, and if it does it will
> settle from this record. Contribute because you want the commons to exist.

Hard rules: **no currency symbol, no fiat estimate, no projection, no date.**
Pending and final are shown separately — "still being scored" is a true and
non-anxious explanation for a number that moved. `last_refreshed_at: null`
renders as "Not synced yet", never a confident `0.0`. **No gamification** — no
streaks, leaderboards, or levels. The audience is developers giving away work
product; a progress ring insults them and makes the credit framing look like
manipulation.

## Steady state

### Tray / menu bar

Icon precedence: attention (numeric badge) → unhealthy (amber dot) → paused
(struck through) → idle.

**The badge counts decisions owed, not sessions found.** If it shows 3, there
are exactly three things to say yes or no to. Never credit, never queue total.

The menu lists what is waiting, per project, with sizes. Those lines are
**not** approve buttons — see below. It shows unattended uploads when any
armed project exists, a week summary, pause, open window, settings, quit.

Quit is explicit about consequences, because users must not have to guess
whether closing the window stops contributing. **The correct wording depends
on the platform's process model, and getting it wrong is a lie about whether
the machine is still watching:**

- Where the application HOSTS the daemon in-process (macOS, Windows), quitting
  the app stops the watcher, because the app *is* it. Say that:

  > Quitting stops Trace Commons watching for finished sessions. Nothing is
  > queued or sent until you open it again. Anything already waiting stays
  > waiting.
  > [ Cancel ]  [ Quit ]

- Where a separate daemon runs under a service manager (Linux with the systemd
  unit), quitting the window leaves it running. Say that instead:

  > The background watcher keeps running and will keep queuing sessions.
  > Nothing will be sent while nobody's approving.
  > [ Quit ]  [ Quit and stop watching ]

An earlier draft of this spec gave only the second wording, which is false on
the platform the first application was built for.

Pause offers `For 1 hour` / `Until tomorrow morning` / `Until I turn it back
on`, backed by `pause {until}` so a timed pause survives the app quitting.

### When the app may interrupt

Exactly five things. Everything else is an ambient indicator.

1. The 4-hour digest, only with pending work, suppressed under Do Not Disturb
   and coalesced into the next window.
2. A superseded approval — the user decided and the system declined to honour
   it, so they must know.
3. Entries expiring, once, at three days remaining.
4. `queue-full`, the one health state with a consequence and no self-recovery.
5. The first automatic upload from a newly armed project.

### The digest

> **Trace Commons**
> 3 sessions ready from trace-commons-server and dotfiles.
> Nothing is sent until you review them.
> [ Review ]   [ Not now ]

`Not now` does nothing but dismiss. Its presence is what makes the
notification feel non-coercive.

## The review moment

**Design premise: never ask the user to judge redaction quality.** They
cannot, and an interface that shows redacted text beside an Approve button is
asking for a rubber stamp. Ask the two questions they *can* answer: is this
project OK to share at all, and is there anything specific in here that must
not leave?

### Queue row

Project label (disambiguated), agent, when, duration, turn count, and the
**redacted opening prompt** — which is what identifies a session to its
author; a timestamp is not. Then:

> Would send 84 KB  ·  scrubbed: 12 secrets, 4 tokens, 31 paths
> Scrubbing is pattern-based. It misses things it hasn't seen before.
>
> [ Look inside ]                        [ Not this one ]

`Would send` is the **redacted** size from v1.1 preview. The redaction receipt
proves scrubbing ran and calibrates: `scrubbed: 0` on a session that obviously
touched a `.env` is a signal the user can act on. The residual-risk line is
always shown and never hidden.

The row has **no** `Contribute` button. An earlier draft of this spec put one
there, which contradicts the preview-then-approve rule stated below: approving
from the row is approving without looking, which is the misclick the rule
exists to prevent. `Contribute` lives in the preview sheet and nowhere else.

"Not this one" rather than "Dismiss", because dismiss and ignore are different
decisions and the words must be too. Its tooltip: *"Skips this session only.
This project will keep being offered."*

### Look inside

Four tabs, in this order:

1. **Search** — default, cursor pre-focused. *"Search this trace for anything
   you need to be sure isn't in it."* Type a client name, get `0 matches` or
   jump-to-context. **This is the highest-value affordance in the product**:
   it gives someone under NDA certainty in five seconds without reading 148
   turns. Recent searches persist so the second trace is one keystroke.
2. **What's in it** — files touched (redacted), tools invoked with counts,
   model, turn count.
3. **Exactly what would be sent** — the redacted transcript, with redactions
   rendered as visible inline chips (`[SECRET]`, `[PATH]`) rather than
   deletions, so the user can see *where* scrubbing fired.
4. **Permissions** — the consent scopes this upload will carry, restated at
   the moment of consent rather than only at onboarding.

Body and search come from the in-process FFI preview, so there is no paging
and no size cap.

### Approving

**Preview-then-approve only.** No upload action exists in a notification, and
the tray's only forward action is Review. Blind approval of a real transcript
is the unrecoverable misclick.

After approving, a five-second undo:

> Sending… **[ Undo ]** (4)

backed by `cancel`. Trivially cheap; converts a misclick from permanent to
non-event. `Contribute` advances to the next entry in the sheet, so three
sessions is three deliberate clicks in one flow. There is no select-all.

## History

Three groups, never one column of mixed semantics:

> ✓  In the commons              9
> ◷  Being reviewed for privacy  2      What's this? →
> ·  Waiting to be scored        1

Quarantine, expanded:

> **Held for privacy review — 2 traces**
>
> A person at Trace Commons reads these before they enter the commons. It
> happens when automated checks see something that might be personal or
> sensitive and can't decide on its own.
>
> **These have not been rejected, and they have not been shared with anyone
> but the reviewer.** They are sitting still.
>
> Typical wait: we don't have a reliable number yet.
>
> [ Withdraw these traces ]

Obligations, in order:

1. **Never state a turnaround time that cannot be honoured.** "Usually 48
   hours" that becomes two months is worse than admitting there is no number.
2. **Withdraw is first-class and always available.**
3. **Show the real backlog once it is large**: *"2 of your traces are among 48
   waiting for review."* A contributor told they are in a queue is annoyed; one
   told nothing feels singled out. Annoyed is survivable.
4. **Render `explanations` verbatim.** "Held because a passage looked like a
   personal address" is enormously better than a status word.

## Failure states

Health states are ambient, not interruptive, because none of them can lose
data — the daemon suspends the expiry clock for every blocking label. The UI
should say so.

| State | Surface | Copy |
|---|---|---|
| `not-logged-in` | Amber dot, banner | **Not connected.** Sessions are being queued, but nothing can be sent until you reconnect. Nothing has been lost. `[ Reconnect ]` |
| `pii-filter-unavailable` | Amber dot | **The extra privacy scan isn't reachable.** Your traces are waiting rather than going out unscanned. Retrying automatically. |
| `privacy-filter-canary-failed` | Amber dot | **The privacy scan failed its own self-test**, so nothing is being sent through it. This is deliberate — a scan we can't verify doesn't get used. |
| `near-ai-notice-not-acknowledged` | Amber dot + banner with action | **One thing to confirm.** You chose the extra privacy scan, which sends message text to NEAR AI. Confirm you're OK with that and contributions resume. `[ Review and confirm ]` |
| `claim-mint-failed`, `ingest-unreachable` | Grey dot, merged | **Can't reach Trace Commons right now.** Your queue is safe; it'll retry on its own. |
| `daily-cap-reached` | Menu line only | **Daily limit reached.** The rest goes out tomorrow. |
| `queue-full` | **Notification** | **Trace Commons has stopped queuing new sessions** — 500 are already waiting. Review or clear some to start again. |
| Expiring entries | **Notification**, once at 3 days | **4 sessions you haven't decided on will be dropped in 3 days.** Dropped means never sent. `[ Review ]  [ Let them go ]` |
| Loop not running | Window replaces itself | **The background watcher isn't running.** Nothing is being watched or sent right now. `[ Start it ]` |

Two copy rules throughout: **never name the mechanism** — "privacy filter",
"claim", "ingest", "canary" are internal words — and **always state the data
consequence**: "nothing was sent unscanned", "your queue is safe", "nothing
has been lost".

Connection state is a first-class UI state, not an error. Not-running is the
common case on first launch and needs a "start it" affordance, never a spinner
that never resolves.

## Arming a project

Allowed from the app. A plain toggle, but never silent:

> **Contribute from trace-commons-server automatically?**
>
> Every future session in this project will be scrubbed and contributed
> **without asking you**. You won't review them first.
>
> You can turn this off at any time.
>
> [ Not now ]   [ Turn on automatic contributing ]

Paired with a persistent "Armed: 2 projects" row that never collapses, a
notification the first time a project auto-uploads, and a weekly summary of
what went out unattended.

## Why sessions were not offered

A disclosure row — "Sessions not offered (14) ›" — expanding to plain-language
reasons from `eligibility_reasons`: still being written, already contributed,
grew but not enough to resend. Prevents a whole class of suspicion at almost
no cost, since the reasons are already computed.

## Acceptance checklist

A shell is done when: onboarding completes without a terminal; the queue shows
redacted sizes and redaction counts; search finds a planted string in a real
session; approving shows an undo that works; no notification can upload
anything; quarantine reads as held-not-rejected and offers withdraw; every
health state renders its sentence above; arming shows the confirmation and the
first auto-upload notifies; quitting explains what continues.

## Out of scope

- Any change to scoring or credit calculation.
- Operator-facing review tooling.
- Multi-account switching.

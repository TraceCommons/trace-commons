# Trace withdrawal in the macOS app

Branch `macos-withdrawal-ui`, off `main` at `33f25a34`.

The daemon methods and the CLI landed in #252. Neither shell exposed them, so
a contributor could withdraw from a terminal but not from the app that
persuaded them to contribute. This wires the app's History screen to
`withdraw`, per trace.

## What the feature actually has to get right

Withdrawal means three different things, and the server distinguishes them on
the wire (`TRACE_WITHDRAWAL_REACH_*` in the ingest binary,
`DistributionReach` in `crates/trace-commons-contributor/src/withdraw.rs`):

| tier | what it achieves |
|---|---|
| `not_distributed` | never entered the commons; content deleted, nothing distributed |
| `commons_not_distributed` | in the commons, never published; content deleted, excluded going forward |
| `commons_distributed` | in the commons and already published; content deleted and excluded going forward, but **copies already distributed cannot be recalled** |

The single thing this UI must not do is let a contributor believe they
achieved more erasure than they did. Everything else — where the button sits,
what colour it is — is negotiable against that.

## The structural problem: the tier is not knowable before the call

The tier is computed by the server *during* the withdrawal, from live export
membership: the endpoint's rule is `status != Accepted` -> `not_distributed`,
otherwise `count_trace_export_memberships > 0` decides between the two commons
tiers. It arrives in the response.

The confirmation has to be shown before that response exists, and nothing the
daemon gives this app carries export membership — `HistoryRecord` has a
`status` and nothing else bearing on it. So the confirmation is keyed on what
this machine actually knows:

- `submitted` / `quarantined` -> `not_distributed`, exactly. The server's own
  rule is the record's status, so this mapping is not a guess. The canonical
  `not_distributed` body is shown alone.
- `accepted` -> **either** commons tier, and this app cannot tell which.
  Showing only the `commons_not_distributed` body would be claiming more
  erasure than may have been achieved. So both canonical bodies are shown, the
  app says plainly that it cannot tell them apart from here, and the
  `commons_distributed` body is the one weighted in coral with a warning
  glyph. A contributor who reads that and withdraws anyway has not been misled
  if the trace turns out to have been published; one shown only the gentler
  body would have been.
- anything else -> the `commons_distributed` body, because the furthest tier
  cannot be ruled out.

The exact tier is then reported after the fact, from what the server actually
applied. That is a report, not the confirmation; the confirmation still comes
first, as the contract requires.

## The copy

Verbatim from the "Canonical confirmation copy" table in
`docs/contributor-daemon-ipc-v1_1.md`, held in `WithdrawalCopy` and pinned by
`WithdrawalCopyCheck`. Nothing was paraphrased.

**`not_distributed`** (shown for `submitted` and `quarantined`):

> Withdraw this trace?
>
> This trace never entered the commons. Withdrawing deletes it. Nothing was
> distributed and nothing needs recalling.
>
> Credit already recorded stays.
>
> [ Keep it ] [ Withdraw ]

**`accepted`** — both bodies, because the tier is not knowable here:

> Withdraw this trace?
>
> This trace is in the commons. Whether it has already gone into a published
> export or benchmark is decided on the server, and this app cannot tell from
> here which of these two applies:
>
> - This trace is in the commons but has not been included in any published
>   export or benchmark yet. Withdrawing deletes it and excludes it from
>   everything published from here on.
> - **This trace has already been included in a published export or benchmark.
>   Withdrawing deletes our copy and excludes it from everything published from
>   here on, but copies that have already been distributed cannot be recalled.
>   Withdrawing does not undo that.**
>
> Credit already recorded stays.
>
> [ Keep it ] [ Withdraw anyway ]

**Unrecognised status** — the same, reduced to the `commons_distributed` body
alone under "This app does not recognise what stage this trace reached, so it
cannot rule out the furthest one".

**Afterwards**, never a generic "withdrawn": `"Withdrawn. "` plus the canonical
body of the tier the server actually applied. If the daemon sends a tier label
this build does not know, the app says it cannot tell whether the trace had
already been published rather than assuming the mild answer.

**Credit.** "Credit already recorded stays." is verified, not assumed: the
endpoint's response sets `credit_retained: true` unconditionally, with the
comment "Always true. Withdrawal is not a punishment: credit already awarded
stays awarded". The app says only that — nothing about amounts, settlement, or
worth, and nothing implying withdrawal reverses anything.

**When it does not happen.** Every failure sentence opens with "Nothing was
withdrawn and nothing was deleted." A contributor must not walk away from a
failed withdrawal believing their trace was taken back. The
`account-session-required` case says what is missing (account sign-in, which
this build does not have) rather than reading as a generic error.

**`not_found`** is handled and discloses nothing: "There is no trace with that
id under your account." It is unreachable today —
`daemon/withdraw.rs` collapses every `WithdrawError` into the single label
`withdraw-failed`, so the 404 the client crate carefully classifies never
reaches this process — but the day that label is passed through is not the day
to be inventing that sentence.

## Bulk withdrawal: not included

The contract's rule 5 permits bulk only if the confirmation can say the
selected traces may fall into different tiers and some may already have been
distributed. That sentence is writable. The reason bulk is still left out is
the rule above it:

- `withdraw_bulk` returns only `withdrawn` and `failed` counts, never a
  per-trace `distribution_reach`. So afterwards there is nothing to report but
  a number, and rule 1 — never a generic "withdrawn" — cannot be honoured at
  all for any of the traces involved.
- It also selects its targets from the local history cache's `status`, which
  can be stale. A trace accepted and exported since the last refresh would be
  swept up under a confirmation that had described it as never having left the
  holding area.

The quarantine group now says so in place of the old disabled button, rather
than leaving the absence unexplained, and points at per-trace withdrawal.

## What changed

- `Models.swift` — `WithdrawalReach` (the server's wire names) and
  `WithdrawalOutcome`, decoded leniently so an unknown tier label leaves the
  reach `nil` rather than discarding a withdrawal that really happened.
- `DaemonClient.swift` — `withdraw(submissionID:)`. No bulk wrapper.
- `AppModel.swift` — `withdraw(_:)`, plus `withdrawals` / `withdrawing` keyed
  by submission. Bypasses the `perform` helper the way `enroll` does: that
  helper would put `"withdraw: account-session-required"` in the screen-level
  error banner, which is not a sentence anyone can act on and leaves it
  genuinely ambiguous, next to a row still reading "In the commons", whether
  the trace was withdrawn. On success `refreshHistory` re-reads the status
  rather than assuming it.
- `Views/WithdrawalCopy.swift` — new. All the copy, and the assertions on it.
- `Views/HistoryView.swift` — the inline confirmation, the per-row outcome,
  `withdrawn` as its own status ("Withdrawn by you", coral, uturn glyph) so a
  withdrawn trace stays on the list reading as withdrawn instead of vanishing,
  and `WithdrawalConfirmationCapture` for the screenshot hook.
- `DebugScreenshot.swift` — one added `render` line. See the note below.

## Constraints honoured

- **Return is bound to nothing.** The confirmation is an inline panel, not an
  alert or a sheet, so no button acquires `.defaultAction` implicitly. "Keep
  it" takes `.cancelAction` (Escape backs out, which is safe); the confirm
  button has no key binding at all.
- **Colour.** The confirm uses `tcPrimaryAction()` — the measured pair
  (`#137C61` + white, 5.14:1 light; `#3FBE9A` + `#0B1F19`, 7.39:1 dark). No
  label was put on the raw accent. Withdrawal's destructive weight is carried
  by the coral *text* role (`TC.coralText`, the darkened light-mode twin that
  clears 4.5:1 on a card face) on the cannot-be-recalled body, plus a warning
  glyph and the words themselves — coral on type, never as a fill, so no new
  fill pair needed measuring.
- No new dependencies, no emojis, nothing touched in `TCBridge/`,
  `SelfTest.swift`, `DaemonHost.swift`, or any FFI or teardown code.

### The one judgement call on DebugScreenshot.swift

The brief forbids touching the FFI and teardown logic in that file, and the
verification step requires looking at rendered screenshots of the new UI. The
capture list lives in that file and rendered no History surface at all. I read
the constraint as protecting the FFI/teardown code in those files and added a
single `render(...)` line — no existing line altered, nothing touching
`model.shutdown()`, `NSApp.terminate`, or any concurrency. If that reading is
wrong, reverting that one line costs the screenshots and nothing else.

`WithdrawalConfirmationCapture` is built from plain `Text` and `Button`:
`ImageRenderer` will not rasterize NSView-backed controls (`Toggle`,
`TextField`, `Menu`, segmented `Picker`), which come out as yellow
placeholders in a capture while being fine in the running app. The captures
below have no placeholders. It renders fabricated records because the demo
state directory has no enrolment and therefore no history at all, so a capture
of the real screen would show an empty list and prove nothing about the copy.

## Verification

- `cargo build -p trace-commons-contributor-ffi` — clean.
- `cd macos && swift build` — clean, no warnings.
- Screenshots regenerated in both appearances via `macos/scripts/run-demo.sh`
  (which always rebuilds) and inspected. `macos-shell-withdrawal.png` renders
  correctly in dark and light: all three confirmations legible, the coral body
  clearly the weighted one in both, no yellow placeholders, no clipping.
- Self-test passes and contains no trace content: `opening prompt: chars=97
  nonempty=true`, `redacted body contains raw AWS key: false`, `redacted body
  contains raw GitHub token: false`. Byte-identical between the dark and light
  runs.
- Baseline captured before claiming that: the self-test's trailing `last
  action error: list_projects: failed` was reproduced on a stashed tree at
  `33f25a34`, so it is pre-existing on `main` and unrelated to this change.

## What I could not verify

- **No withdrawal has ever succeeded from this app, and none can yet.** Both
  daemon methods answer `unavailable` / `account-session-required` before any
  request leaves the machine — the daemon holds a device key and withdrawal is
  authenticated by an account session, which nothing in the tree acquires or
  stores. So the success path, the tier-specific outcome sentences, and the
  `withdrawn` row state are exercised only against fabricated records here.
  They will get their first real exercise the day account sign-in lands.
- The tier the server would apply to any specific pilot trace, for the same
  reason.
- The `not_found` sentence is unreachable through the current daemon and was
  not exercised.

## Drift risk worth knowing about

This app cannot call `confirmation_prompt` in
`crates/trace-commons-contributor/src/withdraw.rs` — it is not on the IPC
contract and the C ABI does not export it — so `WithdrawalCopy` is a second
copy of wording that is supposed to have exactly one. Note that the Rust
function and the document's canonical table are themselves already two
different wordings of the same three tiers; this app follows the document,
which is what the three-application rule points at.

`WithdrawalCopyCheck` is the mitigation: it asserts the canonical bodies are
intact, that an `accepted` trace is never shown only the gentler body, that a
not-yet-in-the-commons trace is never told it was excluded from exports it was
never in, that every failure sentence opens by saying nothing happened, that
the not-found sentence discloses neither existence nor ownership, and that no
outcome is reported as a bare "withdrawn". The Swift package has no test
target, so the History screen evaluates it and renders a visible defect banner
when it fails, rather than leaving assertions nobody runs.

## Not done here

`docs/contributor-daemon-ipc-v1_1.md` on `main` still documents the old, wrong
tier names (`in_commons` / `distributed`) and has no canonical copy table. The
fix is on branch `withdrawal-copy-canonical` and is deliberately not duplicated
into this branch, to avoid a conflicting identical change. This app already
uses the correct names, which come from the Rust, not the document.

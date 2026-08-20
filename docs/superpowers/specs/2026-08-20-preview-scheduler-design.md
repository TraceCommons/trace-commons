# A bounded preview job scheduler in the contributor daemon

Status: implemented for previews. The approve path and the uploader are
named as future consumers with a stated seam; neither is converted here.

## The problem, measured

A contributor with roughly 500 queued sessions launched the macOS app.
Within three minutes it had consumed five minutes of CPU (about 1.7 cores
sustained), grown to 1.34 GB resident, and the machine's one-minute load
average reached 649. The app was force-quit; it is still force-quit.
Sampling showed essentially all of the time inside serde JSON parsing,
reached through the C ABI.

The cause is not subtle. `AppModel.loadMissingSummaries()`
(`macos/Sources/TraceCommonsApp/AppModel.swift:462`) loops over every queued
entry and spawns a `Task.detached` per entry calling `previewSummary`. Each
of those runs the full preview pipeline: read the whole session file, parse
it, run the redaction pipeline, build and serialize a redacted envelope.
Nothing anywhere bounded how many ran at once, so the answer was "as many as
there are queued entries".

The corpus on that machine, which every number below is grounded in:

| source | files | total | largest | mean |
| --- | --- | --- | --- | --- |
| Claude Code | 1,028 | 0.9 GB | 29.8 MB | ~0.9 MB |
| Codex | 3,069 | 10.8 GB | 367.5 MB | ~3.5 MB |

Two facts follow from that table and drive most of this design. First, the
mean session is small and the tail is enormous — three orders of magnitude
between the Codex mean and the Codex maximum. Second, the aggregate is far
larger than the machine's memory, so any design whose worst case scales with
the number of queued entries is wrong by construction, not merely slow.

## Where the fix belongs

In the daemon.

All three shells request previews. Only the daemon can see the total. A
per-shell bound is the same rule written three times in three languages,
with nothing keeping the three honest — and the current state of that
asymmetry is the argument: the GTK shell already has a crude bound (a single
worker thread, deliberately, "so a slow preview cannot stall the event
stream", `crates/trace-commons-contributor-gtk/src/worker.rs`), the Windows
shell has its own arrangement, and the macOS shell has nothing at all. A
daemon-side bound makes that class of divergence impossible rather than
merely discouraged.

The new module is
`crates/trace-commons-contributor/src/daemon/preview_scheduler.rs`. It
contains a queue, a worker pool, a dedup table, and a result cache. It does
not contain any part of the preview pipeline: it calls
`ipc::build_and_pin_preview` (which calls `preview::build_preview`) through
a trait and stores whatever comes back as an opaque JSON object.

## Scope: preview-specific queue, not a general work queue

An audit found three unbounded whole-session paths in the daemon, and this
design deliberately fixes one of them.

1. **Previews** — fixed here.
2. **Project-wide approve** — `handle_approve` in `daemon/ipc.rs`. With
   `all: true` or a `project_id`, it builds and pins an envelope for every
   unpinned entry, sequentially, inside one IPC request. Sequential is the
   right shape for memory (the peak is one build, not N), but one click can
   start hundreds of full read-parse-redact-serialize passes over files up
   to 367 MB, with no progress reporting and no way to cancel. Shipped in
   0.4.2; not yet exercised at scale.
3. **The uploader** — `daemon/uploader.rs` reads and hashes each approved
   session in full, one at a time, from the supervisor task.

**The queue built here is preview-specific.** The reasoning:

The three paths agree on the resource they contend for and disagree on
everything else. A preview may be deferred indefinitely, dropped, cancelled,
served stale from cache, or refused outright for size, and the worst
consequence is a card that says "still working". An approval's envelope
build may do **none** of those things, for a reason that is a correctness
guarantee rather than a preference: an entry with no
`previewed_envelope_digest` is not refused at upload.
`approved_envelope_for` returns `Ok(None)`, `submit` reads `None` as "build
one", and the uploader then silently constructs a fresh envelope and sends
it. The pin is what makes it impossible for a contributor to send bytes they
were never shown. An approval that completed with its pin merely *queued*
would be a regression in the consent guarantee, traded for a performance
win. That is not a trade this product makes.

A single queue serving both would therefore need a per-kind policy table —
droppable vs. must-complete, cacheable vs. not, cancellable vs. not,
deferrable vs. not — plus a fairness rule between kinds that nobody has
requirements for yet. Writing that table speculatively, with one real
consumer, is how the wrong abstraction gets locked in. The honest sequencing
is: bound previews correctly now, and convert the other two when their own
semantics have been worked out, against a seam that already exists.

### The seam, for the other two consumers

What a future consumer needs from this module is already isolated:

- **`PreviewJobRunner`** is the whole coupling to the work itself: `run(job)
  -> future<outcome>` and `deliver(entry_id, &outcome)`. A second job kind
  is a second implementation of a sibling trait, not a change to the queue.
- **`PreviewScheduler::take_next` / `finish`** are the queue discipline,
  separate from `worker_loop`, and are public precisely so a different
  worker shape can drive them.
- **`admits(bytes)`** is admission control, separate from both.

**For the approve path** to become a consumer, what is needed is not queue
code but a decision about the request. Approve's builds cannot be deferred
past the response without changing what `approve` means, because the
response is what tells the contributor the approval succeeded and the
pin is what makes that true. The seam is therefore a *non-droppable,
non-cancellable, must-complete-before-reply* job kind whose only shared
behaviour with previews is the concurrency bound and the result cache —
and the cache is the actual prize: a project-wide approve over entries the
shell has already previewed should find every envelope already built and do
almost no work at all. Concretely: teach `build_and_pin_preview` to consult
the scheduler's cache before building, keyed on the same `PreviewKey`. That
is a small change and it is *not* made here, because a cache hit must
guarantee the pin was actually written, and today the pin is a side effect
inside `build_and_pin_preview` rather than something the cached value
records. Making the cached outcome carry "the envelope was pinned under
digest X" is the prerequisite, and it deserves its own review.

**For the uploader** the seam is easier and the payoff smaller: it is
already serial and already unattended, so what it wants is not a queue but
to share the *bound* — to not be reading a 367 MB session at the same moment
two previews are. That is a daemon-wide admission gate (a semaphore all
three acquire), which is a different object from the queue in this module.
It is deliberately not built here: with one consumer it would be a semaphore
with a permit count and no contention, i.e. speculative.

### What must never be cancellable or evictable

- **An approval's envelope pin.** Stated above; it is the consent
  guarantee, not an optimisation.
- **Nothing else.** Preview jobs are freely droppable, and the uploader's
  reads are already governed by the queue file rather than by scheduling.

## The queue model

One arrival-ordered `VecDeque` of jobs, plus:

- `queued: HashSet<Uuid>` — membership, for O(1) dedup.
- `running: HashMap<Uuid, PreviewKey>` — what each worker holds, and the
  bytes it holds it against.
- `cancelled: HashSet<Uuid>` — running entries whose result is to be thrown
  away.
- `visible: HashSet<Uuid>` — what a shell says is on screen.
- `cache: HashMap<PreviewKey, PreviewOutcome>`.

`take_next` scans the deque for the first job whose entry is visible and
falls back to the front. That is O(n) per claim, which at 500 entries is a
few hundred pointer comparisons under a lock already held — irrelevant next
to a job that reads a megabyte off disk. A heap would be the right structure
at a queue depth this design never reaches.

## Concurrency: two workers

`PREVIEW_WORKERS = 2`.

**Memory is the binding constraint, not CPU.** A preview holds the file
bytes, the parsed form, the redacted form, and the serialized envelope alive
at once, so its peak resident set is a multiple of the session size. The
scheduler's worst case is therefore `workers x admission_cap x k`. With the
64 MiB cap below, two workers keep the pool's peak in the hundreds of
megabytes. The incident reached 1.34 GB and was still climbing; four workers
would put a machine back inside that range on a corpus with a fat tail.

**The CPU argument agrees, and is why the number is not derived from
`available_parallelism`.** This is a background daemon whose visible job is
to stay invisible. The cores it does not take are the ones the contributor's
editor and the shell rendering these cards get to use. A machine-scaled pool
would take *more* of a big machine, which is exactly backwards: a
contributor with 16 cores has not asked for 8 of them.

**Why not one.** One would also end the incident, and was rejected for a
stated reason: with a single worker, one 64 MiB session stalls every card on
screen behind it, and the visible-first ordering cannot help, because
ordering decides what starts next and never what is already running. Two
means a slow job costs at most half the throughput. This is the same
trade-off the GTK shell's single thread makes, and the reason to move the
bound into the daemon is partly to stop making it that badly.

**Why not three or more.** Nothing in the measurement suggests throughput is
the problem. 4,097 sessions at two concurrent builds still drains a queue
faster than a contributor reviews it, and the failure mode being fixed is
resource consumption, not latency.

## Priority

`preview_visible` replaces the visible set wholesale. Wholesale, not
add/remove, because a shell knows its own visible set after a scroll and
does not know which entries left it — an incremental interface would make
each shell diff something the daemon can diff for free, three times, in
three languages. The call takes one lock, touches no jobs, and is intended
to be sent on every scroll settle.

**Visibility decides order, never membership.** An entry that scrolls off
keeps its place in the queue. A shell that wants it dropped says so with
`preview_cancel`. Conflating the two would mean a fast scroll through a list
cancelled and re-enqueued everything it passed.

Ordering is: first job in the deque whose entry is visible; otherwise the
front. Arrival order is the tie-break inside each class.

## Cancellation, honestly

`preview_cancel` (and `dismiss`, which cancels implicitly) does two
different things depending on where the entry is:

- **Queued:** removed. No work is done. This is real cancellation.
- **Running:** flagged. The result is discarded rather than delivered or
  cached.

**A running job cannot be interrupted mid-parse.** `preview::build_preview`
reads, parses, redacts, and serializes with no cancellation point, and the
expensive stretch — the serde parse every sample in the incident was sitting
in — is a single call the scheduler does not get control back from. Aborting
the task would be the only way, and a task aborted at an arbitrary point
inside the pin path is not obviously safe to abort. So cancelling a running
entry costs exactly the CPU that job had left to spend. What it buys is that
no event is published and nothing is cached.

The bound on how much wasted CPU that can ever be is `PREVIEW_WORKERS` jobs'
worth. **That is why the bound is the load-bearing part of this design and
cancellation is not.** A shell should not be built on the assumption that
cancelling is free or immediate.

A cancelled running job's result is discarded rather than cached, even
though caching it would be free value. An entry is cancelled because it was
dismissed or scrolled away; holding a redacted summary of a trace the
contributor just declined is retention with no consumer.

## Admission control by size

`MAX_PREVIEW_SESSION_BYTES = 64 MiB`, measured against the queue entry's
`size_bytes` (the whole group, including delegated subagent transcripts).

Grounded in the table above: the largest Claude session measured is 29.8 MB,
so every one of them is admitted with room to spare; the Codex mean is
3.5 MB, two orders of magnitude below the cap. What the cap excludes is the
Codex tail, where a single rollout is a hundred times the mean.

**The exact excluded fraction is not computed, and is not claimed.** The
measurements available are counts, totals, and maxima; a percentile cannot
be recovered from those. The claim being made is "the mean is two orders of
magnitude below the cap", not "the cap admits 99.x%".

**An over-cap session is refused visibly and carries no size estimate.** The
outcome is `too_large` with `raw_session_bytes` (a `stat`, not an estimate)
and `limit_bytes`. There is no `would_send_bytes`, because a would-send
number is a claim about an envelope that was never built, and the preview
card is a consent surface. Showing an estimate there as though it were exact
is the one failure mode this product cannot have. A shell renders "too large
to preview" and the raw size.

This is a **preview** policy, not an upload policy. Nothing here decides
whether such a session can be contributed; `approve` still builds and pins
it, and `approved_envelope::save`'s own size check still governs. Computing
the exact number later — on demand, for one entry, when a contributor asks
— stays open and is the obvious follow-up.

## The result cache

Key: `(path, file size, file mtime, group size, config fingerprint)`.

The first three are the memoization key `source::codex::peek_cwd_memoized`
already uses, and the reasoning transfers verbatim: a file whose size and
mtime are unchanged has unchanged contents, and this is not a trust
boundary, because anyone able to backdate an mtime can equally well write
whatever they like into the file.

Two more components, each for a specific failure the three-part key has:

- **`group_bytes`** — the entry's own `size_bytes`, which for a Claude
  session covers the primary file *plus every delegated subagent
  transcript*. A subagent file growing does not change the primary file's
  stat, so the three-part key would keep answering with a summary of fewer
  bytes than an upload would send. The watcher refreshes this on its poll.
- **`config_fingerprint`** — `preview::input_fingerprint`, so a change to
  consent scopes, identity, or privacy-filter settings invalidates every
  cached card rather than leaving the contributor reading one built under
  the old configuration. An unenrolled device uses a fixed label instead;
  enrolling therefore invalidates every unenrolled preview, which is
  required, since an unenrolled build is a placeholder-identity artifact
  that is deliberately never pinned.

Building the key costs one `stat` per request. That is the cheapest honest
option: keying on the queue entry alone cannot see a file rewritten in place
at the same size, and re-resolving the session ref through discovery is a
full session-root scan to answer a question about one file.

**Residual, stated rather than buried:** a subagent transcript written
between one watcher poll and the next is not reflected until that poll.
Entries are only offered once eligibility judges the whole group quiescent,
so the window is one in which the entry is being superseded anyway — but it
is a window.

**Lifetime: process, not disk.** The cache dies with the daemon. That covers
the incident and the common case — the shells restart far more often than
the daemon, and a relaunched app finds every preview it asked for last time
already built — and it does *not* cover a daemon restart, which redoes the
work. An on-disk cache is deferred deliberately: cached previews are
redacted trace summaries, and putting them on disk is a retention decision
with its own review, not a performance tweak. `daemon::approved_envelope`
already persists envelopes for pinned entries and is the existing precedent
for what that review would have to cover.

Capacity is `PREVIEW_CACHE_CAP = 4096`, cleared wholesale at the bound,
mirroring `CWD_MEMO_CAP` in both value and reasoning.

## IPC additions

All additive. `preview`, `preview_body`, and `preview_turns` are untouched
in request shape, response shape, and behaviour: the CLI and the C ABI still
use the blocking path, and a `v1_1` client that ignores the new methods is
unaffected. Full wire detail is in
`docs/contributor-daemon-ipc-v1_1.md`.

- **`preview_request`** `{entry_id}` → `{entry_id, state}` where `state` is
  `queued`, `running`, `ready`, `too_large`, or `failed`. Never blocks on a
  build. `ready` carries `summary`; `too_large` carries `raw_session_bytes`
  and `limit_bytes`; `failed` carries `code` and `label`.
- **`preview_visible`** `{entry_ids[]}` → `{visible}`.
- **`preview_cancel`** `{entry_id}` → `{entry_id, dropped}`.
- **`preview_ready`** event, on the existing subscription stream, carrying
  exactly the object `preview_request` returns for a cache hit.

The event goes on the existing `broadcast` channel that `subscribe` already
serves, rather than any new delivery mechanism: shells already run that loop
and already handle `resync_required` when they fall behind it.

One deliberate difference from `preview`: `preview_request`'s `summary` does
**not** carry an `entry` object, where `preview`'s response does. The
summary is cached, and a queue entry's state changes underneath it;
embedding one would let a cached preview assert a stale state. Shells
already hold entries from `list_pending` and `snapshot`.

`dismiss` now cancels any scheduled preview for the entry it refuses.
`approve` deliberately does not — approve needs the envelope, and
cancelling the build it is about to require would be self-defeating.

## What each shell must do

Not done here; the shells are adapted separately.

1. Replace the fan-out (`loadMissingSummaries` and its equivalents) with one
   `preview_request` per card. Do not wait for a response before drawing;
   draw a pending card.
2. Handle `preview_ready` on the existing subscription and fill the card in.
3. Send `preview_visible` with the on-screen entry ids after each scroll
   settles. Cheap, idempotent, and safe to repeat.
4. Send `preview_cancel` when a card is dismissed or leaves the list for
   good. Not on every scroll — visibility already handles ordering.
5. Render `too_large` as "too large to preview", showing
   `raw_session_bytes` only. **Never** synthesize a would-send number for
   it.
6. GTK: its single worker thread becomes redundant for previews. Removing it
   is optional and is not a correctness matter, since the daemon now bounds
   the work either way.

## Verification

In `preview_scheduler.rs` (unit) and `tests/daemon_ipc_contract.rs`
(over a real socket). Each asserts a value, not an enqueue:

- 400 requests put exactly `PREVIEW_WORKERS` jobs in flight, no third job
  starts while those two are parked, and the peak observed across a full
  drain of all 400 equals the bound.
- 50 requests for one entry produce exactly one run and exactly one
  delivery.
- A cancelled running entry delivers nothing and caches nothing; a
  cancelled queued entry is never taken.
- A cache hit returns the real summary with no run, and size, mtime, group
  size, and config fingerprint each invalidate independently.
- A visible entry requested after 400 others is the next job to start —
  asserted on a one-worker pool, so the claim is about queue discipline and
  not about which of two workers won a race.
- A 367.5 MB session is never parsed, and its refusal carries no
  `would_send_bytes` and no `summary`.
- Over the socket: `preview_request` returns `queued` without building,
  the `preview_ready` event carries a real envelope digest and non-zero
  redaction counts and no unredacted trace text, and the second request is
  answered `ready` with the identical digest and publishes no second event.

## Rejected alternatives

**Bound it in each shell.** Three implementations, three languages, already
diverged before anyone tried. Rejected in the brief and confirmed by the
current state of the three.

**Abort the tokio task to cancel a running build.** The only mechanism that
would actually stop a parse. Rejected because the pin path is not obviously
abort-safe, and because the bound already caps the waste at two jobs.

**Estimate `would_send_bytes` for over-cap sessions from the raw size.**
Rejected: the preview card is a consent surface and the ratio between raw
bytes and envelope bytes is not stable — for the small fixture in the
contract test the envelope is *larger* than the raw file.

**One general whole-session queue now.** Rejected above: it would need a
per-kind policy table written against one real consumer, and the kind that
matters most (approve) has a correctness constraint that previews do not.

**Persist the cache to disk.** Rejected for now: cached previews are
redacted trace summaries, and writing them to disk is a retention decision
needing its own review rather than a performance tweak.

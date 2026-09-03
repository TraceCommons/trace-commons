# Folder-first queue, and what the scrubber will admit to

An alpha contributor ran the 0.7.0 contributor app against a real machine --
149 sessions waiting -- and reported ten things. This design answers all
ten across the three shells (macOS, GTK, Windows).

The report is worth reading as one complaint rather than ten, because
that is what it is: **at 149 sessions the queue stops being a list and
becomes a haystack, and the app still renders it as a list.** Every
navigation item below follows from that. The scrubber items are a second,
separate complaint -- the app tells a contributor how much it removed and
refuses to say what -- and that one is about trust, not scale.

Related: `2026-08-20-one-click-submit-design.md` (whose single-entry-group
rule this design retires, with reasons), and
`crates/trace-commons-contributor/src/daemon/policy.rs`, whose path-privacy
invariant this design relaxes in exactly one place and nowhere else.

## What is not changing

Stating these first, because three of the ten items look like they ask
for one of them and do not:

- **A local filesystem path still never reaches `daemon-audit.jsonl`, OS
  notification text, or a `HistoryRecord`.** `an_audit_entry_never_carries_a_path`
  keeps passing, unmodified. The relaxation below is to the IPC response
  only.
- **`Look inside` keeps its emphasis as the row's primary action.** Item 9
  asks for the card to be clickable, not for the button to go away, and the
  reasoning recorded in `QueueView.swift`'s `actions` -- that promoting
  `Submit` would change what the app advises on the screen where its advice
  matters most -- is untouched by making the card a second route to the same
  place.
- **The removed values themselves are never rendered.** Item 3 asks to see
  "what doesn't go". It is answered below without ever drawing a secret on
  screen, and the reason that distinction is worth the extra work is that a
  window listing every credential a session touched is a worse artifact than
  the session was.

---

## 1. The daemon changes, which land once for three shells

### 1.1 A project's path, on the socket only

Queue entries and `list_projects` rows gain `project_path: String`,
`~`-abbreviated for display. `project_label` is unchanged: still the bare
basename from `project_label_for`, still the only project string that
crosses into any audit, notification, or history sink.

This is a deliberate widening of the rule stated in `policy.rs`'s
`project_id_for` doc comment -- "the privacy rule is that a project key, a
local filesystem path, never crosses the socket". The rule was written
because a label lands in three sinks a path must not reach -- the audit
log, notification text, and history. That reasoning is
sound and survives intact for `project_label`; it was never an argument
about the socket itself, which carries the contributor's own transcript
bodies under the preview exemption already. A path is strictly less
sensitive than the transcript recorded inside it.

So the field is added, and the invariant is re-pointed rather than removed:

> `project_path` may be rendered. It may not be logged, audited, notified,
> or persisted to history.

A new test asserts the second half by walking every audit and history sink
for the new field's absence, mirroring the existing path test rather than
replacing it.

**Why this is necessary and not merely nice.** The reporter did not ask for
paths out of curiosity. Three of their ten items -- "it doesn't unify the
same folder", "I have the same path 2-3 times", "folder name should be more
prominent" -- are the same observation: `disambiguated_label` renders two
different projects as `api` and `api (3f9c)`, and a contributor holding
those two labels cannot tell which is which. The hash suffix was built to
keep them *distinct*; it was never able to make them *identifiable*. A path
is the only thing that can.

### 1.2 Normalizing the project key

`project_key_for` keys on the raw `cwd` string an agent recorded. Two
sessions in the same directory therefore land in different groups whenever
the recorded strings differ, which they routinely do. Add a normalization
pass ahead of the key:

1. Resolve symlinks (`/var` vs `/private/var` on macOS is the common one).
2. Strip trailing separators.
3. Case-fold on case-insensitive volumes.
4. Walk up to the nearest enclosing VCS root (`.git`, `.hg`, `.jj`), if one
   exists within a bounded number of levels.

Step 4 is the one that unifies Codex with Claude Code, because the two do
not agree on whether the cwd is the repo root or the subdirectory the
session happened to start in. It is also the one with a cost: two sibling
subdirectories of one repo become one project, and a contributor who wanted
them separate cannot get that back.

The cost is paid down by keeping the raw recorded cwd on the entry (as
`session_path`, same rendering rules as `project_path`) and showing it in
the folder detail view, so nothing is lost -- the sessions are grouped by
repo and still say individually where they ran.

Steps 1-3 are unconditional. Step 4 is worth stating as reversible: it is a
single function, and if the merge turns out to be wrong for real users it
comes out without touching anything else.

Normalization changes `project_id_for`'s input, and ids are derived rather
than stored, so **every existing project id changes on upgrade.** What that
actually breaks is `daemon-projects.json`: modes keyed by the old key are
orphaned, and a project a contributor had set to `ignore` would silently
re-arm. That is not acceptable. Migration: on first load after upgrade,
re-key the policy file through the normalizer, merging any two entries that
collapse to one key by taking the **more restrictive** mode. The merge rule
is the safety property here and gets its own test.

---

## 2. Queue: folders first (items 1, 2, 8)

### 2.1 Root

The queue root lists folders, not sessions:

```
149 sessions waiting for your decision

  ironwire                                   12 sessions   6.1 MB   >
  ~/code/ironwire
  [ Submit all (12) ]  [ Submit all as v ]  [ Ignore ]

  trace-commons-server                        8 sessions   3.4 MB   >
  ~/code/trace-commons-server
  [ Submit all (8) ]   [ Submit all as v ]  [ Ignore ]
```

The folder name is the row's largest text. Today it is the *smallest* text
on the line -- `TC.Font_.meta` in `inkSecondary`, beside a primary-styled
`Submit all` -- which is the direct subject of the reporter's first item.
The line currently reads as a button with a caption; it should read as a
place with actions.

`QueueGrouping` already computes id, label, bytes, and entries in one pass,
so the root needs no new grouping work -- only the count and byte totals it
already carries.

### 2.2 Detail

Selecting a folder pushes a view listing that folder's sessions: today's
cards, unchanged, scoped to one project, with a back affordance and the
folder's name and path as the heading. Each card gains the `session_path`
line from 1.2 when it differs from the folder's own path.

### 2.3 A one-session folder gets `Submit all` too

`ProjectQueueGroup` hides `Submit all` when `count == 1`, on the recorded
reasoning that a single-entry group "offers no second way to do what its one
row's own `Submit` already does". That was true of a flat list, where the
row and the group header were on screen together. Under drill-in the row is
one level down, so a contributor with a one-session folder would have to
open the folder to do the thing the folder is offering -- which is exactly
the reporter's item 8.

The rule expires with the layout it was written for. Every folder row
carries `Submit all (n)`, including `n = 1`. The comment in
`ProjectQueueGroup` is updated to say so rather than deleted, so the next
reader finds the history instead of rediscovering the argument.

### 2.4 The card is clickable (item 9)

The whole card becomes the hit target for `Look inside`. The button stays,
with its current emphasis, for the reasons in "What is not changing".
`Submit`, `Not this one`, and the button itself stop propagation.

---

## 3. The scrubber says what it removed (items 3, 5, 7)

### 3.1 Redactions are already visible, and the app is hiding them

`DeterministicTraceRedactor` does not delete a matched value. It substitutes
a typed placeholder from `PlaceholderMap::placeholder_for`:

```
<PRIVATE_LOCAL_PATH_1>   <PRIVATE_SECRET_3>   <PRIVATE_CONTEXTUAL_ENTROPY_2>
```

Those tokens are **already in the bytes `tc_preview_body` returns.** The
shells render them as ordinary transcript text and the contributor scrolls
past them.

So the whole of item 3's first half needs no protocol change, no FFI change,
and no new data crossing any boundary. The shells scan the body and style
each hit as a labelled chip in the transcript.

**Two token shapes, not one, and the difference matters.** Only
`apply_placeholder_regex` mints the numbered `<PRIVATE_([A-Z0-9_]+)_(\d+)>`
form, and it is called for exactly two labels: `local_path` and
`private_email`. Everything else is replaced with one of three FIXED
tokens, none of them numbered:

| Token | Covers |
|---|---|
| `[REDACTED]` | `secret`, `secret:{pattern}`, `secret:contextual_entropy`, `secret:split_literal`, `sensitive_field` |
| `<REDACTED_PRIVATE_KEY>` | `secret:pem_private_key` -- angle-bracketed but carrying no index, so it does NOT match the numbered pattern |
| `[REDACTED:{label}]` | `tool_sensitive_field{:action}`, and every `privacy_filter:{label}` |

Only the third carries a label, so only the third can name its own category
in a mark. The first two can say that something left and not what.

That includes secrets, which is the category a contributor most wants to
see. A shell scanning only for the numbered form would mark every path and
no secret, while the summary panel beside it reports those secrets as
removed. The scan must recognise both shapes, and the ordinal must be
optional in the type rather than faked with a zero. The reporter's "show list of things that got removed" is
answered by the transcript itself, in place, which is better than a list
because it also answers *where*.

The token numbering is per distinct value -- the same path twice gets the
same token -- which the summary line should also use. `185 local path` is an
occurrence count; `185 local path (12 distinct)` is the number a person is
actually trying to estimate risk from, and it comes free from the highest
index per label.

Distinct counts come from the placeholder map, so they exist for exactly the
two labels that mint placeholders. `3 secret` will never carry a distinct
suffix. That is correct rather than a gap -- there is no second number to
report -- but it means no test may assert a distinct count for a secret
fixture, and the rendering must omit the suffix rather than print
`(0 distinct)` -- which, beside a non-zero occurrence count, reads as
"nothing was removed".

The two layers differ on the wire and a shell must not confuse them.
`PrivacyMetadata::redaction_distinct_counts` is
`skip_serializing_if = "BTreeMap::is_empty"`, so on a secrets-only envelope
the key is absent from the JSON entirely. `PreviewSummary` carries no such
attribute, so a shell reading a preview always sees the key, as `{}`. Either
way the renderer's rule is the same: no entry means no distinct count is
available for that label, not that the count is zero.

**A caveat the UI must carry, not bury.** Placeholders appear where
redaction *rewrote* a typed field. The detector scans every leaf; the
rewriter does not reach all of them. So a body with no placeholder in a
region is not a body with nothing sensitive in that region, and the existing
`ScrubbingCaveat` sentence is what says so. Highlighting makes the app look
more thorough than it is, and that is precisely the moment the caveat earns
its place -- it stays, next to the highlights, not at the bottom of the
screen.

### 3.1b The removed-summary panel

Marking placeholders in place answers *where*. It does not answer the thing
the reporter actually asked for -- "so I can right away see what doesn't go"
-- because collecting the marks means scrolling the whole transcript, which
is the opposite of right away. The card's one-line figure is the only
at-a-glance part, and it is a count, not a list.

So the preview's scrubbing tab gains a summary panel: one row per category,
with what that category is, how many times it fired, and how many distinct
values that covered. No matched text, ever -- the value is gone by
construction and the row says what KIND of thing left, not what it was.

**Labels are an open, namespaced vocabulary, and the panel must be built for
that.** The redactor emits `local_path` and `secret`, but also
`secret:{pattern_name}`, `privacy_filter:{label}`,
`tool_sensitive_field:{action}`, and `residual_secret_at:{schema_path}` --
the last three generated, so the set is not closed and a shell cannot hold a
complete table of it. Three rules follow:

1. **Group by family**, the part before the first `:`. A session that
   tripped nine different secret patterns is one `secret` row summing them,
   with the sub-labels on a detail line, not nine rows.
2. **An unrecognized family gets a neutral description**, never a guessed
   one, and is **never dropped**. Hiding a category because this build has no
   words for it would understate what happened, which is the one direction
   this panel must not fail in.
3. Sub-labels are safe to render. They are schema-shaped identifiers by
   construction -- `log_residual_secret_locations` depends on that same
   property -- never contributor strings.

The descriptions, which are the panel's actual value to a reader who has
never seen these words:

| Family | What it is |
|---|---|
| `local_path` | File paths from this machine. |
| `secret` | API keys, tokens, private keys, and high-entropy strings found next to credential words. |
| `privacy_filter` | Names, emails, and other personal details the privacy model found in prose. |
| `sensitive_field` | Fields whose name marks them sensitive, like `password` or `authorization`. |
| `tool_sensitive_field` | Tool-call arguments whose name marks them sensitive. |
| anything else | Removed by a pattern this version has no description for. |

#### `residual_secret_at` is not a removal, and the card currently says it is

`DeterministicTraceRedactor` sets `redaction_counts: report.counts` -- the
whole report. That report includes `residual_secret_at:{path}`, which
`note_residual_secret_location` increments when a secret was **detected and
NOT removed**: a credential inside a human correction, which is preserved by
design, or a field the typed traversal never visits, which is a real gap.

Both reach the shells in the same map as every genuine removal, and all three
render that map under the heading **"Removed by pattern"**. A session with a
surviving secret therefore reports it today as a thing that was taken out.
That is a pre-existing defect, it is exactly backwards, and it lands on the
one screen where a contributor is deciding whether to send something.

The panel is where it gets fixed, because the panel is the first surface with
room to say two different things:

- **Removed** -- every family except `residual_secret_at`.
- **Found, and still in what would be sent** -- `residual_secret_at`, in the
  attention tone, with its schema paths listed so the contributor can go and
  look.

The card's one-line figure gets the narrower half of the same fix: it excludes
`residual_secret_at` from the "removed by pattern" total rather than trying to
explain it in a strip that has no room. A session whose only count is a
residual therefore reads `nothing matched` on the card and carries the
attention state -- which is true, and is what the gold chip already exists to
say.

### 3.2 Search answers "was it removed?" (item 3, second half)

`tc_preview_search` scans the redacted body, by an absolute stated rule.
Searching it for a value that was removed correctly returns zero matches,
which is indistinguishable from the value never having been there -- and
those are the two answers a worried contributor most needs to tell apart.

Add one FFI entry point:

```c
/* Count occurrences of needle in the PRE-redaction session text of an
 * entry. Returns the match count, or -1 on error. Reports a COUNT ONLY:
 * no offsets, no context, no bytes. */
int32_t tc_search_original(tc_handle*, const char* entry_id, const char* needle);
```

It takes the handle and an entry id rather than a `tc_preview*`, which is
not a detail. `tc_preview` holds exactly `body` and `summary_json`, both
post-redaction; it has no pre-redaction bytes and must not acquire any.
Hanging the raw session off the preview would keep an unredacted transcript
resident in the shell's address space for as long as the sheet stays open,
which is a worse property than the one this call exists to provide. Taking
the handle instead lets the daemon re-read the session file, count, and drop
it -- so the raw bytes live for the duration of one call, on the side of the
boundary that already reads them.

The result renders as:

```
Search: "acme-corp"
  3 matches -- all 3 were removed
```

with the three honest cases: present and removed, present and still there
(the alarming one, already the existing amber path), and absent entirely.

This does widen the preview exemption, and the widening should be named
rather than slipped in. Today the exemption is bounded to post-redaction
content. This adds a **count-only oracle over pre-redaction content**, on a
live preview the contributor already opened, for a needle the contributor
themselves just typed. It returns a number and never a byte. It is not
logged. The bound is that a caller learns only the answer to a question they
already knew how to ask -- which is the entire point of the search tab, and
is why the count is safe where a context snippet would not be.

### 3.3 Recent searches stop recording prefixes (item 5)

`PreviewSheet.swift:825` runs the search on `onChange(of: needle)`, and
`run()` calls `RecentSearches.remember` on every non-empty result. Typing
`xyz` therefore records `x`, `xy`, and `xyz`, and the recents strip -- six
slots -- fills with prefixes of one word.

Live search stays; it is the good part. `remember` moves out of `run()` and
onto the explicit commit paths only (`onSubmit`, the `Search` button). Same
defect and same fix in the GTK and Windows shells, whose recents lists are
the same in-memory design for the same documented reason.

### 3.4 "nothing matched" gets something to do (item 7)

The gold chip is correct and stays gold: a session where no pattern fired is
the one worth slowing down on, and `ScrubbingCaveat` records why. What it
lacks is a next step -- the reporter's "it's a bit unclear what to do with
it" is a complaint about an affordance, not about a tone.

The chip becomes a control. Activating it opens the preview on the search
tab, which is the thing to do about it. The caveat line gains a clause
saying so.

---

## 4. Top-of-window state (item 10), partially

The nav item shows a shield glyph in three states -- clear, waiting,
attention (any waiting entry that matched nothing, or was trimmed to fit) --
**and keeps the numeric badge.**

The request was to replace the count with an icon. Not adopting that half:
at 149 the count is the reporter's own most-used signal, and an icon that
means "some" is a downgrade exactly at the scale that produced this
feedback. The shield adds the state the count could never carry; it does not
substitute for it.

---

## 5. History, grouped the same way (item 11)

`HistoryView` renders one flat list under a section header.
It takes the same folder-first drill-in as the queue, over the same
`QueueGrouping`, so the two screens navigate identically.

`HistoryRecord` today carries `project_label` and nothing else about the
project -- no id. Grouping on the label is not an option: a label is a
display name, is not unique across two projects, and grouping on it would
merge them, which is the same mistake `QueueGroup`'s own doc comment exists
to forbid.

So `HistoryRecord` gains `project_id`, and **not** `project_path` -- history
is one of the three sinks §1.1 protects. The id is admissible in that sink by
construction rather than by policy: `project_id_for` is a one-way SHA-256
prefix that "leaks no path component", which is the property it was built
for. Adding it therefore does not weaken the invariant the sink is protected
by; a path would.

The folder path shown in history is then resolved client-side, by matching
the record's `project_id` against the live `list_projects` response. A
record whose project the daemon no longer knows renders with its label
alone -- the honest outcome, and no fallback path is needed.

One consequence to state: §1.2 re-keys projects, so ids minted before the
upgrade do not match ids minted after it. Historic records will not resolve
to a path and will group under their labels. Backfilling is not possible --
the daemon does not retain the old key -- and is not worth faking. History
gets folder grouping for everything submitted after the upgrade, and older
records group by label, which is what they already do today.

---

## 6. Testing

- **Daemon.** Normalization table test (symlink, trailing slash, case,
  VCS-root walk, bounded depth, no-repo fallback). Policy re-key migration,
  with the more-restrictive-wins merge asserted directly. `project_path`
  present in the IPC response and absent from every audit and history sink.
- **Protocol.** Placeholder numbering is per distinct value (an existing
  property, currently untested, that §3.1's distinct-count now depends on).
- **FFI.** `tc_preview_search_original` returns counts for a value that was
  redacted away; returns 0 for an absent needle; refuses a stale pointer the
  same way its siblings do; and a test asserting it returns no bytes.
- **Shells.** Recents record one entry for one committed search, not three.
  Placeholder chips render for each label. macOS via `swift test` (the
  `macOS app tests` CI job), GTK and Windows via their existing suites.

## 7. Sequencing

Each is a PR:

1. Daemon: key normalization + policy re-key migration.
2. Daemon: `project_path` / `session_path` on the socket and `project_id`
   on `HistoryRecord`, with the sink test.
3. Protocol/FFI: `tc_preview_search_original` + distinct-count exposure.
4. macOS: queue drill-in, card click, shield, single-folder submit.
5. macOS: preview -- placeholder chips, original-search, recents fix,
   nothing-matched affordance.
6. macOS: history drill-in.
7. GTK: 4-6 combined.
8. Windows: 4-6 combined.

1-3 are prerequisites for 4-8 and are worth landing and living with first.
The three shells are independent of each other and can go in any order; the
reporter is on macOS, so 4-6 first.

The C ABI header exists in two copies that CI enforces byte-identical --
step 3 edits both.

## Not in this design

- Anything about *which* sessions are worth submitting. The queue is being
  made navigable, not filtered or ranked.
- Bulk selection across folders. `Submit all` remains per-project, which is
  the unit `submitProject` actually acts on.
- Persisting recent searches. They stay in memory, for the reason
  `RecentSearches` already records.

# Per-harness routing in the Model calls destination — design

## Why

The Model calls destination has one switch: whether this computer answers model
calls at all. It says nothing about *which tools send calls here*, and there is
no way to find out from the app.

The offer copy already promises the missing half:

> Turning this on does not change where any tool sends its calls. That stays a
> separate choice, **made one tool at a time**.
> — `OFFER_NO_REPOINT`

That per-tool choice is real, but today it is made by hand, in each tool's own
config file, outside this application, with no way to see the result. A
contributor who turns the switch on and expects something to happen sees
nothing — which is the same failure that put this feature on a buried settings
toggle in the first place.

**This slice gates the next release.** The destination alone is not a
releasable state.

## What already exists — the important finding

Almost none of this needs building. IronWire ships a crate,
`ironwire_agents`, whose entire purpose is this problem:

> Pointing a coding agent's own config file at IronWire, without taking it over.
> [...] the control API has to answer "which tools does this machine have, and
> are they pointed at us" as well.

It already carries the safety rules that make editing a file we do not own
acceptable at all, quoted from its own header:

- **Never rewrite a file we cannot parse.** A user's own syntax error must not
  come back looking like ours.
- **Fill an empty slot; leave a full one alone.** A value already in the key is
  another proxy or a deliberate choice, and taking it over would move someone's
  traffic without telling them. It is reported, not overwritten.
- **Remove only what we put there.**

The API is a close fit for the surface we want:

- `tools::all(&Catalog) -> Vec<Tool>`, where `Tool` is `{ id, name,
  config_path: Option<PathBuf>, installed: bool, wired: bool, connect_command:
  String }`. `wired` is precisely "whether its config currently sends calls
  here".
- `tools::plan_connect(id, port, &Catalog) -> Result<Planned, Error>` and
  `tools::plan_disconnect(id, &Catalog)`.
- `Planned { path, changes: Vec<String>, occupied: Vec<(String, String)> }` with
  `is_noop()`. `changes` is already phrased in words, and `occupied` names the
  slots it refused to take over.
- `tools::commit(&Planned) -> io::Result<Option<PathBuf>>`.

Two tools are built in — `claude` ("Claude Code") and `codex` ("Codex"). Anything
further arrives through a **signed catalog** (`ironwire_catalog::schema::AgentEntry`).

## A correction to the chosen mechanism

The design was chosen as: adopt a harness once by writing its config, then
toggle serve/pass-through **inside** IronWire, so ordinary flipping never
touches a file.

**That switch does not exist** -- but not for the reason first recorded here.

An earlier draft of this section claimed the control API was only
`/_ironwire/health` and `/_ironwire/status`. **That was wrong**, and it came
from a grep that matched full-path string literals while the routes are
registered as segments on a nested router. `ironwire_proxy/src/control.rs:491`
routes eleven: `/status`, `/backends`, `/pin`, `/admission-binding`,
`/settings`, `/privacy`, `/consent`, `/tools`, `/probe`, `/log`, `/events`,
`/health`. This daemon already calls `/settings`.

The conclusion survives, for a better reason. `POST /_ironwire/tools` is
connect/disconnect, not serve/pass-through: its handler calls `plan_connect` or
`plan_disconnect` and then **commits in the same request**. There is still no
per-tool pass-through anywhere, so ordinary flipping cannot avoid a config
write.

That atomicity is also why this design calls `ironwire_agents` in process rather
than going through the control API: `/tools` offers **no preview**. It decides
and writes in one step, which is exactly the step this design puts a
confirmation in front of. Working in process is what keeps plan and commit
separable.

So this spec is written against connect/disconnect. That is a smaller loss than
it first appears, because the three rules above already remove the hazards that
made a per-toggle config write look risky:

- Disconnect needs no saved original — it removes only what IronWire put there.
- Connect cannot clobber a value the contributor set — an occupied slot is
  reported, never overwritten.
- An unparseable file is refused rather than rewritten.

What is genuinely lost is speed and reversibility: each flip is file I/O against
a tool that may rewrite its own config, rather than an in-memory state change.
If that proves annoying in use, the fix is an upstream IronWire capability —
a per-tool pass-through in the control API — and this design should be revisited
then rather than worked around here.

## The flow, and why the decomposition changes

The mechanism above is sound. The *shape* it was going to be dropped into is
not, and this section supersedes a naive reading of "add a list to the
destination".

### The defect in today's shape

Today the contributor is asked one question: "answer model calls on this
computer?" That question is about a **listener**, and nobody wants a listener.
Turning it on and connecting no tools achieves nothing observable -- which is
precisely the "I turned it on and nothing happened" failure that had this
feature sitting on a buried settings toggle. Adding a tool list beside the
switch fixes discoverability while leaving that empty middle intact.

### The unit of decision is a tool

"Claude Code -- send its calls here" is a sentence with a meaning a contributor
can act on. "Answer model calls on this computer" is an implementation detail
they are currently asked to reason about first, before anything can happen.

So the destination leads with the harness list, and connecting a harness is the
primary action. The listener starts because a tool needed it, not as a
precondition the contributor must discover.

### The exposure decision stays separate, but moves

One consequence genuinely does not follow from "connect Claude Code": the
listener is open to **everything** on this machine, not only the tool just
connected. That is `OFFER_EXPOSURE`, and it is the reason the switch exists at
all.

It must therefore still be asked -- but as a **gate on the first connect**,
where it is finally about something concrete, rather than as a standalone
switch flipped into a void.

**Amended after implementation: it IS asked again, deliberately.** This section
originally said the question is not asked once answered, deferring to
`offer_asked_once`. What shipped is `connect_needs_exposure(listener_on)`, which
is wider than `should_offer` on purpose: a contributor who answered once and then
used the kill switch is making the exposure decision afresh when a connect would
reopen the listener, and silently reopening it would be the fail-open this
surface exists to prevent. The first-run offer is asked once
(`offer_asked_once` already covers this).

The master switch does not disappear. It becomes what it actually is: a kill
switch -- **stop answering everything** -- which is a real thing to want, is the
safe direction, and is exactly what the tray's off action already does.

### Configured is not working

`Tool.wired` proves a config file has the right value in it. It does not prove
a single call was ever answered. A contributor who connects a harness, restarts
it, and still sees no evidence is back in the original failure.

So each harness reports three states, not two:

| state | meaning |
|---|---|
| not connected | its config does not send calls here |
| connected, nothing seen | config is right; no call has arrived yet |
| answering | a call actually arrived, with when |

Only the third means it works, and it is the one the surface should make
obvious.

**Attribution is approximate, and must be described as such.** An earlier draft
said the family comes from the ledger's `path` field. **That was wrong** -- two
independent readings of the pinned revision found `path` is the path beneath the
facade, and the protocol family is the `facade` column, literally `"anthropic"`
or `"openai"`. Our `RoutedExchange` already carries it, so no parsing is needed.

The ambiguity is also narrower than that draft claimed. A catalog tool's family
IS knowable, from `AgentSetting.facade`. So the only unattributable case is two
CONNECTED tools sharing one family -- never a tool of unknown family. That case
gets its own state and is NOT painted as working, because "one of these two
answered" is not the same claim as "this one is answering".

The surface must not claim per-tool activity it cannot support: attribute where
the family is unambiguous, and otherwise report that a call arrived without
naming a tool.

### The states that are not the happy path

These carry most of the real experience and need real copy, not a fallback:

- **Nothing detected.** Say what was looked for, not "no tools". An empty list
  that explains nothing is indistinguishable from a broken one.
- **Installed, slot already taken.** `Planned.occupied` means the contributor
  already sends that tool's calls somewhere. Show the value, say it was left
  alone, and do not offer to take it over.
- **Needs restarting.** The config changed under a running process. Say so, and
  keep saying it until a call arrives.
- **Config unparseable.** Refused deliberately. Distinguish it from "nothing to
  change", and name the file.

### The resulting first run

1. Destination lists the harnesses found on this machine, each with its state.
2. Contributor turns on the one they care about.
3. First connect only: the exposure sentence, and an explicit accept.
4. Preview of the exact file change; confirm.
5. "Restart Claude Code" until a call is seen.
6. State becomes *answering*, with when -- the proof that was missing.

Nothing in that sequence asks about a listener.

## Scope

**In:** the harness list as the destination's primary surface; connect and
disconnect one at a time with a preview of the exact file change; the exposure
question moved to a first-connect gate; the master switch reduced to a kill
switch; the three-valued per-harness state including whether a call has actually
been answered; and the non-happy-path states above.

**Out:** any change to the master switch's behaviour; the global hotkey (still
cut 2); routing decisions per model or per project; anything that writes more
than one tool's config in a single action.

## Design

### 1. The list

The destination gains a section listing every harness `tools::all` reports, each
showing: its name, whether it is installed, whether it currently sends calls
here, and the config file that would be edited. The file path is shown always,
not on demand — a tool nobody expected to be configured is a question about
*which file*, every time.

A harness that is not installed is listed and disabled rather than hidden.
Hiding it makes the absence of a tool indistinguishable from the app not knowing
about it.

When no catalog is present, only the two built-in tools appear. The surface must
say that the list is what this machine knows about, not a claim about every tool
that exists.

### 2. Connect and disconnect are previewed, never immediate

A toggle calls `plan_connect` / `plan_disconnect` and shows the resulting
`Planned` before anything is written:

- `changes` — what would change, in IronWire's own words.
- `occupied` — slots left alone because the contributor is already using them.
  This is the case where the honest answer is "we did not take that over", and
  it must be shown, not swallowed.
- `is_noop()` — nothing to do; say so rather than showing an empty confirmation.

Only on confirmation does `commit` run. **We are editing a file we do not own;
the contributor sees the diff first.** This mirrors the reason the master switch
became a destination: the consequence is stated where the decision is made.

### 3. Restarting

A harness holding an old setting in a running process is shown as needing a
restart, so the list never claims a tool is sending calls here when the process
in front of the contributor is not. This is the same rule as the tone: the
surface reports what is true, not what was asked for.

### 4. Wording

Every string comes from `private_inference_copy.rs`, and the banned-word sweep
applies with full force. It rejects **`route`**, `proxy`, `backend`, `endpoint`,
`localhost`, `private`, `secure`, `encrypt`, `anonym`, `protect`, `credit`,
`earn`, and vendor names.

So this surface may not say "routes through the proxy", "points at the backend",
or "the local endpoint". It says what happens instead: a tool **sends its calls
here**, and this computer **answers** them. Tool names themselves ("Claude
Code", "Codex") come from `Tool.name`, which is IronWire's, not ours to
restate.

`connect_command` is a command string, not prose, and is shown verbatim as a
fallback for a contributor who would rather do it themselves.

## Testing

- A harness whose slot is occupied is reported as occupied and **not**
  overwritten — the rule most likely to be broken by a well-meaning
  simplification.
- An unparseable config is refused, and the refusal is distinguishable from
  "nothing to change".
- `is_noop()` produces a distinct message rather than an empty preview.
- Disconnect removes only what IronWire wrote, leaving neighbouring keys intact.
- A tool that is not installed cannot be connected.
- The list degrades to the two built-in tools with no catalog, and says so.
- No shell authors a harness string; every sentence resolves to the copy module.
- The banned words, including `route`, appear nowhere in the new copy.

## Risks

- **Editing files we do not own.** The main hazard, and the reason preview
  precedes commit. IronWire's three rules do the heavy lifting; this design
  must not route around them for convenience.
- **A tool rewriting its own config underneath us.** `wired` is read each time
  the list is shown rather than cached, so the surface corrects itself.
- **Catalog absence read as tool absence.** Mitigated by saying what the list
  is.
- **Per-flip file I/O.** Accepted, with the upstream fix named above.

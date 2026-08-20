# Client end-to-end verification — design

Status: design, not yet implemented.
Scope: sub-project D. Operator tooling and a runbook. No client or server
behaviour changes.

## Why this exists

Sub-projects A, B and C get each contributor app from "double-click" to "the
daemon is running". None of them establishes that contributing works.

The distinction matters more than it sounds, because of how far the macOS
client fell short. `AppModel.start()`
(`macos/Sources/TraceCommonsApp/AppModel.swift:112`) resolves a state
directory, and on failure sets `startup = .refused` and returns without
constructing a daemon or a client. Every one of the 32 daemon methods
(`crates/trace-commons-contributor/src/daemon/ipc.rs:193-226`) is behind that
construction. So on the shipped macOS build, `enroll`, `preview`, `approve`,
`set_consent_scopes`, `withdraw` and the rest have never executed — not once,
in any contributor's hands. There is no field evidence about any of them,
because there was no field.

Linux and Windows do start, being fail-open, so their daemon paths at least
run. But nobody has driven either as an *installed artifact* — the flatpak,
the MSIX — from a clean machine through to an accepted submission. The
end-to-end path this project has actually proven is the CLI's.

A green CI and a notarized DMG were both true on 2026-08-18 while the macOS
app could not start. That is the precise failure this slice exists to make
impossible: verification that runs against the thing a contributor installs,
not against the code that went into it.

## What this is not

It is not the fixes. A, B and C are the fixes; D is the check that says
whether they worked, and it will surface defects that belong to new slices
rather than to this one.

It is not a replacement for the unit and integration suites. Those assert
that components behave; this asserts that a human with a downloaded artifact
can contribute a trace.

It is not a load test, a soak test, or a performance measurement.

## The path under test

One pass, per platform, in order. Each step names the daemon method or surface
it exercises so a failure lands somewhere specific.

1. **Install** the real artifact: `brew install --cask trace-commons` or the
   notarized DMG; on Linux, the published flatpak, installed exactly as the
   release notes tell a contributor to
   (`.github/workflows/release-apps.yml:1083`):

   ```
   flatpak install --from \
     https://storage.googleapis.com/tracecommons-flatpak/ai.tracecommons.Contributor.flatpakref
   ```

   on Windows, the MSIX or the self-contained zip.

   The Linux install is not a convenience wrapper around a local build: the
   flatpakref points at the OSTree repo under
   `https://storage.googleapis.com/tracecommons-flatpak/repo`, published by
   `.github/workflows/release-apps.yml:987` calling
   `scripts/flatpak/publish-repo.sh` against the `tracecommons-flatpak`
   bucket (`:986`). Note the repo lives under `/repo`, not at the bucket root.

   **GPG verification is part of what this step proves.** The flatpakref
   carries an inline signing key and `publish-repo.sh:47` refuses to publish
   a repo with no signed summary, so a successful `flatpak install --from`
   exercises the signature chain rather than merely fetching bytes. A pass
   records that the install completed without a signature override; an
   install that needed `--no-gpg-verify` is a failure, not a workaround.
2. **First launch**, from the platform's normal launcher — Finder, the
   application grid, the Start menu. Never from a shell. See "Why the launcher
   matters" below.
3. **Declare roots** through the new roots step that
   `docs/superpowers/specs/2026-08-19-fail-closed-roots-parity-design.md`
   adds. Confirms the fail-closed refusal is escapable.
4. **Enroll** with a verification invite — `enroll`, and the deep link if the
   platform registers one.
5. **Consent scopes** — `consent_options`, `set_consent_scopes`.
6. **Discover projects** — `list_projects`, `set_project_mode`.
7. **Watch a session**: generate a synthetic session in the declared root and
   confirm it appears — `list_pending`, `status`.
8. **Preview** it — `preview`, `preview_body`, `preview_turns`.
9. **Redaction and privacy scan**: confirm the seeded synthetic secrets are
   found and replaced, and that the residual risk label is what the fixture
   was built to produce.
10. **Consent and submit** — `approve`.
11. **Server outcome**: accepted, or quarantined with a stated reason.
12. **Read back** — `list_history`, `history_rollup`, and the client's credit
    surface.
13. **Withdraw** — `withdraw`. This doubles as the cleanup step; see below.
14. **Update** — confirm that whichever update channel *this install method
    actually uses* offers the current version. See "Update channels are part
    of the path".

Steps 3, 13 and 14 are new to this list relative to how anyone has previously
described "the flow", and all three are deliberate: 3 is the thing A adds, 13
is the only honest way to run 1-12 repeatedly against a real server, and 14
covers a failure that is live right now.

## Three problems that make this harder than running the app

### 1. Identities cannot simply be reset

`/v1/onboard` is not idempotent; an invite code is redeemable exactly once.
`docs/release-runbook.md:45-56` records why the Homebrew cask deliberately
does not zap `~/Library/Application Support/trace-commons/contributor.json`:
deleting it strands a contributor whose invite is already spent and cannot be
reissued.

So the obvious reset — delete the state directory, run again — burns an invite
per run and, done on the wrong machine, destroys a real identity. Verification
that is unsafe to repeat will not be repeated.

**The design.** A dedicated verification invite, minted per campaign through
the admin API and revoked when the campaign closes:

```
POST /v1/admin/invites
{"tenant_mode":"fixed","fixed_tenant_id":"<verification tenant>","max_uses":25}
```

`tenant_mode: "fixed"` with `fixed_tenant_id` is one of exactly two accepted
shapes (`crates/trace-commons-server/src/trace_invite_admin.rs:204-217`); a
`max_uses` of zero is rejected (`:219-224`). Minting through the admin API
rather than the `--mint-invites` CLI path matters here: the handler
invalidates the in-process cache in the same request, so the code is
redeemable immediately, whereas a CLI mint waits up to one allowlist refresh
interval (`docs/operator/pilot-allowlist.md:103-112`). A verification run that
begins by failing for sixty seconds trains the operator to ignore failures.

`max_uses: 25` is sized for three platforms, several passes, and retries,
while staying small enough that a leaked code is a contained incident. Close
the campaign by revoking it — `POST /v1/admin/invites/$INVITE_HASH/revoke`,
which drops the cache entry in the same request
(`docs/operator/pilot-allowlist.md:121-129`).

**Not the hackathon invites.** Reusing the shared `max_uses: 2000` event
invites was considered and rejected. Verification traffic would be
indistinguishable from participant traffic in the same tenant, and consuming
uses from a live event invite makes a verification run a potential outage for
real participants. Their tenants also carry event semantics that verification
data would pollute.

**Clean machine state, without deleting anything.** Each pass runs against a
state directory that has never been enrolled — but on macOS after A lands, the
state directory resolves by default to
`~/Library/Application Support/trace-commons`, and the one override that could
redirect it is `TRACE_COMMONS_CONTRIBUTOR_DIR`, which a Finder launch cannot
set and which the runbook forbids anyway.

The resolution is that "clean" means a clean *user account*, not a clean
directory: a throwaway local account on macOS, a fresh VM or container for the
flatpak, a VM snapshot for Windows. This is the single largest operational
cost in this slice and it should be stated plainly rather than discovered.
The Windows named-pipe ACL job already establishes the precedent that a second
local account is an acceptable price for a check nothing else can make
(`.github/workflows/ci.yml:226`, the `windows named-pipe ACL` job, which
creates a second non-administrator account precisely because the property
under test is not observable any other way).

**Operational cost, stated.** Per campaign: one invite minted and revoked;
up to 25 invite uses; one throwaway macOS account; one Linux VM or container;
one Windows VM snapshot restored per pass. Per release: one campaign.

### 2. Verification traces are real traces on a real server

They are uploaded, scored, and stored. Left alone they become indistinguishable
from contributor data.

**The design.** Every verification identity enrolls into one fixed
verification tenant, which is what `tenant_mode: "fixed"` buys. Verification
submissions are therefore a queryable, purgeable set that never mixes with a
pilot or event tenant, and no per-trace marking convention has to be invented
or remembered.

**Cleanup is step 13, not an afterthought.** Each submitted trace is withdrawn
through the client's own `withdraw` method, backed by
`/v1/account/traces/{submission_id}/withdraw`
(`crates/trace-commons-server/src/bin/trace-commons-ingest.rs:6898`), with
`/v1/account/traces` (`:6888`) enumerating what remains. Running cleanup
through the contributor-facing path rather than an admin route is the point:
it exercises the withdrawal promise as part of verification instead of
verifying around it.

**The quarantine trap.** A verification trace assessed HIGH residual risk
becomes `quarantined` with credit held at 0.0, pending a human
(`docs/operator/quarantine-review.md:7-9`). That queue sat at 48 with zero
reviews for 71 days
(`docs/operator/quarantine-review.md:3-4`), and a verification design that
silently adds to it would be making the exact problem it is meant to catch.

So the runbook's closing step is mandatory and its result is recorded whether
or not anything was found: enumerate the verification tenant's quarantined
submissions, withdraw them, and write the count into the pass record. A
campaign with unresolved quarantined rows does not pass.

**The traces are synthetic.** Fixtures generated into a scratch project inside
the declared root, never the operator's real work — which is also what makes
step 9 meaningful, since the fixtures seed known synthetic secrets for the
redaction pass to find. A fixture whose expected redaction counts were authored
alongside the redactor proves less than it appears to; the fixtures should
carry secrets in shapes taken from the detector's own corpus, and the runbook
should say which ones and why.

### 3. Evidence hygiene, with a GUI in frame

This repo's convention is hash-only, label-only: no raw URLs, tokens, ARNs,
contributor identity, trace bodies, or operator-secret material in stored rows,
logs, or committed artifacts (`docs/operator/hash-only-logging.md`).

A GUI verification pass strains this, because the surfaces under test are the
ones that legitimately display exactly that material: the connect screen shows
an invite code, the projects screen shows filesystem paths, the preview sheet
shows trace content by design.

**The rule.** Evidence is a per-step pass/fail record with hashes and labels.
It is not a transcript and not a screenshot album.

Screenshots are permitted only for chrome-level states that contain no trace
content, no path, no code and no identity — the roots-declaration screen in its
empty state, the refusal notice, the Dock icon and menu-bar item, the done
screen. For the preview, consent and connect steps, the evidence is a written
assertion of what was observed, not an image.

Every screenshot passes a documented pre-commit check before it lands:
inspected at full size by the operator against the list above, with the
verification identity and synthetic fixtures meaning that even a mistake
exposes no contributor's data. The check is written into the runbook as a step
with a checkbox, because "be careful with screenshots" is not a control.

Note that the existing committed screenshots under `docs/images/` are demo
captures, not verification evidence, and this rule does not retroactively
govern them.

## Why the launcher matters

On macOS, launching from a shell inherits the operator's environment, including
`TRACE_COMMONS_CONTRIBUTOR_DIR` — the exact variable whose absence is the
defect A fixes. A shell launch would therefore pass while the shipped app
fails, which is how this went unnoticed in the first place: `macos/scripts/run-demo.sh`
exports it.

Every macOS pass launches from Finder. The runbook states this as a hard step
with its own checkbox rather than as advice, and states the failure it is
guarding against, so nobody optimizes it away.

The same reasoning applies more weakly to Linux and Windows: launch from the
desktop environment's own launcher, not a terminal.

## Update channels are part of the path

An installed app that cannot reach the next version is a defect of the same
class as one that cannot start, and it is invisible to every check that only
looks at a fresh install. It is also not hypothetical: **both macOS update
channels are currently dead for a Homebrew-installed contributor.**

The Sparkle appcast is healthy. It is current at 0.3.0, with an enclosure
pointing at the `app-v0.3.0` DMG and an `edSignature`. But
`UpdateController.start()` gates on `mode.startsUpdater` and nothing else
(`macos/Sources/TraceCommonsApp/UpdateController.swift:49`), and
`UpdatePolicy.mode` checks Homebrew first and unconditionally, returning
`.managedByHomebrew` (`macos/Sources/TCUpdates/UpdatePolicy.swift:34-37`).
`startsUpdater` is true only for `.selfUpdating`
(`macos/Sources/TCUpdates/UpdatePolicy.swift:18-21`). So Sparkle never runs
under Homebrew.

That is correct design, and the reasoning above it is right: two managers must
never both believe they own the same file, and the mode carries the `brew`
command that does work. The problem is that the command it hands the user
resolves to a cask that the local tap CHECKOUT reported as 0.1.0. Corrected
2026-08-20: the tap itself was current -- `origin/main` carried 0.3.0 and the
release job's cask-bump step had opened and merged a PR for every release. The
clone on the investigating machine was parked on an old feature branch, and
Homebrew serves whatever branch the tap checkout has out. A stale local clone
looks exactly like stale automation from the outside, which is worth knowing
before concluding a pipeline is broken. Each channel is individually
defensible; the
combination leaves a Homebrew contributor stranded two versions back with the
app telling them, accurately, to run a command that will not move them.

Either channel working alone would have carried them forward. Verification
that checked "is there an update mechanism" would have found two. Only a check
that asks *which channel this install method actually uses* and whether that
channel offers the current version catches this.

Step 14 therefore names the channel per install method, because they differ:

| Install method | Channel under test | What the pass checks |
|---|---|---|
| macOS, Homebrew cask | `brew upgrade --cask trace-commons` | The tap's cask version equals the current release, and the upgrade lands it. Sparkle is expected NOT to run; confirm that too. |
| macOS, direct DMG | Sparkle appcast | The updater constructs, checks, and offers the current version. |
| Linux, flatpak | `flatpak update` against the published repo | The update resolves and verifies its GPG signature. |
| Windows, appinstaller / MSIX | The `.appinstaller` feed | It advertises the current version. |

A campaign passes only when every row that applies to the platforms tested
offers the current version. A stale channel is a release blocker, not a note.

## What can be automated, and what cannot

Honestly, and per platform, because a harness that claims uniform coverage
would be worse than none.

**The daemon path can be automated on all three.** The daemon is Rust, shared,
and driveable over IPC without a GUI. A harness that enrolls, declares roots,
generates a session, previews, approves and withdraws is portable and belongs
in CI.

**The client path can be automated on none of them** without a UI-automation
investment this slice does not propose. What is under test in D is
specifically the part the daemon harness cannot see: whether the installed
bundle launches, whether the roots screen appears, whether the refusal is
escapable, whether the menu-bar item is visible.

**macOS is the hardest case and should not be papered over.**
`.github/workflows/ci.yml` contains nine `ubuntu-latest` jobs and four
`windows-latest` jobs, and zero macOS jobs. The Swift compiles only in
`.github/workflows/release-apps.yml` (`:121` and `:1152`, both `macos-26`), on
a tag. So there is no CI surface on which a macOS client check could run
today, and even if one were added, a hosted runner cannot perform the Finder
launch that makes the check meaningful. The macOS pass is manual, and the
runbook says so.

**Linux is the most automatable.** A flatpak install and launch is scriptable
on `ubuntu-latest`, and the GTK app links the contributor crate in process,
so a headless harness can cover more of the path than on the other two.

**Windows sits in between.** MSIX install and launch are scriptable, and four
`windows-latest` jobs already exist to extend.

The recommendation is to build the portable daemon harness first, wire it into
CI on Linux and Windows, and keep the three client passes manual — with the
runbook as the artifact that makes a manual pass repeatable and auditable
rather than improvised.

## Sequencing

**Before A lands**, D can: author the runbook; provision the verification
tenant and invite; build the synthetic session fixtures; build the portable
daemon harness; and run the Linux and Windows client passes, which start
today because those clients are fail-open. Those two passes are worth running
early precisely because nobody has run them — they will find defects that are
cheaper to fix before A changes the startup path underneath them.

**After A lands**, all three passes run against the new startup path,
including the first macOS pass in the product's history.

**After B and C**, the macOS pass re-runs to cover the Dock icon, the menu-bar
item's visibility, and the `tracecommons://` deep link that C registers.

D never blocks A, B or C. It gates the release that contains them.

## The runbook

Location: `docs/operator/client-end-to-end-verification.md`, indexed from
`docs/operator/README.md` alongside the existing runbooks.

Shape follows `docs/operator/pilot-bootstrap-first-100-traces.md`: a stated
goal, a pre-flight list, the run itself, and a verification checklist of
independent checks where the run is acceptable only if all pass. It is the
GUI-client counterpart to `docs/operator/pilot-contributor-onboarding.md`,
which covers the CLI path.

Contents, per platform:

- Pre-flight: artifact provenance and checksum; verification invite minted and
  its hash recorded; clean user account or VM snapshot ready; fixtures staged.
- The fourteen steps above, each with its expected observation and its
  pass/fail box.
- The evidence rules, including the screenshot check.
- Cleanup: withdraw every submission; enumerate and resolve quarantined rows;
  record both counts; revoke the invite.
- The pass record: a dated table of platform, artifact version, step outcomes,
  defects found, the two cleanup counts, and the update-channel row from
  step 14 naming which channel was exercised and the version it offered.

**Pass/fail gate.** A campaign passes only when all three platforms complete
every step, cleanup is confirmed with counts, and every defect found is filed.
A partial campaign is recorded as a partial campaign; there is no "mostly
passed".

## Wiring it into releases

The intent is that no `app-v*` tag is cut without a completed pass record
against that candidate build.

A caution about how much a prose gate is worth here.
`docs/release-runbook.md:58-72` still says no release has been published and
that the cask carries placeholder checksums — while `app-v0.2.0`, `app-v0.2.1`
and `app-v0.3.0` have all shipped, and the tap serves a real checksum for
0.1.0 -- which was a stale local tap checkout, not stale automation; see the
correction above. The runbook's own text about placeholder checksums IS stale,
three releases after the first shipped. That
same unbumped cask is one half of the dead-update-channel problem in "Update
channels are part of the path" — the failure is not only that a document went
stale, it is that a shipped install method silently stopped carrying users
forward. A release gate that lives only in a document is a gate that goes
stale exactly when it is needed.

So the gate should have a mechanical component even though the pass itself is
manual: the release workflow requires a pass-record file for the version being
tagged, and fails the tag if one is absent. That check is cheap, it runs on
`ubuntu-latest`, and it cannot silently rot the way a paragraph can. It does
not verify the pass was honest — nothing can — but it makes skipping it a
deliberate act.

Updating the stale sections of `docs/release-runbook.md` is out of scope here
and should be its own small slice.

## Rejected alternatives

**Delete the state directory to reset.** Burns an invite per run and risks a
real identity. Replaced by clean user accounts and a bounded verification
invite.

**Reuse the hackathon `max_uses: 2000` invites.** Mixes verification traffic
with live event traffic in the same tenant and risks consuming uses real
participants need.

**Admin-side purge of verification traces.** Rejected in favour of the
contributor `withdraw` path, which cleans up and exercises a shipped promise in
the same action.

**A uniform cross-platform UI-automation harness.** Rejected as out of
proportion to this slice. It is a reasonable follow-up once the manual passes
have shown which steps actually break.

**Verifying against a staging server instead of the pilot.** Tempting, and
worth revisiting, but a staging deployment would not exercise the pilot's real
scorer, allowlist, and invite registry — which is where several of this
product's recent defects have lived. The fixed verification tenant is the
isolation mechanism instead.

## Open questions

- The verification tenant's name and whether it should be excluded from
  community-site aggregates and credit reporting. It should, but the mechanism
  needs checking against how tenants are filtered there.
- Whether the portable daemon harness belongs in this repo's test tree or in
  `docs/operator` tooling. It is closer to the operator binaries than to the
  unit suites.

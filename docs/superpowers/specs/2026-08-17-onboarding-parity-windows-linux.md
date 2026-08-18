# Onboarding parity for the Windows and Linux apps

Status: design, not yet implemented.

## The problem

The Windows and Linux desktop apps cannot enrol a contributor. There is no
invite screen in either, so a contributor who installs only the app reaches a
dead end: the GTK app detects the unenrolled state and says so — its
`UNENROLLED_PREVIEW` copy reads "This device isn't connected yet, so this was
built without your identity and nothing here can be contributed" — and then
offers no way to connect it. The Windows app has no invite handling at all;
the sole `invite` match under `windows/src` is a comment about legacy code
pages.

Only the macOS app can take someone from download to enrolled. Until this is
fixed, the install page has to tell Windows and Linux users to install the CLI
as well and run `login` there once, which it now does.

## What this is not

It is not protocol work, daemon work, or server work. Every method the macOS
onboarding calls is already in the pinned `METHODS` array in
`crates/trace-commons-contributor/src/daemon/ipc.rs`:

    enroll                     consent_options
    set_consent_scopes         list_projects
    set_project_mode           acknowledge_near_ai_notice

All 31 methods are reachable from both apps today. The GTK app has a generic
`Backend::call(method, params)`; the Windows app has `TcDaemon.Call(method,
paramsJson)` over the `tc_call` FFI. Both land in `ipc::handle_local`, which
routes through `block_on_ipc` to `handle_request_async` — so the async-only
methods work from both. The bare `"enroll" => Response::err(ERR_UNAVAILABLE,
"enroll-requires-async")` arm is the non-async dispatch fallback and is not on
either app's path; `handle_local_and_handle_request_async_answer_an_async_method_identically`
pins that.

This slice is therefore entirely UI plus per-platform URL registration.

## Flow to mirror

The screens are already specified for **every** shell, with copy, in
`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
"## Onboarding" — six screens, one decision each. That document is the source
of copy; this one is only the port's engineering notes. Do not paraphrase it
and do not re-derive the wording from the Swift.

macOS is the reference implementation. `OnboardingCoordinatorView` sequences:

    welcome -> connect -> consent -> privacyScan* -> projects -> done

`privacyScan` is conditional: it is shown only where the operator has
configured the second scanner, which the shell learns from `get_settings`.

Two things about that screen matter for this slice specifically. It is **not**
a macOS concern that the ports can skip: `acknowledge_near_ai_notice` is the
only way an app-only contributor clears the notice, because they never see the
CLI's stdout version. A Windows or Linux contributor who never sees this screen
can never acknowledge it. And its disclosure has two halves — that message text
really does leave the machine to a third party, and that nothing is sent at all
if that scanner is unreachable. Cutting either half makes the screen dishonest
in one direction.

From "### 3. Consent scopes": the scope list and its descriptions come from
`consent_options` and are **never hardcoded per shell**. A port that inlines
the four scopes to save a round trip has broken the contract that lets the
operator change them.

## Contract invariants the ports must not break

These are properties of the daemon contract, not macOS styling. A port that
"improves" on any of them regresses the design.

1. **One failure sentence for the whole invite path.** `enroll` never echoes
   the underlying HTTP condition back — it answers `enroll-failed` (see
   "### `enroll`" in `docs/contributor-daemon-ipc-v1_1.md`). The macOS view
   therefore shows a single fixed sentence, "This invite link is no longer
   valid. Ask whoever sent it for a new one.", regardless of what came back,
   and covers both an unparseable invite and one the daemon refused. Surfacing
   a raw code would leak exactly what the contract withholds.

2. **Absent `scopes` means floor scope only**, not "all scopes". Present but
   malformed is an error (`scopes-invalid`). See `parse_scope_names`.

3. **`grant` and `invite` are mutually exclusive** — sending both is
   `grant-and-invite-mutually-exclusive`.

4. **The invite never reaches a log, an audit row, or an error string.** It is
   a credential; see the hash-only rule in CLAUDE.md.

5. **`logged_in` does not mean "onboarded", and must not gate the flow.**
   `enroll` succeeds on screen 2 and flips `status.logged_in` to true there,
   before consent is chosen on screen 3. A port that resumes on `logged_in`
   drops a contributor who quit mid-flow straight into the main window with
   whatever `enroll`'s floor-only default left in place — silently narrower
   consent than they were about to choose, and no prompt to finish. macOS
   instead persists a completion flag keyed by `status.tenant_id`
   (`isOnboardingComplete` / `markOnboardingComplete`) and resumes onboarding
   until the Done screen is reached. Both ports need their own equivalent:
   `GSettings`/state file on Linux, and the app's local settings store on
   Windows — keyed by tenant, never a single global boolean, or re-enrolling
   into a different tenant inherits the old tenant's "done".

## Deep links

Format, from `DeepLink.inviteURL`:

    tracecommons://enroll?invite=<the real issuer URL>

Scheme and host are compared lower-cased. The real invite is an issuer URL
(`https://issuer.example/onboard#CODE`, per `parse_invite` in
`crates/trace-commons-contributor/src/commands.rs`) folded into the `invite`
query parameter, because an issuer link cannot itself open a desktop app.

Both ports must parse this identically, including the case-insensitivity.

### The argv exposure, which macOS does not have

macOS receives these as URL events. A Linux `.desktop` handler
(`MimeType=x-scheme-handler/tracecommons`) and a Windows registry handler
(`HKCU\Software\Classes\tracecommons\shell\open\command`) both receive the URL
as a **command-line argument** instead. Process command lines are readable by
other processes on both platforms — `/proc/<pid>/cmdline` on Linux, and the
Win32 process APIs on Windows.

This matters more than it first appears because **invites are not single-use**:
`max_uses` is a `u32` on the invite registry entry, and the live hackathon
invites are issued with `max_uses: 2000`. An invite captured out of a process
listing is a reusable credential, not a spent token.

This exposure is inherent to argv-based scheme handlers and cannot be fully
eliminated. It can be bounded:

- Hand the URL to the already-running app over the existing IPC channel — the
  ACL'd named pipe on Windows, the Unix socket on Linux — and exit immediately,
  so the argv-bearing process is short-lived.
- Never re-expose the invite after receipt: not in logs, window titles,
  crash reports, or the queue.
- Document the tradeoff where contributors will see it, and keep paste as the
  path we recommend for an invite with a large `max_uses`.

Decide explicitly whether the deep-link handler ships enabled by default. The
paste field carries no such exposure and is sufficient on its own.

## Work

Per platform, mirroring the macOS view set:

- Linux/GTK — connect, consent, projects and welcome/done screens; entry from
  an unenrolled banner (`ui/mod.rs` already has a banner above the
  `adw::ViewStack`); `.desktop` scheme registration including the flatpak
  manifest.
- Windows/WinUI — the same screens in XAML; registry scheme registration, and
  it must work for the unpackaged, self-contained build we actually ship.

Both call the six methods above and nothing new.

## Verification

- The GTK app builds and runs in the Docker image used for the Linux client.
- The WinUI app builds; scheme registration is verified on a real Windows
  host, since it is not observable from a cross-compile — the same reasoning
  that makes the named-pipe ACL job the only `windows-latest` CI job.
- An enrolment against the pilot issuer from each platform, end to end.
- Neither app writes the invite anywhere. Grep the new code for the invite
  variable reaching a log or a display string other than the field itself.

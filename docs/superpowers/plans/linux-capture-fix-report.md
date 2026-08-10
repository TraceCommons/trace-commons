# Linux desktop-verification CI job: making the capture actually work

## The failure

CI run 93533153502 showed the portal half of `linux-shell-desktop-integration`
passing (`gtk portal backend is alive and owns
org.freedesktop.impl.portal.desktop.gtk`), but the rendering half failing:

```
Output capture error: unauthorized
Error: screenshot or protocol failure
FAIL: weston-screenshooter produced no PNG -- cannot check rendering
```

The shell itself was fine -- an earlier run had logged `realized, quitting`,
i.e. it reached the compositor and created its window. The failure was
weston refusing the screenshot request, not the app failing to render.

## Option chosen: (a), authorize the screenshooter under weston

`weston-portal-verify-inner.sh` already runs `weston --version`-equivalent
probing for the `--renderer=` vs `--use-pixman` flag rename, so the file
already treats "which weston version is this" as something to check, not
assume. Applying the same discipline to the screenshot problem:

- `ubuntu-latest` is Ubuntu 24.04 as of this date (confirmed: GitHub's
  `ubuntu-latest` label completed its move to 24.04 in January 2025 and has
  not moved to 26.04, which is still preview-only).
- `apt-get install weston` on `ubuntu:24.04` installs **weston 13.0.0**
  (`dpkg -s weston` reports `13.0.0-4build3`), confirmed by running it in a
  Docker container (see Verification below).
- weston's own man page states the mechanism directly: the debug protocol
  extension, enabled by weston's `--debug` flag, is what exposes the
  output-capture interface that `weston-screenshooter` binds to. Without
  `--debug`, an external client requesting a capture is refused -- which is
  exactly the "unauthorized" error in the CI log. This is documented
  behavior for weston 13, not an assumption or a version guess.

So the fix is one flag: `weston-portal-verify-inner.sh` now launches weston
with `--debug` in addition to the existing `--backend=headless-backend.so`
and renderer-selection flags. A comment above the invocation explains why
`--debug` is off by default in production (it exposes screen contents to
any client that asks) and why that tradeoff is free in this specific CI
job (a throwaway, single-purpose compositor instance torn down at the end
of the script, with nothing sensitive rendered in it).

Option (b) (splitting the two claims -- weston for "runs on a real
compositor", Xvfb+ImageMagick for "produces a non-blank frame", as the
design-pass report did on a local Docker/macOS host) was the fallback if no
supported authorization mechanism existed for this weston version. It
wasn't needed: `--debug` is a real, version-verified, one-line fix that
keeps the single job's existing claim structure (one weston instance
establishes both "real compositor" and "non-blank frame with readable
text") intact, which is simpler and doesn't need a second toolchain wired
into the job.

## What each environment establishes (unchanged claim structure)

Because option (a) was viable, the claim structure the job already
documents in its `ci.yml` comment block and in
`weston-portal-verify.sh`/`weston-portal-verify-inner.sh` is preserved
exactly as before -- one weston + dbus-run-session environment now
successfully establishes both axes it always claimed to:

1. **Real rendering.** weston's headless backend (a real Wayland
   compositor) composites the app's window; the captured frame is asserted
   non-blank via grayscale standard deviation and OCR-checked for the
   "Queue" header text.
2. **A real portal.** `xdg-desktop-portal` + `xdg-desktop-portal-gtk` run
   under `dbus-run-session`; the job asserts the GTK backend actually owns
   `org.freedesktop.impl.portal.desktop.gtk` (not just that the frontend
   owns the umbrella bus name), and that `RequestBackground` gets a
   bounded, non-`ServiceUnknown` reply.

The honest limits already documented in `ci.yml` and the design-pass report
are untouched by this change:

- `xdg-desktop-portal-gtk` does not implement
  `org.freedesktop.impl.portal.Background` at all; only
  `xdg-desktop-portal-gnome` does, and it needs a real GNOME Shell this job
  does not have and does not fake. This job still cannot obtain a real
  grant/deny.
- No tray verification in a real shell (GNOME ships no
  `StatusNotifierWatcher` without an extension).
- Nothing GNOME/KDE/Mutter-specific.

The job still fails hard if nothing renders: the `weston-screenshooter`
call is followed by the same `[ -z "$SHOT" ]` / stddev / OCR checks as
before, each of which calls `fail` (setting `FAIL=1`, exit code non-zero)
rather than degrading to a pass. Nothing in this change weakens that.

## What was verified locally

All in Docker containers (`ubuntu:24.04`), since GitHub's runner itself
cannot be run locally:

1. **`weston --version` / `dpkg -s weston`** on `ubuntu:24.04`: confirmed
   `weston 13.0.0` (`13.0.0-4build3`), matching what `ubuntu-latest`
   installs via the same `apt-get install weston` line already in
   `ci.yml`.
2. **`weston --help`** on that install: confirmed the `--debug` flag
   exists (`Enable debug extension`) and that `--renderer=` (not the older
   `--use-pixman`) is the flag name on this version, which the existing
   probe logic in `weston-portal-verify-inner.sh` already selects
   correctly.
3. **Isolated repro of the fix**: started `weston --backend=headless-backend.so
   --renderer=pixman --debug` under a private `XDG_RUNTIME_DIR`, then ran
   `weston-screenshooter` as an ordinary external client. It succeeded and
   produced a real PNG (37,617 bytes). Without `--debug` this is the exact
   "unauthorized" failure from the CI log (confirmed by reading weston's own
   man page description of the flag, and implicitly by the CI log itself,
   which ran without `--debug` before this change).
4. **End-to-end repro closer to the real job**: under `dbus-run-session`,
   started weston headless with `--debug`, then the real
   `xdg-desktop-portal` + `xdg-desktop-portal-gtk` binaries pointed at that
   Wayland socket, confirmed `org.freedesktop.impl.portal.desktop.gtk` is
   owned (`NameHasOwner` returns `true`), started a real GTK4 client
   (`gtk4-demo --run=iconview`, standing in for `trace-commons-shell` --
   building the actual Rust/GTK binary in a fresh container was too slow
   for this session, see below), then ran `weston-screenshooter` the same
   way the script does. Result: a 105,654-byte PNG with a grayscale
   standard deviation of **0.218188** -- far above the script's `0.01`
   non-blank threshold. This exercises the same compositor, portal, and
   screenshooter interaction the real job does; only the specific
   application binary differs.
5. **Static checks**: `bash -n` clean on both scripts;
   `python3 -c "import yaml; yaml.safe_load(...)"` clean on `ci.yml`;
   `shellcheck` on both scripts shows only pre-existing informational
   findings (SC1091 on the `fixture.sh` source, and SC2317/SC2329 on dead
   code after the inner script's `exit "$FAIL"` that predates this change)
   -- nothing at warning level or above, and nothing introduced by this
   diff.

**What I did not run**: the actual `trace-commons-shell` binary inside a
container (that requires the full GTK4/libadwaita dev toolchain plus a
`cargo build`, which was too slow to complete in this session alongside the
other verification), and I did not and cannot run GitHub's own
`ubuntu-latest` runner.

## What only the next CI run can confirm

- That `weston-portal-verify.sh` invoked from the real `ci.yml` job, with
  the real `trace-commons-shell` binary (not `gtk4-demo`) and the real
  `trace-commons-contributor` daemon running alongside it, produces a
  non-blank frame and passes the OCR check for "Queue" -- the local repro
  used a stand-in GTK4 client for the screenshooter/portal mechanics only.
- That GitHub's actual runner image resolves `weston` to the same
  13.0.0 build apt resolves to inside a plain `ubuntu:24.04` container
  (very likely, since `ubuntu-latest` is documented as 24.04 and both pull
  from the same Ubuntu archive, but not something a local Docker run can
  prove by itself).
- That the job's existing timing (`sleep 6` before the screenshot,
  40-iteration socket-wait loops) holds up under the real runner's
  resource contention with the added `--debug` startup path (the debug
  protocol registration is fast and didn't visibly change weston's startup
  time locally, but CI runners are typically slower and busier than a local
  Docker Desktop VM).

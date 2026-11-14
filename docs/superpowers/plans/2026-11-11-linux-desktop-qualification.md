# Linux desktop qualification audit

Date: 2026-11-11. Source: `07378979837da26f0343e4cd99d0f0d6097ffb64`.
Scope: desktop integration and packaging; not a Linux release approval.

## Executed evidence

| Check | Command / evidence | Result and limit |
|---|---|---|
| Metadata, icon name and version | Disposable Ubuntu 24.04; install desktop-file-utils and appstream; run `bash crates/trace-commons-contributor-gtk/scripts/validate-desktop-entry.sh` with checkout mounted read-only | Exit 0; version 1.0.5 agrees. One pedantic uppercase component-ID notice. No host packages installed. |
| Current-head hosted metadata | `gh run view 43153854476 --json jobs` | [Metadata passed](https://github.com/TraceCommons/trace-commons/actions/runs/43153854476/job/130977821692). |
| Current-head compositor integration | Same query | [Weston job](https://github.com/TraceCommons/trace-commons/actions/runs/43153854476/job/130977821782) still running at audit time; no passing claim for this head. |
| Earlier hosted compositor integration | `gh run view 43107619225 --json jobs` | [Weston job passed](https://github.com/TraceCommons/trace-commons/actions/runs/43107619225/job/130814967053), head `d51a96ef5030ad8451175f63f9e2e4ae30d3697c`. The later control-flow audit below found its independent RequestBackground probe unreachable; this pass does not prove that probe ran. Not GNOME/KDE or current-head qualification. |
| Release workflow | `gh run view 43122606955 --json jobs` | Linux Flatpak skipped. Successful overall release is not Linux build evidence. Linux now runs only on explicit dispatch with platform linux or all. No dispatch performed. |
| Local environment | `docker ps --format '{{.Names}} {{.Image}}'` | No running container at initial inventory. macOS/Colima, no supplied GNOME/KDE session. Metadata container is not a desktop. |

Reproduce the metadata check without accessing a real home directory:

```sh
docker run --rm \
  --mount type=bind,source="$PWD",target=/src,readonly ubuntu:24.04 \
  sh -c 'apt-get update -qq && apt-get install -y -qq --no-install-recommends desktop-file-utils appstream >/tmp/install.log && cd /src && bash crates/trace-commons-contributor-gtk/scripts/validate-desktop-entry.sh'
```

## Desktop matrix requiring execution

All four rows are **not run**. Each needs a logged-in desktop, its portal backend, an installed test package, and a disposable profile with synthetic session fixtures only.

| Session | Required checks | Environment blocker |
|---|---|---|
| GNOME Wayland | Rendering/focus/clipboard, background allow/deny, close/reopen, notifications, sign-in callback, logout/login autostart | No GNOME session, Secret Service or interactive portal. Test stock GNOME without tray, plus a documented tray extension if supported. |
| GNOME X11 | Same plus X11 fallback and scaling | No GNOME X11 session; Weston is Wayland only. |
| KDE Wayland | Same plus real StatusNotifierWatcher actions and Discover update | No Plasma, KDE portal or Discover. |
| KDE X11 | Same plus X11 fallback and scaling | No Plasma X11 session. |

Installed Flatpak qualification additionally requires allowed-root access, denied reads outside those roots, authentication/upload/inference connectivity, Secret Service persistence, an update between two signed test revisions with relaunch, background grant/deny, and login autostart without duplicate daemon ownership. Parsers and mocked portals cannot prove these behaviors. Do not use private traces or production uploads.

## Source findings requiring qualification or repair

1. **Network permission absent:** `flatpak/ai.tracecommons.Contributor.yml` lacks `--share=network` despite cloud/inference behavior. Treat packaged connectivity as blocked pending installed testing and an explicit permission decision; this audit does not broaden permissions.
2. **Unrestricted session bus:** `--socket=session-bus` bypasses filtering, as the manifest itself documents. Listed `--talk-name` entries do not constrain it. Narrowing requires real Secret Service, notifications, portal and SNI regression coverage.
3. **Flatpak login autostart unproven:** `src/autostart.rs` writes beneath `dirs::config_dir()/autostart`, which is private app config in Flatpak. Settings invokes this directly; `src/portal.rs` requests background with `autostart=false`. This does not establish a host login entry. Test native and sandbox installations with and without an existing systemd daemon.
4. **Portal presence is intentionally separate from permission:** `BackendState::Present` explicitly means a backend answered, including denial; the UI does not claim a grant. The original decoder ignored malformed replies and stream termination. The follow-up repair decodes permission outcomes and makes malformed/missing replies Unknown while retaining Present for valid denial/cancellation. Actual desktop allow/deny dialogs remain untested.
5. **Update tests are not an installed update:** `src/update.rs` contains monitor/state tests; this lane did not run an installed signed two-revision update.

The audit initially made no runtime changes. Its narrow follow-up repairs portal reply decoding with serialized-message regression tests; no permission, autostart, update or UI changes. Actual desktop and installed-package tests remain unavailable. The owner selected container-only validation and explicit reporting of these gaps.

## Historical documentation

`linux-desktop-verification-report.md` (2026-08-09) says Weston had never run and locates the job in `ci.yml`. That describes its original pass. The job now lives in `clients.yml`; the successful hosted job above supersedes that historical execution gap. The manifest's leading `UNBUILT` paragraph also coexists with later comments describing actual flatpak-builder runs, so it is not reliable current build status. This audit establishes neither a current Flatpak build nor GNOME/KDE qualification. Keep release disabled until the missing matrix and packaged behaviors are qualified.

## Portal decoder follow-up verification

The two new regression tests first failed against the old ignore-response behavior (four existing tests passed). After implementing decoding, the full GTK workspace test command passed on the macOS development host with existing GTK libraries:

```sh
RUSTFLAGS='-D warnings' CARGO_INCREMENTAL=0 cargo test --locked --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
```

Synthetic serialized D-Bus messages cover granted, denied, cancelled, other response, missing response, missing permission result, unknown response code and malformed body. They exercise decoding and presence classification without a desktop or permission prompt. The protocol interpretation follows [Request.Response](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Request.html) and [Background results](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Background.html). This is unit evidence only, not GNOME/KDE or installed Flatpak qualification.

## Independent portal probe reachability repair

Source inspection found an unconditional `exit "$FAIL"` between the rendering checks and the independent `PORTAL_UP` / RequestBackground probe. The latter block could never execute. Historical successful Weston jobs therefore establish only the checks actually reached; their job name is not evidence that the independent probe ran.

Moved the final exit below the portal block, preserving accumulated rendering failures and adding portal failures to the same exit status. Existing screenshot/OCR and mounted-test gates are unchanged. The wrapper now runs six executable control-flow regressions before starting services. They execute the production tail with mocked D-Bus boundaries and cover probe reachability, preservation of prior failures, missing service ownership, timeout, absent-service replies and bounded non-absence replies. All six failed before repair (eight failing assertions including subcases), then passed. Bash syntax and diff whitespace checks passed.

These are control-flow regressions, not a live portal result. The corrected full Weston job and GNOME/KDE/installed Flatpak matrix remain distinct validation requirements. No screenshots or source logs were uploaded by this repair.

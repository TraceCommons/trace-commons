<!--
  DRAFT -- needs the owner's approval before it is used anywhere.

  Every Windows parity slice this note describes (W2 to W25) is still an open
  draft pull request on 2026-11-19 (#2207 to #2225). Nothing below has
  shipped. Trim each line to what has merged before the release is cut, and
  date and version the file then (docs/announcements/<date>-v<x-y-z>.md).

  The whole note is draft copy; the list at the end names the phrases that
  are not core copy at all.
-->

# Near AI Gateway for Windows

Near AI Gateway on Windows now looks and works like the Mac app. The same
window, the same first run, the same Settings, and the same words.

For Windows 10 22H2 or later and Windows 11, 64-bit.

## What changed

- **The Mac window.** The Mac's three-pane window (#2196, merged).
- **The Mac first run.** Choosing the tools to watch, connecting to Private AI,
  seeing what your traces could earn, choosing which data uses to allow,
  reading the Terms of Service, and signing in.
- **Home, the tray icon and the edge rail** match the Mac's Home, menu bar
  icon and edge rail. The tray icon shows the app's mark and, when something
  needs you, a mark of its own.
- **Traces** has the Mac's list, map and transfers.
- **History** has the Mac's detail, preview and Missions.
- **Private AI, Insights and About** match the Mac.
- **Settings** has the Mac's sections.
- **Consent** follows the Mac's flow, and the app shows its own name
  everywhere.
- **Motion.** The Mac's animations, including credit arriving on the tray and
  taskbar icons, behind the same switch in Settings. Reduce motion in Windows
  turns them off.

## Installing

- **App (recommended):** install the signed package from
  https://storage.googleapis.com/tracecommons-flatpak/windows/NearAIGateway.appinstaller.
  Windows keeps it up to date from that address. It installs for your user
  account only and needs no administrator rights.
- **Portable zip:** `trace-commons-app-windows-x86_64-<version>.zip` on this
  release. Unzip it and run `TraceCommons.exe`.
- **Command line:** `trace-commons-windows-x86_64-<version>.zip` on this
  release. `scripts/install.ps1` installs the command line from its own
  release stream (`contributor-v*`), not from this page.

Every download is signed by Iqlusion Inc and timestamped.

Windows 10 versions before 22H2 are no longer supported.

Nothing leaves your machine until you approve an upload.

<!--
  The whole note is draft copy. Beyond the Mac's existing feature names, every
  sentence is new and needs approval, including "App (recommended)", "needs no
  administrator rights", "installs for your user account only" and "Every
  download is signed by Iqlusion Inc and timestamped".
  [DRAFT WORD] list for the owner:
  - "Near AI Gateway for Windows" (title)
  - "Portable zip"
  - "Command line"
  - "Windows 10 versions before 22H2 are no longer supported."
  All other feature names (Home, Traces, History, Private AI, Insights, About,
  Settings, Missions, Consent, Terms of Service) are the Mac's existing words.
-->

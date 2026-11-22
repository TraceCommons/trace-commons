# Red Team Managed Sessions (CLI)

Red Team sessions run OpenCode inside an isolated `rts` microVM: on Apple Silicon macOS, in a Hyper-V virtual machine on Windows 11 Pro, Enterprise or Education, or in a Firecracker microVM on x86-64 Linux. It is available in every macOS, Windows and x86-64 Linux build with no feature flag. On Linux only the host daemon runs it: a daemon inside a Flatpak sandbox reports it unavailable. aarch64 Linux has no Red Team. Inference goes through NEAR AI via an embedded IronWire proxy with receipt capture; the NEAR AI key never enters OpenCode or the guest VM.

## Launching a Red Team session

Interactive terminal launch:

```bash
trace-commons-contributor launch red-team \
  --input ./target-repo \
  --prompt "list /in and write /out/ls.txt"
```

Non-interactive / agent-driven launch (opens a new terminal window and prints JSON `{"session_id": "..."}`):

```bash
trace-commons-contributor --json launch red-team \
  --input ./target-repo \
  --allow-host example.com:443 \
  --packages \
  --cpus 4 \
  --memory 4G \
  --prompt "Inspect /in and write findings to /out/report.md" \
  --new-terminal
```

## Red Team options (`red-team` only)

- `--input <PATH>` (repeatable): Host file or directory copied (never symlinked) into `/in`. Missing paths, symlinks, `/`, and `$HOME` are refused before VM creation.
- `--allow-host <ENTRY>` (repeatable): Explicit outbound host allowlist entry (`host[:port]` or `[ipv6]:port`). Loopback, link-local, metadata, and private ranges are refused. Network is off by default.
- `--packages`: Adds the standard package-registry allowlist preset (`dl-cdn.alpinelinux.org`, `pypi.org`, `files.pythonhosted.org`, `registry.npmjs.org`, `index.crates.io`, `static.crates.io`, `crates.io` on port 443).
- `--cpus <N>`: Guest vCPU count.
- `--memory <SIZE>`: Guest memory size in MiB, or with `M`/`G` suffix (minimum `64`).

## Universal launch options (every managed tool)

- `--prompt <TEXT>` / `--prompt-file <PATH>`: Mutually exclusive initial prompt (UTF-8, at most 64 KiB) passed to Claude Code (`-- <prompt>`), Codex (`-- <prompt>`), OpenCode (`--prompt <prompt>`), or Red Team (`LaunchSpec.initial_prompt`).
- `--new-terminal`: Asks the running daemon to open the prepared session in a new terminal window and prints its session ID (or `{"session_id": "..."}` with `--json`). Required when stdin is not a TTY.

## Windows host requirements and "Enable Red Team"

The Windows backend needs the Hyper-V feature on and the user in the Hyper-V
Administrators group (`S-1-5-32-578`). Windows Home has no Hyper-V and is
unsupported. The `red_team` row of `managed_snapshot` reports `installed: false`
with `host_problem` (`home_edition`, `hyper_v_missing` or
`not_in_hyper_v_administrators`) and `enable_offered` (false only for Windows
Home) while the host cannot run it.

`managed_tool_setup {"tool":"red_team","action":"enable"}` runs the fixed
`enable-red-team.ps1` that ships beside `near-ai.exe` in the signed package,
elevated through a UAC prompt. The request carries no arguments. The daemon
resolves the script beside its own executable (a regular file, not a link, under
that directory) and passes only the user SID read from its own process token,
validated as `S-1-5-21-` plus at least four decimal sub-authorities. The reply is
`{"opened":true,"outcome":...}` with `sign_out_needed` (the group applies after
signing out and back in), `restart_needed` (Hyper-V was turned on; restart
Windows), `home_unsupported`, `declined` (UAC cancelled) or `failed`. The app
explains the change before the prompt with the `red_team_enable_explain` copy.



## Linux host requirements

The Linux backend needs `/dev/kvm` opened by the user and a systemd user
session. The `red_team` row of `managed_snapshot` reports `installed: false`
with `host_problem` while the host cannot run it:

- `no_kvm`: there is no `/dev/kvm`. Explained only.
- `kvm_not_accessible`: `/dev/kvm` exists but this user cannot read and write
  it. The row carries `fix_command`, `sudo usermod -aG kvm "$USER"`. The app
  shows it with "Copy command" and never runs it (no pkexec, no system
  changes); the person signs out of every session and back in afterwards.
- `no_user_session`: mvm bounds each VM in a `systemd-run` scope, which needs
  `systemd-run` on `PATH` and a user session bus (`DBUS_SESSION_BUS_ADDRESS`,
  or `$XDG_RUNTIME_DIR/bus`). Explained only. (mvm-core `spawn_scope`,
  `mechanism_launcher`.)

`enable_offered` is false on Linux; "Enable Red Team" is a Windows action.

A daemon inside a Flatpak sandbox sends no `red_team` row and reports
`capabilities.daemon_on_host: false`; the GTK app then says Red Team needs the
host service and shows the background service's install command with "Copy
command".

The runtime archives (rts 0.1.8) are pinned in `managed/red_team_release.rs`:
`RTS_RELEASE` (macOS), `RTS_RELEASE_WINDOWS` and `RTS_RELEASE_LINUX`
(`rts-0.1.8-x86_64-unknown-linux-gnu.tar.gz`). A test fails if any pin is an
all-zero placeholder.

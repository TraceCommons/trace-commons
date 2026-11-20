# Red Team Managed Sessions (CLI)

Red Team sessions run OpenCode inside an isolated `rts` microVM on Apple Silicon macOS. It is available in every macOS build with no feature flag; other platforms report it unavailable. Inference goes through NEAR AI via an embedded IronWire proxy with receipt capture; the NEAR AI key never enters OpenCode or the guest VM.

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

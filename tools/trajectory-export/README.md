# trajectory-export

Export local agent-harness sessions to [Letta Trajectory][t] v1 files, so the
Trace Commons contributor CLI can read them.

```bash
npx @tracecommons/trajectory-export           # what is on this machine?
npx @tracecommons/trajectory-export --all     # export the newest from each
trace-commons-contributor submit              # submit what was written
```

## Why this exists

`@letta-ai/trajectory` is a library, not a command: no published version
declares a `bin`, so `npx @letta-ai/trajectory` fails with "could not
determine executable to run". Trace Commons documented that command for
months, and contributors at a hackathon lost time to it. This is the CLI that
instruction assumed.

It wraps two upstream calls: `listTrajectories` to find sessions in a
harness's local store, and `normalizeTranscript` to convert one.

## What it writes

`<source>-<id>.trajectory.json` in the working directory, JSON Lines.

The suffix is load-bearing. The contributor CLI auto-discovers
`*.trajectory.json` in the working directory and nothing else, so that a stray
`session.json` never joins a submission. Any other name reintroduces the
`--trajectory` flag this tool exists to remove.

## Sources

`atif`, `copilot-cli`, `cursor`, `droid`, `gemini-cli`, `hermes`,
`letta-code`, `omp`, `openclaw`, `opencode`, `openhands`, `pi`.

**`claude-code` and `codex` are deliberately absent.** The contributor CLI
reads both natively, straight from their stores, with no conversion step and
no Node on the machine. Run `trace-commons-contributor submit` instead.

**`deepagents` is absent** for a different reason: it is a checkpoint source
rather than a transcript one and needs upstream's checkpoint API.

Several sources are **export-only** -- upstream can normalize their
transcripts but cannot enumerate their stores, `gemini-cli` among them. For
those, export from the harness and name the file:

```bash
npx @tracecommons/trajectory-export --source gemini-cli --input session.json
```

## Privacy

No network access, no telemetry. It reads the stores you name and writes only
into `--out-dir` (default: the working directory).

Nothing here is redacted. Redaction happens in the contributor CLI, on the way
into an envelope. Treat the files this writes as raw session content: they
contain whatever your harness recorded, including anything you pasted while
working.

## Development

```bash
npm install
npm test
```

[t]: https://github.com/letta-ai/trajectory

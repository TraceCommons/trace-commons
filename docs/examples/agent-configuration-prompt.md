# Sample prompt: configure Trace Commons

Copy the prompt below into a terminal-capable AI agent and replace the bracketed details. These are instructions for the agent; the example preferences are not a request to change your machine until you choose them.

---

Configure my Trace Commons / Near.ai app through its CLI.

Target app/channel: [production or staging, and installation location if nonstandard].
Requested settings: [for example, disable notifications and enable trace-value estimates].
Folder policies: [absolute folder paths and ignore / notify / automatic; for automatic, say new traces only or include existing backlog].
Trace-value question: [for example, show estimates for pending traces in these folders, or none].

Use the app's bundled `near-ai` CLI when available and its matching channel/configuration directory. Read `--version` and `--help`; first run `--json config show`, which requires the running daemon without creating missing state. Only after it succeeds, check `--json daemon status` and confirm `daemon_running: true`. If the selected app is unavailable, explain that instead of changing another app or creating offline state.

1. Read `--json config schema` and `--json config show`. Use the running daemon's schema to discover supported settings. Do not infer writable fields from the full settings output or edit settings files directly.
2. Read `--json daemon projects` to identify folder paths, saved `folder_mode`, effective `mode`, and backlog scope. Read `--json daemon pending` for requested trace estimates. Report an absent estimate as unknown. `settlement_estimate.low_usd_micros` and `high_usd_micros` are an estimated inference-credit range: divide by 1,000,000 for dollars, never promise payment, and never interpret legacy `credit_estimate` as dollars. Inspect aggregate provenance from `daemon status` if available. Mark any saved-state response as stale rather than live.
3. Create a minimal JSON object containing only my requested supported settings. Run `--json config preview --file <patch-file>` and inspect every resulting change, including canonical/alias fields and redacted changes. If the preview matches my request, run `--json config apply --file <patch-file>` and read the resulting settings/runtime status. Ask only when my requested scope is unclear or the proposed changes exceed it. An approved settings change is not permission to enroll, upload traces, or change accounts.
4. For explicitly requested folder changes, use `daemon project <absolute-path> --mode ignore|notify|auto`. For new traces only, use `--mode auto --from-now`. Use `--include-backlog` only if I requested existing traces too. Automatic mode otherwise includes backlog by default, so do not omit the scope flag. Do not select the unresolved-folder bucket for automatic mode.
5. Verify settings with `config show`, folder policies with `daemon projects`, and live status with `daemon status`. Report what changed, what remains unknown, and anything that requires browser sign-in or app permissions. Saving a preference does not prove a service started successfully. Never expose secrets or claim that preview applied a change.

Do not approve or submit individual traces, enroll an account, forget credentials, start a second daemon, or change unrequested settings. Preserve unrelated settings. If commands are unavailable in this version, report the needed app/CLI update rather than bypassing the API.

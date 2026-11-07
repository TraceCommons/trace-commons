# Configure the app with an AI agent

An agent with terminal access can use `near-ai` to discover the running app's settings, preview changes, apply them, inspect trace value estimates, manage folder upload policies, and finish setup after configuring the app. No MCP server is required. Give the agent the [sample prompt](examples/agent-configuration-prompt.md) with your desired changes.

These configuration commands require a build containing this feature in both the CLI and the app. They do not modify settings files directly. The app's daemon validates and persists changes using the same settings path as the UI.

## Select the app and channel

Use the CLI bundled with the app you want to configure. For the production macOS app:

```sh
TC_CLI='/Applications/Near.ai.app/Contents/MacOS/near-ai'
TC_DIR="$HOME/Library/Application Support/trace-commons"
"$TC_CLI" --version
"$TC_CLI" --config-dir "$TC_DIR" --json config show
"$TC_CLI" --config-dir "$TC_DIR" --json daemon status
```

For staging, use both the staging CLI and its separate directory:

```sh
TC_CLI='/Applications/Near.ai Staging.app/Contents/MacOS/near-ai'
TC_DIR="$HOME/Library/Application Support/trace-commons-staging"
"$TC_CLI" --version
"$TC_CLI" --config-dir "$TC_DIR" --json config show
"$TC_CLI" --config-dir "$TC_DIR" --json daemon status
```

Start with `config show`: it requires the running app and does not create a missing state directory. Then check that `daemon_running` is `true` in status. Launch the chosen app if it is not running. Do not start a second daemon against the same directory or substitute the other channel's CLI. `--config-dir` selects local state; it does not change a binary's compiled channel.

On Linux or Windows, use the installed `near-ai` binary, or the equivalent `trace-commons-contributor` binary, and the app's actual state directory. See the [platform directory table](../README.md#uninstalling). Respect any configured directory override. Paths and app installation locations may differ; confirm them before acting.

## Discover and inspect settings

```sh
"$TC_CLI" config --help
"$TC_CLI" --config-dir "$TC_DIR" --json config schema
"$TC_CLI" --config-dir "$TC_DIR" --json config show
```

`config schema` asks the running daemon for its supported patch schema, including field types, enum choices, numeric bounds, and descriptions. Use its `patch_schema.properties` to discover writable settings rather than guessing names from UI labels. The schema and preview responses have versioned `schema_version` fields.

`config show` returns effective saved settings with credentials and source paths redacted. It includes read-only fields: **do not copy its entire output into an apply file**. Write a partial object containing only the settings the user asked to change. Folder rules and accounts have dedicated commands described below.

## Preview and apply a settings patch

For example, to disable desktop notifications and enable the app's trace-value estimates:

```sh
cat > /tmp/trace-commons-settings.json <<'JSON'
{
  "notifications_enabled": false,
  "settlement_estimate_enabled": true
}
JSON
"$TC_CLI" --config-dir "$TC_DIR" --json config preview \
  --file /tmp/trace-commons-settings.json
"$TC_CLI" --config-dir "$TC_DIR" --json config apply \
  --file /tmp/trace-commons-settings.json
"$TC_CLI" --config-dir "$TC_DIR" --json config show
```

Use a task-specific file location when sharing a machine. Patch files are JSON objects, not RFC 6902 JSON Patch arrays. Empty objects, duplicate keys, invalid types or values, unknown settings, and files larger than 1 MiB are refused. Nested objects follow the schema's documented semantics: some merge selected subfields while source declarations replace a declaration. `null` is accepted only where the schema says so.

Preview validates a cloned candidate and returns `applied: false` with `changes`: canonical field names, before/after values, and a `redacted` flag. Aliases can affect a differently named canonical field. Side effects within settings, such as clearing an offer flag, appear in the preview too. A hidden path change is still reported as a redacted change; its actual path is not echoed. Preview does not write files, start services, change folder rules, or approve uploads.

Apply validates again and updates only the supplied settings through the daemon. A preview does not reserve the state: the UI or another client may change settings in between. Recheck the result and run `config show` after applying. Preserve any runtime outcome returned by the setter, such as `private_inference_state`; saving `private_inference: true` does not prove that authentication or proxy startup succeeded.

Applying settings does not finish setup. Complete browser sign-in and enrollment through their existing flows, then use the setup commands below to choose data uses and finish onboarding. These commands do not grant operating-system permissions or approve traces.

## Finish setup from the CLI

After the agent has configured the app, it can finish setup without walking
through the first-run screens:

```sh
"$TC_CLI" --config-dir "$TC_DIR" --json config setup

# The agent may choose these uses on the user's behalf.
"$TC_CLI" --config-dir "$TC_DIR" --json config complete \
  --scopes debugging_evaluation,model_training

"$TC_CLI" --config-dir "$TC_DIR" --json config setup
"$TC_CLI" --config-dir "$TC_DIR" --json daemon status
```

`config setup` reports the current enrollment's completion state, readiness,
blocking reason, saved data-use scopes and available choices. Read the returned
scope descriptions before choosing. `--scopes` is a comma-separated list of
protocol scope names; `debugging_evaluation` is the required floor. Use only
`debugging_evaluation` to choose no optional uses. The example also permits
model training; it is an example choice, not a required default.

An agent can make this data-use choice for the user. It does not need a separate
confirmation in the graphical app. To keep an existing explicit choice, run
`config complete` without `--scopes`. A fresh enrollment whose scopes have not
been chosen needs `--scopes` or an earlier explicit `consent scopes --scopes ...`
command. Unknown scope names are refused.

Completion requires the running daemon, a usable enrollment and declared
Claude/Codex sources. An explicit `off` declaration counts. It does not sign in,
enroll, change folder modes, arm automatic sharing, approve a backlog, or open
OS permission prompts. Handle any reported missing prerequisites through the
existing configuration or account commands, then finish setup again.

Completion is bound to the current enrollment and app state directory. The
command verifies that the enrollment has not changed before saving. Repeating
it is safe; a different account or channel does not inherit it. The daemon
persists the marker and publishes a status update, so an open native first-run
screen hands off to the normal app, and the marker survives restarts. Check
`status.setup_complete` rather than editing GUI preferences or marker files.
Existing GUI completion markers and older-daemon behavior remain supported.

If saving completion fails after an explicit data-use change was saved, inspect
the returned state before retrying; do not assume the scope choice was rolled
back. Completion itself does not add consent scopes. The frozen Tauri shell
continues to use its existing local onboarding flow.

## Discover trace value estimates

```sh
"$TC_CLI" --config-dir "$TC_DIR" --json daemon pending
"$TC_CLI" --config-dir "$TC_DIR" --json daemon status
```

`daemon pending` returns queue rows in `pending`, with `entry_id`, source and project identity. A row may contain:

```json
{
  "entry_id": "example-entry-id",
  "project_label": "example-project",
  "settlement_estimate": {
    "low_usd_micros": 120000,
    "high_usd_micros": 350000,
    "drawn": true
  }
}
```

This illustrative range is **$0.12–$0.35 of estimated inference credit**, not a guaranteed payment. Divide USD micros by 1,000,000 to display dollars. An optional `tier` gives the estimate's category. Do not treat the legacy `credit_estimate` field as dollars; it can be present with `drawn: false`.

A missing estimate means unknown, not zero. The estimate switch may be off, the pricing table may be unavailable or stale, features may be missing, or the trace may already have been sent. `daemon status` can provide aggregate estimate context including `known`, `unknown`, `table_version`, `pricing_version`, and `estimated_at`. Report that context when available; do not invent it for individual rows. Estimates may appear asynchronously after enabling them.

This is discovery of pending traces known to the daemon, not a valuation API for arbitrary files. These read commands neither approve nor upload traces. `daemon pending` and `daemon projects` include `daemon_running`; false means the response is reconstructed from saved local state and may be stale.

## Inspect and change folder policies

```sh
"$TC_CLI" --config-dir "$TC_DIR" --json daemon projects
```

For each row in `projects`:

| Field | Meaning |
| --- | --- |
| `project_id`, `project_label`, `project_path` | Folder identity and display path |
| `folder_mode` | The folder's saved rule |
| `mode` | Effective rule, including any global override |
| `configured` | Whether an explicit folder rule exists |
| `from_now` | For automatic folders, whether existing backlog was excluded |
| `automatic_disclosure` | Automatic privacy-review disclosure label (`model_scrubbed` or `patterns_only`) |
| `is_unresolved_bucket` | Traces without a resolved working folder; cannot be made automatic |

Modes are `ignore`, `notify_only`, and `auto_upload`. The CLI flags use the shorter names `ignore`, `notify`, and `auto`:

```sh
# Ignore a folder explicitly chosen by the user.
"$TC_CLI" --config-dir "$TC_DIR" --json daemon project \
  /absolute/path/to/project --mode ignore

# Make NEW traces automatic; keep the existing backlog waiting.
"$TC_CLI" --config-dir "$TC_DIR" --json daemon project \
  /absolute/path/to/project --mode auto --from-now

# Include the existing backlog when the user explicitly requested that.
"$TC_CLI" --config-dir "$TC_DIR" --json daemon project \
  /absolute/path/to/project --mode auto --include-backlog

# Return to notifying rather than automatically approving new traces.
"$TC_CLI" --config-dir "$TC_DIR" --json daemon project \
  /absolute/path/to/project --mode notify

"$TC_CLI" --config-dir "$TC_DIR" --json daemon projects
```

**Entering automatic mode without `--from-now` includes the existing backlog by default.** Use an explicit flag so the user's intended scope is visible. An already-automatic folder can retain its existing hold when no flag is supplied. These policy commands can enable uploads subject to the app's existing enrollment, consent, privacy and other gates; they are not harmless display preferences.

Resolve `~` in a displayed project path before passing it as an argument, and use an existing absolute folder path. Do not infer folders from display labels alone. Check both `mode` and `folder_mode` after the change: a global rule may override the folder's own setting. The legacy project command can save a rule while the daemon is unavailable; check live status first when configuring the running app.

## Accounts and managed sessions

Use existing account/session commands rather than treating credentials as settings:

```sh
"$TC_CLI" accounts --help
"$TC_CLI" sessions --help
"$TC_CLI" launch --help
```

Managed-session agent options are also discoverable in `config schema` as
`managed_cli_options`. They share the values edited in **Settings → Tools**:

```sh
cat > /tmp/trace-commons-agent-options.json <<'JSON'
{
  "managed_cli_options": {
    "codex": "-c model_reasoning_effort=high --sandbox workspace-write",
    "claude": "--model sonnet"
  }
}
JSON
"$TC_CLI" --config-dir "$TC_DIR" --json config preview --file /tmp/trace-commons-agent-options.json
"$TC_CLI" --config-dir "$TC_DIR" --json config apply --file /tmp/trace-commons-agent-options.json
"$TC_CLI" --config-dir "$TC_DIR" --json config show
```

Omitted agents keep their options; use an empty string to reset one. These values
apply to new managed coding sessions only. Quote values containing spaces.
Arguments are passed literally without shell expansion. Account routing, profile
and session identity remain managed by Trace Commons; overriding their flags is
refused. Select OpenCode’s model in its launch dialog. For Codex model selection,
use `--model`; the launcher’s route default takes precedence over `-c model=...`.

Folder-specific launch choices use `managed_folder_settings`. For example:

```json
{
  "managed_folder_settings": [{
    "tool": "codex",
    "cwd": "/absolute/path/to/project",
    "model": null,
    "collect_traces": true,
    "cli_options": "-c model_reasoning_effort=high"
  }]
}
```

Use an existing absolute folder. Preview and apply this partial settings object
with the same `config` commands above. Each supplied folder/agent entry replaces
its choices; other entries are preserved. To forget one, apply an entry with
`tool`, `cwd` and `"reset": true`. These profiles contain no account selection.
Concurrent runs can share an account and folder; collection uses exact native
session identities and never sweeps other sessions from the profile.
The native folder-selection dialog restores the same choices and shows
**Collect trace for review** beside the model and CLI options.

`launch` restores folder choices unless explicitly overridden. Use `--model` or
`--default-model`, `--collect-traces` or `--no-collect-traces`, and `--cli-options`
(an empty string clears arguments) for the new session. Account selection remains
the existing interface/CLI choice. Collection occurs after the session ends and
continues through the existing review and contribution gates; enabling it does
not authorize an upload.

Keep account operations within the user's requested scope. Do not read credential files, copy tokens into patch files, or treat a sign-in button as proof that login completed.

## Errors and compatibility

Configuration commands require a live daemon. A missing daemon is an error, not a request to create offline settings. Older daemons may not support schema or preview; update the selected app and matching CLI instead of bypassing validation by editing JSON files. Inspect the structured error and exit status. A failed validation must not be reported as an applied change.

For the underlying protocol, see the [daemon IPC reference](contributor-daemon-ipc-v1_1.md). For an agent-ready workflow, copy the [sample prompt](examples/agent-configuration-prompt.md).

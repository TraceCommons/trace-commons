# Saved accounts and managed sessions

Keep Trace Commons running, then open the model-calls screen. Use **Add account**
to choose Claude Code or Codex, a connection, and a label such as Personal or Work.
For a subscription, **Save and sign in** opens the native tool's sign-in flow in
a terminal with a separate profile. Repeat Add account for each additional
subscription. For an API key or NEAR AI key, enter the key in the secure field;
Trace Commons stores it in the operating system credential store.

**Launch managed session** lets you choose an account and project. The app names
the destination terminal before launch. **Use as default** changes the default
for future managed launches. Changing a default never changes an existing
session. **Change global settings** contains the existing preview-and-commit
controls for standard tool settings; those changes apply after restarting the
tool. Managed launches do not edit those settings.

The CLI is named `near-ai`; `trace-commons-contributor` remains a compatibility
command. Installers install both names from the same verified release bytes.

```sh
near-ai accounts add --tool claude --label Personal
near-ai accounts add --tool claude --label Work
near-ai accounts add --tool codex --label Work
near-ai accounts list
near-ai accounts select Personal --tool claude
near-ai launch claude --account Personal --cwd /path/to/project
near-ai launch codex --account Work --cwd /path/to/another-project
near-ai sessions list
```

CLI launches use the current terminal or multiplexer pane. All managed launches,
including those initiated from the CLI, appear in the app's managed-session list.
Standard sessions and multiple managed accounts can run in parallel.

For an API key, supply it through stdin using a secret-manager pipe, rather than
a command argument or shell-history literal:

```sh
secret-manager-command | near-ai accounts add --tool codex --connection api-key --label Work --key-stdin
```

`secret-manager-command` is a placeholder for your existing secret manager.
Use `--connection near-ai` for a NEAR AI key. Subscription credentials remain
owned by the native tool; they are not pasted into Trace Commons. Account IDs
can be used when labels are duplicated. Removing the selected account requires
an explicit new selection before a CLI launch without `--account`.

## Availability and recovery

- Supported native adapter versions begin at Claude Code 2.1.289 (major 2) and
  Codex 0.160.0 (major 0). Older or unrecognized versions are refused before
  login, logout, or managed use.
- App terminal adapters target Terminal on macOS, Windows Terminal when found,
  and `x-terminal-emulator` on an unsandboxed Linux installation. The packaged
  `near-ai` helper must be present. Flatpak host-terminal launching is unavailable;
  it requires an explicit host bridge. No terminal-focus operation is advertised.
- Each NEAR AI session owns an isolated IronWire proxy and key. It waits for
  NEAR's model catalogue and pins its own proxy to the first advertised model.
  It never changes the standard proxy or falls back to another provider/account.
  Model selection within that catalogue is a follow-up UI capability.
- A profile cannot be reconnected, have its key replaced, or be removed while a
  managed session or sign-in holds it. Multiple coding sessions may share the
  same account, but sign-in requires that account to be idle.
- Starting becomes Running only when the helper reports the native child.
  Terminal exits and confirmed setup failures have private durable receipts that
  reconcile after daemon downtime. An unredeemed launch expires after two minutes.
  Unknown means liveness has not been confirmed, not that it is safe to relaunch
  or remove the account. Forced helper termination may require local recovery.
- A project's explicit provider/auth overrides are refused when they could
  contradict the selected connection. Ordinary project settings are preserved.

No real subscription sign-in or provider-key validation is performed by the
fixture test suite. Native sign-in and external-terminal behavior must also be
smoke-tested on each release platform.

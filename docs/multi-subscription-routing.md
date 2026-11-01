# Multiple subscription logins in private inference

The contributor's embedded inference proxy is IronWire, taken as a git
dependency on [nearai/ironwire](https://github.com/nearai/ironwire) at the
revision pinned in `crates/trace-commons-contributor/Cargo.toml`. Since
a2ae5d97 that revision includes
[nearai/ironwire#64](https://github.com/nearai/ironwire/pull/64), which routes
more than one Claude Code or Codex login, each as itself. #64 is the merged
form of Jay Zalowitz's multi-subscription contribution (nearai/ironwire#59,
first proposed for Trace Commons in #1513). Nothing of IronWire is vendored
into this repository.

This is the private-inference path. Saved accounts under `accounts` and native
sessions under `launch` keep their existing profile ownership and behavior.
Accounts are not automatically enrolled into proxy routing or granted consent.

## Configure an additional login

Private inference reads `$IRONWIRE_HOME/config.toml`, or
`~/.ironwire/config.toml` when no override is set. Each additional Claude Code
or Codex login needs its own directory and backend ID:

```toml
[[backends]]
id = "claude-work"
kind = "claude-subscription"
config_dir = "~/.claude-work"

[[backends]]
id = "codex-work"
kind = "codex-subscription"
config_dir = "~/.codex-work"
```

The native tools own login and refresh:

```sh
CLAUDE_CONFIG_DIR=~/.claude-work claude auth login --claudeai
CODEX_HOME=~/.codex-work codex login
```

Trace Commons does not ship the `ironwire` CLI. To preview and connect those
exact directories, install it from upstream at the same revision and use its
`connect` command:

```sh
cargo install --git https://github.com/nearai/ironwire --rev a2ae5d97aaee005c30dd51b7a873e6eba06940af ironwire
ironwire connect claude --config-dir ~/.claude-work --subscription --dry-run
ironwire connect claude --config-dir ~/.claude-work --subscription
ironwire connect codex --config-dir ~/.codex-work --subscription
```

Connecting asks for that login's subscription consent, records it bound to
that login, writes the backend entry if it is not already declared, and points
the client that uses that directory at its account-bound local URL. It does
not authorize another directory's credential. An embedded start of private
inference never grants consent. Restart private inference after editing the
backend configuration. Its existing switch starts the embedded proxy; no
separate IronWire service is required.

## Which file the app connects

The app's Claude Code row (connect, status and the status-line repair) always
reads and writes the default login's `~/.claude/settings.json`, whatever
`CLAUDE_CONFIG_DIR` the daemon was started with. That file receives the plain
facade URL, `http://127.0.0.1:<port>/anthropic`, and the plain facade is served
by the default login's credential. Following `CLAUDE_CONFIG_DIR` instead would
point another login's Claude Code at the default login's token: one account's
requests on another account's subscription. IronWire #64 fixed exactly that,
and Trace Commons keeps the fix
(`daemon::harness::tests::claude_code_is_connected_in_the_default_login_whatever_claude_config_dir_says`).

A Claude Code you run under `CLAUDE_CONFIG_DIR` is a login of its own. Declare
it with `config_dir` as above and connect it with `ironwire connect claude
--config-dir`, which writes the account-bound URL into that directory's
`settings.json`. The app does not show or edit that file. A plain URL never
replaces an account-bound one already in a file.

Codex is unchanged: its row follows `CODEX_HOME`, as Codex itself does.

## Routing rules

The plain facade belongs to the unmoved default login. Additional logins use
`/anthropic/account/<backend-id>` or `/openai/account/<backend-id>/v1`.
Cross-account fallback defaults to off. Enabling
`[subscriptions] cross_account_fallback = true` is an explicit choice to permit
same-product fallback; providers can observe it. It does not relax client
identity checks, credential issuer boundaries, or encrypted reasoning's
original-account requirement. A replay of encrypted reasoning goes only to the
account that minted it; after a restart it is inferred only when a single
account could have minted it, and refused when that is ambiguous. Start a fresh
session rather than moving one.

Quota remains provider-observed or unknown. More declared accounts do not imply
more available capacity. Bounded concurrency and replay-state limits are
documented upstream in
[`docs/SCALE.md`](https://github.com/nearai/ironwire/blob/a2ae5d97aaee005c30dd51b7a873e6eba06940af/docs/SCALE.md),
and the login model in
[`docs/DESIGN.md`](https://github.com/nearai/ironwire/blob/a2ae5d97aaee005c30dd51b7a873e6eba06940af/docs/DESIGN.md).

## Validate a change

IronWire's own conformance suite runs in nearai/ironwire. On this side:

```sh
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib daemon::private_inference
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib daemon::harness
```

`declared_subscription_logins_survive_an_embedded_start_without_granting_consent`
pins that declared logins register on an embedded start, stay unconsented and
unauthenticated, keep cross-account fallback off, and leave `config.toml`
unchanged.

## Default tool connections in Settings

Use **Settings → Private AI → Default tool connections** to Connect or
Disconnect Claude Code, Codex and OpenCode. Connect shows the proposed changes;
files change only after confirmation. These are saved defaults for ordinary
tool launches, separate from the account chosen for a managed session.
Project settings and environment overrides can take precedence.

The proxy must be running and have an answering account before a new connection
can be prepared. The existing Private AI switch stops or starts the listener.
Turning it off keeps explicit tool connections; Disconnect removes a tool's
routing settings even while the proxy is off or the tool is uninstalled.

OpenCode versions 1 and 2 use their respective provider schemas. The preview
selects an actual model advertised by the running proxy. Previous model
selections are saved in a private adjacent routing record. Disconnect restores
only values still owned by this connection. Model choices that still name
this connection return to their previous provider; new agent overrides naming
it are removed so they inherit the restored default. Choices moved to another
provider and unrelated edits are kept. Disconnect does not replace the whole
configuration with an old backup. A malformed or
unsupported configuration is refused, including commented JSONC that cannot
be edited losslessly. No managed-profile permissions policy is copied into
global settings. Production and staging use separate routing records, and a
connection owned by the other app channel must be disconnected before these
global defaults can be changed.

See the official [OpenCode V1 configuration](https://opencode.ai/docs/config/)
and [OpenCode V2 configuration](https://opencode.ai/v2/docs/config/) references
for configuration precedence.

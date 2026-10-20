# Multiple subscription logins in private inference

The contributor's embedded inference proxy uses the local routing source in
`imports/ironwire`, including the multi-subscription contribution originally
submitted as nearai/ironwire#59. Provenance and merge decisions are recorded in
[`SOURCE.md`](../imports/ironwire/SOURCE.md).

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

The imported CLI can preview and connect those exact directories. From the
Trace Commons repository root:

```sh
cargo run --manifest-path imports/ironwire/Cargo.toml -- connect claude --config-dir ~/.claude-work --subscription --dry-run
cargo run --manifest-path imports/ironwire/Cargo.toml -- connect claude --config-dir ~/.claude-work --subscription
cargo run --manifest-path imports/ironwire/Cargo.toml -- connect codex --config-dir ~/.codex-work --subscription
```

Connecting asks for that login's subscription consent and prepares its
account-bound local URL. It does not authorize another directory's credential.
Restart private inference after editing the backend configuration. Its existing
switch starts the embedded proxy; no separate IronWire service is required.
The imported CLI and menu-bar app are retained source and test tools, not new
artifacts installed by Trace Commons packaging.

The plain facade belongs to the unmoved default login. Additional logins use
`/anthropic/account/<backend-id>` or `/openai/account/<backend-id>/v1`.
Cross-account fallback defaults to off. Enabling
`[subscriptions] cross_account_fallback = true` is an explicit choice to permit
same-product fallback; providers can observe it. It does not relax client
identity checks, credential issuer boundaries, or encrypted reasoning's
original-account requirement. Native encrypted continuation without a retained
matching origin is refused; start a fresh session rather than moving it.

Quota remains provider-observed or unknown. More declared accounts do not imply
more available capacity. Bounded concurrency and replay-state limits are
documented in [`SCALE.md`](../imports/ironwire/docs/SCALE.md), alongside the live
provider acceptance work still required.

## Validate a change

The imported workspace is separate from the root workspace, so run both:

```sh
CARGO_INCREMENTAL=0 RUSTFLAGS='-D warnings' cargo test --locked --manifest-path imports/ironwire/Cargo.toml --workspace --all-features
CARGO_INCREMENTAL=0 RUSTFLAGS='-D warnings' cargo clippy --locked --manifest-path imports/ironwire/Cargo.toml --workspace --all-targets --all-features -- -D warnings
cargo fmt --manifest-path imports/ironwire/Cargo.toml --all -- --check
CARGO_INCREMENTAL=0 RUSTFLAGS='-D warnings' cargo test --locked -p trace-commons-contributor
```

The conformance suite includes native-byte preservation, own-account routing,
encrypted replay isolation, issuer/redirect refusal, bounded credential work,
1,024 fixture-backed logins and 10,000 declared entries. It uses synthetic
credentials and loopback providers; it does not establish real provider policy
acceptance or real-account exhaustion and recovery.

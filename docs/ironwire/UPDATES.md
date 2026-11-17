# Updates

Two independent channels, deliberately:

| Channel | Carries | Cadence | Applied by |
|---|---|---|---|
| **Release** | The binary | Whenever we ship | The user, or their package manager |
| **Catalog** | Provider values that break us when they change | Same day if needed | The daemon, automatically |

IronWire **never updates its own binary.** The catalog channel exists so that it
does not need to.

---

## 1. Why notify-only

Most CLIs can self-update safely because they run between invocations. IronWire
cannot:

- It is a **daemon in the critical path**, often holding a streamed response
  that runs for many minutes. `PROTOCOL.md` §5 says an interrupted stream past
  the first byte is unrecoverable — so restarting ourselves mid-turn would cause
  exactly the outage the product exists to prevent. "Your agent never dies at
  the rate limit… unless we updated ourselves" is not a product.
- It **holds the user's credentials**. A self-update is arbitrary code execution
  as the user, with access to their subscription tokens. That is a much larger
  thing to ask for than "keep my linter current".
- It is usually **owned by a package manager**. Self-updating a Homebrew or apt
  install desyncs the manager from reality — brew believes v1 is installed, the
  file is v2, and the next `brew upgrade` clobbers or conflicts. This is the
  single most common auto-updater bug, and the reason `InstallMethod::detect`
  exists.

So: `ironwire update` reports, and prints **the command that belongs to the
user's install** — `brew upgrade ironwire`, `apt install --only-upgrade`, and so
on. Only the shell installer, the one channel with no external manager, gets a
self-install command.

### What the check does and does not send

The update check is the only request IronWire makes that is not the user's own
work, so it is bounded (`TRUST.md` §7):

- At most **once every 24 hours**, cached in `$IRONWIRE_HOME/update.json`.
- **Never blocks startup.** A release server outage must not stop a proxy.
- **No install id, no per-check identifier.** The request carries a version and
  an OS name in the user agent, and nothing else.
- **Kill switch**: `updates.check = false` in `config.toml`. It is honoured
  before the first check, so switching it off means no request is ever made —
  not "one last one". An embedding host that does not own that file says the
  same thing in code with `UpdateChecks::Off` (`docs/EMBEDDING.md`).

### `minimum_supported`

The manifest carries a floor as well as a latest version. Below the floor,
IronWire is likely *broken* against current provider APIs rather than merely
old, and the notification says so in stronger words. Without that distinction,
an update notice is noise a user learns to skip — and then misses the one that
mattered.

---

## 2. The catalog channel

IronWire depends on values it does not control and cannot predict: an
`anthropic-beta` flag, an API version string, the prefix that identifies Claude
Code, a model catalogue. When a provider changes one, **every deployed IronWire
breaks at once** — and the fix is a one-line string.

Shipping that string as a binary through five package ecosystems, then waiting
for users to upgrade, is days. Shipping it as signed data is minutes. So the
values live in a signed, versioned document the daemon refreshes on its own.

This is also what makes notify-only viable: the urgent class of breakage is
fixed without a binary at all.

### The security property

The obvious objection is that a remotely-updatable document controlling a
credential-holding proxy is a supply-chain hole. The design answers it
structurally rather than with validation:

> **The catalog may say what to set, never what to set it to.**

Base URLs, credential file locations, and the credential→host binding stay
compiled into the binary. Whoever holds the signing key can change *which beta
flag we send*, and which of *our own* façades a tool is pointed at; they cannot
change *where we send the token*. `TRUST.md` I2 holds even against a compromised
signing key.

The mechanism is still structural rather than validating. A catalog entry names
a façade, not an address, so the redirect is unrepresentable rather than
checked-for. Where a document *does* have to name something — the file a tool
keeps its config in — the shape is constrained instead: a dotdir under the
user's home, a `.json` or `.toml` file, `.` and `..` refused, separators outside
the charset. `ironwire_catalog::schema` has a test per constraint, and the
original name-walk still runs over the provider constants, which name nothing.

See `TRUST.md` §6 for why the earlier, broader wording was replaced.

### Rules

1. **Verify before parse.** An unsigned document is never deserialized.
2. **Never go backwards.** A document whose `serial` is at or below the
   installed one is refused. This is the rollback guard: without it, replaying
   an old signed document re-exposes a provider workaround already corrected.
3. **Never fail closed onto nothing.** A missing, corrupt, tampered, or
   too-new document leaves the previously installed one — or the compiled-in
   defaults — in force. A fresh install with no network works, because the
   defaults are the values that were correct when the binary shipped.
4. **A newer schema is refused, not partially applied.** Half-understanding a
   provider workaround is worse than using what we shipped with.

### What it carries today

| Field | Why it is here |
|---|---|
| `anthropic.api_version` | Header value the API validates |
| `anthropic.oauth_beta` | The flag that gates OAuth bearer auth — the value most likely to change under us, and the reason this channel exists |
| `client_identity.claude_code_system_prefix` | How we recognise Claude Code (`TRUST.md` §3) |
| `client_identity.codex_instructions_marker` | The same for Codex |
| `models` | Per-backend catalogues, which move faster than our releases |

Unknown fields are tolerated so an older binary survives a newer document.

### Signing

The public key is compiled in (`ironwire_catalog::CATALOG_PUBLIC_KEY`); a key
fetched at runtime is not a root of trust. The private half lives in release
signing infrastructure, not in this repository.

**Today the constant is a placeholder that cannot verify anything**, so every
document is refused and the daemon runs on compiled-in defaults. That is the
correct failure direction while release signing does not yet exist, and it means
the channel is inert rather than dangerous before it is real.

---

## 3. What is deliberately not built

- **Self-update of the daemon binary.** See §1. If it is ever added it belongs to
  the shell-installer channel only, and needs staged download plus drain-and-exec
  so no in-flight stream is cut. This applies whether the update arrives from the
  shell installer or from inside an app bundle that happens to carry the daemon —
  see `TRUST.md` §6.
- **Auto-apply at idle.** A reasonable opt-in later. Not a default: a proxy that
  changes its own behaviour unprompted is hard to trust and harder to debug.
- **A catalog field that names a host or path.** See §2. This is a `TRUST.md`
  change, not a schema change.
- **Telemetry beyond the version check.** There is none, and none is planned.

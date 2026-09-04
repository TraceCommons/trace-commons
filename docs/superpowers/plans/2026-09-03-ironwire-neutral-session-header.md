# IronWire Neutral Session Header Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let any client name its session to IronWire with one header, `x-ironwire-session-id`, so agents that send no native session header (Aider, Cline, Roo) can be joined to their ledger rows.

**Architecture:** `client_session_id` in the proxy reads the neutral header first and falls back to the per-protocol header it reads today. The neutral header is addressed to IronWire, so it is not forwarded upstream. Nothing else changes: the ledger column, its validation rule, and the two native headers stay exactly as `docs/PROTOCOL.md` states them.

**Tech Stack:** Rust, axum `HeaderMap`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-09-03-ironwire-agent-alignment.md` section 4.3, in the trace-commons-server repo.

## Global Constraints

- This work is in `/Users/zakimanian/code/ironwire`, NOT in trace-commons-server. Use `git -C /Users/zakimanian/code/ironwire` for every git command and absolute paths for every file. Never `cd` into it.
- Branch from `origin/main` (`ba3e52e` at time of writing) on a new branch `neutral-session-header`. Do not touch `main` or `session-id-contract`.
- IronWire CI runs `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features` with `RUSTFLAGS=-D warnings`. Run both before every commit:
  ```bash
  cargo fmt --all --manifest-path /Users/zakimanian/code/ironwire/Cargo.toml
  RUSTFLAGS="-D warnings" cargo clippy --all-targets --all-features --manifest-path /Users/zakimanian/code/ironwire/Cargo.toml
  RUSTFLAGS="-D warnings" cargo test --manifest-path /Users/zakimanian/code/ironwire/Cargo.toml -p ironwire_upstream -p ironwire_proxy
  ```
- Commit style: short imperative subject, no prefix, no emoji. Match `git -C /Users/zakimanian/code/ironwire log --oneline -5`.
- Do NOT push and do NOT open a PR. Leave the branch local; the user decides.
- No new headers other than `x-ironwire-session-id`. No change to the 200-byte / `[A-Za-z0-9_:.-]` validation rule.

---

### Task 1: Name the neutral header and keep it off the wire upstream

**Files:**
- Modify: `/Users/zakimanian/code/ironwire/crates/ironwire_upstream/src/headers.rs`

**Interfaces:**
- Produces: `pub const NEUTRAL_SESSION_HEADER: &str = "x-ironwire-session-id";` and `forward_request_header(NEUTRAL_SESSION_HEADER) == false`.

- [x] **Step 1: Write the failing tests**

Add to the `tests` module in `headers.rs`:

```rust
    #[test]
    fn the_neutral_session_header_is_addressed_to_us_and_stops_here() {
        // A client that names its session to IronWire is talking to this hop,
        // not to the provider. Forwarding it would hand a provider a header
        // it never asked for, and would leak that a proxy is in the path.
        assert_eq!(NEUTRAL_SESSION_HEADER, "x-ironwire-session-id");
        assert!(!forward_request_header(NEUTRAL_SESSION_HEADER));
        assert!(!forward_request_header("X-IronWire-Session-Id"));
    }

    #[test]
    fn the_native_session_headers_still_reach_the_provider() {
        // Adding a header of our own must not change what happens to theirs.
        for protocol in [
            Protocol::AnthropicMessages,
            Protocol::OpenAiResponses,
            Protocol::OpenAiChat,
        ] {
            assert!(forward_request_header(client_session_header(protocol)));
        }
    }
```

- [x] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path /Users/zakimanian/code/ironwire/Cargo.toml -p ironwire_upstream headers`
Expected: compile error, `NEUTRAL_SESSION_HEADER` not found.

- [x] **Step 3: Implement**

In `headers.rs`, after the `REWRITTEN` const:

```rust
/// Headers addressed to IronWire itself. Read here, never forwarded: a
/// provider did not ask for them, and forwarding one would tell it there is a
/// proxy in the path.
const ADDRESSED_TO_US: &[&str] = &[NEUTRAL_SESSION_HEADER];

/// The one header any client can use to name its session to IronWire.
///
/// The two native headers below are what Claude Code and Codex already send.
/// Everything else that speaks Chat Completions -- Aider, Cline, Roo -- sends
/// no session header at all, so their rows can never be attributed. A client
/// or wrapper that can add a request header sets this one and becomes
/// attributable without IronWire having to know it exists. It takes
/// precedence over the native header when both are present, because it is
/// the one the client chose to address to us.
pub const NEUTRAL_SESSION_HEADER: &str = "x-ironwire-session-id";
```

Change `forward_request_header`:

```rust
pub fn forward_request_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !HOP_BY_HOP.contains(&lower.as_str())
        && !AUTH.contains(&lower.as_str())
        && !REWRITTEN.contains(&lower.as_str())
        && !ADDRESSED_TO_US.contains(&lower.as_str())
}
```

- [x] **Step 4: Run tests to verify they pass**

Run: `RUSTFLAGS="-D warnings" cargo test --manifest-path /Users/zakimanian/code/ironwire/Cargo.toml -p ironwire_upstream`
Expected: all pass, including the pre-existing `reading_a_session_header_does_not_stop_it_reaching_the_provider`.

- [x] **Step 5: fmt, clippy, commit**

```bash
cargo fmt --all --manifest-path /Users/zakimanian/code/ironwire/Cargo.toml
RUSTFLAGS="-D warnings" cargo clippy --all-targets --all-features --manifest-path /Users/zakimanian/code/ironwire/Cargo.toml
git -C /Users/zakimanian/code/ironwire add crates/ironwire_upstream/src/headers.rs
git -C /Users/zakimanian/code/ironwire commit -m "Name a session header any client can send, and keep it off the wire"
```

---

### Task 2: Read the neutral header first in every facade

**Files:**
- Modify: `/Users/zakimanian/code/ironwire/crates/ironwire_proxy/src/facade/mod.rs:16-30`
- Modify: `/Users/zakimanian/code/ironwire/docs/PROTOCOL.md:95-135`

**Interfaces:**
- Consumes: `ironwire_upstream::headers::NEUTRAL_SESSION_HEADER` from Task 1.
- `client_session_id(headers, protocol)` signature unchanged; both facades (`anthropic.rs:178`, `openai.rs:233`) keep calling it as they do.

- [x] **Step 1: Write the failing tests**

Add to the `join_contract` module in `facade/mod.rs`:

```rust
    /// A client with no native session header can still name its session.
    #[test]
    fn the_neutral_header_is_read_on_every_facade() {
        let id = "aider-2026-09-03-0001";
        for protocol in [
            Protocol::AnthropicMessages,
            Protocol::OpenAiResponses,
            Protocol::OpenAiChat,
        ] {
            assert_eq!(
                client_session_id(&with("x-ironwire-session-id", id), protocol).as_deref(),
                Some(id),
                "{protocol:?} must read the neutral header"
            );
        }
    }

    /// The header the client addressed to us wins over the one it addresses
    /// to the provider. A wrapper that sets ours has said which id it wants
    /// the row filed under.
    #[test]
    fn the_neutral_header_takes_precedence_over_the_native_one() {
        let mut headers = with("x-claude-code-session-id", "native-id");
        headers.insert(
            "x-ironwire-session-id",
            axum::http::HeaderValue::from_static("chosen-id"),
        );
        assert_eq!(
            client_session_id(&headers, Protocol::AnthropicMessages).as_deref(),
            Some("chosen-id")
        );
    }

    /// The same validation applies: a hostile neutral header is dropped, and
    /// does not fall through to the native one either.
    #[test]
    fn a_hostile_neutral_header_is_dropped_not_bypassed() {
        let mut headers = with("x-claude-code-session-id", "native-id");
        headers.insert(
            "x-ironwire-session-id",
            axum::http::HeaderValue::from_static("not a session id"),
        );
        assert_eq!(
            client_session_id(&headers, Protocol::AnthropicMessages),
            None,
            "a client that sent us garbage does not get its native id filed instead"
        );
    }
```

Check `Protocol` derives `Debug` (it is used in `{protocol:?}`); if not, replace the format arg with a plain string.

- [x] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path /Users/zakimanian/code/ironwire/Cargo.toml -p ironwire_proxy join_contract`
Expected: FAIL, `the_neutral_header_is_read_on_every_facade` returns `None`.

- [x] **Step 3: Implement**

Replace the body of `client_session_id` in `facade/mod.rs`:

```rust
pub(crate) fn client_session_id(
    headers: &axum::http::HeaderMap,
    protocol: ironwire_core::protocol::Protocol,
) -> Option<String> {
    // The header addressed to us wins; the native one is the fallback. A
    // neutral header that fails validation is a client that sent us garbage,
    // and the answer is nothing -- not its native id filed instead.
    let name = if headers.contains_key(ironwire_upstream::headers::NEUTRAL_SESSION_HEADER) {
        ironwire_upstream::headers::NEUTRAL_SESSION_HEADER
    } else {
        ironwire_upstream::headers::client_session_header(protocol)
    };
    let raw = headers.get(name)?.to_str().ok()?;
    let ok = !raw.is_empty()
        && raw.len() <= 200
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'));
    ok.then(|| raw.to_string())
}
```

Update the doc comment above it to mention the neutral header and its precedence.

- [x] **Step 4: Run tests to verify they pass**

Run: `RUSTFLAGS="-D warnings" cargo test --manifest-path /Users/zakimanian/code/ironwire/Cargo.toml -p ironwire_proxy facade`
Expected: all pass, including the pre-existing `the_session_id_is_stored_verbatim` and `a_session_id_that_is_not_an_identifier_is_dropped`.

- [x] **Step 5: Document the contract**

In `docs/PROTOCOL.md`, replace the two-row facade/header table under "The client's session id, and what it is safe to join on" with:

```markdown
| Façade | Header read |
|---|---|
| Any | `x-ironwire-session-id`, when present (takes precedence) |
| Anthropic (`Protocol::AnthropicMessages`) | `x-claude-code-session-id` |
| OpenAI (`Protocol::OpenAiResponses`, `Protocol::OpenAiChat`) | `session-id` |

`x-ironwire-session-id` is addressed to IronWire and is **not forwarded** to
the provider. The two native headers are forwarded untouched, as before. A
client that sends no native session header -- Aider, Cline, Roo -- can set
the neutral one wherever its provider settings allow an extra request
header, and its rows become attributable without an IronWire release.
```

- [x] **Step 6: fmt, clippy, full test, commit**

```bash
cargo fmt --all --manifest-path /Users/zakimanian/code/ironwire/Cargo.toml
RUSTFLAGS="-D warnings" cargo clippy --all-targets --all-features --manifest-path /Users/zakimanian/code/ironwire/Cargo.toml
RUSTFLAGS="-D warnings" cargo test --manifest-path /Users/zakimanian/code/ironwire/Cargo.toml --workspace
git -C /Users/zakimanian/code/ironwire add crates/ironwire_proxy/src/facade/mod.rs docs/PROTOCOL.md
git -C /Users/zakimanian/code/ironwire commit -m "Read the neutral session header first on every facade"
```

Report the final `git -C /Users/zakimanian/code/ironwire log --oneline origin/main..neutral-session-header` and the test summary lines verbatim.

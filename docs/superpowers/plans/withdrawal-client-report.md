# Trace withdrawal — client-side implementation report

Date: 2026-08-08
Branch/worktree: `withdrawal-client`
Scope: client only, per
`docs/superpowers/specs/2026-08-08-trace-withdrawal-design.md`. The server
endpoint and migration are out of scope here and were not touched; this
worktree does not modify `crates/trace-commons-server`.

## What was built

1. **`crates/trace-commons-contributor/src/withdraw.rs`** (new) — an HTTP
   client for `POST /v1/account/traces/{submission_id}/withdraw`, built on
   the same `trace_commons_operator_client::Client` builder every other
   ingest call in this crate uses, but with an explicit bearer token
   (an account-session token) instead of a device-key-derived claim. Reports
   a `DistributionReach` (`not_distributed` / `in_commons` / `distributed`)
   and classifies errors (`SessionInvalid`/`NotFound`/`Unavailable`). Also
   holds `confirmation_prompt(reach, export_published_on)`, the tier-aware
   confirmation copy from the design doc, verbatim for the two tiers the
   doc gives literal wording for (quarantined, and accepted-and-exported);
   the un-specified middle tier (`InCommons`) uses only the paragraph shared
   by both given examples rather than inventing wording. Tested against a
   real axum stub server on an ephemeral port, following
   `submit.rs`'s test pattern exactly (18 tests).

   The server endpoint does not exist in this worktree (built in a sibling
   worktree in parallel), so the wire response shape
   (`{"distribution_reach": "..."}`) is this client's own reading of the
   design doc, not confirmed against the real server. **This should be
   reconciled once both land** — see "What I could not verify" below.

2. **`crates/trace-commons-contributor/src/daemon/withdraw.rs`** (new) —
   the `withdraw` and `withdraw_bulk` IPC handlers. Both validate params,
   then check for an account session via `account_session_token(shared)`.
   That function always returns `None` today: `ContributorConfig` stores
   only a device key and what was minted against it, and building real
   account-session acquisition/storage was explicitly out of scope for this
   task. So both handlers always answer `unavailable` /
   `account-session-required` — a distinct, documented error rather than a
   generic failure. The rest of each handler (the real call to
   `crate::withdraw::call_withdraw`, updating the local history cache via
   the new `daemon::history::mark_withdrawn`, returning the tier) is real,
   working code exercised directly against a live axum stub in
   `withdraw.rs`'s tests — it is simply unreached in production until
   account-session storage exists, at which point `account_session_token`
   is the one function that needs to change.

3. **`daemon::history`**: added `STATUS_WITHDRAWN`, a `withdrawn_at` field
   on `HistoryRecord` (`#[serde(default)]`, so old cache files still parse),
   and `mark_withdrawn(records, submission_id, at)`. Noted in a doc comment:
   `join` (called by `refresh_history`) rebuilds every record from the
   server's status read-back, which does not yet report a withdrawn status
   of its own, so a `refresh_history` after a withdrawal would currently
   drop the local marker — flagged for whoever wires the real
   account-session flow, not fixed here since it's inert until then.

4. **CLI**: `trace-commons-contributor daemon withdraw <submission_id>` and
   `daemon withdraw --all-quarantined`, following the `daemon approve`
   pattern exactly (`commands::daemon_call`, which prefers the running
   daemon's socket via `daemon::client::try_call` and falls back to direct
   file access only when nothing is listening — same path every other
   mutating daemon command already uses, so this one is not silently
   overwritten by a running daemon). On the specific
   `account-session-required` error, prints an explanation instead of the
   generic `render` bail.

5. **`METHODS`** in `daemon/ipc.rs`: added `withdraw`, `withdraw_bulk` in
   sorted position, length constant `24` → `26`, wired into
   `handle_request_async` (both do real, if currently gated, network I/O,
   so they follow the same async-dispatch pattern as `preview`/`enroll`).
   The existing conformance test
   (`daemon_ipc_contract::hello_advertises_exactly_the_documented_method_set`)
   passes, confirming `hello`'s advertised list matches `METHODS`.

6. **`docs/contributor-daemon-ipc-v1_1.md`**: both methods added to the
   method table, plus a new "Withdrawal" section documenting the response
   shape, the three tiers, `withdraw_bulk`'s accepted statuses, and the
   account-session limitation in the same terms as the code's doc comments.

## What I deliberately did not build

- **No account-session acquisition/storage.** The task was explicit that
  this is separate work and not to be invented here. `account_session_token`
  is a single, clearly documented seam returning `None`; nothing else
  depends on how that gets filled in later.
- **No interactive CLI confirmation prompt.** `confirmation_prompt` exists
  and is tested, but I did not wire it into an interactive
  yes/no flow in `daemon withdraw` — there is no existing pattern for that
  in this CLI (submit's picker is a selection prompt, not a
  confirm-or-cancel one), and since every real withdrawal call fails closed
  with `account-session-required` today, an interactive prompt in front of
  it would have no real UI/UX validation possible yet. The function is
  ready for whichever shell (this CLI's own future prompt, or a native
  app) needs it.
- Did not touch `crates/trace-commons-server` or any migration.

## What I could not verify

- **The real server's response wire shape.** I could not check the sibling
  worktree building the endpoint, so `{"distribution_reach": "not_distributed"
  | "in_commons" | "distributed"}` is my own reading of the design doc's
  three-tier table, not confirmed against the actual server contract. If the
  real server uses different field/value names, `withdraw.rs`'s
  `WithdrawResponseBody` needs a one-file update — nothing else in this
  branch depends on the exact wire strings beyond that struct and the
  `reach_label` passthrough in `daemon/withdraw.rs`.
- Whether the sibling server work returns 404 for both "not yours" and
  "does not exist" as the design doc requires — asserted only against my own
  stub, not the real server.
- No live daemon/CLI smoke test against a real running daemon+socket beyond
  the existing `daemon_ipc_contract.rs` integration tests (which spin up a
  real daemon over a real unix socket and did pass).

## Verification

```
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor -p trace-commons-contributor-ffi
```
343 (lib) + 6 + 5 + 16 + 4 + 2 + 12 + 1 + 4 (1 ignored, pre-existing) + 13 +
32 + 2 = 440 passed, 0 failed, 1 ignored (pre-existing, unrelated) across
both crates' unit and integration test binaries — no regression from the
419-passed baseline.

```
cargo clippy -p trace-commons-contributor --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```
Clean, no warnings.

`cargo fmt --all` run; `git status --short` after showed only the files
listed below changed (no formatter-driven rewrites of unrelated files).

## Files touched

- `crates/trace-commons-contributor/src/withdraw.rs` (new)
- `crates/trace-commons-contributor/src/daemon/withdraw.rs` (new)
- `crates/trace-commons-contributor/src/daemon/history.rs`
- `crates/trace-commons-contributor/src/daemon/ipc.rs`
- `crates/trace-commons-contributor/src/daemon/mod.rs`
- `crates/trace-commons-contributor/src/commands.rs`
- `crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs`
- `crates/trace-commons-contributor/src/lib.rs`
- `docs/contributor-daemon-ipc-v1_1.md`

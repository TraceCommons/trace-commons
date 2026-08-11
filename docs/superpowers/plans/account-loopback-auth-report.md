# Account loopback sign-in: what was built, and what it does not fix

Branch: `account-loopback-auth`. Closes the account-session gap that made
`withdraw` / `withdraw_bulk` return `account-session-required` before any
request left the machine.

## The flow

1. The app binds `127.0.0.1:0`, then `POST /v1/account/native/authorize` with
   `sha256(verifier)` (S256 only) and the exact loopback redirect
   `http://127.0.0.1:<port>/trace-commons/native-auth/callback`. This endpoint
   is unauthenticated and confers nothing: it parks a challenge and a redirect
   in a 5-minute in-process store, bound to no account and no tenant.
2. The app mints a login link through the EXISTING device-authenticated
   `POST /v1/account/login-links` and opens the browser at
   `/account/login?code=...&native=<request_id>`.
3. The human clicks Activate. `POST /account/login/confirm` redeems the link
   exactly as before, and — when `native` names a live pending request — also
   mints a one-time code and 303s the browser to the registered loopback URI.
4. The listener serves that one request, answers a small page, and is dropped.
   The port does not stay open.
5. `POST /v1/account/native/token` takes `(request_id, code, code_verifier)`
   and returns a `tcn1_` bearer backed by a real `trace_sessions` row
   (`client_kind = 'native'`, 12h TTL).

`account_auth_middleware` now accepts either the cookie or a `tcn1_` bearer.
The prefix is the whole dispatch, so a device upload claim and a native session
token can never be confused in either direction.

## Why it is a session row, not a new credential type

Because the token IS an ordinary `trace_sessions` row, everything that already
governs sessions governs it with no second code path: expiry, the idle cap,
rotation-on-use, and `POST /v1/account/sessions/revoke-all`. Rotation is handed
back in an `x-trace-commons-session-token` response header — the bearer
analogue of `Set-Cookie`, on the same channel to the same already-authenticated
caller.

A native session is pinned WEAK for the strong-authenticator gate
(`is_strong_session()` admits only `passkey`/`near`). It can read and withdraw;
it can never change authenticators or redirect a payout. The resolver also
refuses a non-`native` session presented as a `tcn1_` bearer, so a browser
cookie's secret cannot be replayed over this transport.

## Attacks with tests

Server, no PostgreSQL needed (they run in CI; the pg-gated account tests do
not):

- Intercepted code without the verifier fails, and no session is created —
  `an_intercepted_code_without_the_verifier_is_useless`.
- Replayed code fails and mints no second session — `a_replayed_code_fails`.
- Code presented against a different `request_id` fails —
  `a_code_presented_against_another_request_fails`.
- Non-loopback redirects refused at the endpoint —
  `native_authorize_refuses_a_redirect_that_is_not_loopback`; and exhaustively
  in `account_native_auth::rejects_every_non_loopback_spelling`
  (`localhost`, `[::1]`, other hosts, userinfo trick, `https`, uppercase
  scheme, wrong/trailing/traversal paths, added query or fragment, missing /
  privileged / leading-zero / non-numeric / out-of-range ports).
- `plain` PKCE refused — `native_authorize_refuses_plain_pkce`.
- Expired token denied — `an_expired_native_token_is_denied`.
- `revoke-all` kills the token — `revoke_all_kills_a_native_token`.
- Logout revokes the native session row —
  `logging_out_a_native_token_revokes_its_session_row`.
- A `'web'` session secret is not usable as a native bearer —
  `a_browser_session_secret_is_not_usable_as_a_native_bearer`.
- A pending request is single-use —
  `a_stale_native_request_id_cannot_be_completed_twice`.
- S256 derivation pinned to RFC 7636's appendix B vector.

Client:

- The listener binds `127.0.0.1` only, serves one request, and the port is
  closed afterwards — `the_listener_serves_one_request_and_then_the_port_is_closed`.
- A callback on any other path is refused — `a_callback_on_another_path_is_refused`.
- Expired / about-to-expire tokens are not offered; withdrawal reports
  `account-session-required` instead of presenting one and 401ing.
- The token is stored 0600 — `a_live_token_is_offered_and_stored_at_0600`.
- Client and server constants are cross-checked in-process
  (`trace-commons-server` is already a dev-dependency), so the redirect path,
  the PKCE method, the verifier shape, and the challenge derivation cannot
  drift apart.

## What this does NOT fix, and you should decide about it

**A pre-existing authorization gap on `main`, deliberately not described here.**

While building this I found a way `/v1/account/*` resolves callers today that
undercuts the property the loopback design was chosen to protect. It is
unrelated to the code in this branch: this flow adds no authority the device
key did not already have, so the instruction was satisfied -- but the property
that instruction protects is not currently true of the server.

**The specifics are withheld from this file on purpose.** This repository is
public, and the repository's own rule is that unfixed-vulnerability detail does
not go into commit messages, PR bodies, or committed documents. They have been
raised privately instead.

Closing it is a behaviour change for already-shipped clients (the macOS app and
the daemon), so it belongs in its own change rather than riding along with this
one.

## Other limitations

- The pending-request and issued-code stores are in-process, exactly like the
  WebAuthn `CeremonyStore` and the account rate limiters. Correct for the
  single-host pilot; a horizontally-scaled deployment must move them to a
  shared short-TTL store. Documented at both stores.
- A stale or expired `native` request id on the confirm POST is not an error:
  the human's browser login genuinely succeeded, so it falls through to the
  ordinary account-view redirect and the app times out. The contributor sees a
  failed sign-in from the app, not a failed login in the browser.
- `sign_out` removes the local token even when the server-side revoke call
  fails. Leaving a live token on disk because the network was down would be
  worse; it expires on its own regardless.

## Dependencies and migrations

- **No new dependencies.** `ring::constant_time` is deprecated upstream and
  `subtle` is not in this tree, so `secret_eq` is a short non-short-circuiting
  compare with `black_box`, rather than a new crate.
- **One migration**, `V44__native_session_client_kind.sql`: widens the existing
  `trace_sessions_client_kind_check` to admit `'native'`, mirroring what V32
  and V33 did for `'passkey'` and `'near'`. No new table, so no new RLS policy;
  `trace_sessions` already has forced RLS from V30. Wired into the hand-rolled
  `run_migrations`. V42 is still held by the unmerged invites branch and was
  not reused.

## Verification

- `cargo fmt --all --check` — clean.
- `RUSTFLAGS='-D warnings' cargo check --workspace --bins` — clean.
- `cargo clippy --workspace --all-targets` with the CI allow-list — clean.
- `RUSTFLAGS='-D warnings' cargo test --workspace` — baseline on
  `origin/main` was 2216 passed / 0 failed; see the PR body for the final
  count.
- `daemon::ipc::tests::a_project_id_never_carries_a_path_component` is flaky
  independently of this branch: it fails when a random tempdir path segment
  (observed: `d8`) happens to appear in the project-id hash. Five consecutive
  reruns pass. Not introduced here, and worth a separate fix.
- Not verified: nothing was run against a live PostgreSQL or the pilot host.
  The V44 migration, `issue_native_session`, and the pg-gated account resolver
  tests are unexercised by this run.

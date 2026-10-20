# Subscription scale and verification

IronWire supports separately declared Claude Code and Codex logins, each with
its own credential directory, consent and account-bound facade URL. An account
is not a guessed amount of capacity: quota remains provider-observed or
`Unknown`. Adding subscriptions does not relax client identity, credential-host,
privacy or wire-fidelity rules.

## Tested workload

`crates/ironwire_proxy/tests/bulk_real_logins.rs` creates 512 Claude credential
files and 512 Codex credential files with distinct synthetic accounts. It
constructs the actual upstream backend implementations, observes all 1,024
statuses, checks consent locations, and sends 1,536 requests through the real
facades with 16 concurrent requests. A loopback mock checks each bearer,
ChatGPT account header, session and native body. The 512 Codex second turns
replay encrypted reasoning on the same account.

This uses real file readers and HTTP implementations, synthetic credentials
and a loopback provider. It makes no real provider or Keychain calls. On the
local development Mac, a debug run completed the fixture workload in 27.52
seconds with 62,783,488 bytes (59.9 MiB) peak process RSS. These observations
are not a production latency target or a provider capacity estimate.

Other regression tests cover:

- Four concurrent requests and control probes across 1,024 synthetic
  subscriptions, with ordered results and bounded concurrency.
- Default bound routing reads exactly its own subscription credential;
  cross-account fallback explicitly observes the matching product's pool.
  Missing status is never interpreted as authentication.
- 10,000 config entries validate without repeated linear duplicate scans,
  including a duplicate directory or id at the end of the list.
- 1,024 new sessions cannot displace a still-retained encrypted replay origin.
  Relogin, missing scope, a pin, expiry and capacity eviction cannot assign
  encrypted history to another account.
- A runtime with only one blocking worker can finish real credential status
  and a mock probe. Cancelled callers cannot release the capacity held by
  still-running blocking credential jobs.
- Overloaded early accounts do not consume the retry budget needed to reach
  a healthy later account.

## Resource bounds and limits

Registry status and probe work each have eight shared slots. Actual blocking
credential reads have eight shared slots too, held until the read finishes even
if its caller is cancelled. A macOS Keychain subprocess has a five-second
budget and bounded output, and is killed and reaped on timeout. Only an absent Keychain
item permits credential-file fallback; a lock, error or timeout refuses it. First-party
backends with the same read timeout share their HTTP client and TLS root store;
authentication remains on each individual request. The shared timeout cache
retains at most 16 clients; additional distinct timeouts are uncached.

Default routing avoids reading other subscriptions' credentials. Routing still
builds a candidate list proportional to the registered backend count. With
cross-account fallback enabled it observes every matching product's login in
bounded batches; a large pool of slow or locked credential stores can increase
latency. Control status intentionally observes every registered backend.

The route-affinity cache retains at most 512 conversations. The separate native
Responses replay-origin cache retains at most 4,096 origins with a 24-hour idle
TTL. Affinity eviction can lose cache warmth. Origin eviction, expiry or daemon
restart refuses native encrypted continuation instead of guessing its account.
Start a fresh session; retained origins can continue after restoring the
original login. Larger or persistent origin state needs an explicit retention
and privacy design.

The macOS menu uses lazy sections and indexed settings lookup. Its offscreen
1,024-row regression fell from 10.986 seconds before virtualization to 0.108
seconds afterward on the development Mac. The complete account list remains
available; live scroll and repeated polling still need interactive acceptance.

## Reproduce locally

```sh
cargo test -p ironwire_proxy --test bulk_subscriptions -- --nocapture
cargo test -p ironwire_proxy --test bulk_real_logins -- --nocapture
cargo test -p ironwire_proxy --test registry_blocking_workers
cargo test -p ironwire_core ten_thousand_distinct_logins
cargo test --workspace --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

To contrast opt-in whole-product scans with default routing, set
`IRONWIRE_SCALE_FULLSCAN=1` for `bulk_real_logins`. Timing and RSS should be
measured on the test executable itself, after compilation, with other build
jobs idle. Retain machine details and raw output when comparing runs.

## Remaining acceptance

The roadmap remains authoritative for uncompleted work. In particular, this
suite does not prove two live accounts per provider, locked-Keychain behavior
under a large real pool, signed Claude thinking across accounts, real provider
exhaustion and recovery, an eight-hour coding session, or native encrypted
resume across daemon restarts. Signing, publication, trace contribution and
later privacy milestones retain their separate prerequisites and acceptance
criteria. Local fixture success does not close those goals.

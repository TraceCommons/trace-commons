# Reproduce the pinned Codex writer fixtures

Pinned upstream revision: `c4017a87aacc7558002b7cb510025e967c1d765e`

The complete developer-only generator is committed beside this file as
`generator.rs`. It is not contributor production code. To reproduce from a
clean upstream checkout:

```sh
git clone https://github.com/openai/codex /private/tmp/openai-codex-fixture-gen
git -C /private/tmp/openai-codex-fixture-gen checkout c4017a87aacc7558002b7cb510025e967c1d765e
cp generator.rs /private/tmp/openai-codex-fixture-gen/codex-rs/core/tests/suite/insights_direct_profile_writer_fixture.rs
printf '\npub mod insights_direct_profile_writer_fixture;\n' >> /private/tmp/openai-codex-fixture-gen/codex-rs/core/tests/suite/mod.rs
```

The committed generator must have SHA-256
`d21d35c7d2bfb9d4d603dfad45aeabdbf28f96ae9bccc13029294566516a8bcc`.

Run from the resulting `codex-rs` directory:

```sh
CARGO_HOME=/private/tmp/codex-fixture-cargo \
CARGO_TARGET_DIR=/private/tmp/codex-fixture-target \
CARGO_INCREMENTAL=0 \
RUST_MIN_STACK=8388608 \
cargo test -p codex-core --test all insights_direct_profile_writer_fixture -- --nocapture
```

`RUST_MIN_STACK=8388608` matches the pinned upstream CI setting. The default test-thread stack overflowed after the build on this host; the 8 MiB setting passed.

The test uses `test_codex`, localhost Wiremock Responses SSE, `start_or_steer_turn`, and `flush_rollout`. It makes no live provider request and reads no private trace. It deterministically sanitizes values while preserving JSONL record order, keys, value kinds, optional-field presence, and equality relationships among IDs and paths.

Verify byte reproducibility by recording these hashes, rerunning the test, and comparing:

```sh
shasum -a 256 /private/tmp/trace-insights-codex-writer-fixtures/*.jsonl \
  /private/tmp/trace-insights-codex-writer-fixtures/manifest.json
```

Expected hashes:

```text
55ffa88eda7b84ffbb38297158df61af5e7dc958eba685192fec380483ab2962  codex-alpha-direct.jsonl
05ae5ba57052466c1ffd814540459e479cc1f43ca132b3a5ebf4913a21c17e93  codex-beta-direct.jsonl
a2abd4c0c8727c209dac540fa07b7e45245a1a68c5ee84bd2e099d39faff8e21  manifest.json
```

The emitted `cli_version` is `0.0.0`. This identifies this source-built fixture only and does not qualify any released or production Codex version.

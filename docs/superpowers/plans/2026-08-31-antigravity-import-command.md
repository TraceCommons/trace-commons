# Antigravity Import Command Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A one-shot `trace-commons-contributor import-antigravity` command that reads conversations from Antigravity's local language server API and stages them as Trajectory-v1 files for the existing queue, preview, redaction and approval path.

**Architecture:** Four units — `endpoint` (process discovery and a bounded, token-verified port probe), `client` (Connect/JSON RPC behind a trait), `convert` (API JSON to Trajectory-v1 records, pure), and the command (orchestration and staging). The daemon gains no new capability; the existing `trajectory` source reads what the command writes.

**Tech Stack:** Rust 2024, `sysinfo` 0.36 (new), `reqwest` and `serde_json` (already present), the existing `trajectory` source as the consumer.

**Spec:** `docs/superpowers/specs/2026-08-31-antigravity-import-command-design.md`

## Global Constraints

- Verify with the UNFILTERED suite: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor` (no `--lib`, no filter). A scoped command hid a broken integration test for seven tasks on the previous attempt.
- Also: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins`, and `cargo clippy -p trace-commons-contributor --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching` (never widen this list), and `cargo fmt --all`.
- After any dependency change, regenerate the Flatpak vendor set (see Task 1) or `--test release_pipeline` fails.
- No emojis anywhere including UI copy. Commit subjects short, imperative, no `feat:`/`fix:` prefix.
- Every error is a fixed, content-free `&'static str` reason label. Never a path, prompt, tool argument, token, or session content.
- Fail closed. A conversation that cannot be read is refused, never silently partial.
- `sysinfo = "0.36"` is the ONLY new dependency, pinned to the 0.36.1 already in this workspace's lockfile. Not 0.39.x, which needs Rust 1.95 against this workspace's 1.92 floor. Add nothing else without explicit approval.
- Multi-turn is a merge gate, not a feature. See the spec's acceptance criteria; Task 5 pins all five.

---

### Task 1: Remove the file-reading implementation

The previous approach read Antigravity's SQLite files. It is fully implemented and passing, and it is being replaced, not amended. Removing it first gives every later task a clean base and keeps the two approaches from being confused in review.

**Files:**
- Delete: `crates/trace-commons-contributor/src/source/antigravity/` (whole directory: `mod.rs`, `decode.rs`, `store.rs`, `convert.rs`)
- Delete: `crates/trace-commons-contributor/tests/fixtures/antigravity/conversation.db` and its `README.md`
- Modify: `crates/trace-commons-contributor/src/source/mod.rs` (remove `pub mod antigravity;`, the `SOURCE_ANTIGRAVITY` constant, its `NATIVE_SOURCES` row, and its tests)
- Modify: `crates/trace-commons-contributor/src/daemon/settings.rs` (remove `antigravity_source`, its `source_settings_key` arm, `antigravity_declared`, its `source_roots()` line, its `apply_settings_object` arm, and its tests)
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (remove `antigravity_mode`, the `obj.remove("antigravity_source")`, the `antigravity_source_mode` insert, and the antigravity leak test)
- Modify: `crates/trace-commons-contributor/src/source/discovery.rs` (remove the antigravity `describe(...)` row, `DB_EXTENSION` if now unused, and its test)
- Modify: `macos/Sources/TCShellCore/SourceCandidate.swift`, `macos/Sources/TCShellCore/SessionRoots.swift` (remove the `antigravity` case and its field/arms)
- Modify: `crates/trace-commons-contributor-gtk/src/copy.rs`, `src/ui/roots.rs` (remove `ROOTS_ANTIGRAVITY` and its arm)
- Modify: `crates/trace-commons-contributor/Cargo.toml` (remove `rusqlite` and `prost`; KEEP `tempfile`, the new command still stages files)

**Interfaces:**
- Consumes: nothing.
- Produces: a tree with no Antigravity code, building green on all three build systems.

- [ ] **Step 1: Delete the source module and fixture**

```bash
git rm -r crates/trace-commons-contributor/src/source/antigravity
git rm -r crates/trace-commons-contributor/tests/fixtures/antigravity
```

- [ ] **Step 2: Remove every reference, compiler-guided**

Run `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins` repeatedly and fix each error. The compiler will find the Rust references; the Swift and GTK ones will not appear until you build those, so do them explicitly from the file list above.

Do NOT leave a stub, a `#[cfg(feature)]`, or a commented-out block. This code is in git history and on the `antigravity-file-reading-archive` branch if it is ever wanted.

- [ ] **Step 3: Remove the dependencies and regenerate the vendor set**

Remove `rusqlite` and `prost` from `crates/trace-commons-contributor/Cargo.toml`. Then update BOTH lockfiles and the Flatpak vendor set, or `--test release_pipeline` will fail:

```bash
cargo check -p trace-commons-contributor --bins
cargo metadata --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml --format-version 1 > /dev/null
```

For the vendor set, follow the procedure in `crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml` (around line 170): fetch `flatpak-cargo-generator.py` from `flatpak/flatpak-builder-tools`, run it in a Python virtualenv in a temp directory (never the system or user Python), against `crates/trace-commons-contributor-gtk/Cargo.lock`, writing `crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json`.

Report the entry count before and after. Entries should DROP by roughly the 14 that were added for rusqlite and prost. A large unexplained change means it ran against the wrong lockfile — stop and report.

- [ ] **Step 4: Verify all three build systems**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
cargo build -p trace-commons-contributor-ffi && (cd macos && swift test)
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
```

All three must pass, `release_pipeline` included. Paste the output.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Remove the file-reading Antigravity source"
```

---

### Task 2: Commit the redacted API fixtures

Every later task tests against these. They are real captures, not authored files.

**Files:**
- Create: `crates/trace-commons-contributor/tests/fixtures/antigravity/steps-single-turn.json`
- Create: `crates/trace-commons-contributor/tests/fixtures/antigravity/steps-multi-turn.json`
- Create: `crates/trace-commons-contributor/tests/fixtures/antigravity/descriptions.json`
- Create: `crates/trace-commons-contributor/tests/fixtures/antigravity/README.md`

**Interfaces:**
- Produces: the three fixture paths, and a `fixture_path(name: &str) -> PathBuf` test helper defined in Task 4's module.

- [ ] **Step 1: Copy the captures**

They are already on disk, captured from a live instance:

```bash
mkdir -p crates/trace-commons-contributor/tests/fixtures/antigravity
cp .superpowers/sdd/2026-08-29-antigravity-source/api-capture-steps.json \
   crates/trace-commons-contributor/tests/fixtures/antigravity/steps-single-turn.json
cp .superpowers/sdd/2026-08-29-antigravity-source/api-capture-multiturn.json \
   crates/trace-commons-contributor/tests/fixtures/antigravity/steps-multi-turn.json
cp .superpowers/sdd/2026-08-29-antigravity-source/api-capture-descriptions.json \
   crates/trace-commons-contributor/tests/fixtures/antigravity/descriptions.json
```

- [ ] **Step 2: Redact, and verify the redaction**

This repository is PUBLIC. Two things must not be committed.

The operator username appears in workspace URIs and file paths. Replace `zakimanian` with `anonymized` — exactly 10 bytes either way, though these are JSON so length does not matter here as it did for the protobuf fixture.

`toolCalls[].thinkingSignature` is an opaque encrypted blob of model internals. Replace each value with the literal string `REDACTED-THINKING-SIGNATURE`, which doubles as the marker a later test asserts never escapes.

```python
import json, pathlib
for name in ["steps-single-turn.json", "steps-multi-turn.json", "descriptions.json"]:
    p = pathlib.Path("crates/trace-commons-contributor/tests/fixtures/antigravity") / name
    text = p.read_text().replace("zakimanian", "anonymized")
    doc = json.loads(text)
    def scrub(node):
        if isinstance(node, dict):
            for k, v in node.items():
                if k == "thinkingSignature" and isinstance(v, str):
                    node[k] = "REDACTED-THINKING-SIGNATURE"
                else:
                    scrub(v)
        elif isinstance(node, list):
            for v in node: scrub(v)
    scrub(doc)
    p.write_text(json.dumps(doc, indent=2))
```

Then verify, and paste the output:

```bash
grep -c 'zakimanian' crates/trace-commons-contributor/tests/fixtures/antigravity/*.json
grep -c 'You are Antigravity\|<USER_REQUEST>' crates/trace-commons-contributor/tests/fixtures/antigravity/*.json
python3 -c "
import json
d=json.load(open('crates/trace-commons-contributor/tests/fixtures/antigravity/steps-multi-turn.json'))
print('steps:', len(d['steps']))
print('user turns:', sum(1 for s in d['steps'] if s.get('type')=='CORTEX_STEP_TYPE_USER_INPUT'))
"
```

Expected: `0` for the username in every file, `0` for the system-prompt markers, 48 steps and 2 user turns in the multi-turn fixture. If the username count is not zero, STOP and do not commit.

- [ ] **Step 3: Write the fixture README**

Record: captured from a live Antigravity instance on macOS on 2026-08-31 via `GetCascadeTrajectorySteps` and `GetUserTrajectoryDescriptions`; that they are real responses with two documented modifications (username scrubbed, `thinkingSignature` values replaced); that `steps-multi-turn.json` is the same conversation as `steps-single-turn.json` after a second turn was added, which is why it has 48 steps to the other's 23; and that `REDACTED-THINKING-SIGNATURE` is a substitution, not a value Antigravity produces.

- [ ] **Step 4: Commit**

```bash
git add crates/trace-commons-contributor/tests/fixtures/antigravity/
git commit -m "Add redacted Antigravity API captures as fixtures"
```

---

### Task 3: Find the language server endpoint

**Files:**
- Create: `crates/trace-commons-contributor/src/antigravity/mod.rs`
- Create: `crates/trace-commons-contributor/src/antigravity/endpoint.rs`
- Modify: `crates/trace-commons-contributor/src/lib.rs` (add `pub mod antigravity;`)
- Modify: `crates/trace-commons-contributor/Cargo.toml` (add `sysinfo`)

Note the module lives at `src/antigravity/`, NOT under `src/source/`. This is not a `TraceSource`; the `trajectory` source is what the daemon sees.

**Interfaces:**
- Produces:

```rust
pub(crate) struct Endpoint { pub port: u16, pub token: String }
pub(crate) const ERR_NOT_RUNNING: &str = "antigravity-not-running";
pub(crate) const ERR_API_NOT_FOUND: &str = "antigravity-api-not-found";

/// Candidate processes, from sysinfo. Split out so the probe is testable
/// without a running IDE.
pub(crate) struct Candidate { pub token: String, pub extension_server_port: u16 }
pub(crate) fn candidates_from(cmdlines: &[Vec<String>]) -> Vec<Candidate>;
pub(crate) fn discover() -> anyhow::Result<Endpoint>;
```

- [ ] **Step 1: Add the dependency in its own commit**

```toml
# Reads the Antigravity language server's command line to find its CSRF
# token. Pinned to 0.36 to match the version already in this workspace's
# lockfile via mistralrs-core; 0.39.x requires Rust 1.95 against this
# workspace's 1.92 floor.
sysinfo = { version = "0.36", default-features = false, features = ["system"] }
```

Then regenerate the vendor set exactly as in Task 1 Step 3, and confirm `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --test release_pipeline` passes. Commit this alone:

```bash
git add crates/trace-commons-contributor/Cargo.toml Cargo.lock crates/trace-commons-contributor-gtk/Cargo.lock crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json
git commit -m "Add sysinfo to the contributor crate"
```

If `default-features = false, features = ["system"]` does not compile, use the smallest feature set that does and say which in your report.

- [ ] **Step 2: Write the failing test for command-line parsing**

```rust
#[test]
fn a_language_server_command_line_yields_its_token_and_extension_port() {
    let cmdlines = vec![vec![
        "/Applications/Antigravity IDE.app/Contents/Resources/app/extensions/antigravity/bin/language_server_macos_arm".to_string(),
        "--enable_lsp".to_string(),
        "--csrf_token".to_string(),
        "114d1b72-7bc2-4c3c-b165-196ce5403d72".to_string(),
        "--extension_server_port".to_string(),
        "65402".to_string(),
    ]];
    let found = candidates_from(&cmdlines);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].token, "114d1b72-7bc2-4c3c-b165-196ce5403d72");
    assert_eq!(found[0].extension_server_port, 65402);
}

#[test]
fn an_unrelated_process_is_not_a_candidate() {
    let cmdlines = vec![vec!["/usr/bin/ssh".to_string(), "--csrf_token".to_string(), "x".to_string()]];
    assert!(candidates_from(&cmdlines).is_empty());
}

#[test]
fn a_language_server_without_a_token_is_not_a_candidate() {
    let cmdlines = vec![vec!["language_server_macos_arm".to_string(), "--enable_lsp".to_string()]];
    assert!(candidates_from(&cmdlines).is_empty());
}
```

- [ ] **Step 3: Run and confirm they fail**

Run: `cargo test -p trace-commons-contributor --lib antigravity::endpoint`
Expected: FAIL — `candidates_from` does not exist.

- [ ] **Step 4: Implement parsing**

Match on the executable's file name beginning with `language_server_`, then scan for `--csrf_token` and `--extension_server_port` followed by their values. Skip any process missing either. Never log or return the token in an error.

- [ ] **Step 5: Implement discovery and the bounded probe**

`discover()` collects command lines via `sysinfo::System::new_all()` and `process.cmd()`, builds candidates, and for each probes `extension_server_port + 1 ..= extension_server_port + 64` on `127.0.0.1`.

The probe identifies the API positively, in two steps, and both must pass:

1. A POST to `/exa.language_server_pb.LanguageServerService/GetUserTrajectoryDescriptions` with no CSRF header returns HTTP 401 and a body containing `"unauthenticated"`.
2. The same POST WITH `x-codeium-csrf-token: <that candidate's token>` returns HTTP 200.

Only a port passing both is the endpoint. The token match is what proves the intended process was reached rather than something else on a nearby port. Use a short per-port timeout (250ms) so a full sweep of 64 ports stays under a few seconds.

No candidates at all is `ERR_NOT_RUNNING`. Candidates but no port passing both checks is `ERR_API_NOT_FOUND`.

- [ ] **Step 6: Add the live-instance test, self-skipping**

```rust
/// Exercises the real probe. Skips loudly when Antigravity is not running,
/// because CI has no IDE -- the skip must be visible so a permanently
/// skipped test is not mistaken for coverage.
#[test]
fn discovery_finds_a_live_endpoint_when_antigravity_is_running() {
    match discover() {
        Ok(e) => assert!(e.port > 0 && !e.token.is_empty()),
        Err(err) => {
            eprintln!("skipping: no live Antigravity endpoint ({err})");
        }
    }
}
```

- [ ] **Step 7: Verify and commit**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
cargo fmt --all
git add crates/trace-commons-contributor/src/antigravity crates/trace-commons-contributor/src/lib.rs
git commit -m "Find the Antigravity language server by a token-verified probe"
```

---

### Task 4: The API client

**Files:**
- Create: `crates/trace-commons-contributor/src/antigravity/client.rs`

**Interfaces:**
- Consumes: `Endpoint` (Task 3).
- Produces:

```rust
pub(crate) struct TrajectoryDescription {
    pub trajectory_id: String,
    pub cascade_id: Option<String>,
    pub workspace_uri: Option<String>,
    pub git_branch: Option<String>,
}

/// A trait so `convert` and the command are testable against recorded
/// responses with no IDE running. Only the live implementation touches the
/// network.
pub(crate) trait AntigravityApi {
    fn list_trajectories(&self) -> anyhow::Result<Vec<TrajectoryDescription>>;
    fn fetch_steps(&self, cascade_id: &str) -> anyhow::Result<serde_json::Value>;
}

pub(crate) struct HttpApi { /* endpoint + reqwest blocking client */ }
pub(crate) const ERR_API_FAILED: &str = "antigravity-api-failed";

#[cfg(test)]
pub(crate) fn fixture_path(name: &str) -> std::path::PathBuf;
#[cfg(test)]
pub(crate) struct FixtureApi { /* serves the committed fixtures */ }
/// The description matching the fixture conversations, so convert tests do
/// not each rebuild one. Workspace URI
/// `file:///Users/anonymized/code/trace-commons-server`, branch from the
/// recorded listing.
#[cfg(test)]
pub(crate) fn desc_fixture() -> TrajectoryDescription;
```

**The identifier trap, which the spec records and this task must respect:** a conversation's FILE NAME is its cascade id; the `trajectory_id` inside it is a different UUID. `fetch_steps` sends `{"cascadeId": "..."}`. Sending `trajectoryId`, or sending the wrong UUID under either name, returns the same generic `trajectory not found` an empty request produces — so a wrong identifier is not self-diagnosing and will look like "no such conversation".

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn descriptions_parse_from_the_recorded_response() {
    let api = FixtureApi::new();
    let list = api.list_trajectories().expect("fixture must parse");
    assert!(!list.is_empty());
    let first = &list[0];
    assert!(first.workspace_uri.as_deref().unwrap().starts_with("file:///"));
    assert!(!first.trajectory_id.is_empty());
}

#[test]
fn steps_parse_from_the_recorded_multi_turn_response() {
    let api = FixtureApi::new();
    let doc = api.fetch_steps("multi-turn").expect("fixture must parse");
    let steps = doc["steps"].as_array().expect("steps is an array");
    assert_eq!(steps.len(), 48);
}
```

- [ ] **Step 2: Run and confirm they fail**

Run: `cargo test -p trace-commons-contributor --lib antigravity::client`
Expected: FAIL — the types do not exist.

- [ ] **Step 3: Implement**

`HttpApi` POSTs to `http://127.0.0.1:<port>/exa.language_server_pb.LanguageServerService/<Method>` with `Content-Type: application/json` and `x-codeium-csrf-token: <token>`, body `{}` for the listing and `{"cascadeId": "..."}` for steps.

Every failure — transport, non-200, malformed JSON — maps to `ERR_API_FAILED`. A `reqwest` error's `Display` can contain the URL, so it must never reach the label.

`FixtureApi` reads the committed fixtures: `"multi-turn"` serves `steps-multi-turn.json`, anything else serves `steps-single-turn.json`.

- [ ] **Step 4: Verify and commit**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
cargo fmt --all
git add crates/trace-commons-contributor/src/antigravity/client.rs crates/trace-commons-contributor/src/antigravity/mod.rs
git commit -m "Read Antigravity trajectories over the local API"
```

---

### Task 5: Convert API steps to Trajectory-v1

This task carries the merge gate. Read the spec's "Multi-turn is a first-class requirement" section before starting; all five criteria are pinned here.

**Files:**
- Create: `crates/trace-commons-contributor/src/antigravity/convert.rs`

**Interfaces:**
- Consumes: the `serde_json::Value` from `fetch_steps`, and a `TrajectoryDescription`.
- Produces: `pub(crate) fn to_trajectory_v1(steps: &serde_json::Value, desc: &TrajectoryDescription) -> anyhow::Result<Vec<serde_json::Value>>` and `pub(crate) const ERR_NO_CONTENT: &str = "antigravity-no-content";`

**The output format**, which the existing `trajectory` source parses — see `source/trajectory.rs`:

```json
[
  {"role":"meta","source":"antigravity","cwd":"/path","model":"...","git_branch":"main"},
  {"role":"user","content":"...","timestamp":"2026-08-29T10:13:36.119719Z"},
  {"role":"reasoning","content":"...","timestamp":"..."},
  {"role":"assistant","content":null,"tool_calls":[{"id":"call_304828","name":"list_dir","args":"{...}"}],"timestamp":"..."},
  {"role":"tool","tool_call_id":"call_304828","content":"...","timestamp":"..."},
  {"role":"assistant","content":"...","timestamp":"..."}
]
```

**Pairing is load-bearing and fails closed on its own.** `source/trajectory.rs` REJECTS the whole file on an orphaned `tool_call_id`. The API's tool-result steps (`LIST_DIRECTORY`, `VIEW_FILE`, `RUN_COMMAND`) were not observed to carry the call id, so pair them positionally: a tool-result step answers the most recent unanswered `toolCalls[]` entry from the preceding `PLANNER_RESPONSE`. Verify this against the multi-turn fixture — if the counts do not line up, investigate `metadata.sourceTrajectoryStepInfo`, which is a candidate link and is not yet decoded, and report what you find.

- [ ] **Step 1: Write the multi-turn gate tests**

```rust
#[test]
fn every_user_turn_becomes_its_own_event_in_conversation_order() {
    let api = FixtureApi::new();
    let doc = api.fetch_steps("multi-turn").unwrap();
    let out = to_trajectory_v1(&doc, &desc_fixture()).expect("must convert");

    let user_positions: Vec<usize> = out.iter().enumerate()
        .filter(|(_, r)| r["role"] == "user")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(user_positions.len(), 2, "two user turns in, two out -- never collapsed");
    assert!(
        user_positions[1] > user_positions[0] + 1,
        "the second user turn must be interleaved after the first turn's agent work, \
         not adjacent to it -- front-loading is the defect this design replaced"
    );
}

#[test]
fn each_user_turn_carries_its_own_real_timestamp() {
    let api = FixtureApi::new();
    let doc = api.fetch_steps("multi-turn").unwrap();
    let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
    let ts: Vec<&str> = out.iter()
        .filter(|r| r["role"] == "user")
        .map(|r| r["timestamp"].as_str().expect("a user turn has a timestamp"))
        .collect();
    assert_eq!(ts.len(), 2);
    assert!(ts[0].starts_with("2026-08-29"), "first turn keeps its own day");
    assert!(ts[1].starts_with("2026-08-31"), "second turn keeps its own day");
    assert_ne!(ts[0], ts[1], "no timestamp inherited from a neighbour");
}

#[test]
fn nothing_from_the_model_internals_reaches_the_output() {
    let api = FixtureApi::new();
    let doc = api.fetch_steps("multi-turn").unwrap();
    let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
    let rendered = serde_json::to_string(&out).unwrap();
    assert!(!rendered.contains("REDACTED-THINKING-SIGNATURE"),
        "thinkingSignature is opaque model internals and must not be carried");
    assert!(!rendered.contains("thinkingSignature"));
}

#[test]
fn an_unrecognised_step_type_becomes_one_opaque_event_not_a_failure() {
    let mut doc = FixtureApi::new().fetch_steps("multi-turn").unwrap();
    doc["steps"].as_array_mut().unwrap().push(serde_json::json!({
        "type": "CORTEX_STEP_TYPE_SOMETHING_GOOGLE_ADDED_LATER",
        "metadata": {"createdAt": "2026-09-01T00:00:00Z"}
    }));
    to_trajectory_v1(&doc, &desc_fixture()).expect("an unknown step type must not fail the conversation");
}

#[test]
fn a_conversation_with_no_user_or_assistant_content_is_refused() {
    let doc = serde_json::json!({"steps": []});
    let err = to_trajectory_v1(&doc, &desc_fixture()).unwrap_err().to_string();
    assert_eq!(err, "antigravity-no-content");
}
```

- [ ] **Step 2: Run and confirm they fail**

Run: `cargo test -p trace-commons-contributor --lib antigravity::convert`
Expected: FAIL — `to_trajectory_v1` does not exist.

- [ ] **Step 3: Implement the mapping**

Emit the `meta` record first, from the `TrajectoryDescription`: `source: "antigravity"`, `cwd` from `workspace_uri` with the `file://` scheme removed and percent-escapes decoded, `git_branch`, and `model` from the first step's `metadata.generatorModel`.

Then walk `steps` IN ORDER and map by `type`:
- `CORTEX_STEP_TYPE_USER_INPUT` → `{"role":"user", "content": userInput.userResponse, "timestamp": metadata.createdAt}`
- `CORTEX_STEP_TYPE_PLANNER_RESPONSE` with `thinking` → a `reasoning` record
- `PLANNER_RESPONSE` with `response` → an `assistant` record with `content`
- `PLANNER_RESPONSE` with `toolCalls` → an `assistant` record with `content: null` and `tool_calls`, taking only `id`, `name` and `argumentsJson` — never `thinkingSignature`
- `LIST_DIRECTORY`, `VIEW_FILE`, `RUN_COMMAND` → a `tool` record whose `tool_call_id` is the matched call
- **Every other `CORTEX_STEP_TYPE_*`, recognised or not, emits no record at all.** It must not fail the conversation.

That last rule is a deliberate departure from the spec's original wording, which said an unrecognised step becomes one `Opaque` event. Trajectory-v1 has exactly five roles — `meta`, `user`, `reasoning`, `assistant`, `tool` — and no way to express an opaque one. Inventing a sixth role would be rejected by the reader; emitting an empty `assistant` record would put a fabricated turn in the transcript. Dropping preserves the property that actually matters — a step kind Google adds later costs one step, never a session — and the spec is corrected to match. Say in your report if you find a shape where dropping loses something a contributor would miss.

Refuse with `ERR_NO_CONTENT` when no `user` and no `assistant` record was produced.

- [ ] **Step 4: Add the round-trip test**

The staged file must parse back through the reader that will actually consume it:

```rust
#[test]
fn the_output_round_trips_through_the_trajectory_reader() {
    let api = FixtureApi::new();
    let doc = api.fetch_steps("multi-turn").unwrap();
    let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
    let bytes = serde_json::to_vec(&out).unwrap();

    let parsed = crate::source::trajectory::parse_trajectory(&bytes)
        .expect("the trajectory reader must accept what we write");
    assert_eq!(parsed.source, "antigravity");
    let users = parsed.events.iter()
        .filter(|e| e.kind == crate::source::SessionEventKind::User)
        .count();
    assert_eq!(users, 2, "both turns survive the round trip, in order");
}
```

`parse_trajectory` is currently private to its module; make it `pub(crate)` if needed and say so in your report.

- [ ] **Step 4b: Pin the fifth gate criterion — a grown conversation re-hashes**

The two fixtures are the SAME conversation before and after a second turn, which makes this directly testable rather than argued:

```rust
#[test]
fn a_conversation_that_gains_turns_stages_different_bytes() {
    let api = FixtureApi::new();
    let before = to_trajectory_v1(&api.fetch_steps("single-turn").unwrap(), &desc_fixture()).unwrap();
    let after = to_trajectory_v1(&api.fetch_steps("multi-turn").unwrap(), &desc_fixture()).unwrap();

    let a = serde_json::to_vec(&before).unwrap();
    let b = serde_json::to_vec(&after).unwrap();
    assert_ne!(
        crate::source::session_hash(&a),
        crate::source::session_hash(&b),
        "a conversation that gained a turn must not be suppressed as a duplicate \
         of its earlier self -- the later turns would never be collected"
    );
}
```

This is the criterion the spec lists last and it is the one most easily missed, because it only fails for a contributor who imports, keeps talking, and imports again.

- [ ] **Step 5: Verify and commit**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
cargo fmt --all
git add crates/trace-commons-contributor/src/antigravity/convert.rs crates/trace-commons-contributor/src/antigravity/mod.rs crates/trace-commons-contributor/src/source/trajectory.rs
git commit -m "Convert Antigravity API steps to Trajectory-v1 records"
```

---

### Task 6: The import command

**Files:**
- Create: `crates/trace-commons-contributor/src/antigravity/import.rs`
- Modify: `crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs` (add the subcommand)
- Modify: `crates/trace-commons-contributor/src/commands.rs` (add the entry point)

**Interfaces:**
- Consumes: `discover()`, `AntigravityApi`, `to_trajectory_v1`.
- Produces: `pub fn import_antigravity(store: &ConfigStore, project: Option<&str>, all: bool) -> anyhow::Result<ImportSummary>` and `pub struct ImportSummary { pub imported: usize, pub skipped_other_projects: usize, pub staged_dir: PathBuf }`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn only_conversations_matching_the_project_are_staged() {
    let dir = tempfile::tempdir().unwrap();
    let api = FixtureApi::new();
    let summary = import_with(&api, dir.path(), Some("/Users/anonymized/code/trace-commons-server"), false)
        .expect("import must succeed");
    assert_eq!(summary.imported, 1);

    let staged: Vec<_> = std::fs::read_dir(dir.path()).unwrap().filter_map(Result::ok).collect();
    assert_eq!(staged.len(), 1, "one file per imported conversation");
}

#[test]
fn a_project_filter_matching_nothing_stages_nothing_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let api = FixtureApi::new();
    let summary = import_with(&api, dir.path(), Some("/somewhere/else"), false).unwrap();
    assert_eq!(summary.imported, 0);
    assert!(summary.skipped_other_projects > 0,
        "an empty result must be distinguishable from 'no conversations exist'");
}
```

`import_with` is the testable inner function taking an `&dyn AntigravityApi` and a staging directory; `import_antigravity` is the thin wrapper that supplies the live API and the real staging path.

- [ ] **Step 2: Run and confirm they fail**

Run: `cargo test -p trace-commons-contributor --lib antigravity::import`
Expected: FAIL — the function does not exist.

- [ ] **Step 3: Implement**

List trajectories, filter by `trajectoryScope.workspaceUri` against the project path unless `--all`, fetch each match's steps by CASCADE id, convert, and write `<staging>/<cascade-id>.json`. Staging lives under the config directory via `ConfigStore`, never in the contributor's project.

Count and report conversations skipped for belonging to another project. An empty import must never be indistinguishable from "you have no conversations" — that failure mode is why the previous design's `--project` gap mattered.

- [ ] **Step 4: Wire the subcommand**

```rust
/// Import Antigravity conversations. Requires the Antigravity IDE to be
/// running, since its conversations are only readable through the local
/// API it serves.
ImportAntigravity {
    /// Only import conversations for this project (default: the current directory)
    #[arg(long)]
    project: Option<PathBuf>,
    /// Import every conversation the running instance exposes, not just this project's
    #[arg(long, conflicts_with = "project")]
    all: bool,
},
```

Print a human summary, and the JSON form when `--json` is set, following the shape other subcommands use.

- [ ] **Step 5: Verify and commit**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
cargo fmt --all
git add -A
git commit -m "Add the import-antigravity command"
```

---

### Task 7: Documentation and final verification

- [ ] **Step 1: Run everything, unfiltered**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
cargo clippy -p trace-commons-contributor --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all -- --check
cargo build -p trace-commons-contributor-ffi && (cd macos && swift test)
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
```

Paste actual output for each. `release_pipeline` must pass — it is the test that catches a stale Flatpak vendor set.

- [ ] **Step 2: Content checks**

```bash
grep -rn 'antigravity-' crates/trace-commons-contributor/src/antigravity/
grep -rc 'zakimanian\|thinkingSignature' crates/trace-commons-contributor/tests/fixtures/antigravity/
```

Every label must be a fixed `&'static str` with nothing interpolated — in particular no token, port, or URL. The second must be 0 for every file.

- [ ] **Step 3: Update the README**

Add Antigravity to `crates/trace-commons-contributor/README.md`, stating: it is imported with `import-antigravity` rather than watched; the Antigravity IDE must be running because conversations are only readable through the local API it serves; only conversations the running instance has loaded are reachable, so opening the relevant project first matters; and that both current and legacy conversation formats are readable this way.

- [ ] **Step 4: Commit**

```bash
git add crates/trace-commons-contributor/README.md
git commit -m "Document the Antigravity import command"
```

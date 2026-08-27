import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { SOURCES, NATIVE_SOURCES } from "../src/sources.mjs";
import { safeId, SUFFIX, probe, exportInput } from "../src/export.mjs";

test("the natively-read harnesses are not offered for export", () => {
  // Routing claude-code or codex through here would be strictly worse than the
  // Rust CLI's native readers, and would perturb event extraction the pilot has
  // already scored against.
  for (const native of NATIVE_SOURCES) {
    assert.ok(!SOURCES.includes(native), `${native} must not be an export source`);
  }
});

test("deepagents is not offered: it needs the checkpoint API, not normalizeTranscript", () => {
  assert.ok(!SOURCES.includes("deepagents"));
});

test("the suffix is the one the contributor CLI auto-discovers", () => {
  // Slice B discovers *.trajectory.json in the working directory and nothing
  // else. Any other suffix silently reintroduces the --trajectory step.
  assert.equal(SUFFIX, ".trajectory.json");
});

test("ids are made filesystem-safe without collapsing to nothing", () => {
  assert.equal(safeId("a/b\\c"), "a-b-c");
  assert.equal(safeId("2026-03-03T19:27:11.183Z_7e90"), "2026-03-03T19-27-11.183Z_7e90");
  assert.ok(safeId("x".repeat(500)).length <= 100);
});

test("an export-only source is reported as such, not as a failure", async () => {
  // Upstream can normalize gemini-cli transcripts but cannot enumerate its
  // store, and says so with a listing_unavailable code. Reporting that as an
  // error would tell a contributor something broke when nothing did.
  const r = await probe("gemini-cli", 1);
  assert.equal(r.listingUnsupported, true);
  assert.equal(r.error, null);
  assert.deepEqual(r.items, []);
});

test("a session upstream refuses is skipped by name, not thrown", async () => {
  // normalizeTranscript throws missing_assistant_records for a transcript
  // where someone typed and quit. That is an ordinary session, and one of them
  // must not abort an --all run over the other eleven harnesses.
  const dir = await mkdtemp(join(tmpdir(), "tjx-"));
  const input = join(dir, "rollout-useronly.jsonl");
  await writeFile(
    input,
    [
      JSON.stringify({
        type: "session_meta",
        payload: { id: "22222222-2222-4222-8222-222222222222", cwd: "/w", timestamp: "2026-07-30T10:00:00Z" },
      }),
      JSON.stringify({
        type: "response_item",
        payload: { type: "message", role: "user", content: [{ type: "input_text", text: "hello" }] },
        timestamp: "2026-07-30T10:00:01Z",
      }),
    ].join("\n"),
  );
  const r = await exportInput("codex", input, dir);
  assert.equal(r.skipped, "missing_assistant_records");
  assert.equal(r.path, null);
});

test("exportInput writes a discoverable file from a named transcript", async () => {
  const dir = await mkdtemp(join(tmpdir(), "tjx-"));
  // A minimal codex rollout: upstream normalizes it, and it carries no real
  // session content.
  const rollout = [
    JSON.stringify({
      type: "session_meta",
      payload: { id: "11111111-1111-4111-8111-111111111111", cwd: "/w", timestamp: "2026-07-30T10:00:00Z" },
    }),
    JSON.stringify({
      type: "response_item",
      payload: { type: "message", role: "user", content: [{ type: "input_text", text: "hello" }] },
      timestamp: "2026-07-30T10:00:01Z",
    }),
    JSON.stringify({
      type: "response_item",
      payload: { type: "message", role: "assistant", content: [{ type: "output_text", text: "hi" }] },
      timestamp: "2026-07-30T10:00:02Z",
    }),
  ].join("\n");
  const input = join(dir, "rollout-test.jsonl");
  await writeFile(input, rollout);

  const r = await exportInput("codex", input, dir);
  assert.equal(r.skipped, null);
  assert.ok(r.path.endsWith(SUFFIX), `expected ${SUFFIX}, got ${r.path}`);
  assert.ok(r.records > 0);

  const written = await readFile(r.path, "utf8");
  const first = JSON.parse(written.split("\n")[0]);
  assert.equal(first.role, "meta");
  assert.equal(first.source, "codex");
});

test("the bin entry survives publishing", async () => {
  // npm silently strips a bin whose path is written "./bin/x.mjs" -- it warns
  // once at publish time and drops the entry, producing a CLI package with no
  // command in it. That is precisely the defect in @letta-ai/trajectory that
  // this tool exists to work around, so it must not be the defect in this one.
  //
  // The guard is the path shape, because that is what npm normalizes. Anything
  // npm would rewrite is something a publish would have quietly changed.
  const pkg = JSON.parse(
    await readFile(new URL("../package.json", import.meta.url), "utf8"),
  );
  const entries = Object.entries(pkg.bin ?? {});
  assert.equal(entries.length, 1, "expected exactly one bin entry");
  const [name, path] = entries[0];
  assert.equal(name, "trajectory-export");
  assert.ok(!path.startsWith("./"), `bin path must not start with "./": ${path}`);
  assert.equal(path, "bin/trajectory-export.mjs");
  assert.ok(pkg.files.includes("bin"), "bin/ must be in files or the command is not shipped");
});

import assert from "node:assert/strict";
import { test } from "node:test";
import { coreStatusSchema } from "./types.ts";

function status(extra = {}) {
  return {
    state_dir: "test",
    startup: "running",
    daemon: {
      schema_version: "1",
      logged_in: true,
      tenant_id: "test",
      consent_scopes: [],
      paused: false,
      queue_depth: 17,
      health: { last_error_label: null, since: null },
      ...extra,
    },
  };
}

test("status preserves decisions owed independently of the upload queue", () => {
  for (const count of [0, 3]) {
    const parsed = coreStatusSchema.parse(status({ decisions_owed: count }));
    assert.equal(parsed.daemon.decisions_owed, count);
    assert.equal(parsed.daemon.queue_depth, 17);
  }
});

test("an older daemon reports decisions unavailable instead of zero", () => {
  assert.equal(coreStatusSchema.parse(status()).daemon.decisions_owed, null);
});

test("an invalid decisions count is unavailable without losing other status", () => {
  for (const decisions_owed of [null, -1, 1.5, "3", Number.NaN]) {
    const parsed = coreStatusSchema.parse(status({ decisions_owed }));
    assert.equal(parsed.daemon.decisions_owed, null);
    assert.equal(parsed.daemon.queue_depth, 17);
  }
});

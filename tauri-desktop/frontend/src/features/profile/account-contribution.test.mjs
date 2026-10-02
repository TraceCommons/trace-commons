import assert from "node:assert/strict";
import { test } from "node:test";
import { nextInviteAttempt, contributionLine } from "./account-contribution.ts";

test("invite retries reuse the key until the code or account scope changes", () => {
  let minted = 0;
  const newKey = () => `key-${++minted}`;
  const first = nextInviteAttempt(null, "synthetic-code", newKey);
  assert.equal(nextInviteAttempt(first, "synthetic-code", newKey).key, first.key);
  assert.notEqual(nextInviteAttempt(first, "different-code", newKey).key, first.key);
  assert.notEqual(nextInviteAttempt(null, "synthetic-code", newKey).key, first.key);
  assert.equal(minted, 3);
});
test("readiness uses the daemon line and refuses missing status copy", () => {
  const line = "Ready to contribute. Accepted contributions may earn pending credit.";
  assert.equal(contributionLine({ line }), line);
  for (const value of [null, {}, { line: null }, { line: "" }]) {
    assert.throws(() => contributionLine(value), /account-contribution-unavailable/);
  }
});

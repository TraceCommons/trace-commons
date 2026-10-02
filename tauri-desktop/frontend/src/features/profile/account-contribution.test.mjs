import assert from "node:assert/strict";
import { test } from "node:test";
import { nextInviteAttempt, contributionLine, scopedContributionLine } from "./account-contribution.ts";

test("invite retries reuse the key until the code or account scope changes", () => {
  let minted = 0;
  const newKey = () => `key-${++minted}`;
  const first = nextInviteAttempt(null, "synthetic-code", newKey, "scope-a");
  assert.equal(nextInviteAttempt(first, "synthetic-code", newKey, "scope-a").key, first.key);
  assert.notEqual(nextInviteAttempt(first, "different-code", newKey, "scope-a").key, first.key);
  assert.notEqual(nextInviteAttempt(null, "synthetic-code", newKey, "scope-a").key, first.key);
  assert.equal(minted, 3);
});
test("readiness uses the daemon line and refuses missing status copy", () => {
  const line = "Ready to contribute. Accepted contributions may earn pending credit.";
  assert.equal(contributionLine({ line }), line);
  for (const value of [null, {}, { line: null }, { line: "" }]) {
    assert.throws(() => contributionLine(value), /account-contribution-unavailable/);
  }
});

test("same-tenant logout, replacement, or ingest scope invalidates attempts and replies", () => {
  const old = nextInviteAttempt(null, "same-code", () => "key-a", "scope-a");
  const fresh = nextInviteAttempt(old, "same-code", () => "key-b", "scope-b");
  assert.notEqual(fresh.key, old.key);
  const reply = {account_scope:"scope-a", line:"Ready", tenant_id:"same-tenant", logged_in:true};
  assert.equal(scopedContributionLine(reply, "scope-a"), "Ready");
  assert.throws(() => scopedContributionLine(reply, "scope-b"), /account-session-changed/);
});

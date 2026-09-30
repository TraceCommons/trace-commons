import assert from "node:assert/strict";
import { test } from "node:test";
import {
  canWithdrawStatus,
  historyStatusLabel,
} from "./withdrawal-eligibility.ts";

const sharedHistoryCopy = {
  status_awaiting_pii_backstop: "Waiting for privacy review",
};

test("a privacy-backstop hold reads with the contributor core's label", () => {
  assert.equal(
    historyStatusLabel("awaiting_pii_backstop", sharedHistoryCopy),
    "Waiting for privacy review",
  );
});

test("a privacy-backstop hold is not labelled before shared copy arrives", () => {
  assert.equal(historyStatusLabel("awaiting_pii_backstop"), "Status unavailable");
  assert.equal(historyStatusLabel("a-status-from-the-future"), "Status unavailable");
});

test("known statuses keep their labels with or without shared copy", () => {
  assert.equal(historyStatusLabel("accepted"), "In the commons");
  assert.equal(
    historyStatusLabel("quarantined", sharedHistoryCopy),
    "Held for privacy review",
  );
});

test("a submission held on the privacy backstop can be withdrawn", () => {
  // The server's withdraw handler has no status gate: any owned submission
  // that is not accepted withdraws as `not_distributed`.
  assert.equal(canWithdrawStatus("awaiting_pii_backstop"), true);
  for (const status of ["accepted", "submitted", "quarantined"]) {
    assert.equal(canWithdrawStatus(status), true, status);
  }
  for (const status of ["withdrawn", "revoked", "purged", "expired", "unknown"]) {
    assert.equal(canWithdrawStatus(status), false, status);
  }
});

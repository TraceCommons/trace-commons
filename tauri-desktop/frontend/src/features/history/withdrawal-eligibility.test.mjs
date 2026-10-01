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

test("a processing receipt reads and withdraws as waiting to be scored", () => {
  // `processing` is the versioned pipeline's receipt status: uploaded, no
  // verdict yet. It is `submitted` in every respect a row shows.
  assert.equal(historyStatusLabel("processing"), "Waiting to be scored");
  assert.equal(
    historyStatusLabel("processing", sharedHistoryCopy),
    "Waiting to be scored",
  );
  assert.equal(canWithdrawStatus("processing"), true);
});

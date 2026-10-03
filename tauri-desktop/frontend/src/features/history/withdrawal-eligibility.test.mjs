import assert from "node:assert/strict";
import { test } from "node:test";
import {
  canWithdrawStatus,
  historyStatusLabel,
} from "./withdrawal-eligibility.ts";

// The disclosure bundle's `history_ui`, as the core exports it
// (`history_copy::STATUS_LABELS`).
const sharedHistoryCopy = {
  status_awaiting_pii_backstop: "Waiting for privacy review",
  status_unavailable: "Status unavailable",
  status_labels: {
    submitted: "Waiting to be scored",
    processing: "Waiting to be scored",
    received: "Received",
    accepted: "In the commons",
    quarantined: "Held for privacy review",
    awaiting_pii_backstop: "Waiting for privacy review",
    rejected: "Rejected",
    revoked: "Withdrawn",
    withdrawn: "Withdrawn by you",
    expired: "Expired",
    purged: "Purged",
  },
};

test("a privacy-backstop hold reads with the contributor core's label", () => {
  assert.equal(
    historyStatusLabel("awaiting_pii_backstop", sharedHistoryCopy),
    "Waiting for privacy review",
  );
});

test("every status in the core's table reads as its label", () => {
  for (const [status, label] of Object.entries(sharedHistoryCopy.status_labels)) {
    assert.equal(historyStatusLabel(status, sharedHistoryCopy), label, status);
  }
});

test("a status the core names reads as the core's word, not a typed one", () => {
  // The words are looked up, so a word the core changes changes here too.
  const renamed = {
    ...sharedHistoryCopy,
    status_labels: { ...sharedHistoryCopy.status_labels, revoked: "Taken back" },
  };
  assert.equal(historyStatusLabel("revoked", renamed), "Taken back");
});

test("an unrecognised status reads with the contributor core's label", () => {
  for (const status of ["a-status-from-the-future", "", "constructor", "toString"]) {
    assert.equal(
      historyStatusLabel(status, sharedHistoryCopy),
      sharedHistoryCopy.status_unavailable,
      status,
    );
    // And it is terminal: no Withdraw beside it.
    assert.equal(canWithdrawStatus(status), false, status);
  }
});

test("no status is labelled before shared copy arrives", () => {
  // No typed fallback: the label is the core's, and until it arrives the
  // row says nothing rather than something of its own.
  for (const status of ["accepted", "awaiting_pii_backstop", "a-status-from-the-future"]) {
    assert.equal(historyStatusLabel(status), null, status);
  }
});

test("every withdrawable status has a label", () => {
  for (const status of [
    "accepted",
    "submitted",
    "quarantined",
    "received",
    "rejected",
    "awaiting_pii_backstop",
  ]) {
    assert.equal(canWithdrawStatus(status), true, status);
    assert.notEqual(
      historyStatusLabel(status, sharedHistoryCopy),
      sharedHistoryCopy.status_unavailable,
      status,
    );
  }
});

test("a submission held on the privacy backstop can be withdrawn", () => {
  // The server's withdraw handler has no status gate: any owned submission
  // that is not accepted withdraws as `not_distributed`.
  assert.equal(canWithdrawStatus("awaiting_pii_backstop"), true);
  // macOS's allowlist (ContributionStatusPresentation.openValues).
  for (const status of ["accepted", "submitted", "quarantined", "received", "rejected"]) {
    assert.equal(canWithdrawStatus(status), true, status);
  }
  for (const status of ["withdrawn", "revoked", "purged", "expired", "unknown"]) {
    assert.equal(canWithdrawStatus(status), false, status);
  }
});

// `awaiting_pii_backstop` is a pre-acceptance hold. The server's withdraw
// endpoint does not gate on status: any owned submission that is not
// `accepted` withdraws as `not_distributed`, and the macOS shell offers it on
// the same terms (WithdrawalCopy.Stage.notInTheCommons).
const withdrawableStatuses = new Set([
  "accepted",
  "submitted",
  "quarantined",
  "awaiting_pii_backstop",
]);

// `processing` is the versioned pipeline's receipt status: uploaded, no
// verdict yet, which is what `submitted` means here. Every history surface
// reads a status through this one mapping -- the label, the withdraw option,
// the count and the filter -- so `processing` cannot get a different answer
// from any of them. A history refresh that reads the server's status for the
// row replaces it.
export function historyStatusBucket(status: string) {
  return status === "processing" ? "submitted" : status;
}

export function canWithdrawStatus(status: string) {
  return withdrawableStatuses.has(historyStatusBucket(status));
}

export type SharedHistoryStatusCopy = {
  status_awaiting_pii_backstop: string;
};

export function historyStatusLabel(
  status: string,
  shared?: SharedHistoryStatusCopy | null,
) {
  const labels: Record<string, string> = {
    accepted: "In the commons",
    submitted: "Waiting to be scored",
    quarantined: "Held for privacy review",
    withdrawn: "Withdrawn by you",
    revoked: "No longer available",
    purged: "Removed",
    expired: "Expired",
  };
  if (status === "awaiting_pii_backstop" && shared) {
    return shared.status_awaiting_pii_backstop;
  }
  return labels[historyStatusBucket(status)] ?? "Status unavailable";
}

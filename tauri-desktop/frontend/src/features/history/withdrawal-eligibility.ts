// `awaiting_pii_backstop` is a pre-acceptance hold. The server's withdraw
// endpoint does not gate on status: any owned submission that is not
// `accepted` withdraws as `not_distributed`, and the macOS shell offers it on
// the same terms (WithdrawalCopy.Stage.notInTheCommons).
//
// The set is the macOS app's (`ContributionStatusPresentation.openValues`),
// the parity target, so `received` and `rejected` are open too. Anything
// else, including a status this build does not recognise, is closed.
const withdrawableStatuses = new Set([
  "accepted",
  "submitted",
  "received",
  "quarantined",
  "awaiting_pii_backstop",
  "rejected",
]);

export function canWithdrawStatus(status: string) {
  return withdrawableStatuses.has(status);
}

export type SharedHistoryStatusCopy = {
  status_awaiting_pii_backstop: string;
  status_unavailable: string;
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
  // An unrecognised status reads as the core's label
  // (`history_copy::STATUS_UNAVAILABLE`), the words every shell shows. Until
  // the shared copy arrives there is no label, rather than a typed one.
  return labels[status] ?? shared?.status_unavailable ?? null;
}

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
  status_unavailable: string;
  /** The core's word for each wire status (`history_copy::STATUS_LABELS`). */
  status_labels: Record<string, string>;
};

// History's word for a status is the core's (`history_copy::STATUS_LABELS`,
// carried as the disclosure bundle's `history_ui.status_labels`), the table
// every shell reads. A status it does not name reads as the core's
// `history_copy::STATUS_UNAVAILABLE`. Until the shared copy arrives there is
// no label, rather than a typed one.
export function historyStatusLabel(
  status: string,
  shared?: SharedHistoryStatusCopy | null,
) {
  if (!shared) {
    return null;
  }
  const bucket = historyStatusBucket(status);
  // Own keys only: "constructor" is not a status.
  return Object.prototype.hasOwnProperty.call(shared.status_labels, bucket)
    ? shared.status_labels[bucket]
    : shared.status_unavailable;
}

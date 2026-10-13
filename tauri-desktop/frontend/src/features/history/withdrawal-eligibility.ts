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
// verdict reported. It is not "no verdict": a trace that Admission rejected
// or quarantined gets the same receipt, and the verdict arrives only when a
// history refresh reads the server's status for the row. The daemon files it
// with `submitted` (`daemon::history::status_bucket`) and sends that as the
// row's `bucket`, which every history surface reads -- the label, the
// withdraw option, the count and the filter. A daemon from before `bucket`
// leaves it out, and the row reads as its own status.
export function historyRecordBucket(record: {
  status: string;
  bucket?: string;
}) {
  return record.bucket ?? record.status;
}

export function canWithdrawBucket(bucket: string) {
  return withdrawableStatuses.has(bucket);
}

export type SharedHistoryStatusCopy = {
  status_awaiting_pii_backstop: string;
  status_unavailable: string;
  /** The core's word for each wire status (`history_copy::STATUS_LABELS`). */
  status_labels: Record<string, string>;
};

// History's word for a status is the core's (`history_copy::STATUS_LABELS`,
// carried as the disclosure bundle's `history_ui.status_labels`), the table
// every shell reads, looked up by the row's bucket. A status it does not name
// reads as the core's `history_copy::STATUS_UNAVAILABLE`. Until the shared
// copy arrives there is no label, rather than a typed one.
export function historyStatusLabel(
  bucket: string,
  shared?: SharedHistoryStatusCopy | null,
) {
  if (!shared) {
    return null;
  }
  // Own keys only: "constructor" is not a status.
  return Object.prototype.hasOwnProperty.call(shared.status_labels, bucket)
    ? shared.status_labels[bucket]
    : shared.status_unavailable;
}

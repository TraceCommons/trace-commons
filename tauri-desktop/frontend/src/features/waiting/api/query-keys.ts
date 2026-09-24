export const waitingKeys = {
  scope: (account: string) => ["account", account, "waiting"] as const,
  list: (account: string) => [...waitingKeys.scope(account), "list"] as const,
  outcomeCounts: (account: string) =>
    [...waitingKeys.scope(account), "outcome-counts"] as const,
  witnessSupport: (account: string) =>
    [...waitingKeys.scope(account), "witness-review-support"] as const,
  arming: (account: string) =>
    [...waitingKeys.scope(account), "arming"] as const,
  privateInference: (account: string) =>
    [...waitingKeys.scope(account), "private-inference"] as const,
  preview: (account: string, entryId: string) =>
    [...waitingKeys.scope(account), "preview", entryId] as const,
  previewBody: (account: string, entryId: string) =>
    [...waitingKeys.scope(account), "preview-body", entryId] as const,
  previewTurns: (account: string, entryId: string, digest: string) =>
    [...waitingKeys.scope(account), "preview-turns", entryId, digest] as const,
  certificateCopy: (account: string, evidenceAdmitted: boolean) =>
    [
      ...waitingKeys.scope(account),
      "certificate-copy",
      evidenceAdmitted,
    ] as const,
};

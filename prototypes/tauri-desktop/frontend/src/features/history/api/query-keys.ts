export const historyKeys = {
  scope: (account: string) => ["account", account, "history"] as const,
  list: (account: string) => [...historyKeys.scope(account), "list"] as const,
  detail: (account: string, submissionId: string) =>
    [...historyKeys.scope(account), "detail", submissionId] as const,
  skillCopy: () => ["local", "history", "skill-copy"] as const,
  skillCandidate: (account: string, submissionId: string) =>
    [...historyKeys.scope(account), "skill-candidate", submissionId] as const,
  skillInstallStatus: (account: string, submissionId: string) =>
    [
      ...historyKeys.scope(account),
      "skill-install-status",
      submissionId,
    ] as const,
};

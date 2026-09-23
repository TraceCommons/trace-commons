const withdrawableStatuses = new Set(["accepted", "submitted", "quarantined"]);

export function canWithdrawStatus(status: string) {
  return withdrawableStatuses.has(status);
}

export function historyStatusLabel(status: string) {
  const labels: Record<string, string> = {
    accepted: "In the commons",
    submitted: "Waiting to be scored",
    quarantined: "Held for privacy review",
    withdrawn: "Withdrawn by you",
    revoked: "No longer available",
    purged: "Removed",
    expired: "Expired",
  };
  return labels[status] ?? "Status unavailable";
}

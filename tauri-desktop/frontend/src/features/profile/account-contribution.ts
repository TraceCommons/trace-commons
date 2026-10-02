export type InviteAttempt = { code: string; key: string; scope: string };

export function nextInviteAttempt(previous: InviteAttempt | null, code: string, newKey: () => string, scope: string): InviteAttempt {
  return previous?.code === code && previous.scope === scope ? previous : { code, key: newKey(), scope };
}

export function contributionLine(value: unknown): string {
  if (typeof value !== "object" || value === null || !("line" in value) ||
      typeof value.line !== "string" || !value.line.trim()) throw new Error("account-contribution-unavailable");
  return value.line;
}

export function scopedContributionLine(value: unknown, scope: string): string {
  if (typeof value !== "object" || value === null || !("account_scope" in value) || value.account_scope !== scope) throw new Error("account-session-changed");
  return contributionLine(value);
}

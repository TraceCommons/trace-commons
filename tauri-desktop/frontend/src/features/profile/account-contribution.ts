export type InviteAttempt = { code: string; key: string };

export function nextInviteAttempt(previous: InviteAttempt | null, code: string, newKey: () => string): InviteAttempt {
  return previous?.code === code ? previous : { code, key: newKey() };
}

export function contributionLine(value: unknown): string {
  if (typeof value !== "object" || value === null || !("line" in value) ||
      typeof value.line !== "string" || !value.line.trim()) throw new Error("account-contribution-unavailable");
  return value.line;
}

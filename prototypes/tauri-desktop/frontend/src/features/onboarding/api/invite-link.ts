export type ResolvedInvite = {
  value: string;
  issuerHost: string;
};

export function resolveInvite(raw: string): ResolvedInvite | null {
  const value = raw.trim();
  try {
    const url = new URL(value);
    if (
      !["https:", "http:"].includes(url.protocol) ||
      !url.hostname ||
      url.username ||
      url.password
    ) {
      return null;
    }
    const code = url.hash.slice(1) || url.searchParams.get("code") || "";
    if (!code.trim()) return null;
    return { value, issuerHost: url.hostname };
  } catch {
    return null;
  }
}

export const privateAiKeys = {
  scope: (account: string) => ["account", account, "private-ai"] as const,
  credential: (account: string) =>
    [...privateAiKeys.scope(account), "credential"] as const,
  balance: (account: string) =>
    [...privateAiKeys.scope(account), "balance"] as const,
  funding: (account: string) =>
    [...privateAiKeys.scope(account), "funding"] as const,
  harnesses: () => ["local", "private-ai", "harnesses"] as const,
};

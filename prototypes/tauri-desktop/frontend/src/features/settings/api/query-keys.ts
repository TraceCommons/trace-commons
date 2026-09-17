export const settingsKeys = {
  scope: (account: string) => ["account", account, "settings"] as const,
  snapshot: (account: string) =>
    [...settingsKeys.scope(account), "snapshot"] as const,
  projects: (account: string) =>
    [...settingsKeys.scope(account), "projects"] as const,
  audit: (account: string) =>
    [...settingsKeys.scope(account), "audit"] as const,
  tokenStorage: (account: string) =>
    [...settingsKeys.scope(account), "token-storage"] as const,
  witness: (account: string) =>
    [...settingsKeys.scope(account), "witness"] as const,
  routingDiscovery: () => ["local", "settings", "routing-discovery"] as const,
};

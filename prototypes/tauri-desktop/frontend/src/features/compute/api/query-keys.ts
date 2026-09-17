export const computeKeys = {
  status: (account: string) =>
    ["account", account, "compute", "status"] as const,
};

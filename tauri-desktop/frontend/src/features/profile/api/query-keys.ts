export const profileKeys = {
  public: (account: string) =>
    ["account", account, "profile", "public"] as const,
};

export const onboardingKeys = {
  scope: (account: string) => ["account", account, "onboarding"] as const,
  consentOptions: (account: string) =>
    [...onboardingKeys.scope(account), "consent-options"] as const,
  scrubberPatterns: () => ["local", "onboarding", "scrubber-patterns"] as const,
};

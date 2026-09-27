export {
  type AutomaticGrant,
  acknowledgeNearAiNotice,
  getAutomaticGrant,
  getConsentOptions,
  setConsentScopes,
  withdrawAutomaticGrant,
} from "./api/onboarding-api";
export { onboardingKeys } from "./api/query-keys";
export { useOnboardingCompletion } from "./hooks/use-onboarding-completion";
export type { ConsentOption } from "./types";
export { type WithdrawOutcome, withdrawAndConfirm } from "./flow1";

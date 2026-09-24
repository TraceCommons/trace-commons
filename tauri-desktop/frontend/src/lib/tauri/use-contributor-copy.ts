import { useQueries, useQuery } from "@tanstack/react-query";
import {
  getArmingOfferCopy,
  getContributorDisclosureCopy,
  getEligibilityCopy,
  getEligibilityGroupCopy,
  getProjectIgnoreCopy,
  getQuitConfirmationCopy,
  getRedactionSummary,
  getResidualSecretLine,
  getWithdrawalConfirmationPrompt,
  getWitnessReviewCopy,
} from "./contributor-copy-api";

const copyKeys = {
  armingOffer: (projectLabel: string, count: number) =>
    ["contributor-copy", "arming-offer", projectLabel, count] as const,
  disclosure: ["contributor-copy", "disclosure"] as const,
  witnessReview: ["contributor-copy", "witness-review"] as const,
  withdrawalPrompt: ["contributor-copy", "withdrawal-prompt"] as const,
  quitConfirmation: ["contributor-copy", "quit-confirmation"] as const,
  eligibility: (label: string, reason: string | null) =>
    ["contributor-copy", "eligibility", label, reason] as const,
  eligibilityGroup: (pending: number, contributable: number | null) =>
    ["contributor-copy", "eligibility-group", pending, contributable] as const,
  projectIgnore: (label: string, pending: number) =>
    ["contributor-copy", "project-ignore", label, pending] as const,
  residualSecret: (count: number, sites: string[]) =>
    ["contributor-copy", "residual-secret", count, ...sites] as const,
  redactions: (redactions: Record<string, number>, distinct: Record<string, number>) =>
    [
      "contributor-copy",
      "redaction-summary",
      JSON.stringify(redactions),
      JSON.stringify(distinct),
    ] as const,
};

export function useContributorDisclosureCopy() {
  return useQuery({
    queryKey: copyKeys.disclosure,
    queryFn: getContributorDisclosureCopy,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useArmingOfferCopy(projectLabel: string, count: number) {
  return useQuery({
    queryKey: copyKeys.armingOffer(projectLabel, count),
    queryFn: () => getArmingOfferCopy(projectLabel, count),
    enabled: projectLabel.length > 0 && count > 0,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useWitnessReviewCopy() {
  return useQuery({
    queryKey: copyKeys.witnessReview,
    queryFn: getWitnessReviewCopy,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/**
 * Whether this app hosts the watcher or is attached to one can change while
 * it runs, so the prompt is re-read every time it opens rather than cached.
 */
export function useQuitConfirmationCopy(open: boolean) {
  return useQuery({
    queryKey: copyKeys.quitConfirmation,
    queryFn: getQuitConfirmationCopy,
    enabled: open,
    staleTime: 0,
    gcTime: 0,
  });
}

export function useWithdrawalConfirmationPrompt(enabled = true) {
  return useQuery({
    queryKey: copyKeys.withdrawalPrompt,
    queryFn: getWithdrawalConfirmationPrompt,
    staleTime: Number.POSITIVE_INFINITY,
    enabled,
  });
}

export function useEligibilityCopy(
  label: string | null | undefined,
  reason: string | null | undefined,
) {
  const eligibilityLabel = label ?? "";
  const eligibilityReason = reason ?? null;
  return useQuery({
    queryKey: copyKeys.eligibility(eligibilityLabel, eligibilityReason),
    queryFn: () => getEligibilityCopy(eligibilityLabel, eligibilityReason),
    enabled: eligibilityLabel.length > 0,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useEligibilityGroupCopy(
  entries: Array<{
    eligibility?: string | null;
    eligibility_reason?: string | null;
  }>,
) {
  const labelled = entries.filter(
    (entry) => typeof entry.eligibility === "string" && entry.eligibility.length > 0,
  );
  const eligibilityQueries = useQueries({
    queries: labelled.map((entry) => ({
      queryKey: copyKeys.eligibility(
        entry.eligibility ?? "",
        entry.eligibility_reason ?? null,
      ),
      queryFn: () =>
        getEligibilityCopy(
          entry.eligibility ?? "",
          entry.eligibility_reason ?? null,
        ),
      staleTime: Number.POSITIVE_INFINITY,
    })),
  });
  const copyUnavailable = eligibilityQueries.some((query) => query.isError);
  const copyPending = eligibilityQueries.some((query) => query.isPending);
  let labelIndex = 0;
  const eligibleCount = entries.reduce((count, entry) => {
    if (!entry.eligibility) return count + 1;
    const query = eligibilityQueries[labelIndex];
    labelIndex += 1;
    return count + (query?.data?.can_contribute === true ? 1 : 0);
  }, 0);
  const eligibilityMissing = labelled.length === 0;
  const contributable = eligibilityMissing ? null : eligibleCount;
  const group = useQuery({
    queryKey: copyKeys.eligibilityGroup(entries.length, contributable),
    queryFn: () => getEligibilityGroupCopy(entries.length, contributable),
    staleTime: Number.POSITIVE_INFINITY,
    enabled: entries.length > 0 && !copyPending && !copyUnavailable,
  });

  return {
    ...group,
    isPending: copyPending || group.isPending,
    isError: copyUnavailable || group.isError,
  };
}

export function useProjectIgnoreCopy(projectLabel: string, pending: number) {
  return useQuery({
    queryKey: copyKeys.projectIgnore(projectLabel, pending),
    queryFn: () => getProjectIgnoreCopy(projectLabel, pending),
    enabled: projectLabel.length > 0 && pending >= 0,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useResidualSecretCopy(redactions: Record<string, number>) {
  const residualEntries = Object.entries(redactions).filter(
    ([label, count]) => label.startsWith("residual_secret_at:") && count > 0,
  );
  const count = residualEntries.reduce((total, [, value]) => total + value, 0);
  const sites = residualEntries
    .map(([label]) => label.slice("residual_secret_at:".length))
    .sort();
  return useQuery({
    queryKey: copyKeys.residualSecret(count, sites),
    queryFn: () => getResidualSecretLine(count, sites),
    enabled: count > 0,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useRedactionSummary(
  redactions: Record<string, number>,
  distinct: Record<string, number>,
  enabled = true,
) {
  return useQuery({
    queryKey: copyKeys.redactions(redactions, distinct),
    queryFn: () => getRedactionSummary(redactions, distinct),
    enabled,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

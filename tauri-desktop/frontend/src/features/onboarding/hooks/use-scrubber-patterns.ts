import { useQuery } from "@tanstack/react-query";
import { getScrubberPatternNames } from "../api/onboarding-api";
import { onboardingKeys } from "../api/query-keys";

export function useScrubberPatterns(open: boolean) {
  const query = useQuery({
    queryKey: onboardingKeys.scrubberPatterns(),
    queryFn: getScrubberPatternNames,
    enabled: open,
  });
  return {
    ...query,
    names: query.data ?? [],
    state: (!open
      ? "idle"
      : query.isPending
        ? "loading"
        : query.isError
          ? "error"
          : "ready") as "idle" | "loading" | "ready" | "error",
    error: query.isError
      ? "Scrubbing details unavailable. Nothing changes until the detector list can be read."
      : null,
    refresh: async () => {
      await query.refetch();
    },
  };
}

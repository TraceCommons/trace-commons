import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import {
  getConsentOptions,
  setConsentScopes,
} from "../../onboarding/api/onboarding-api";
import { onboardingKeys } from "../../onboarding/api/query-keys";
import type { ConsentOption } from "../../onboarding/types";
import { profileKeys } from "../../profile/api/query-keys";
import { settingsKeys } from "../api/query-keys";

export function useConsentSettings() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const query = useQuery<ConsentOption[]>({
    queryKey: onboardingKeys.consentOptions(core.scope),
    queryFn: getConsentOptions,
    enabled: core.isSuccess,
  });
  const mutation = useMutation({
    mutationFn: (scopes: string[]) => setConsentScopes(scopes),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: onboardingKeys.consentOptions(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: profileKeys.public(core.scope),
        }),
      ]);
    },
  });

  const toggle = (scopes: string[]) => mutation.mutateAsync(scopes);

  return {
    ...query,
    options: query.data ?? [],
    state: (mutation.isPending
      ? "busy"
      : query.isPending
        ? "loading"
        : query.isError
          ? "error"
          : "ready") as "busy" | "loading" | "ready" | "error",
    error: query.isError
      ? "Consent options unavailable. Refresh after Rust core starts."
      : mutation.isError
        ? "Consent choices were not changed."
        : null,
    refresh: async () => {
      await query.refetch();
    },
    toggle,
    mutation,
  };
}

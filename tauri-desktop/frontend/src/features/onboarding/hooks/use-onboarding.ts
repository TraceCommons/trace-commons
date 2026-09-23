import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { getCoreStatus, retryDaemonStartup } from "../../../lib/tauri/core-api";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { profileKeys } from "../../profile/public";
import { settingsKeys } from "../../settings/public";
import {
  acknowledgeNearAiNotice,
  enrollWithInvite,
  getConsentOptions,
  setConsentScopes,
} from "../api/onboarding-api";
import { onboardingKeys } from "../api/query-keys";
import { rootsContinueError, rootsReadiness } from "../roots-readiness";
import type { ConsentOption } from "../types";

export type OnboardingStep =
  | "welcome"
  | "roots"
  | "connect"
  | "consent"
  | "privacy"
  | "projects"
  | "done";

function previousStep(
  current: OnboardingStep,
  privacyIncluded: boolean,
): OnboardingStep {
  switch (current) {
    case "roots":
      return "welcome";
    case "connect":
      return "roots";
    case "consent":
      return "connect";
    case "privacy":
      return "consent";
    case "projects":
      return privacyIncluded ? "privacy" : "consent";
    default:
      return current;
  }
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Onboarding hook coordinates resumable steps and safety-gated mutations.
export function useOnboarding(alreadyEnrolled: boolean) {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const optionsQuery = useQuery<ConsentOption[]>({
    queryKey: onboardingKeys.consentOptions(core.scope),
    queryFn: getConsentOptions,
    enabled: core.isSuccess,
  });
  const [step, setStep] = useState<OnboardingStep>(
    alreadyEnrolled ? "consent" : "welcome",
  );
  const [privacyIncluded, setPrivacyIncluded] = useState(false);
  const options = optionsQuery.data ?? [];
  const enrollMutation = useMutation({
    mutationFn: (invite: string) => enrollWithInvite(invite.trim()),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: profileKeys.public(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: onboardingKeys.consentOptions(core.scope),
        }),
      ]);
    },
  });
  const consentMutation = useMutation({
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
  const privacyMutation = useMutation({
    mutationFn: async (scan: boolean) => {
      if (scan) await acknowledgeNearAiNotice();
    },
    onSuccess: async () => {
      await Promise.all([
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
  // Enrollment needs a running daemon. Continue waits for it rather than
  // advancing to a Connect step that would fail for an unrelated reason.
  const startMutation = useMutation({
    mutationFn: async () => {
      await retryDaemonStartup();
      const status = await queryClient.fetchQuery({
        queryKey: coreKeys.status,
        queryFn: getCoreStatus,
        staleTime: 0,
      });
      if (status.startup !== "running") throw new Error("daemon-not-running");
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
      ]);
    },
  });
  const rootsStartError = startMutation.isError
    ? rootsContinueError(startMutation.error)
    : null;
  const refreshEnrollment = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: coreKeys.status }),
      queryClient.invalidateQueries({
        queryKey: settingsKeys.snapshot(core.scope),
      }),
      queryClient.invalidateQueries({
        queryKey: profileKeys.public(core.scope),
      }),
      queryClient.invalidateQueries({
        queryKey: onboardingKeys.consentOptions(core.scope),
      }),
    ]);
  };
  const state =
    enrollMutation.isPending ||
    consentMutation.isPending ||
    privacyMutation.isPending
      ? "busy"
      : optionsQuery.isPending
        ? "loading"
        : optionsQuery.isError ||
            enrollMutation.isError ||
            consentMutation.isError ||
            privacyMutation.isError
          ? "error"
          : "ready";
  const error = optionsQuery.isError
    ? "Consent options unavailable. Restart Rust core and retry."
    : enrollMutation.isError
      ? "Enrollment failed. Check invite link and allowed issuer configuration."
      : consentMutation.isError
        ? "Consent choices were not saved. Nothing proceeds until the daemon confirms them."
        : privacyMutation.isError
          ? "Privacy-scan choice was not saved."
          : null;
  const startRoots = () => {
    startMutation.reset();
    setStep("roots");
  };
  const continueRoots = async (snapshot: Record<string, unknown>) => {
    const readiness = rootsReadiness(snapshot, core.data?.startup);
    if (readiness === "undeclared") return;
    if (readiness === "running") {
      setStep("connect");
      return;
    }
    try {
      await startMutation.mutateAsync();
      setStep("connect");
    } catch {
      // The roots step renders the start failure from shared copy.
    }
  };
  const back = () => {
    setStep((current) => previousStep(current, privacyIncluded));
  };
  const enroll = async (invite: string) => {
    try {
      await enrollMutation.mutateAsync(invite);
      setStep("consent");
    } catch {
      // The mutation state supplies the existing error copy.
    }
  };
  const markEnrolled = async () => {
    try {
      await refreshEnrollment();
      setStep("consent");
    } catch {
      // Query state remains authoritative; do not advance on a stale refresh.
    }
  };
  const saveConsent = async (scopes: string[], showPrivacy: boolean) => {
    try {
      await consentMutation.mutateAsync(scopes);
      setPrivacyIncluded(showPrivacy);
      setStep(showPrivacy ? "privacy" : "projects");
    } catch {
      // The mutation state supplies the existing error copy.
    }
  };
  const savePrivacy = async (scan: boolean) => {
    try {
      await privacyMutation.mutateAsync(scan);
      setStep("projects");
    } catch {
      // The mutation state supplies the existing error copy.
    }
  };
  const finishProjects = () => {
    setStep("done");
  };
  return {
    ...optionsQuery,
    options,
    step,
    state: state as "busy" | "loading" | "ready" | "error",
    error,
    refresh: async () => {
      await optionsQuery.refetch();
    },
    startRoots,
    continueRoots,
    startingDaemon: startMutation.isPending,
    rootsStartError,
    back,
    enroll,
    markEnrolled,
    saveConsent,
    savePrivacy,
    finishProjects,
    setStep,
    optionsQuery,
    enrollMutation,
    consentMutation,
    privacyMutation,
  };
}

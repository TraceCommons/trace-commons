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
  grantAutomatic,
  setConsentScopes,
  withdrawAutomaticGrant,
} from "../api/onboarding-api";
import { onboardingKeys } from "../api/query-keys";
import {
  type ContributionPath,
  type Flow1Progress,
  grantBlockers,
  initialFlow1Progress,
  requestGrant,
} from "../flow1";
import { rootsContinueError, rootsReadiness } from "../roots-readiness";
import type { ConsentOption } from "../types";

// The order is the spec's (connect-and-forget design, R7): source roots are
// settled before connect, the scope picker runs immediately after connect
// and before the path question, and the automatic path reaches the grant
// only through both disclosure screens.
export type OnboardingStep =
  | "welcome"
  | "roots"
  | "connect"
  | "consent"
  | "path"
  | "privacy"
  | "disclosure_scrub"
  | "disclosure_witness"
  | "grant"
  | "projects"
  | "done";

function afterPrivacy(path: ContributionPath | null): OnboardingStep {
  return path === "automatic" ? "disclosure_scrub" : "projects";
}

function previousStep(
  current: OnboardingStep,
  privacyIncluded: boolean,
  path: ContributionPath | null,
  scopesChosen: boolean,
): OnboardingStep {
  switch (current) {
    case "roots":
      return "welcome";
    case "connect":
      return "roots";
    case "consent":
      return "connect";
    case "path":
      return "consent";
    case "privacy":
      return scopesChosen ? "path" : "consent";
    case "disclosure_scrub":
      return privacyIncluded ? "privacy" : "path";
    case "disclosure_witness":
      return "disclosure_scrub";
    case "grant":
      return "disclosure_witness";
    case "projects":
      if (privacyIncluded) return "privacy";
      return scopesChosen && path !== null ? "path" : "consent";
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
  const [progress, setProgress] = useState<Flow1Progress>(initialFlow1Progress);
  const options = optionsQuery.data ?? [];
  // Connected is the daemon's answer, never a step the shell remembers.
  const flow1: Flow1Progress = {
    ...progress,
    connected: core.data?.daemon.logged_in === true,
  };
  const invalidateAccount = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: coreKeys.status }),
      queryClient.invalidateQueries({
        queryKey: settingsKeys.snapshot(core.scope),
      }),
      queryClient.invalidateQueries({
        queryKey: profileKeys.public(core.scope),
      }),
    ]);
  const enrollMutation = useMutation({
    mutationFn: (invite: string) => enrollWithInvite(invite.trim()),
    onSuccess: async () => {
      await Promise.all([
        invalidateAccount(),
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
        invalidateAccount(),
        queryClient.invalidateQueries({
          queryKey: onboardingKeys.consentOptions(core.scope),
        }),
      ]);
    },
  });
  const privacyMutation = useMutation({
    mutationFn: async (scan: boolean) => {
      if (scan) await acknowledgeNearAiNotice();
    },
    onSuccess: invalidateAccount,
  });
  const grantMutation = useMutation({
    mutationFn: (current: Flow1Progress) => requestGrant(current, grantAutomatic),
    onSuccess: invalidateAccount,
  });
  const withdrawMutation = useMutation({
    mutationFn: withdrawAutomaticGrant,
    onSuccess: async () => {
      grantMutation.reset();
      await invalidateAccount();
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
      invalidateAccount(),
      queryClient.invalidateQueries({
        queryKey: onboardingKeys.consentOptions(core.scope),
      }),
    ]);
  };
  const state =
    enrollMutation.isPending ||
    consentMutation.isPending ||
    privacyMutation.isPending ||
    grantMutation.isPending ||
    withdrawMutation.isPending
      ? "busy"
      : optionsQuery.isPending
        ? "loading"
        : optionsQuery.isError ||
            enrollMutation.isError ||
            consentMutation.isError ||
            privacyMutation.isError ||
            grantMutation.isError ||
            withdrawMutation.isError
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
          : grantMutation.isError
            ? "Automatic contributing was not turned on. Nothing changed."
            : withdrawMutation.isError
              ? "Automatic contributing was not turned off. Try again from Settings."
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
    setStep((current) =>
      previousStep(
        current,
        privacyIncluded,
        progress.path,
        progress.scopesSaved !== null,
      ),
    );
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
  // The scope picker's Continue: the daemon saves what was chosen, and only
  // then does the path question appear.
  const saveConsent = async (scopes: string[]) => {
    try {
      await consentMutation.mutateAsync(scopes);
      setProgress((current) => ({
        ...current,
        scopesSaved: scopes,
        path: null,
        scrubDisclosureSeen: false,
        witnessDisclosureSeen: false,
      }));
      setStep("path");
    } catch {
      // The mutation state supplies the existing error copy.
    }
  };
  // R7: declining to choose is not a floor-scope grant. Nothing is saved,
  // no grant is possible, and the contributor lands on Flow 2.
  const declineScopes = (showPrivacy: boolean) => {
    setProgress({ ...initialFlow1Progress, path: "ask_first" });
    setPrivacyIncluded(showPrivacy);
    setStep(showPrivacy ? "privacy" : "projects");
  };
  const choosePath = (path: ContributionPath, showPrivacy: boolean) => {
    setProgress((current) => ({
      ...current,
      path,
      scrubDisclosureSeen: false,
      witnessDisclosureSeen: false,
    }));
    setPrivacyIncluded(showPrivacy);
    setStep(showPrivacy ? "privacy" : afterPrivacy(path));
  };
  const savePrivacy = async (scan: boolean) => {
    try {
      await privacyMutation.mutateAsync(scan);
      setStep(afterPrivacy(progress.path));
    } catch {
      // The mutation state supplies the existing error copy.
    }
  };
  const acknowledgeScrubDisclosure = () => {
    setProgress((current) => ({ ...current, scrubDisclosureSeen: true }));
    setStep("disclosure_witness");
  };
  const acknowledgeWitnessDisclosure = () => {
    setProgress((current) => ({ ...current, witnessDisclosureSeen: true }));
    setStep("grant");
  };
  const grant = async () => {
    try {
      await grantMutation.mutateAsync(flow1);
      setStep("done");
    } catch {
      // The mutation state supplies the error copy; nothing was granted.
    }
  };
  // Leaving the grant screen without granting is Flow 2, not a half grant.
  const skipGrant = () => {
    setProgress((current) => ({ ...current, path: "ask_first" }));
    setStep("projects");
  };
  const withdrawGrant = async () => {
    try {
      await withdrawMutation.mutateAsync();
    } catch {
      // The mutation state supplies the error copy.
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
    declineScopes,
    choosePath,
    savePrivacy,
    acknowledgeScrubDisclosure,
    acknowledgeWitnessDisclosure,
    grant,
    grantBlockers: grantBlockers(flow1),
    granted: grantMutation.data?.granted === true,
    skipGrant,
    withdrawGrant,
    withdrawn: withdrawMutation.data === true,
    finishProjects,
    progress: flow1,
    setStep,
    optionsQuery,
    enrollMutation,
    consentMutation,
    privacyMutation,
    grantMutation,
  };
}

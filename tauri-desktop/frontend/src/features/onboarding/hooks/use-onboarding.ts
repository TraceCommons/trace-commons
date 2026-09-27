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
  getAutomaticGrant,
  getConsentOptions,
  grantAutomatic,
  setConsentScopes,
  withdrawAutomaticGrant,
} from "../api/onboarding-api";
import { onboardingKeys } from "../api/query-keys";
import {
  acknowledgeWitnessDisclosure as withWitnessRead,
  afterPrivacy,
  type ContributionPath,
  decideLater,
  type Flow1Progress,
  goBack,
  grantBlockers,
  initialFlow1Progress,
  type OnboardingStep,
  requestGrant,
  withdrawAndConfirm,
} from "../flow1";
import { rootsContinueError, rootsReadiness } from "../roots-readiness";
import type { ConsentOption } from "../types";

export type { OnboardingStep } from "../flow1";

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
  const invalidateGrant = () =>
    queryClient.invalidateQueries({
      queryKey: onboardingKeys.automaticGrant(core.scope),
    });
  const grantMutation = useMutation({
    mutationFn: (current: Flow1Progress) =>
      requestGrant(current, grantAutomatic),
    onSuccess: async () => {
      await Promise.all([invalidateAccount(), invalidateGrant()]);
    },
  });
  // Confirmed from the daemon's status after the withdraw, not from the
  // withdraw call's answer (`withdrawAndConfirm`).
  const withdrawMutation = useMutation({
    mutationFn: () =>
      withdrawAndConfirm(withdrawAutomaticGrant, getAutomaticGrant),
    onSettled: async () => {
      await Promise.all([invalidateAccount(), invalidateGrant()]);
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
              ? "Automatic contributing was not turned off. Try again here or from Settings, under Automatic contributing."
              : withdrawMutation.data === "still_granted"
                ? "Automatic contributing is still on. Try again here or from Settings, under Automatic contributing."
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
    const next = goBack(progress, step, privacyIncluded);
    setProgress(next.progress);
    setStep(next.step);
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
        witnessShown: null,
      }));
      setStep("path");
    } catch {
      // The mutation state supplies the existing error copy.
    }
  };
  // R7: declining to choose is not a floor-scope grant. Nothing is saved,
  // no grant is possible, and the contributor lands on Flow 2.
  const declineScopes = (showPrivacy: boolean) => {
    const next = decideLater(showPrivacy);
    setProgress(next.progress);
    setPrivacyIncluded(next.privacyIncluded);
    setStep(next.step);
  };
  const choosePath = (path: ContributionPath, showPrivacy: boolean) => {
    setProgress((current) => ({
      ...current,
      path,
      scrubDisclosureSeen: false,
      witnessDisclosureSeen: false,
      witnessShown: null,
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
  // `signingAddress` is the witness the screen showed, `null` for none; the
  // grant is given under it or refused.
  const acknowledgeWitnessDisclosure = (signingAddress: string | null) => {
    setProgress((current) => withWitnessRead(current, signingAddress));
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
    withdrawn: withdrawMutation.data === "withdrawn",
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

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import {
  type CredentialStatus,
  getCredentialStatus,
  openExternalUrl,
  privateAiKeys,
  startCredential,
} from "../../private-ai";
import { nearAiAccountEnroll } from "../api/onboarding-api";

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: This hook owns the three-state sign-in and enrollment safety flow.
export function useOnboardingNearAi(onEnrolled: () => void) {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const [commons, setCommons] = useState("");
  const [provider, setProvider] = useState("github");
  const [browserUrl, setBrowserUrl] = useState<string | null>(null);
  const credential = useQuery<CredentialStatus>({
    queryKey: privateAiKeys.credential(core.scope),
    queryFn: getCredentialStatus,
    enabled: core.isSuccess,
    refetchInterval: (query) =>
      query.state.data?.state === "obtaining" ? 2000 : false,
  });
  const start = useMutation({
    mutationFn: () => startCredential(provider),
    onSuccess: async (result) => {
      setBrowserUrl(result.browserUrl);
      await queryClient.invalidateQueries({
        queryKey: privateAiKeys.credential(core.scope),
      });
    },
  });
  const open = useMutation({ mutationFn: openExternalUrl });
  const join = useMutation({
    mutationFn: () => nearAiAccountEnroll(commons.trim()),
    onSuccess: (result) => {
      if (result.enrolled) onEnrolled();
    },
  });
  const signedIn = credential.data?.session_state === "present";
  const busy = start.isPending || open.isPending || join.isPending;
  const error = credential.isError
    ? "NEAR AI sign-in status unavailable. Start Rust core and retry."
    : start.isError
      ? "NEAR AI sign-in could not start. Check provider availability."
      : open.isError
        ? "Sign-in destination could not be opened."
        : join.isError
          ? "NEAR AI enrollment failed. Check commons availability and try again."
          : join.isSuccess && !join.data.enrolled
            ? (join.data.message ?? "NEAR AI enrollment was not confirmed.")
            : null;
  return {
    commons,
    setCommons,
    provider,
    setProvider,
    browserUrl,
    credential,
    signedIn,
    busy,
    error,
    start,
    open,
    join,
  };
}

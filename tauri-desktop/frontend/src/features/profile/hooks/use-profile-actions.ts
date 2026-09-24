import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { settingsKeys } from "../../settings/public";
import { publishProfile, withdrawProfile } from "../api/profile-api";
import { profileKeys } from "../api/query-keys";

export function useProfileActions() {
  const queryClient = useQueryClient();
  const core = useCoreStatus();
  const [state, setState] = useState<
    "idle" | "publishing" | "withdrawing" | "error"
  >("idle");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const publishMutation = useMutation({
    mutationFn: ({ handle, bio }: { handle: string; bio: string }) =>
      publishProfile(handle, bio.trim() || null),
    onSuccess: async ({ profile }) => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: profileKeys.public(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
      ]);
      queryClient.setQueryData(profileKeys.public(core.scope), profile);
    },
  });
  const withdrawMutation = useMutation({
    mutationFn: withdrawProfile,
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: profileKeys.public(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
      ]);
    },
  });

  async function publish(handle: string, bio: string) {
    const cleanHandle = handle.trim();
    if (!cleanHandle) {
      setState("error");
      setError("Handle is required.");
      setNotice(null);
      return false;
    }
    setState("publishing");
    setError(null);
    setNotice(null);
    try {
      const result = await publishMutation.mutateAsync({
        handle: cleanHandle,
        bio,
      });
      setState("idle");
      if (!result.handlePersisted) {
        setNotice(
          "Profile is public, but this device could not save its local copy. It may disappear here after refresh or restart.",
        );
      }
      return result;
    } catch {
      setState("error");
      setError(
        "Profile was not published. Check local enrollment and network access.",
      );
      return null;
    }
  }

  async function withdraw() {
    setState("withdrawing");
    setError(null);
    setNotice(null);
    try {
      await withdrawMutation.mutateAsync();
      setState("idle");
      return true;
    } catch {
      setState("error");
      setError("Profile was not withdrawn.");
      return false;
    }
  }

  return {
    state,
    error,
    notice,
    publish,
    withdraw,
    isPending: publishMutation.isPending || withdrawMutation.isPending,
    publishMutation,
    withdrawMutation,
  };
}

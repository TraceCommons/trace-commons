import { useMutation, useQueryClient } from "@tanstack/react-query";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { type BehaviorSetting, saveBehaviorSetting } from "../api/behavior-api";
import { settingsKeys } from "../api/query-keys";

export function useBehaviorSettings() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: ({
      setting,
      value,
    }: {
      setting: BehaviorSetting;
      value: number;
    }) => saveBehaviorSetting(setting, value),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
      ]);
    },
  });

  return {
    busy: mutation.isPending ? (mutation.variables?.setting ?? null) : null,
    error: mutation.isError
      ? "Setting was not changed. Use a value inside the displayed range."
      : null,
    save: (setting: BehaviorSetting, value: number) =>
      mutation.mutateAsync({ setting, value }),
    mutation,
  };
}

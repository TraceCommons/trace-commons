import { useMutation, useQueryClient } from "@tanstack/react-query";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { settingsKeys } from "../api/query-keys";
import {
  type SourceMode,
  type SourceName,
  setSourceDeclaration,
} from "../api/source-roots-api";

export function useSourceRoots() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: ({
      source,
      mode,
      path,
    }: {
      source: SourceName;
      mode: SourceMode;
      path: string;
    }) =>
      setSourceDeclaration(source, mode, mode === "watch" ? path : undefined),
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
    busy: mutation.isPending,
    error: mutation.isError
      ? "Source declaration was not saved. Use an absolute directory path."
      : null,
    save: (source: SourceName, mode: SourceMode, path: string) =>
      mutation.mutateAsync({ source, mode, path }),
    mutation,
  };
}

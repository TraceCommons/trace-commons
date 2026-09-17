import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import {
  changeProjectMode,
  getProjects,
  type ProjectMode,
} from "../api/projects-api";
import { settingsKeys } from "../api/query-keys";

export function useProjects() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: settingsKeys.projects(core.scope),
    queryFn: getProjects,
    enabled: core.isSuccess,
  });
  const mutation = useMutation({
    mutationFn: ({
      projectId,
      mode,
    }: {
      projectId: string;
      mode: ProjectMode;
    }) => changeProjectMode(projectId, mode),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: settingsKeys.projects(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
      ]);
    },
  });

  return {
    ...query,
    projects: query.data ?? [],
    state: (mutation.isPending
      ? "busy"
      : query.isPending
        ? "loading"
        : query.isError
          ? "error"
          : "ready") as "busy" | "loading" | "ready" | "error",
    error: query.isError
      ? "Projects unavailable. Refresh after Rust core starts."
      : mutation.isError
        ? "Project mode was not changed."
        : null,
    refresh: async () => {
      await query.refetch();
    },
    setMode: (projectId: string, mode: ProjectMode) =>
      mutation.mutateAsync({ projectId, mode }),
    mutation,
  };
}

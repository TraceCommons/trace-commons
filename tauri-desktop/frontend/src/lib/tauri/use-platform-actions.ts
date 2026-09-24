import { useMutation } from "@tanstack/react-query";
import { openExternalUrl, pickDirectory } from "./platform-api";

export type DirectoryPurpose = "repository" | "source_root";

export function useDirectoryPicker() {
  const mutation = useMutation({
    mutationFn: (purpose: DirectoryPurpose) => pickDirectory(purpose),
  });
  return {
    ...mutation,
    pick: mutation.mutateAsync,
  };
}

export function useExternalUrl() {
  const mutation = useMutation({
    mutationFn: openExternalUrl,
  });
  return {
    ...mutation,
    open: mutation.mutateAsync,
  };
}

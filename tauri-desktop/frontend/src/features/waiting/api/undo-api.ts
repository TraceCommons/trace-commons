import { invokeTauriDiscardResult } from "../../../lib/tauri/core-api";

export type UndoScope = {
  kind: "entry" | "project";
  id: string;
  hold_until: string | null;
  label: string;
};

export async function cancelApproval(scope: UndoScope) {
  return invokeTauriDiscardResult(
    scope.kind === "entry" ? "cancel_entry" : "cancel_project",
    scope.kind === "entry" ? { entryId: scope.id } : { projectId: scope.id },
  );
}

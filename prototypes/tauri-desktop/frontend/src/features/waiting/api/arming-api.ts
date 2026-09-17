import { invokeTauri } from "../../../lib/tauri/core-api";

export type ArmingOffer = {
  project_id: string;
  project_label: string;
  contributed_count: number;
};

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error("Invalid arming response");
  return value as Record<string, unknown>;
}

export async function getArmingSuggestion(): Promise<ArmingOffer | null> {
  const value = record(await invokeTauri<unknown>("arming_suggestion"));
  if (Object.keys(value).length === 0) return null;
  if (
    typeof value.project_id !== "string" ||
    typeof value.project_label !== "string" ||
    typeof value.contributed_count !== "number"
  )
    throw new Error("Invalid arming response");
  return {
    project_id: value.project_id,
    project_label: value.project_label,
    contributed_count: value.contributed_count,
  };
}

export async function acceptArming(projectId: string) {
  return invokeTauri<unknown>("accept_arming", { projectId });
}
export async function declineArming(projectId: string) {
  return invokeTauri<unknown>("decline_arming", { projectId });
}

import { invokeTauri } from "../../../lib/tauri/core-api";

export type ProjectMode = "notify_only" | "auto_upload" | "ignore";
export type Project = {
  project_id: string;
  project_label: string;
  project_path: string;
  mode: ProjectMode;
  configured: boolean;
  is_unresolved_bucket: boolean;
  pending_count?: number;
  contributable_count?: number;
};

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error("Invalid projects response");
  return value as Record<string, unknown>;
}
function string(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid project field: ${key}`);
  return value[key] as string;
}
function boolean(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "boolean")
    throw new Error(`Invalid project field: ${key}`);
  return value[key] as boolean;
}
function optionalNumber(value: Record<string, unknown>, key: string) {
  if (value[key] !== undefined && typeof value[key] !== "number")
    throw new Error(`Invalid project field: ${key}`);
  return value[key] as number | undefined;
}

export async function getProjects(): Promise<Project[]> {
  const value = record(await invokeTauri<unknown>("list_projects"));
  if (!Array.isArray(value.projects)) throw new Error("Invalid project rows");
  return value.projects.map((raw) => {
    const row = record(raw);
    const mode = string(row, "mode");
    if (!["notify_only", "auto_upload", "ignore"].includes(mode))
      throw new Error("Invalid project mode");
    return {
      project_id: string(row, "project_id"),
      project_label: string(row, "project_label"),
      project_path: string(row, "project_path"),
      mode: mode as ProjectMode,
      configured: boolean(row, "configured"),
      is_unresolved_bucket: boolean(row, "is_unresolved_bucket"),
      pending_count: optionalNumber(row, "pending_count"),
      contributable_count: optionalNumber(row, "contributable_count"),
    };
  });
}

export async function changeProjectMode(projectId: string, mode: ProjectMode) {
  return invokeTauri<unknown>("set_project_mode", { projectId, mode });
}

import { invokeTauri } from "../../../lib/tauri/core-api";

export type AuditEntry = {
  at: string;
  action: string;
  project_label: string | null;
  detail: string | null;
};

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("Invalid audit response");
  }
  return value as Record<string, unknown>;
}

export async function getAudit(): Promise<AuditEntry[]> {
  const value = record(await invokeTauri("list_audit"));
  if (!Array.isArray(value.entries)) throw new Error("Invalid audit entries");
  return value.entries.map((raw) => {
    const entry = record(raw);
    if (
      typeof entry.at !== "string" ||
      typeof entry.action !== "string" ||
      (entry.project_label !== null &&
        typeof entry.project_label !== "string") ||
      (entry.detail !== null && typeof entry.detail !== "string")
    ) {
      throw new Error("Invalid audit entry");
    }
    return entry as unknown as AuditEntry;
  });
}

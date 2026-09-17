import { invokeTauri } from "../../../lib/tauri/core-api";
import type { ComputeSnapshot } from "../types";

function parseSnapshot(value: unknown): ComputeSnapshot {
  if (typeof value !== "object" || value === null)
    throw new Error("Invalid compute response");
  const item = value as Record<string, unknown>;
  if (
    typeof item.state !== "string" ||
    typeof item.reason !== "string" ||
    typeof item.title !== "string" ||
    typeof item.detail !== "string" ||
    typeof item.consent_granted !== "boolean" ||
    (item.ram_allowance_gib !== null &&
      typeof item.ram_allowance_gib !== "number") ||
    typeof item.available !== "boolean" ||
    typeof item.can_enable !== "boolean" ||
    typeof item.can_resume !== "boolean" ||
    typeof item.can_pause !== "boolean" ||
    typeof item.worker_stopped !== "boolean"
  )
    throw new Error("Invalid compute response");
  const copy = item.copy;
  if (typeof copy !== "object" || copy === null)
    throw new Error("Invalid compute copy");
  const text = copy as Record<string, unknown>;
  const copyKeys = [
    "introduction",
    "allowance_label",
    "allowance_detail",
    "enable",
    "resume",
    "pause",
    "disable",
  ];
  if (copyKeys.some((key) => typeof text[key] !== "string"))
    throw new Error("Invalid compute copy");
  return item as unknown as ComputeSnapshot;
}

export async function getComputeStatus() {
  return parseSnapshot(await invokeTauri<unknown>("compute_status"));
}
export async function changeCompute(
  command:
    | "enable_compute"
    | "resume_compute"
    | "pause_compute"
    | "disable_compute",
  ramAllowance?: number,
) {
  return parseSnapshot(
    await invokeTauri<unknown>(
      command,
      ramAllowance === undefined ? {} : { ramAllowanceGib: ramAllowance },
    ),
  );
}

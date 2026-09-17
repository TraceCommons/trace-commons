import { daemonCall, invokeTauri } from "../../../lib/tauri/core-api";
import type { SettingsSnapshot } from "../types";

export async function getSettings(): Promise<SettingsSnapshot> {
  const value = await daemonCall<unknown>("get_settings");
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error("Invalid settings response");
  return value as SettingsSnapshot;
}

export function setDaemonState(command: "pause_daemon" | "resume_daemon") {
  return invokeTauri(command);
}

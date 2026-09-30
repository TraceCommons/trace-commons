import { z } from "zod";
import {
  daemonCall,
  invokeTauriDiscardResult,
} from "../../../lib/tauri/core-api";
import type { SettingsSnapshot } from "../types";

const settingsSnapshotSchema = z.record(z.string(), z.unknown());

export async function getSettings(): Promise<SettingsSnapshot> {
  const value = await daemonCall("get_settings");
  const result = settingsSnapshotSchema.safeParse(value);
  if (!result.success) {
    throw new Error("Invalid settings response");
  }
  return result.data;
}

export function setDaemonState(command: "pause_daemon" | "resume_daemon") {
  return invokeTauriDiscardResult(command);
}

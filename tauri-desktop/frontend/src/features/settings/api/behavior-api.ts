import { invokeTauriDiscardResult } from "../../../lib/tauri/core-api";

export type BehaviorSetting =
  | "quiescence"
  | "approval_hold"
  | "digest";

export async function saveBehaviorSetting(
  setting: BehaviorSetting,
  value: number,
) {
  const commands: Record<BehaviorSetting, [string, string]> = {
    quiescence: ["set_quiescence_minutes", "minutes"],
    approval_hold: ["set_approval_hold_seconds", "seconds"],
    digest: ["set_digest_hours", "hours"],
  };
  const [command, key] = commands[setting];
  return invokeTauriDiscardResult(command, { [key]: value });
}

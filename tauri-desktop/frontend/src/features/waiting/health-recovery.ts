import type { ContributorDisclosureCopy } from "../../lib/tauri/contributor-copy-api";

export const NEAR_AI_NOTICE_LABEL = "near-ai-notice-not-acknowledged";

type PrivacyScanCopy = ContributorDisclosureCopy["privacy_scan"];

export type NearAiNoticeRecovery =
  | { kind: "none" }
  | { kind: "loading" }
  | { kind: "unavailable" }
  | { kind: "ready"; copy: PrivacyScanCopy };

// Fail closed: without the shared notice text there is nothing to confirm,
// so no confirmation is offered.
export function nearAiNoticeRecovery(
  label: string | null | undefined,
  copy: PrivacyScanCopy | undefined,
  copyFailed: boolean,
): NearAiNoticeRecovery {
  if (label !== NEAR_AI_NOTICE_LABEL) return { kind: "none" };
  if (copy) return { kind: "ready", copy };
  return copyFailed ? { kind: "unavailable" } : { kind: "loading" };
}

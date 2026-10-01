import { privateAiStateCopy } from "../../../lib/tauri/contributor-copy-api";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import { useSettings } from "../../settings/public";

/**
 * What Private AI is doing right now, in the core's words: the sentence for
 * the runtime state the daemon reports, and whether an indicator may paint
 * it as working. `line` is null until both the settings snapshot and the
 * core's copy have loaded, so nothing is said in the meantime. The switch
 * (`private_inference`) says what was asked for; this says what is true.
 */
export function usePrivateAiState(): {
  line: string | null;
  working: boolean;
} {
  const settings = useSettings();
  const disclosure = useContributorDisclosureCopy();
  if (!settings.data || !disclosure.data) return { line: null, working: false };
  return privateAiStateCopy(
    disclosure.data.private_inference,
    settings.data.private_inference_state,
  );
}

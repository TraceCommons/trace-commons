import { invokeTauri } from "../../../lib/tauri/core-api";

export type SourceName = "claude" | "codex" | "gemini" | "cline" | "opencode";
export type SourceMode = "watch" | "off";

export async function setSourceDeclaration(
  source: SourceName,
  mode: SourceMode,
  path?: string,
) {
  return invokeTauri<unknown>("set_source_declaration", { source, mode, path });
}

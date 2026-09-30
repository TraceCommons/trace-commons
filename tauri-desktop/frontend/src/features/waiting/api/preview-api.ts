import { invokeTauri } from "../../../lib/tauri/core-api";

export type PreviewBodyPage = {
  total_bytes: number;
  offset: number;
  chunk: string;
  next_offset: number | null;
  body_digest: string;
  envelope_digest: string;
  enrolled: boolean;
};
export type PreviewTurn = {
  index: number;
  role: string;
  tool_name?: string | null;
  byte_offset: number;
  byte_len: number;
};

function record(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error(`Invalid preview ${label}`);
  return value as Record<string, unknown>;
}
function string(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid preview field: ${key}`);
  return value[key] as string;
}
function number(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "number")
    throw new Error(`Invalid preview field: ${key}`);
  return value[key] as number;
}
function nullableNumber(value: Record<string, unknown>, key: string) {
  if (value[key] === null || value[key] === undefined) return null;
  return number(value, key);
}

export async function getPreviewBodyPage(
  entryId: string,
  offset = 0,
  bodyDigest?: string,
): Promise<PreviewBodyPage> {
  const item = record(
    await invokeTauri("preview_body", {
      entryId,
      offset,
      limit: 256 * 1024,
      bodyDigest: bodyDigest ?? null,
    }),
    "body",
  );
  if (typeof item.chunk !== "string" || typeof item.enrolled !== "boolean")
    throw new Error("Invalid preview body");
  return {
    total_bytes: number(item, "total_bytes"),
    offset: number(item, "offset"),
    chunk: item.chunk,
    next_offset: nullableNumber(item, "next_offset"),
    body_digest: string(item, "body_digest"),
    envelope_digest: string(item, "envelope_digest"),
    enrolled: item.enrolled,
  };
}

export async function getPreviewTurns(
  entryId: string,
  bodyDigest: string,
): Promise<PreviewTurn[]> {
  const item = record(
    await invokeTauri("preview_turns", { entryId, bodyDigest }),
    "turns",
  );
  if (!Array.isArray(item.turns)) throw new Error("Invalid preview turns");
  return item.turns.map((value) => {
    const turn = record(value, "turn");
    if (
      typeof turn.index !== "number" ||
      typeof turn.role !== "string" ||
      typeof turn.byte_offset !== "number" ||
      typeof turn.byte_len !== "number"
    )
      throw new Error("Invalid preview turn");
    return {
      index: turn.index,
      role: turn.role,
      tool_name:
        turn.tool_name === undefined || turn.tool_name === null
          ? null
          : string(turn, "tool_name"),
      byte_offset: turn.byte_offset,
      byte_len: turn.byte_len,
    };
  });
}

export async function searchOriginal(entryId: string, needle: string) {
  const item = record(
    await invokeTauri("search_original", { entryId, needle }),
    "search",
  );
  return number(item, "matches");
}

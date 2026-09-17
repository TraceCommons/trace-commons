import { invokeTauri } from "../../../lib/tauri/core-api";

export type RoutingDiscovery = {
  found: boolean;
  port: number | null;
  token_path: string | null;
};
export type RoutingEvidence = {
  outcome: string;
  tools: Array<{ id: string; installed: boolean; wired: boolean }>;
};

function record(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error(`Invalid routing ${label}`);
  return value as Record<string, unknown>;
}

function nullableNumber(value: Record<string, unknown>, key: string) {
  if (value[key] === undefined || value[key] === null) return null;
  if (typeof value[key] !== "number")
    throw new Error(`Invalid routing field: ${key}`);
  return value[key];
}

function nullableString(value: Record<string, unknown>, key: string) {
  if (value[key] === undefined || value[key] === null) return null;
  if (typeof value[key] !== "string")
    throw new Error(`Invalid routing field: ${key}`);
  return value[key];
}

function parseDiscovery(value: unknown): RoutingDiscovery {
  const item = record(value, "discovery");
  if (typeof item.found !== "boolean")
    throw new Error("Invalid routing discovery");
  return {
    found: item.found,
    port: nullableNumber(item, "port"),
    token_path: nullableString(item, "token_path"),
  };
}

function parseEvidence(value: unknown): RoutingEvidence {
  const item = record(value, "evidence");
  if (typeof item.outcome !== "string" || !Array.isArray(item.tools))
    throw new Error("Invalid routing evidence");
  const tools = item.tools.map((value) => {
    const tool = record(value, "tool");
    if (
      typeof tool.id !== "string" ||
      typeof tool.installed !== "boolean" ||
      typeof tool.wired !== "boolean"
    )
      throw new Error("Invalid routing tool");
    return { id: tool.id, installed: tool.installed, wired: tool.wired };
  });
  return { outcome: item.outcome, tools };
}

export async function discoverRouting() {
  return parseDiscovery(await invokeTauri<unknown>("discover_routing"));
}
export async function configureRouting(
  enabled: boolean,
  port: number,
  tokenDir: string,
) {
  return invokeTauri<unknown>("configure_routing", {
    enabled,
    port,
    token_dir: tokenDir.trim() || null,
  });
}
export async function probeRouting(port: number, tokenDir: string) {
  return record(
    await invokeTauri<unknown>("probe_routing", {
      port,
      token_dir: tokenDir.trim() || null,
    }),
    "probe",
  );
}
export async function probeRoutedTools(port: number, tokenDir: string) {
  return parseEvidence(
    await invokeTauri<unknown>("probe_routed_tools", {
      port,
      token_dir: tokenDir.trim() || null,
    }),
  );
}

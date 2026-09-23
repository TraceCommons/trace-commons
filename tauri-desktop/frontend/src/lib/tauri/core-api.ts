import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { CoreStatus } from "./types";
import { coreStatusSchema } from "./types";

function tauriInvoke(
  command: string,
  args: Record<string, unknown> = {},
): Promise<unknown> {
  if (!isTauri()) throw new Error("Rust core is available only inside Tauri");
  return invoke<unknown>(command, args).catch((error: unknown) => {
    throw normalizeInvokeError(error);
  });
}

function normalizeInvokeError(error: unknown): Error {
  if (error instanceof Error) return error;
  if (typeof error === "string" && error.trim() !== "") {
    return new Error(error);
  }
  if (isRecord(error) && typeof error.message === "string") {
    return new Error(error.message);
  }
  return new Error("Rust command failed");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function parseCoreStatus(value: unknown): CoreStatus {
  const result = coreStatusSchema.safeParse(value);
  if (!result.success) {
    throw new Error("Rust core returned an invalid status payload");
  }
  return result.data;
}

export async function getCoreStatus(): Promise<CoreStatus> {
  if (!isTauri()) {
    return {
      state_dir: "browser-preview",
      startup: "needs_roots",
      daemon: {
        schema_version: "browser-preview",
        logged_in: false,
        tenant_id: null,
        consent_scopes: [],
        paused: false,
        queue_depth: 0,
        health: { last_error_label: null, since: null },
      },
    };
  }
  return parseCoreStatus(await tauriInvoke("core_status"));
}

export async function retryDaemonStartup(): Promise<void> {
  await invokeTauriVoid("retry_daemon_startup");
}

export function isTauriRuntime(): boolean {
  return isTauri();
}

type DaemonReadMethod =
  | "get_public_profile"
  | "get_settings"
  | "history_rollup"
  | "list_history"
  | "list_pending"
  | "queue_outcome_counts";

export function daemonCall(
  method: DaemonReadMethod,
  params: Record<string, unknown> = {},
): Promise<unknown> {
  return tauriInvoke("daemon_call", { method, params });
}

export function invokeTauri(
  command: string,
  args: Record<string, unknown> = {},
): Promise<unknown> {
  return tauriInvoke(command, args);
}

export function invokeTauriBytes(
  command: string,
  body: Uint8Array,
  headers: Record<string, string> = {},
): Promise<unknown> {
  if (!isTauri()) throw new Error("Rust core is available only inside Tauri");
  for (const [name, value] of Object.entries(headers)) {
    if (
      name.length > 128 ||
      !/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(name) ||
      value.length > 128 ||
      !/^[\x20-\x7e]*$/.test(value)
    ) {
      throw new Error("Invalid raw upload header");
    }
  }
  return invoke<unknown>(command, body, { headers }).catch(
    (error: unknown) => {
      throw normalizeInvokeError(error);
    },
  );
}

export async function invokeTauriVoid(
  command: string,
  args: Record<string, unknown> = {},
): Promise<void> {
  const result = await tauriInvoke(command, args);
  if (result !== null && result !== undefined) {
    throw new Error("Rust command returned an unexpected value");
  }
}

export async function invokeTauriDiscardResult(
  command: string,
  args: Record<string, unknown> = {},
): Promise<void> {
  await tauriInvoke(command, args);
}

export async function listenTauri(
  event: string,
  handler: (payload: unknown) => void,
): Promise<() => void> {
  if (!isTauri()) return () => {};
  return listen<unknown>(event, ({ payload }) => handler(payload));
}

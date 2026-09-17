import type { CoreStatus } from "./types";

type TauriWindow = Window & {
  __TAURI__?: {
    core: {
      invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
    };
    event?: {
      listen<T>(
        event: string,
        handler: (event: { payload: T }) => void,
      ): Promise<() => void>;
    };
  };
};

function tauriInvoke<T>(command: string, args: Record<string, unknown> = {}) {
  const invoke = (window as TauriWindow).__TAURI__?.core.invoke<T>;
  if (!invoke) throw new Error("Rust core is available only inside Tauri");
  return invoke(command, args);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function parseCoreStatus(value: unknown): CoreStatus {
  if (!isRecord(value) || !isRecord(value.daemon)) {
    throw new Error("Rust core returned an invalid status payload");
  }
  const daemon = value.daemon;
  const healthValue = daemon.health;
  if (!isRecord(healthValue)) {
    throw new Error("Rust core returned an invalid status payload");
  }
  const health = healthValue;
  if (
    typeof value.prototype !== "boolean" ||
    typeof value.state_dir !== "string" ||
    typeof daemon.schema_version !== "string" ||
    typeof daemon.logged_in !== "boolean" ||
    (daemon.tenant_id !== null && typeof daemon.tenant_id !== "string") ||
    !Array.isArray(daemon.consent_scopes) ||
    !daemon.consent_scopes.every((scope) => typeof scope === "string") ||
    typeof daemon.paused !== "boolean" ||
    typeof daemon.queue_depth !== "number" ||
    (health.last_error_label !== null &&
      typeof health.last_error_label !== "string") ||
    (health.since !== null && typeof health.since !== "string")
  ) {
    throw new Error("Rust core returned an invalid status payload");
  }
  return value as unknown as CoreStatus;
}

export async function getCoreStatus(): Promise<CoreStatus> {
  const invoke = (window as TauriWindow).__TAURI__?.core.invoke<unknown>;
  if (!invoke) {
    return {
      prototype: true,
      state_dir: "browser-preview",
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
  return parseCoreStatus(await invoke("core_status"));
}

export async function daemonCall<T>(
  method: string,
  params: Record<string, unknown> = {},
): Promise<T> {
  return tauriInvoke<T>("daemon_call", { method, params });
}

export function invokeTauri<T>(
  command: string,
  args: Record<string, unknown> = {},
) {
  return tauriInvoke<T>(command, args);
}

export async function listenTauri<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  const tauri = (window as TauriWindow).__TAURI__;
  if (!tauri?.event) return () => {};
  return tauri.event.listen<T>(event, ({ payload }) => handler(payload));
}

import { invokeTauri } from "./core-api";

export type CapabilityState =
  | "available"
  | "configured"
  | "enabled"
  | "unavailable"
  | "denied"
  | "requires_approval"
  | "managed"
  | "unmanaged"
  | "unknown"
  | "not_registered"
  | "not_found";

export type PlatformCapabilities = {
  schema_version: string;
  os: string;
  package: {
    name: string;
    version: string;
    identifier: string;
    bundled: boolean;
  };
  startup: { state: CapabilityState; status: number };
  notifications: { state: CapabilityState; permission: number };
  updates: {
    state: CapabilityState;
    owner: string;
    action: string;
    feed_configured: boolean;
  };
  deep_links: { state: CapabilityState; scheme: string };
  tray: { state: CapabilityState };
};

function platformRecord(
  value: unknown,
  label: string,
): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`Invalid platform ${label}`);
  }
  return value as Record<string, unknown>;
}

function stringField(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "string") {
    throw new Error(`Invalid platform field: ${key}`);
  }
  return value[key] as string;
}

function booleanField(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "boolean") {
    throw new Error(`Invalid platform field: ${key}`);
  }
  return value[key] as boolean;
}

function integerField(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "number" || !Number.isInteger(value[key])) {
    throw new Error(`Invalid platform field: ${key}`);
  }
  return value[key] as number;
}

function stateField(value: Record<string, unknown>, key: string) {
  const state = stringField(value, key);
  const allowed: CapabilityState[] = [
    "available",
    "configured",
    "enabled",
    "unavailable",
    "denied",
    "requires_approval",
    "managed",
    "unmanaged",
    "unknown",
    "not_registered",
    "not_found",
  ];
  if (!allowed.includes(state as CapabilityState)) {
    throw new Error(`Invalid platform state: ${key}`);
  }
  return state as CapabilityState;
}

export function parsePlatformCapabilities(
  value: unknown,
): PlatformCapabilities {
  const item = platformRecord(value, "capabilities");
  const packageValue = platformRecord(item.package, "package");
  const startup = platformRecord(item.startup, "startup");
  const notifications = platformRecord(item.notifications, "notifications");
  const updates = platformRecord(item.updates, "updates");
  const deepLinks = platformRecord(item.deep_links, "deep links");
  const tray = platformRecord(item.tray, "tray");
  return {
    schema_version: stringField(item, "schema_version"),
    os: stringField(item, "os"),
    package: {
      name: stringField(packageValue, "name"),
      version: stringField(packageValue, "version"),
      identifier: stringField(packageValue, "identifier"),
      bundled: booleanField(packageValue, "bundled"),
    },
    startup: {
      state: stateField(startup, "state"),
      status: integerField(startup, "status"),
    },
    notifications: {
      state: stateField(notifications, "state"),
      permission: integerField(notifications, "permission"),
    },
    updates: {
      state: stateField(updates, "state"),
      owner: stringField(updates, "owner"),
      action: stringField(updates, "action"),
      feed_configured: booleanField(updates, "feed_configured"),
    },
    deep_links: {
      state: stateField(deepLinks, "state"),
      scheme: stringField(deepLinks, "scheme"),
    },
    tray: { state: stateField(tray, "state") },
  };
}

export async function getPlatformCapabilities() {
  return parsePlatformCapabilities(
    await invokeTauri<unknown>("platform_capabilities"),
  );
}

export function requestNotificationPermission() {
  return invokeTauri<unknown>("request_notification_permission");
}

export function setStartAtLogin(enabled: boolean) {
  return invokeTauri<unknown>("set_start_at_login", { enabled });
}

export function openSystemSettings(area: "notifications" | "login_items") {
  return invokeTauri<void>("open_system_settings", { area });
}

export function quitApp() {
  return invokeTauri<void>("quit_app");
}

export function updateStatus() {
  return invokeTauri<unknown>("update_status");
}

export async function pickDirectory(
  purpose: "repository" | "source_root",
): Promise<string> {
  const value = await invokeTauri<unknown>("pick_directory", { purpose });
  if (typeof value !== "string" || value.trim() === "") {
    throw new Error("Directory picker returned an invalid path");
  }
  return value;
}

export function openExternalUrl(url: string) {
  return invokeTauri<void>("open_external_url", { url });
}

export type EnrollmentDeepLink = {
  kind: "enroll";
  invite: string;
};

export type PublicRunDeepLink = {
  kind: "public_run";
  slug: string;
  url: string;
};

export type CredentialDeepLink = {
  kind: "credential";
  provider: string;
};

export type NavigateDeepLink = {
  kind: "navigate";
  path: "/waiting";
};

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("Invalid deep-link response");
  }
  return value as Record<string, unknown>;
}

export async function consumeDeepLink(): Promise<
  | EnrollmentDeepLink
  | PublicRunDeepLink
  | CredentialDeepLink
  | NavigateDeepLink
  | null
> {
  const value = await invokeTauri<unknown>("consume_deep_link");
  if (value === null) return null;
  const response = record(value);
  if (response.kind === "enroll" && typeof response.invite === "string") {
    return { kind: "enroll", invite: response.invite };
  }
  if (
    response.kind === "public_run" &&
    typeof response.slug === "string" &&
    typeof response.url === "string"
  ) {
    return { kind: "public_run", slug: response.slug, url: response.url };
  }
  if (
    response.kind === "credential" &&
    typeof response.provider === "string"
  ) {
    return { kind: "credential", provider: response.provider };
  }
  if (response.kind === "navigate" && response.path === "/waiting") {
    return { kind: "navigate", path: "/waiting" };
  }
  throw new Error("Invalid deep-link response");
}

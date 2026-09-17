import { invokeTauri } from "../../../lib/tauri/core-api";
import type { ConsentOption } from "../types";

export type NativeWalletView = {
  flow_id: string;
  state: string;
  busy: boolean;
  can_check: boolean;
  can_start: boolean;
  can_edit: boolean;
  can_cancel: boolean;
  wait: boolean;
  message: string;
  tone: string;
  glyph: string;
  browser_url: string | null;
};

function record(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null)
    throw new Error(`Invalid onboarding ${label}`);
  return value as Record<string, unknown>;
}
function string(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid onboarding field: ${key}`);
  return value[key] as string;
}
function boolean(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "boolean")
    throw new Error(`Invalid onboarding field: ${key}`);
  return value[key] as boolean;
}

export async function getConsentOptions(): Promise<ConsentOption[]> {
  const response = record(
    await invokeTauri<unknown>("consent_options"),
    "consent response",
  );
  if (!Array.isArray(response.scopes))
    throw new Error("Invalid onboarding scopes");
  return response.scopes.map((value) => {
    const scope = record(value, "scope");
    return {
      name: string(scope, "name"),
      description: string(scope, "description"),
      always_on: boolean(scope, "always_on"),
      grants_data_use: boolean(scope, "grants_data_use"),
    };
  });
}
export async function enrollWithInvite(invite: string) {
  const response = record(
    await invokeTauri<unknown>("enroll_with_invite", { invite }),
    "enrollment response",
  );
  return response;
}

function nullableString(value: Record<string, unknown>, key: string) {
  if (value[key] === null || value[key] === undefined) return null;
  if (typeof value[key] !== "string")
    throw new Error(`Invalid onboarding field: ${key}`);
  return value[key];
}

function parseNativeWalletView(value: unknown): NativeWalletView {
  const response = record(value, "wallet response");
  for (const key of ["flow_id", "state", "message", "tone", "glyph"]) {
    string(response, key);
  }
  for (const key of [
    "busy",
    "can_check",
    "can_start",
    "can_edit",
    "can_cancel",
    "wait",
  ]) {
    boolean(response, key);
  }
  return {
    flow_id: response.flow_id as string,
    state: response.state as string,
    busy: response.busy as boolean,
    can_check: response.can_check as boolean,
    can_start: response.can_start as boolean,
    can_edit: response.can_edit as boolean,
    can_cancel: response.can_cancel as boolean,
    wait: response.wait as boolean,
    message: response.message as string,
    tone: response.tone as string,
    glyph: response.glyph as string,
    browser_url: nullableString(response, "browser_url"),
  };
}

export async function nativeWalletFlow(input: {
  action: "open" | "check" | "start" | "wait" | "cancel";
  flowId?: string;
  commons?: string;
  account?: string;
}) {
  return parseNativeWalletView(
    await invokeTauri<unknown>("native_wallet_flow", {
      action: input.action,
      flowId: input.flowId ?? "",
      commons: input.commons ?? "",
      account: input.account ?? "",
    }),
  );
}

export async function openNativeWalletUrl(url: string, commons: string) {
  return invokeTauri<void>("open_native_wallet_url", { url, commons });
}

export async function nearAiAccountEnroll(commons: string) {
  const response = record(
    await invokeTauri<unknown>("near_ai_account_enroll", { commons }),
    "NEAR AI enrollment response",
  );
  if (typeof response.enrolled !== "boolean") {
    throw new Error("Invalid NEAR AI enrollment response");
  }
  return {
    enrolled: response.enrolled,
    message:
      response.message === undefined || response.message === null
        ? null
        : string(response, "message"),
  };
}
export async function setConsentScopes(scopes: string[]) {
  return invokeTauri<unknown>("set_consent_scopes", { scopes });
}
export async function acknowledgeNearAiNotice() {
  return invokeTauri<unknown>("acknowledge_near_ai_notice");
}
export async function getScrubberPatternNames(): Promise<string[]> {
  const response = record(
    await invokeTauri<unknown>("scrubber_pattern_names"),
    "scrubber response",
  );
  if (
    !Array.isArray(response.names) ||
    !response.names.every((name) => typeof name === "string")
  )
    throw new Error("Invalid scrubber names");
  return response.names as string[];
}

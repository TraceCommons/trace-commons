import {
  invokeTauri,
  invokeTauriDiscardResult,
} from "../../../lib/tauri/core-api";

export type TokenStorage = {
  capture_enabled: boolean;
  capture_label: string;
  capture_confirmation: string;
  capture_notice: string;
  state_line: string;
  scope_note: string;
  cleanup_label: string;
  discard_label: string;
  discard_confirmation: string;
  failure_line: string;
};

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error("Invalid privacy response");
  return value as Record<string, unknown>;
}

function stringField(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid privacy field: ${key}`);
  return value[key] as string;
}

export async function getTokenStorage(): Promise<TokenStorage> {
  const value = record(await invokeTauri("token_storage_status"));
  if (typeof value.capture_enabled !== "boolean")
    throw new Error("Invalid token capture state");
  return {
    capture_enabled: value.capture_enabled,
    capture_label: stringField(value, "capture_label"),
    capture_confirmation: stringField(value, "capture_confirmation"),
    capture_notice: stringField(value, "capture_notice"),
    state_line: stringField(value, "state_line"),
    scope_note: stringField(value, "scope_note"),
    cleanup_label: stringField(value, "cleanup_label"),
    discard_label: stringField(value, "discard_label"),
    discard_confirmation: stringField(value, "discard_confirmation"),
    failure_line: stringField(value, "failure_line"),
  };
}

export function setInferenceEvidence(enabled: boolean, confirmed: boolean) {
  return invokeTauriDiscardResult("set_inference_evidence", {
    enabled,
    confirmed,
  });
}
export function setTokenContribution(enabled: boolean, confirmed: boolean) {
  return invokeTauriDiscardResult("set_token_contribution", {
    enabled,
    confirmed,
  });
}
export function setTokenCapture(enabled: boolean, confirmed: boolean) {
  return invokeTauriDiscardResult("set_token_capture", { enabled, confirmed });
}
export function cleanTokenStorage(discard: boolean, confirmed: boolean) {
  return invokeTauriDiscardResult("clean_token_storage", {
    discard,
    confirmed,
  });
}

import { invokeTauri } from "../../../lib/tauri/core-api";

export type WitnessStatus = {
  state:
    | "absent"
    | "pinned"
    | "refusing_unpinned"
    | "refusing_pin_malformed"
    | "refusing_inference_receipts_missing"
    | "not_enrolled"
    | "settings_unreadable"
    | string;
  state_code: number;
  refusal: string | null;
  url: string | null;
  signing_address: string | null;
  pinned_measurement_count: number;
  pinned_measurement_line?: string | null;
  pinned_measurements: string[];
};

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error("Invalid witness response");
  return value as Record<string, unknown>;
}
function stringOrNull(value: Record<string, unknown>, key: string) {
  if (value[key] === null || value[key] === undefined) return null;
  if (typeof value[key] !== "string")
    throw new Error(`Invalid witness field: ${key}`);
  return value[key];
}

function parse(value: unknown): WitnessStatus {
  const item = record(value);
  if (
    typeof item.state !== "string" ||
    typeof item.state_code !== "number" ||
    typeof item.pinned_measurement_count !== "number" ||
    !Array.isArray(item.pinned_measurements) ||
    !item.pinned_measurements.every((entry) => typeof entry === "string")
  )
    throw new Error("Invalid witness response");
  if (
    item.refusal !== null &&
    item.refusal !== undefined &&
    typeof item.refusal !== "string"
  )
    throw new Error("Invalid witness refusal");
  if (
    item.pinned_measurement_line !== null &&
    item.pinned_measurement_line !== undefined &&
    typeof item.pinned_measurement_line !== "string"
  )
    throw new Error("Invalid witness measurement line");
  return {
    state: item.state,
    state_code: item.state_code,
    refusal:
      item.refusal === undefined ? null : (item.refusal as string | null),
    url: stringOrNull(item, "url"),
    signing_address: stringOrNull(item, "signing_address"),
    pinned_measurement_count: item.pinned_measurement_count,
    pinned_measurement_line:
      item.pinned_measurement_line === undefined
        ? null
        : (item.pinned_measurement_line as string | null),
    pinned_measurements: item.pinned_measurements as string[],
  };
}

export async function getWitnessStatus() {
  return parse(await invokeTauri<unknown>("witness_status"));
}
export async function configureWitness(
  url: string,
  signingAddress: string,
  measurements: string[],
) {
  return parse(
    await invokeTauri<unknown>("configure_witness", {
      url,
      signingAddress,
      measurements,
    }),
  );
}
export async function clearWitness() {
  return parse(await invokeTauri<unknown>("clear_witness"));
}

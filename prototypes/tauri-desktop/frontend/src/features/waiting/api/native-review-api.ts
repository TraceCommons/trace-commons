import { invokeTauri } from "../../../lib/tauri/core-api";

type RecordValue = Record<string, unknown>;

function record(value: unknown, label: string): RecordValue {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`Invalid ${label} response`);
  }
  return value as RecordValue;
}

function string(value: RecordValue, key: string) {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid review field: ${key}`);
  return value[key] as string;
}

function view(value: unknown) {
  const item = record(value, "review view");
  if (typeof item.ready !== "boolean")
    throw new Error("Invalid review readiness");
  return {
    ready: item.ready,
    state: string(item, "state"),
    message: string(item, "message"),
  };
}

function witnessView(value: unknown) {
  const item = record(value, "witness view");
  return {
    state: string(item, "state"),
    message: string(item, "message"),
  };
}

export type AdmissionPreparation = {
  ready: boolean;
  state: string;
  message: string;
};

export type WitnessReview = {
  ready: boolean;
  state: string;
  message: string | null;
  summary: string | null;
};

export async function getWitnessReviewSupport(): Promise<boolean> {
  const value = await invokeTauri<unknown>("witness_preview_support");
  if (typeof value !== "boolean")
    throw new Error("Invalid witness support response");
  return value;
}

export async function prepareAdmissionSession(
  entryId: string,
  backend: string,
): Promise<AdmissionPreparation> {
  const response = record(
    await invokeTauri<unknown>("prepare_admission_session", {
      entryId,
      backend,
    }),
    "admission",
  );
  const rendered = view(response.view);
  return {
    ready: rendered.ready,
    state: rendered.state,
    message: rendered.message,
  };
}

export async function requestWitnessReview(
  entryId: string,
): Promise<WitnessReview> {
  const response = record(
    await invokeTauri<unknown>("witness_preview_request", { entryId }),
    "witness review",
  );
  const rendered =
    response.view === undefined ? null : witnessView(response.view);
  return {
    ready: response.status === "ready",
    state:
      rendered?.state ?? (response.status === "ready" ? "Ready" : "Unknown"),
    message: rendered?.message ?? null,
    summary: response.status === "ready" ? "Witness review is ready." : null,
  };
}

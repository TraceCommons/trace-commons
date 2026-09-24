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

function count(value: RecordValue, key: string): number {
  const item = value[key];
  if (typeof item !== "number" || !Number.isSafeInteger(item) || item < 0)
    throw new Error(`Invalid witness summary field: ${key}`);
  return item;
}

function witnessSummary(value: unknown): WitnessReviewSummary {
  const item = record(value, "witness summary");
  const redactions = record(item.redactions, "witness redactions");
  if (
    Object.values(redactions).some(
      (value) =>
        typeof value !== "number" || !Number.isSafeInteger(value) || value < 0,
    )
  ) {
    throw new Error("Invalid witness summary redactions");
  }
  return {
    would_send_bytes: count(item, "would_send_bytes"),
    event_count: count(item, "event_count"),
    redactions: redactions as Record<string, number>,
    residual_risk: string(item, "residual_risk"),
  };
}

export type AdmissionPreparation = {
  ready: boolean;
  state: string;
  message: string;
};

export type WitnessReviewSummary = {
  would_send_bytes: number;
  event_count: number;
  redactions: Record<string, number>;
  residual_risk: string;
};

export type WitnessReview =
  | {
      ready: true;
      state: string;
      message: null;
      summary: WitnessReviewSummary;
    }
  | {
      ready: false;
      state: string;
      message: string | null;
      summary: null;
    };

export async function getWitnessReviewSupport(): Promise<boolean> {
  const value = await invokeTauri("witness_preview_support");
  if (typeof value !== "boolean")
    throw new Error("Invalid witness support response");
  return value;
}

export async function prepareAdmissionSession(
  entryId: string,
  backend: string,
  confirmed: boolean,
): Promise<AdmissionPreparation> {
  const response = record(
    await invokeTauri("prepare_admission_session", {
      entryId,
      backend,
      confirmed,
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
  rawSessionConfirmed: boolean,
): Promise<WitnessReview> {
  const response = record(
    await invokeTauri("witness_preview_request", {
      entryId,
      rawSessionConfirmed,
    }),
    "witness review",
  );
  const rendered =
    response.view === undefined ? null : witnessView(response.view);
  if (response.status === "ready") {
    return {
      ready: true,
      state: rendered?.state ?? "Ready",
      message: null,
      summary: witnessSummary(response.summary),
    };
  }
  return {
    ready: false,
    state: rendered?.state ?? "Unknown",
    message: rendered?.message ?? null,
    summary: null,
  };
}

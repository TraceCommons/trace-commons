import { invokeTauri, invokeTauriVoid } from "./core-api";

export type AccountSessionStatus = {
  signed_in: boolean;
  expires_at: string | null;
};

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("Invalid account session response");
  }
  return value as Record<string, unknown>;
}

function parseSessionStatus(value: unknown): AccountSessionStatus {
  const result = record(value);
  if (
    typeof result.signed_in !== "boolean" ||
    (result.expires_at !== null && typeof result.expires_at !== "string")
  ) {
    throw new Error("Invalid account session status");
  }
  return {
    signed_in: result.signed_in,
    expires_at: result.expires_at as string | null,
  };
}

export async function getAccountSessionStatus(): Promise<AccountSessionStatus> {
  return parseSessionStatus(await invokeTauri("account_session_status"));
}

export async function signInToAccount(): Promise<AccountSessionStatus> {
  const result = record(await invokeTauri("account_sign_in"));
  if (
    result.signed_in !== true ||
    typeof result.account_id !== "string" ||
    typeof result.expires_at !== "string"
  ) {
    throw new Error("Invalid account sign-in response");
  }
  return { signed_in: true, expires_at: result.expires_at };
}

export async function openAccountSignInUrl(url: string): Promise<void> {
  await invokeTauriVoid("open_account_sign_in_url", { url });
}

import { invokeTauri } from "../../../lib/tauri/core-api";

export type CredentialStatus = {
  state: string;
  session_state: string;
  cleanup_pending: boolean;
  attempt_status?: string;
  attempt_id?: string;
  keychain?: {
    state: string;
    inference_present: boolean;
    session_present: boolean;
    migration: string;
    key_prefix?: string;
    minted_at?: string;
    session_expires_at?: string;
  };
  view?: {
    state_line: string;
    action: "obtain" | "cancel" | "forget" | "none";
  };
};
export type BalanceStatus = {
  state: string;
  view?: {
    state_line: string;
    remaining_line: string;
    limit_line: string;
    spent_line: string;
  };
};
export type FundingStatus = {
  state: string;
  message: string;
  organizationId?: string;
  organizationName?: string;
  connectionRevision?: string;
  browserUrl?: string;
};
type RecordValue = Record<string, unknown>;

function record(value: unknown): RecordValue {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error("Invalid Private AI response");
  return value as RecordValue;
}
function string(value: RecordValue, key: string) {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid Private AI field: ${key}`);
  return value[key] as string;
}

export function parseCredentialStatus(value: unknown): CredentialStatus {
  const item = record(value);
  if (typeof item.cleanup_pending !== "boolean")
    throw new Error("Invalid Private AI cleanup state");
  const view =
    item.view === undefined ? undefined : parseCredentialView(item.view);
  const keychain =
    item.keychain === undefined ? undefined : parseKeychain(item.keychain);
  return {
    state: string(item, "state"),
    session_state: string(item, "session_state"),
    cleanup_pending: item.cleanup_pending,
    attempt_status:
      item.attempt_status === undefined
        ? undefined
        : string(item, "attempt_status"),
    attempt_id:
      item.attempt_id === undefined ? undefined : string(item, "attempt_id"),
    keychain,
    view,
  };
}

function parseKeychain(value: unknown): CredentialStatus["keychain"] {
  const item = record(value);
  if (
    typeof item.inference_present !== "boolean" ||
    typeof item.session_present !== "boolean"
  ) {
    throw new Error("Invalid Private AI keychain state");
  }
  const optional = (key: string) =>
    item[key] === undefined ? undefined : string(item, key);
  return {
    state: string(item, "state"),
    inference_present: item.inference_present,
    session_present: item.session_present,
    migration: string(item, "migration"),
    key_prefix: optional("key_prefix"),
    minted_at: optional("minted_at"),
    session_expires_at: optional("session_expires_at"),
  };
}

export async function getCredentialStatus() {
  return parseCredentialStatus(
    await invokeTauri<unknown>("private_ai_credential_status"),
  );
}
export async function getBalance(): Promise<BalanceStatus> {
  const value = record(await invokeTauri<unknown>("private_ai_balance"));
  return {
    state: string(value, "state"),
    view: value.view === undefined ? undefined : parseBalanceView(value.view),
  };
}
export async function getFunding(expected?: {
  organizationId: string;
  connectionRevision: string;
}): Promise<FundingStatus> {
  const value = record(
    await invokeTauri<unknown>(
      "private_ai_funding",
      expected
        ? {
            expectedOrganizationId: expected.organizationId,
            expectedConnectionRevision: expected.connectionRevision,
          }
        : {},
    ),
  );
  const view = record(value.view);
  const result: FundingStatus = {
    state: string(value, "state"),
    message: string(view, "message"),
  };
  if (result.state === "ready") {
    const organizationId = string(value, "organization_id");
    const organizationName = string(value, "organization_name");
    const connectionRevision = string(value, "connection_revision");
    const browserUrl = string(value, "browser_url");
    const canonical = `https://cloud.near.ai/dashboard/organizations/${organizationId}/credits`;
    if (
      browserUrl !== canonical ||
      connectionRevision.length !== 64 ||
      !/^[a-f0-9]+$/.test(connectionRevision)
    )
      throw new Error("Invalid funding destination");
    result.organizationId = organizationId;
    result.organizationName = organizationName;
    result.connectionRevision = connectionRevision;
    result.browserUrl = browserUrl;
  }
  return result;
}
export async function startCredential(provider: string) {
  const value = record(
    await invokeTauri<unknown>("start_private_ai_credential", { provider }),
  );
  return {
    browserUrl: string(value, "browser_url"),
    attemptId: string(value, "attempt_id"),
  };
}
export async function cancelCredential() {
  return invokeTauri<unknown>("cancel_private_ai_credential");
}
export async function forgetCredential() {
  return invokeTauri<unknown>("forget_private_ai_credential");
}
export async function setPrivateInference(enabled: boolean) {
  return invokeTauri<unknown>("set_private_inference", { enabled });
}
export async function openExternalUrl(url: string) {
  return invokeTauri<void>("open_external_url", { url });
}

function parseBalanceView(value: unknown) {
  const item = record(value);
  return {
    state_line: string(item, "state_line"),
    remaining_line: string(item, "remaining_line"),
    limit_line: string(item, "limit_line"),
    spent_line: string(item, "spent_line"),
  };
}
function parseCredentialView(value: unknown) {
  const item = record(value);
  const action = string(item, "action");
  if (!["obtain", "cancel", "forget", "none"].includes(action))
    throw new Error("Invalid Private AI action");
  return {
    state_line: string(item, "state_line"),
    action: action as "obtain" | "cancel" | "forget" | "none",
  };
}

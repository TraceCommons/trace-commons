// The first-run flow's only doorway to the outside world. Everything that
// needs backend work is MOCKED here and returns data from `ftux-mock-data.ts`
// after a short delay, so the screens exercise their loading states.
//
// When the backend lands, replace each mock with the real command:
//   lookupInvite        -> enroll_with_invite (onboarding-api.ts)
//   detectTools         -> daemon source detection (not built yet)
//   listRepoCandidates  -> list_projects + per-session listing (partly built)
//   createPasskey,
//   verifyPasskey,
//   signInWithPasskey   -> WebAuthn for tracecommons.ai (not built yet)
//   signInWithNearAi    -> account_sign_in (account-session-api.ts)
//   finishSetup         -> set_source_declaration, set_project_mode,
//                          set_consent_scopes, and the Private AI switch

import { isTauriRuntime } from "../../../lib/tauri/core-api";
import {
  openExternalUrl,
  pickDirectory,
} from "../../../lib/tauri/platform-api";
import { resolveInvite } from "../../onboarding/api/invite-link";
import type {
  DetectedTool,
  FtuxSettings,
  PasskeyStore,
  RepoCandidate,
} from "../types";
import {
  MOCK_DEFAULT_SELECTION,
  MOCK_ISSUER,
  MOCK_REPOS,
  MOCK_TOOLS,
} from "./ftux-mock-data";

const MOCK_LATENCY_MS = 450;

function later<T>(value: T, ms = MOCK_LATENCY_MS): Promise<T> {
  return new Promise((resolve) => setTimeout(() => resolve(value), ms));
}

export async function lookupInvite(
  raw: string,
): Promise<{ host: string; payRange: string }> {
  // Parsing is real; validating the invite with its issuer is mocked.
  const invite = resolveInvite(raw);
  if (!invite) {
    throw new Error("That is not an invite link. It ends in #code.");
  }
  return later({ host: invite.issuerHost, payRange: MOCK_ISSUER.payRange });
}

export function detectTools(): Promise<DetectedTool[]> {
  return later(MOCK_TOOLS);
}

export function listRepoCandidates(): Promise<{
  repos: RepoCandidate[];
  defaultSelection: Record<string, number[]>;
}> {
  return later({ repos: MOCK_REPOS, defaultSelection: MOCK_DEFAULT_SELECTION });
}

export async function chooseFolder(): Promise<string | null> {
  if (!isTauriRuntime()) return later("~/work/sessions", 150);
  try {
    return await pickDirectory("source_root");
  } catch {
    return null;
  }
}

export async function openExternal(url: string): Promise<void> {
  if (isTauriRuntime()) {
    await openExternalUrl(url);
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

export function createPasskey(
  name: string,
  store: PasskeyStore,
): Promise<{ name: string; store: PasskeyStore }> {
  return later({ name: name.trim(), store }, 700);
}

export function verifyPasskey(): Promise<void> {
  return later(undefined, 600);
}

export function signInWithPasskey(): Promise<{
  name: string;
  store: PasskeyStore;
}> {
  return later({ name: "My trace passkey", store: "passwords" }, 600);
}

export function signInWithNearAi(): Promise<void> {
  return later(undefined, 600);
}

export function finishSetup(_settings: FtuxSettings): Promise<void> {
  return later(undefined, 600);
}

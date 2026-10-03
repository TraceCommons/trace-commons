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
import { automaticChoices } from "../ftux-model";
import type {
  DetectedTool,
  FtuxSettings,
  PasskeyStore,
  RepoCandidate,
} from "../types";
import { MOCK_ISSUER, MOCK_REPOS, MOCK_TOOLS } from "./ftux-mock-data";

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

export function listRepoCandidates(): Promise<RepoCandidate[]> {
  return later(MOCK_REPOS);
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

// Fail closed. Setup offers automatic sharing only through the scrub and
// witness disclosures and then `requestGrant` (flow1.ts), per the consent
// spec's rev 8. That wiring lands in the PR stacked on this one; until then
// an automatic choice is refused rather than saved.
export async function finishSetup(settings: FtuxSettings): Promise<void> {
  if (!settings.baseUse) {
    throw new Error("Tick the first use to contribute.");
  }
  const armed = automaticChoices(settings);
  if (armed.length > 0) {
    throw new Error(
      `Automatic sharing goes through its disclosures and grant, which are not connected in this preview yet (${armed.join(", ")}). Choose "Ask me" for now.`,
    );
  }
  await later(undefined, 600);
}

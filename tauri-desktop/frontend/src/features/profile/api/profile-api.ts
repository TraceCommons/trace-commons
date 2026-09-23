import {
  daemonCall,
  invokeTauri,
  invokeTauriDiscardResult,
} from "../../../lib/tauri/core-api";
import type { PublicProfile } from "../types";

export type ProfilePublishResult = {
  profile: PublicProfile;
  handlePersisted: boolean;
};

function parseProfile(value: unknown): PublicProfile {
  if (typeof value !== "object" || value === null)
    throw new Error("Invalid profile response");
  const profile = value as Record<string, unknown>;
  if (typeof profile.on_roster !== "boolean")
    throw new Error("Invalid profile field: on_roster");
  return {
    on_roster: profile.on_roster,
    handle: nullableString(profile, "handle"),
    bio: nullableString(profile, "bio"),
    public_since: nullableString(profile, "public_since"),
    public_url: nullableString(profile, "public_url"),
  };
}

function nullableString(record: Record<string, unknown>, key: string) {
  if (record[key] === null) return null;
  if (typeof record[key] !== "string")
    throw new Error(`Invalid profile field: ${key}`);
  return record[key] as string;
}

export async function getPublicProfile() {
  return parseProfile(await daemonCall("get_public_profile"));
}

export async function publishProfile(handle: string, bio: string | null) {
  const value = await invokeTauri("publish_profile", { handle, bio });
  if (typeof value !== "object" || value === null)
    throw new Error("Invalid profile publish response");
  const record = value as Record<string, unknown>;
  if (typeof record.handle_persisted !== "boolean")
    throw new Error("Invalid profile publish field: handle_persisted");
  return {
    profile: parseProfile(record),
    handlePersisted: record.handle_persisted,
  } satisfies ProfilePublishResult;
}

export async function withdrawProfile() {
  return invokeTauriDiscardResult("withdraw_profile");
}

import { daemonCall, invokeTauri } from "../../../lib/tauri/core-api";
import type { PublicProfile } from "../types";

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
  return parseProfile(await daemonCall<unknown>("get_public_profile"));
}

export async function publishProfile(handle: string, bio: string | null) {
  return invokeTauri("publish_profile", { handle, bio });
}

export async function withdrawProfile() {
  return invokeTauri("withdraw_profile");
}

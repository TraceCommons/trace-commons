// Renders public/.well-known/apple-app-site-association for the macOS app's
// webcredentials association (native passkeys, Z2 S4 of #1118).
//
// Never ships a bad ID or a placeholder: a malformed TC_APPLE_TEAM_ID writes
// nothing and exits 1. With the ID unset or empty the default is to write
// nothing, warn and exit 0 (the Worker then answers a real 404 for the path,
// which is safe), so unrelated community-site deploys are not blocked while the
// Team ID is undecided. Strict mode (`--require` or TC_AASA_REQUIRED=1) makes
// unset an error too. The Team ID is not a secret (every signed binary carries
// it).
import { mkdir, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const DEFAULT_BUNDLE_ID = "ai.tracecommons.shell";
export const TEAM_ID_PATTERN = /^[A-Z0-9]{10}$/;
const BUNDLE_ID_PATTERN = /^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$/;

// Returns the file body, or null when the Team ID is unset/empty and strict is
// false. Throws on a malformed ID, and on an unset one when strict.
export function renderAasa(env, { strict = false } = {}) {
  const teamId = env.TC_APPLE_TEAM_ID ?? "";
  if (teamId === "" && !strict) {
    return null;
  }
  if (!TEAM_ID_PATTERN.test(teamId)) {
    throw new Error(
      "TC_APPLE_TEAM_ID must be set to the 10 uppercase alphanumeric characters of the Apple Team ID that signs the macOS app",
    );
  }
  const bundleId = env.TC_MACOS_BUNDLE_ID || DEFAULT_BUNDLE_ID;
  if (!BUNDLE_ID_PATTERN.test(bundleId)) {
    throw new Error("TC_MACOS_BUNDLE_ID is not a valid bundle identifier");
  }
  return `${JSON.stringify({ webcredentials: { apps: [`${teamId}.${bundleId}`] } })}\n`;
}

export const OUTPUT_PATH = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "public",
  ".well-known",
  "apple-app-site-association",
);

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  // Drop any stale file first so a failed render can never leave an old one behind.
  await rm(OUTPUT_PATH, { force: true });
  const strict = process.argv.includes("--require") || process.env.TC_AASA_REQUIRED === "1";
  let body;
  try {
    body = renderAasa(process.env, { strict });
  } catch (error) {
    console.error(`render-aasa: ${error.message}`);
    process.exit(1);
  }
  if (body === null) {
    console.warn(
      "render-aasa: AASA not rendered: TC_APPLE_TEAM_ID unset; /.well-known/apple-app-site-association will 404",
    );
    process.exit(0);
  }
  await mkdir(dirname(OUTPUT_PATH), { recursive: true });
  await writeFile(OUTPUT_PATH, body);
  console.log(`render-aasa: wrote ${OUTPUT_PATH}`);
}

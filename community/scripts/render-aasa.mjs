// Renders public/.well-known/apple-app-site-association for the macOS app's
// webcredentials association (native passkeys, Z2 S4 of #1118).
//
// Fails closed: with TC_APPLE_TEAM_ID unset or malformed nothing is written and
// the process exits non-zero, so `npm run deploy:pages` cannot ship a site
// without a valid association, and never ships a placeholder. The Team ID is
// not a secret (every signed binary carries it); it is deploy config only
// because it has not been decided in the tree.
import { mkdir, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const DEFAULT_BUNDLE_ID = "ai.tracecommons.shell";
export const TEAM_ID_PATTERN = /^[A-Z0-9]{10}$/;
const BUNDLE_ID_PATTERN = /^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$/;

export function renderAasa(env) {
  const teamId = env.TC_APPLE_TEAM_ID ?? "";
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
  let body;
  try {
    body = renderAasa(process.env);
  } catch (error) {
    console.error(`render-aasa: ${error.message}`);
    process.exit(1);
  }
  await mkdir(dirname(OUTPUT_PATH), { recursive: true });
  await writeFile(OUTPUT_PATH, body);
  console.log(`render-aasa: wrote ${OUTPUT_PATH}`);
}

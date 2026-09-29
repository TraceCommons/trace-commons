import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { createContext, runInContext } from "node:vm";

const root = new URL("..", import.meta.url).pathname;
const publicDir = join(root, "public");

const [html, css, appJs, publicRunJs, worker, config, snapshotText, experienceText, headers, redirects, wrangler] = await Promise.all([
  readFile(join(publicDir, "index.html"), "utf8"),
  readFile(join(publicDir, "styles.css"), "utf8"),
  readFile(join(publicDir, "app.js"), "utf8"),
  readFile(join(publicDir, "public-runs.js"), "utf8"),
  readFile(join(publicDir, "_worker.js"), "utf8"),
  readFile(join(publicDir, "config.js"), "utf8"),
  readFile(join(publicDir, "snapshot.json"), "utf8"),
  readFile(join(publicDir, "experience.json"), "utf8"),
  readFile(join(publicDir, "_headers"), "utf8"),
  readFile(join(publicDir, "_redirects"), "utf8"),
  readFile(join(root, "wrangler.toml"), "utf8"),
]);
const js = `${appJs}\n${publicRunJs}`;

const snapshot = JSON.parse(snapshotText);
const experience = JSON.parse(experienceText);
const failures = [];

if (!html.includes("/styles.css") || !html.includes("/app.js") || !html.includes("/public-runs.js")) {
  failures.push("index.html must reference styles.css, app.js, and public-runs.js");
}
if (!Array.isArray(snapshot.leaderboard) || snapshot.leaderboard.length < 1) {
  failures.push("snapshot.json must include at least one leaderboard row");
}
if (!snapshot.analytics || !Array.isArray(snapshot.analytics.novelty_histogram)) {
  failures.push("snapshot.json must include analytics.novelty_histogram");
}
if (!experience.current_prompt || !experience.current_prompt.title) {
  failures.push("experience.json must include current_prompt.title");
}
if (!Array.isArray(experience.milestones) || experience.milestones.length < 1) {
  failures.push("experience.json must include at least one milestone");
}
if (!css.includes("@media (max-width: 900px)")) {
  failures.push("styles.css must include the mobile layout breakpoint");
}
if (!js.includes("/experience.json")) {
  failures.push("app.js must load the operator-curated experience feed");
}
if (!config.includes('apiBase: "/api"')) {
  failures.push("config.js must use the same-origin /api proxy");
}
if (!worker.includes("https://ingest.tracecommons.ai")) {
  failures.push("_worker.js must proxy community API requests to the ingest host");
}
if (!worker.includes("env.ASSETS.fetch")) {
  failures.push("_worker.js must continue serving Cloudflare Pages assets");
}
if (!worker.includes("x-tracecommons-proxy")) {
  failures.push("_worker.js must mark proxied responses for smoke-test visibility");
}
if (js.includes("x-trace")) {
  failures.push("app.js must not handle device-key signing headers in the browser");
}
if (js.includes("style=")) {
  failures.push("app.js must not rely on inline styles; keep the CSP deployment-friendly");
}
if (!headers.includes("Content-Security-Policy")) {
  failures.push("_headers must set a Content-Security-Policy");
}
if (!redirects.includes("/contributors/* / 200")) {
  failures.push("_redirects must route contributor pages to the SPA");
}
if (!redirects.includes("/runs/* / 200")) {
  failures.push("_redirects must route published workflow pages to the SPA");
}
if (!js.includes("/v1/community/runs/") || !js.includes("data-use-workflow")) {
  failures.push("app.js must load published workflows and expose Use workflow");
}
if (!js.includes("escapeHtml(item.excerpt") || !js.includes("escapeHtml(run.workflow)")) {
  failures.push("app.js must escape every public workflow text surface");
}
if (!redirects.includes("/brief / 200")) {
  failures.push("_redirects must route the pilot brief to the SPA");
}
if (!wrangler.includes('name = "trace-commons-community"')) {
  failures.push("wrangler.toml must name the Cloudflare Pages project");
}
if (!wrangler.includes('pages_build_output_dir = "public"')) {
  failures.push("wrangler.toml must deploy the public directory");
}
const publicData = `${snapshotText}\n${experienceText}\n${config}\n${worker}`;
if (publicData.includes("Bearer ") || publicData.includes("PRIVATE KEY")) {
  failures.push("public data files must not contain token or key material");
}

const copied = [];
const fallbackCopies = [];
let fallbackBuffer = null;
const workflowResult = { textContent: "" };
const sourceChip = { dataset: {}, textContent: "" };
const browser = createContext({
  URL,
  URLSearchParams,
  Intl,
  console,
  history: { pushState() {} },
  location: { origin: "https://tracecommons.ai", pathname: "/runs/source", search: "" },
  navigator: { clipboard: { async writeText(value) { copied.push(value); } } },
  window: {
    TRACE_COMMONS_COMMUNITY_CONFIG: {},
    addEventListener() {},
  },
  document: {
    addEventListener() {},
    body: { append(buffer) { fallbackBuffer = buffer; } },
    createElement(tag) {
      if (tag !== "textarea") throw new Error("unexpected element");
      return {
        className: "",
        value: "",
        select() {},
        remove() { fallbackBuffer = null; },
      };
    },
    execCommand(command) {
      if (command !== "copy" || !fallbackBuffer) return false;
      fallbackCopies.push(fallbackBuffer.value);
      return true;
    },
    getElementById(id) { return id === "source-chip" ? sourceChip : null; },
    querySelector() { return workflowResult; },
    querySelectorAll() { return []; },
  },
});
runInContext(js, browser);
browser.location.search = "?api=https://attacker.invalid";
if (runInContext("publicRunApiOrigin()", browser) !== "https://ingest.tracecommons.ai") {
  failures.push("published workflows must ignore query-string API origin overrides");
}
browser.hostileRun = {
  slug: "source",
  title: "<script>steal()</script>",
  outcome_summary: "<img src=x onerror=steal()>",
  correction_excerpt: "replace <unsafe>",
  workflow: "Run <tool> safely",
  reuse_permission: "cc_by_4_0",
  evidence: [{ excerpt: "observed <value>" }],
  task_success: "success",
  contributed_version: "trace-contribution/1",
  version: 1,
  published_at: "2026-09-10T00:00:00Z",
  source: null,
  variations: [],
};
const hostileHtml = runInContext("renderPublicRun(hostileRun)", browser);
if (hostileHtml.includes("<script>") || hostileHtml.includes("<img")) {
  failures.push("published workflow rendering must escape approved public text");
}
if (!hostileHtml.includes("&lt;script&gt;") || !hostileHtml.includes("data-use-workflow")) {
  failures.push("published workflow rendering must preserve escaped text and the reuse action");
}
const multilineHtml = runInContext(
  "renderPublicRun({...hostileRun, title: 'Line one\\nLine two', outcome_summary: 'Outcome one\\nOutcome two', correction_excerpt: 'Correction one\\nCorrection two', evidence: [{ excerpt: 'Evidence one\\nEvidence two' }]})",
  browser,
);
if (
  !multilineHtml.includes("Line one\nLine two") ||
  !multilineHtml.includes("Outcome one\nOutcome two") ||
  !multilineHtml.includes("Correction one\nCorrection two") ||
  !multilineHtml.includes("Evidence one\nEvidence two") ||
  !/\.public-run-title,\s*\.public-run-outcome,\s*\.public-run-prose,\s*\.public-run-workflow,\s*\.public-run-evidence li\s*\{[^}]*white-space:\s*pre-wrap;/m.test(css)
) {
  failures.push("published workflow prose must preserve reviewed multiline formatting");
}
const unavailableSourceHtml = runInContext(
  "renderPublicRun({...hostileRun, source_unavailable: true})",
  browser,
);
if (!unavailableSourceHtml.includes("Source workflow unavailable")) {
  failures.push("a withdrawn source must remain visible as unavailable provenance");
}
runInContext("state.publicRun = hostileRun; state.publicRunStatus = 'ready'", browser);
runInContext("updatePublicRunChip()", browser);
if (sourceChip.textContent !== "Reviewed page" || sourceChip.dataset.source !== "public-run") {
  failures.push("published workflow routes must replace the snapshot loading status");
}
await runInContext("copyPublicWorkflow({ textContent: '' })", browser);
if (copied[0] !== "Run <tool> safely\n\nSource: https://tracecommons.ai/runs/source") {
  failures.push("Use workflow must copy the exact instructions and canonical source URL");
}
browser.navigator.clipboard = null;
workflowResult.textContent = "";
await runInContext("copyPublicWorkflow({ textContent: '' })", browser);
if (fallbackCopies[0] !== copied[0] || fallbackBuffer !== null) {
  failures.push("Use workflow must provide and clean up the legacy clipboard fallback");
}
browser.navigator.clipboard = { async writeText() { throw new Error("clipboard denied"); } };
workflowResult.textContent = "";
await runInContext("copyPublicWorkflow({ textContent: '' })", browser);
if (workflowResult.textContent !== "Copy failed. Select the reusable instructions instead.") {
  failures.push("Use workflow must provide an actionable clipboard-denied state");
}

const response = (status, payload, rejectJSON = false) => ({
  status,
  ok: status >= 200 && status < 300,
  async json() {
    if (rejectJSON) throw new Error("invalid JSON");
    return payload;
  },
});
browser.fetch = async () => response(200, browser.hostileRun);
await runInContext("loadPublicRun('source')", browser);
if (runInContext("state.publicRunStatus", browser) !== "ready") {
  failures.push("a valid published workflow response must reach ready state");
}
browser.fetch = async () => response(404, null);
await runInContext("loadPublicRun('missing')", browser);
if (runInContext("state.publicRunStatus", browser) !== "not-found") {
  failures.push("a 404 published workflow response must reach not-found state");
}
browser.fetch = async () => response(500, null);
await runInContext("loadPublicRun('failed')", browser);
if (runInContext("state.publicRunStatus", browser) !== "error") {
  failures.push("a failed published workflow response must reach retryable error state");
}
browser.fetch = async () => response(200, null, true);
await runInContext("loadPublicRun('malformed')", browser);
if (runInContext("state.publicRunStatus", browser) !== "error") {
  failures.push("malformed published workflow JSON must reach retryable error state");
}
let invalidFetches = 0;
browser.fetch = async () => {
  invalidFetches += 1;
  return response(200, browser.hostileRun);
};
await runInContext("loadPublicRun('Invalid Slug')", browser);
if (invalidFetches !== 0 || runInContext("state.publicRunStatus", browser) !== "not-found") {
  failures.push("invalid public slugs must fail locally without a network request");
}

let finishFirst;
const firstResponse = new Promise((resolve) => { finishFirst = resolve; });
browser.fetch = async (url) =>
  url.includes("/first") ? firstResponse : response(200, { ...browser.hostileRun, slug: "second", workflow: "Second workflow" });
const firstLoad = runInContext("loadPublicRun('first')", browser);
await runInContext("loadPublicRun('second')", browser);
finishFirst(response(200, { ...browser.hostileRun, slug: "first", workflow: "First workflow" }));
await firstLoad;
if (
  runInContext("state.publicRunSlug", browser) !== "second" ||
  runInContext("state.publicRun.workflow", browser) !== "Second workflow"
) {
  failures.push("a late route response must not replace the active published workflow");
}

if (failures.length > 0) {
  console.error(failures.map((failure) => `- ${failure}`).join("\n"));
  process.exit(1);
}

console.log("community site checks passed");

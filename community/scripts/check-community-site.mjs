import { readFile } from "node:fs/promises";
import { join } from "node:path";

const root = new URL("..", import.meta.url).pathname;
const publicDir = join(root, "public");

const [html, css, js, worker, config, snapshotText, experienceText, headers, redirects, wrangler] = await Promise.all([
  readFile(join(publicDir, "index.html"), "utf8"),
  readFile(join(publicDir, "styles.css"), "utf8"),
  readFile(join(publicDir, "app.js"), "utf8"),
  readFile(join(publicDir, "_worker.js"), "utf8"),
  readFile(join(publicDir, "config.js"), "utf8"),
  readFile(join(publicDir, "snapshot.json"), "utf8"),
  readFile(join(publicDir, "experience.json"), "utf8"),
  readFile(join(publicDir, "_headers"), "utf8"),
  readFile(join(publicDir, "_redirects"), "utf8"),
  readFile(join(root, "wrangler.toml"), "utf8"),
]);

const snapshot = JSON.parse(snapshotText);
const experience = JSON.parse(experienceText);
const failures = [];

if (!html.includes("/styles.css") || !html.includes("/app.js")) {
  failures.push("index.html must reference styles.css and app.js");
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

if (failures.length > 0) {
  console.error(failures.map((failure) => `- ${failure}`).join("\n"));
  process.exit(1);
}

console.log("community site checks passed");

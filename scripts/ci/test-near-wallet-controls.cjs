// Launched by the ignored Rust test pinned_wallet_controls_obey_production_csp.
// Uses its production-rendered page and actual hash-pinned wallet adapters.
// Adapter downloads are anonymous; every browser wallet/Cloud request is intercepted.
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const assert = require("node:assert/strict");
const { chromium } = require(process.env.TC_PLAYWRIGHT_MODULE || "playwright");
const base = path.resolve(__dirname, "../../crates/trace-commons-contributor/src/daemon/nearai_credential");
const fixture = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const manifest = JSON.parse(fs.readFileSync(base + "/near_connect/manifest.json", "utf8"));
const origin = "http://127.0.0.1:34567";
const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="white"/></svg>';
const handlers = [
  "window.selector.openMobile()",
  "window.selector.openTelegram()",
  "window.selector.openExtension()",
  "window.selector.open('https://apps.apple.com/app/near-mobile/id6443501225')",
  "window.selector.open('https://play.google.com/store/apps/details?id=com.peersyst.nearmobilewallet')",
  "window.selector.open('https://nearmobile.app')",
];
const cases = [
  { id: "hot-wallet", mobile: false, count: 2 },
  { id: "hot-wallet", mobile: true, count: 2 },
  { id: "near-mobile", mobile: false, count: 3 },
];
async function adapter(wallet) {
  const response = await fetch(wallet.executor, { redirect: "error", signal: AbortSignal.timeout(15000) });
  assert(response.ok, "pinned adapter unavailable");
  const chunks = [];
  let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    assert(size <= 2 * 1024 * 1024, "pinned adapter exceeds limit");
    chunks.push(chunk);
  }
  const bytes = Buffer.concat(chunks);
  assert.equal(crypto.createHash("sha256").update(bytes).digest("hex"), wallet.sha256);
  return bytes.toString("utf8");
}
async function run(browser, item, script) {
  const wallet = manifest.wallets.find(wallet => wallet.id === item.id);
  const context = await browser.newContext(item.mobile ? {
    userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148",
  } : {});
  const page = await context.newPage();
  page.setDefaultTimeout(10000);
  const violations = [];
  page.on("console", message => {
    if (message.text().includes("inline event handler")) violations.push(message.text());
  });
  await page.route("**/*", async route => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.origin === origin) {
      const asset = url.pathname.split("/").pop();
      const body = asset === "connector.js" ? fs.readFileSync(base + "/near_connect/near-connect-0.11.4.js", "utf8")
        : asset === "browser.js" ? fs.readFileSync(base + "/near_wallet_browser.js", "utf8")
        : asset === item.id + ".js" ? script : null;
      if (body !== null) return route.fulfill({ contentType: "text/javascript", body });
      assert.equal(url.pathname, "/near-ai/near-wallet/callback", "unexpected local request");
      return route.fulfill({ contentType: "text/html", body: fixture.html });
    }
    if (url.href.includes("/api/v1/web/time")) return route.fulfill({ json: { ts: "1780000000000000000000000" } });
    if (item.id === "hot-wallet" && url.href.includes("/request")) return route.fulfill({ json: {} });
    if (item.id === "hot-wallet" && url.href.includes("/response")) return route.fulfill({ status: 404, body: "pending" });
    if (item.id === "near-mobile" && request.resourceType() === "fetch") return new Promise(() => {});
    if (request.resourceType() === "image") return route.fulfill({
      contentType: "image/svg+xml", headers: { "access-control-allow-origin": "*" }, body: svg,
    });
    return route.abort();
  });
  try {
    await page.goto(origin + "/near-ai/near-wallet/callback#" + fixture.state);
    await page.evaluate(() => {
      window.opened = [];
      window.open = url => { window.opened.push(url); return null; };
    });
    await page.click("#connect");
    await page.getByText(wallet.name, { exact: true }).click();
    const controls = page.frameLocator("iframe").locator("button[onclick], a[onclick]");
    await controls.first().waitFor({ state: "attached" });
    const attributes = await controls.evaluateAll(elements => elements.map(element => element.getAttribute("onclick")));
    assert.equal(attributes.length, item.count);
    for (const body of attributes) assert(handlers.includes(body), "unreviewed adapter handler");
    await controls.evaluateAll(elements => elements.forEach(element => element.click()));
    await page.waitForFunction(count => window.opened.length === count, item.count);
    assert.equal(violations.length, 0, "reviewed wallet action blocked");
    const unreviewedRan = await controls.first().evaluate(() => {
      window.unreviewedRan = false;
      const button = document.createElement("button");
      button.setAttribute("onclick", "window.unreviewedRan=true");
      document.body.append(button);
      button.click();
      return window.unreviewedRan;
    });
    assert.equal(unreviewedRan, false, "unreviewed inline handler executed");
    console.log(`PASS ${item.id} ${item.mobile ? "mobile" : "desktop"}: reviewed controls work; arbitrary handler blocked`);
  } finally { await context.close(); }
}
const deadline = setTimeout(() => { console.error("browser controls timed out"); process.exit(1); }, 60000);
(async () => {
  const scripts = new Map();
  for (const id of new Set(cases.map(item => item.id))) {
    scripts.set(id, await adapter(manifest.wallets.find(wallet => wallet.id === id)));
  }
  const browser = await chromium.launch({ headless: true });
  try {
    for (const item of cases) await run(browser, item, scripts.get(item.id));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; }).finally(() => clearTimeout(deadline));

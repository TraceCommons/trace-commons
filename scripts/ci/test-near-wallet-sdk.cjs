// Real pinned SDK/Chromium tests with a synthetic local executor. No real wallet or Cloud auth.
// Usage: TC_PLAYWRIGHT_MODULE=/absolute/path/to/playwright node scripts/ci/test-near-wallet-sdk.cjs /absolute/browser.js /absolute/pinned-sdk.js
// Only loopback HTTP is served. The wallet proof is synthetic; Rust signature validation is outside this test.
const http = require("node:http"),
  fs = require("node:fs");
const base = require("node:path").resolve(__dirname,
  "../../crates/trace-commons-contributor/src/daemon/nearai_credential");
const browser = process.argv[2] ?? base + "/near_wallet_browser.js";
const sdk = process.argv[3] ?? base + "/near_connect/near-connect-0.11.4.js";
const policyTemplate = fs.readFileSync(base + "/near_wallet_page.rs", "utf8")
  .match(/<meta http-equiv="Content-Security-Policy" content="([^"]+)"/)[1];
const state = "A".repeat(43);
let badAsset = false;
const requests = [];
const adapter = `window.selector.ready({
 async signIn(params) {
  if(params.network!=="mainnet" || params.addFunctionCallKey!==undefined) throw new Error("invalid sign-in contract");
  await window.selector.storage.set("fixture","synthetic");
  window.parent.postMessage({sdkFixture:"signIn",valid:true},"*");
  return [{accountId:"synthetic.near",publicKey:"ed25519:"+"1".repeat(32)}];
 },
 async signMessage(params) {
  if(Object.keys(params).sort().join(",")!=="message,network,nonce,recipient" || params.message!=="Sign in to NEAR AI Cloud" || params.recipient!=="cloud.near.ai" || params.network!=="mainnet" || !(params.nonce instanceof Uint8Array) || params.nonce.length!==32 || params.nonce.some(x=>x!==7)) throw new Error("invalid signing contract");
  if(await window.selector.storage.get("fixture")!=="synthetic") throw new Error("missing sandbox storage");
  window.parent.postMessage({sdkFixture:"signMessage",valid:true},"*");
  return {accountId:"synthetic.near",publicKey:"ed25519:"+"1".repeat(32),signature:btoa("\\0".repeat(64))};
 }
});`;
const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, "http://" + req.headers.host);
  res.setHeader("Cache-Control", "no-store");
  res.setHeader("X-Content-Type-Options", "nosniff");
  if (url.pathname === "/fixture/evidence") {
    res.setHeader("Content-Type", "application/json");
    res.end(JSON.stringify(requests));
    return;
  }
  if (url.pathname === "/fixture/fail-assets") {
    badAsset = true;
    res.end("ok");
    return;
  }
  requests.push({ method: req.method, path: url.pathname, query: url.search });
  if (url.pathname === "/near-ai/near-wallet/assets/connector.js") {
    res.setHeader("Content-Type", "application/javascript");
    res.end(fs.readFileSync(sdk));
    return;
  }
  if (url.pathname === "/near-ai/near-wallet/assets/browser.js") {
    res.setHeader("Content-Type", "application/javascript");
    res.end(fs.readFileSync(browser, "utf8"));
    return;
  }
  if (url.pathname === "/near-ai/near-wallet/assets/fixture-wallet.js") {
    res.setHeader("Content-Type", "application/javascript");
    res.statusCode = badAsset ? 503 : 200;
    res.end(badAsset ? "" : adapter);
    return;
  }
  if (url.pathname === "/near-ai/near-wallet/result") {
    let body = "";
    for await (const c of req) body += c;
    const data = new URLSearchParams(body);
    const valid =
      data.get("state") === state &&
      (data.get("error") === "cancelled" ||
        (data.get("accountId") === "synthetic.near" &&
          data.get("signature") === Buffer.alloc(64).toString("base64")));
    requests.at(-1).validForm = valid;
    res.statusCode = valid ? 200 : 400;
    res.end();
    return;
  }
  if (url.pathname !== "/near-ai/near-wallet/callback") {
    res.statusCode = 404;
    res.end();
    return;
  }
  const origin = "http://" + req.headers.host,
    nonce = "aGVsbG9zeW50aGV0aWM=";
  const cfg = {
    message: "Sign in to NEAR AI Cloud",
    recipient: "cloud.near.ai",
    nonce: Array(32).fill(7),
    expires_at_ms: Date.now() + 299000,
    cspNonce: nonce,
    copy: Object.fromEntries(
      [
        "choose",
        "waiting",
        "received",
        "refused",
        "expired",
        "cancelled",
        "unavailable",
      ].map((x) => [x, "canonical-" + x]),
    ),
    manifest: {
      version: "1.1.0",
      wallets: [
        {
          id: "fixture-wallet",
          name: "Synthetic wallet",
          description: "Local SDK fixture",
          icon: "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==",
          website: origin,
          executor: "/near-ai/near-wallet/assets/fixture-wallet.js",
          type: "sandbox",
          version: "fixture-1",
          features: {
            mainnet: true,
            signMessage: true,
            signInWithoutAddKey: true,
          },
          platform: { web: origin },
          permissions: { storage: true },
        },
      ],
    },
  };
  res.setHeader("Content-Type", "text/html");
  res.end(
    `<!doctype html><meta http-equiv="Content-Security-Policy" content="${policyTemplate.replaceAll("{nonce}", nonce)}"><button id="connect">Connect wallet</button><button id="cancel">Cancel sign-in</button><p id="status">canonical-refused</p><script id="challenge" type="application/json">${JSON.stringify(cfg)}</script><script nonce="${nonce}" src="/near-ai/near-wallet/assets/connector.js"></script><script nonce="${nonce}" src="/near-ai/near-wallet/assets/browser.js"></script>`,
  );
});
const assert = require("node:assert/strict");
const { chromium } = require(
  process.env.TC_PLAYWRIGHT_MODULE ??
    "playwright",
);
(async () => {
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const origin = "http://127.0.0.1:" + server.address().port;
  const browserInstance = await chromium.launch({ headless: true });
  try {
    const context = await browserInstance.newContext();
    const page = await context.newPage();
    page.setDefaultTimeout(5000);
    await page.route("**/*", (route) =>
      new URL(route.request().url()).origin === origin
        ? route.continue()
        : route.abort(),
    );
    async function fresh() {
      await page.goto("about:blank");
      badAsset = false;
      requests.length = 0;
      await page.goto(origin + "/near-ai/near-wallet/callback#" + state);
      await page.waitForFunction(
        () =>
          document.querySelector("#status").textContent === "canonical-choose",
      );
      await page.evaluate(() => {
        window.sdkFixtureEvents = [];
        window.addEventListener("message", (e) => {
          if (e.data.sdkFixture)
            window.sdkFixtureEvents.push(e.data.sdkFixture);
        });
      });
    }
    async function choose() {
      await page
        .locator(".hot-connector-popup .connect-item[data-type=fixture-wallet]")
        .click();
    }
    async function statusIs(value) {
      await page.waitForFunction(
        (v) => document.querySelector("#status").textContent === v,
        value,
      );
    }
    async function openChooser() {
      await page
        .getByRole("button", { name: "Connect wallet", exact: true })
        .click();
      await page
        .locator(".hot-connector-popup .connect-item")
        .waitFor({ state: "visible" });
    }

    await fresh();
    await page.evaluate(async () => {
      const db = await new Promise((resolve, reject) => {
        const r = indexedDB.open("hot-connector", 1);
        r.onupgradeneeded = () => r.result.createObjectStore("wallets");
        r.onsuccess = () => resolve(r.result);
        r.onerror = () => reject(Error("seed open failed"));
      });
      const code =
        'window.parent.postMessage({sdkFixture:"STALE-CANARY"},"*");window.selector.ready({async signIn(){throw Error("STALE-CANARY");}});';
      await new Promise((resolve, reject) => {
        const t = db.transaction("wallets", "readwrite");
        t.objectStore("wallets").put(code, "fixture-wallet:fixture-1");
        t.oncomplete = resolve;
        t.onerror = () => reject(Error("seed write failed"));
      });
      db.close();
    });
    await openChooser();
    assert.equal(
      await page.evaluate(async () =>
        (await indexedDB.databases()).some((db) => db.name === "hot-connector"),
      ),
      false,
      "stale SDK database must be removed before choosing",
    );
    await choose();
    await statusIs("canonical-received");
    assert.deepEqual(await page.evaluate(() => window.sdkFixtureEvents), [
      "signIn",
      "signMessage",
    ]);
    assert.deepEqual(
      requests
        .filter((r) => r.path.endsWith("fixture-wallet.js"))
        .map((r) => r.query === ""),
      [true, false],
    );
    assert.equal(
      requests.filter((r) => r.method === "POST" && r.validForm).length,
      1,
    );
    assert.deepEqual(await page.evaluate(() => Object.keys(localStorage)), []);
    console.log(
      "PASS real IIFE relative preflight/connect/signMessage, stale canary deleted and never executes, original-state form",
    );

    await fresh();
    await page.evaluate(async () => {
      window.blockedFixtureDb = await new Promise((resolve, reject) => {
        const r = indexedDB.open("hot-connector", 1);
        r.onupgradeneeded = () => r.result.createObjectStore("wallets");
        r.onsuccess = () => resolve(r.result);
        r.onerror = () => reject(Error("blocked fixture open failed"));
      });
      window.blockedFixtureDb.onversionchange = () => {
        window.fixtureVersionChange = true;
      };
    });
    await page
      .getByRole("button", { name: "Connect wallet", exact: true })
      .click();
    await statusIs("canonical-refused");
    assert.equal(await page.evaluate(() => window.fixtureVersionChange), true);
    assert.equal(await page.locator("#connect").isDisabled(), true);
    assert.equal(await page.locator("#cancel").isDisabled(), true);
    await page.evaluate(async () => {
      window.blockedFixtureDb.close();
      await new Promise((resolve, reject) => {
        const r = indexedDB.open("hot-connector", 1);
        r.onsuccess = () => {
          r.result.close();
          resolve();
        };
        r.onerror = () => reject(Error("late completion barrier failed"));
      });
    });
    assert.equal(await page.locator(".hot-connector-popup").count(), 0);
    assert.equal(await page.locator("#connect").isDisabled(), true);
    assert.equal(
      requests.filter((r) => r.path.endsWith("fixture-wallet.js")).length,
      0,
    );
    console.log(
      "PASS real IndexedDB blocked deletion terminal; late release cannot resume",
    );

    await fresh();
    await openChooser();
    await page.evaluate(() => {
      const keep = document.createElement("aside");
      keep.id = "unrelated-fixture";
      document.body.append(keep);
      document.querySelector("#cancel").click();
      if (document.querySelector(".hot-connector-popup"))
        throw Error("chooser survived cancel");
    });
    await statusIs("canonical-cancelled");
    assert.equal(await page.locator("#unrelated-fixture").count(), 1);
    assert.equal(
      requests.filter((r) => r.method === "POST" && r.validForm).length,
      1,
    );
    console.log(
      "PASS cancellation with real chooser open removes only owned popup",
    );

    await fresh();
    await page.evaluate(() => {
      window.fixtureHeldMessage = null;
      window.addEventListener(
        "message",
        (event) => {
          if (
            event.data.method === "storage.set" &&
            !window.fixtureHeldMessage
          ) {
            window.fixtureHeldMessage = event;
            event.stopImmediatePropagation();
          }
        },
        true,
      );
    });
    await openChooser();
    await choose();
    await page.waitForFunction(() => window.fixtureHeldMessage !== null);
    assert.equal(await page.locator(".hot-connector-popup iframe").count(), 1);
    await page.evaluate(() => {
      if (localStorage.getItem("fixture-wallet:fixture") !== null)
        throw Error("storage barrier failed");
      document.querySelector("#cancel").click();
      if (document.querySelector(".hot-connector-popup"))
        throw Error("iframe survived cancel");
      window.dispatchEvent(window.fixtureHeldMessage);
    });
    await statusIs("canonical-cancelled");
    assert.equal(
      await page.evaluate(() => localStorage.getItem("fixture-wallet:fixture")),
      null,
    );
    await page.evaluate(async () => {
      const wrapper = document.createElement("div");
      wrapper.className = "hot-connector-popup";
      document.body.append(wrapper);
      await Promise.resolve();
      if (wrapper.isConnected) throw Error("late popup survived");
    });
    assert.equal(await page.locator("iframe").count(), 0);
    console.log(
      "PASS real SDK iframe storage request held at test event barrier; cancellation rejects replay and late popup",
    );

    await fresh();
    await page.evaluate(() => {
      window.fixtureInjectedCalls = 0;
      window.fixtureParentRequests = 0;
      window.addEventListener("message", (e) => {
        if (e.data.type === "near-wallet-injected-request")
          window.fixtureParentRequests++;
      });
    });
    await openChooser();
    await page.evaluate(async () => {
      const config = JSON.parse(
          document.querySelector("#challenge").textContent,
        ),
        manifest = config.manifest.wallets[0];
      window.dispatchEvent(
        new CustomEvent("near-wallet-injected", {
          detail: {
            manifest,
            async signIn() {
              window.fixtureInjectedCalls++;
              throw Error("untrusted injected fixture");
            },
          },
        }),
      );
      const frame = document.createElement("iframe");
      frame.setAttribute("sandbox", "allow-scripts");
      const delivered = new Promise((resolve) => {
        const handler = (e) => {
          if (e.data.type === "fixture-injection-delivered") {
            window.removeEventListener("message", handler);
            window.fixtureInjectionOrigin = e.origin;
            resolve();
          }
        };
        window.addEventListener("message", handler);
      });
      frame.srcdoc =
        "<script nonce=" +
        JSON.stringify(config.cspNonce) +
        ">parent.postMessage(" +
        JSON.stringify({ type: "near-wallet-injected", manifest }) +
        ', "*");parent.postMessage({type:"fixture-injection-delivered"},"*");<' +
        "/script>";
      document.body.append(frame);
      await delivered;
      frame.remove();
    });
    assert.equal(
      await page.evaluate(() => window.fixtureInjectionOrigin),
      "null",
    );
    await choose();
    await statusIs("canonical-received");
    assert.equal(await page.evaluate(() => window.fixtureInjectedCalls), 0);
    assert.equal(await page.evaluate(() => window.fixtureParentRequests), 0);
    assert.deepEqual(await page.evaluate(() => window.sdkFixtureEvents), [
      "signIn",
      "signMessage",
    ]);
    console.log(
      "PASS same-ID custom discovery and real opaque-frame postMessage cannot replace pinned executor",
    );

    await fresh();
    badAsset = true;
    await openChooser();
    await choose();
    await statusIs("canonical-unavailable");
    assert.equal(await page.locator("#connect").isDisabled(), false);
    assert.equal(await page.locator("iframe").count(), 0);
    assert.equal(
      requests.filter((r) => r.path.endsWith("fixture-wallet.js")).length,
      1,
    );
    assert.equal(requests.filter((r) => r.method === "POST").length, 0);
    console.log("PASS HTTP 503 preflight blocks real SDK iframe and deposit");
  } finally {
    await browserInstance.close();
    await new Promise((resolve) => server.close(resolve));
  }
})().catch((error) => {
  console.error(error);
  server.close();
  process.exitCode = 1;
});

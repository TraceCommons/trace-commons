// Synthetic browser state tests; no wallet or Cloud authentication is performed.
// Run: node scripts/ci/test-near-wallet-browser.cjs /absolute/path/to/near_wallet_browser.js
const fs = require("node:fs");
const vm = require("node:vm");
const assert = require("node:assert/strict");
const source = fs.readFileSync(process.argv[2] ?? require("node:path").resolve(
  __dirname, "../../crates/trace-commons-contributor/src/daemon/nearai_credential/near_wallet_browser.js"
), "utf8");
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((a, b) => {
    resolve = a;
    reject = b;
  });
  return { promise, resolve, reject };
};
const flush = async () => {
  for (let i = 0; i < 20; i++) await Promise.resolve();
};
function fixture(options = {}) {
  const state = "A".repeat(43),
    origin = "http://127.0.0.1:43219";
  const copy = Object.fromEntries(
    [
      "choose",
      "waiting",
      "received",
      "refused",
      "expired",
      "cancelled",
      "unavailable",
    ].map((k) => [k, "canonical-" + k]),
  );
  const config = {
    message: "Sign in to NEAR AI Cloud",
    recipient: "cloud.near.ai",
    nonce: Array(32).fill(7),
    expires_at_ms: Date.now() + 299000,
    manifest: {
      version: "1",
      wallets: [
        {
          id: "meteor",
          executor: options.executor ?? "/near-ai/near-wallet/assets/meteor.js",
          features: { signMessage: true, signInWithoutAddKey: true },
        },
      ],
    },
    cspNonce: "testnonce",
    copy,
    ...options.config,
  };
  const element = () => ({
    disabled: false,
    textContent: "canonical-refused",
    handlers: {},
    addEventListener(k, fn) {
      this.handlers[k] = fn;
    },
  });
  const connect = element(),
    cancel = element(),
    status = element(),
    challenge = {
      type: "application/json",
      textContent: JSON.stringify(config),
    };
  const els = { connect, cancel, status, challenge },
    events = {},
    timers = new Map(),
    calls = { construct: [], select: [], connect: [], sign: [], fetch: [] },
    storage = new Map([
      ["meteor:session", "synthetic"],
      ["unrelated", "keep"],
    ]);
  const good = {
    accountId: "synthetic.near",
    publicKey: "ed25519:" + "1".repeat(32),
    signature: btoa("\0".repeat(64)),
  };
  class NearConnector {
    constructor(v) {
      calls.construct.push(v);
    }
    async selectWallet(v) {
      calls.select.push(v);
      return "meteor";
    }
    async connect(v) {
      calls.connect.push(v);
      if (options.connectReject) throw Error("sensitive-wallet-error");
      return {
        manifest: config.manifest.wallets[0],
        signMessage: async (v) => {
          calls.sign.push(v);
          return options.sign ? options.sign.promise : (options.result ?? good);
        },
      };
    }
  }
  const databaseRequests = [];
  const window = {
    indexedDB: {
      deleteDatabase(name) {
        const request = { name };
        databaseRequests.push(request);
        if (options.cacheOutcome !== "pending") {
          Promise.resolve().then(() => {
            if (options.cacheOutcome === "blocked") request.onblocked?.();
            else if (options.cacheOutcome === "error") request.onerror?.();
            else request.onsuccess?.();
          });
        }
        return request;
      },
    },
    location: new URL(origin + "/near-ai/near-wallet/callback#" + state),
    history: {
      replaceState(a, b, path) {
        window.location = new URL(path, origin);
      },
    },
    HOTConnect: { NearConnector },
    setTimeout(fn, delay) {
      const id = timers.size + 1;
      timers.set(id, { fn, delay });
      return id;
    },
    clearTimeout(id) {
      timers.delete(id);
    },
    addEventListener(k, fn) {
      events[k] = fn;
    },
    localStorage: {
      get length() {
        return storage.size;
      },
      key(i) {
        return [...storage.keys()][i];
      },
      removeItem(k) {
        storage.delete(k);
      },
    },
    atob,
    btoa,
    fetch: async (url, opts) => {
      calls.fetch.push({ url, opts });
      if (url === "/near-ai/near-wallet/result" && options.depositReject)
        throw Error("sensitive-network-error");
      return {
        ok:
          url === "/near-ai/near-wallet/result"
            ? options.depositOk !== false
            : options.assetOk !== false,
        body: { cancel: async () => {} },
      };
    },
  };
  window.top = window;
  vm.runInNewContext(source, {
    window,
    document: {
      getElementById: (k) => els[k],
      body: { querySelectorAll: () => [] },
    },
    MutationObserver: class {
      observe() {}
    },
    Date,
    Map,
    URL,
    URLSearchParams,
    Uint8Array,
    AbortController,
    Error,
    Number,
    Object,
    Array,
  });
  return {
    state,
    copy,
    connect,
    cancel,
    status,
    window,
    calls,
    events,
    timers,
    storage,
    config,
    databaseRequests,
  };
}
(async () => {
  let f = fixture();
  assert.equal(f.window.location.hash, "");
  assert.equal(f.calls.construct.length, 0);
  await f.connect.handlers.click();
  assert.equal(f.calls.select.length, 1);
  assert.equal(
    f.calls.connect.length,
    1,
    "relative executor URL must reach SDK connect",
  );
  assert.equal(f.calls.connect[0].walletId, "meteor");
  assert.equal(f.calls.sign[0].callbackUrl, undefined);
  assert.equal(f.calls.sign[0].network, "mainnet");
  assert.equal(
    f.calls.fetch[0].url,
    "http://127.0.0.1:43219/near-ai/near-wallet/assets/meteor.js",
  );
  const opts = f.calls.construct[0];
  assert.equal(opts.customDataStorage, undefined);
  assert.equal(typeof opts.storage.get, "function");
  assert.equal(typeof opts.storage.set, "function");
  assert.equal(typeof opts.storage.remove, "function");
  assert.equal(opts.autoConnect, false);
  assert.deepEqual([...opts.excludedWallets], ["mynearwallet"]);
  assert.equal(opts.signIn, undefined);
  assert.equal(opts.footerBranding, null);
  const post = f.calls.fetch.find((x) => x.url.endsWith("/result"));
  assert.equal(new URLSearchParams(post.opts.body).get("state"), f.state);
  assert.equal(post.opts.credentials, "omit");
  assert.equal(post.opts.redirect, "error");
  assert.equal(f.status.textContent, f.copy.received);
  assert.equal(f.storage.has("meteor:session"), false);
  assert.equal(f.storage.get("unrelated"), "keep");
  console.log(
    "PASS explicit selection, asset preflight, no key options, sign parameters, original state, terminal cleanup",
  );
  let sign = deferred();
  f = fixture({ sign });
  const pending = f.connect.handlers.click();
  await flush();
  await f.connect.handlers.click();
  assert.equal(f.calls.sign.length, 1);
  f.cancel.handlers.click();
  await flush();
  assert.equal(
    new URLSearchParams(f.calls.fetch.at(-1).opts.body).get("error"),
    "cancelled",
  );
  sign.resolve({
    accountId: "late.near",
    publicKey: "ed25519:" + "1".repeat(32),
    signature: btoa("\0".repeat(64)),
  });
  await pending;
  assert.equal(
    f.calls.fetch.filter((x) => x.url.endsWith("/result")).length,
    1,
  );
  assert.equal(f.status.textContent, f.copy.cancelled);
  console.log(
    "PASS duplicate suppression, cancellation, late result rejection",
  );
  f = fixture({ depositReject: true });
  await f.connect.handlers.click();
  await f.connect.handlers.click();
  f.cancel.handlers.click();
  await flush();
  assert.equal(
    f.calls.fetch.filter((x) => x.url.endsWith("/result")).length,
    1,
  );
  assert.equal(f.connect.disabled, true);
  assert.equal(f.status.textContent, f.copy.refused);
  console.log("PASS ambiguous deposit is terminal");
  f = fixture({ assetOk: false });
  await f.connect.handlers.click();
  assert.equal(f.calls.connect.length, 0);
  assert.equal(f.calls.sign.length, 0);
  assert.equal(f.connect.disabled, false);
  assert.equal(f.calls.fetch.length, 1);
  console.log(
    "PASS failed executor preflight prevents SDK connection and permits retry",
  );
  f = fixture({
    result: { accountId: "BAD!", publicKey: "bad", signature: "bad" },
  });
  await f.connect.handlers.click();
  assert.equal(
    f.calls.fetch.filter((x) => x.url.endsWith("/result")).length,
    0,
  );
  assert.equal(f.connect.disabled, false);
  console.log("PASS malformed signature result is bounded and never deposited");
  sign = deferred();
  f = fixture({ sign });
  const late = f.connect.handlers.click();
  await flush();
  [...f.timers.values()].find((t) => t.delay > 3000).fn();
  sign.resolve({});
  await late;
  assert.equal(f.status.textContent, f.copy.expired);
  assert.equal(
    f.calls.fetch.filter((x) => x.url.endsWith("/result")).length,
    0,
  );
  assert.equal(f.cancel.disabled, true);
  console.log("PASS expiry stops pending work");
  sign = deferred();
  f = fixture({ sign });
  const hidden = f.connect.handlers.click();
  await flush();
  f.events.pagehide();
  sign.resolve({});
  await hidden;
  assert.equal(
    f.calls.fetch.filter((x) => x.url.endsWith("/result")).length,
    0,
  );
  assert.equal(f.connect.disabled, true);
  console.log("PASS pagehide discards pending work");
  f = fixture({ config: { nonce: [0] } });
  assert.equal(f.calls.construct.length, 0);
  assert.equal(f.connect.disabled, true);
  assert.equal(f.window.location.hash, "");
  console.log("PASS invalid challenge fails closed after fragment removal");
  for (const executor of [
    "http://127.0.0.1:43219/near-ai/near-wallet/assets/meteor.js",
    "/near-ai/near-wallet/assets/meteor.js",
  ]) {
    f = fixture({ executor });
    await f.connect.handlers.click();
    assert.equal(f.calls.sign.length, 1);
    assert.equal(f.status.textContent, f.copy.received);
  }
  console.log("PASS exact absolute and origin-relative executor URLs");
  for (const executor of [
    "https://wallet.example/near-ai/near-wallet/assets/meteor.js",
    "http://127.0.0.1:43220/near-ai/near-wallet/assets/meteor.js",
    "/near-ai/near-wallet/assets/other.js",
    "/near-ai/near-wallet/assets/meteor.js?nonce=bad",
    "/near-ai/near-wallet/assets/meteor.js#bad",
    "/executor.js",
    "http://user:pass@127.0.0.1:43219/near-ai/near-wallet/assets/meteor.js",
  ]) {
    f = fixture({ executor });
    await f.connect.handlers.click();
    assert.equal(f.calls.fetch.length, 0, executor);
    assert.equal(f.calls.connect.length, 0, executor);
    assert.equal(f.status.textContent, f.copy.unavailable, executor);
  }
  console.log(
    "PASS foreign origin, credentials, wrong executor path, query, and fragment rejected before fetch",
  );
  for (const cacheOutcome of ["blocked", "error"]) {
    f = fixture({ cacheOutcome });
    await f.connect.handlers.click();
    assert.equal(f.databaseRequests.length, 1);
    assert.equal(f.databaseRequests[0].name, "hot-connector");
    assert.equal(f.calls.construct.length, 0);
    assert.equal(f.connect.disabled, true);
    assert.equal(f.cancel.disabled, true);
    assert.equal(f.status.textContent, f.copy.refused);
    f.databaseRequests[0].onsuccess?.();
    await flush();
    assert.equal(f.calls.construct.length, 0);
  }
  console.log(
    "PASS blocked/error cache deletion terminal and late success ignored",
  );
  f = fixture({ cacheOutcome: "pending" });
  const cacheWait = f.connect.handlers.click();
  await flush();
  assert.equal(f.calls.construct.length, 0);
  [...f.timers.values()].find((t) => t.delay === 3000).fn();
  await cacheWait;
  assert.equal(f.status.textContent, f.copy.refused);
  f.databaseRequests[0].onsuccess?.();
  await flush();
  assert.equal(f.calls.construct.length, 0);
  console.log(
    "PASS bounded 3-second cache deletion timeout ignores late completion",
  );
  f = fixture({ cacheOutcome: "pending" });
  const cacheCancel = f.connect.handlers.click();
  await flush();
  f.cancel.handlers.click();
  await flush();
  f.databaseRequests[0].onsuccess?.();
  await cacheCancel;
  assert.equal(f.calls.construct.length, 0);
  assert.equal(f.status.textContent, f.copy.cancelled);
  console.log(
    "PASS cancellation during cache cleanup prevents SDK construction",
  );
  f = fixture({ cacheOutcome: "pending" });
  const cacheHide = f.connect.handlers.click();
  await flush();
  f.events.pagehide();
  f.databaseRequests[0].onsuccess?.();
  await cacheHide;
  assert.equal(f.calls.construct.length, 0);
  console.log(
    "PASS page disposal during cache cleanup prevents SDK construction",
  );
})().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});

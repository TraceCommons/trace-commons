// Behaviour tests for the browser passkey step-up page script (Z2 S7).
// Synthetic: no browser, no server, no real passkey. The script runs in a vm
// context against stub elements built from the page's own HTML (an id the
// page does not declare is null, as in a browser), the page's own copy table,
// and a scripted fetch.
// Run: node scripts/ci/test-step-up-page.cjs
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const assert = require("node:assert/strict");

const dir = path.resolve(
  __dirname,
  "../../crates/trace-commons-server/src/bin/trace_commons_ingest_internal",
);
const source = fs.readFileSync(path.join(dir, "step_up_page.js"), "utf8");
const rust = fs.readFileSync(path.join(dir, "step_up_page.rs"), "utf8");
const finishBodies = JSON.parse(
  fs.readFileSync(path.join(dir, "tests/step_up_finish_bodies.json"), "utf8"),
);

// Every key of STEP_UP_COPY, each mapped to a recognisable value, so a key the
// script names but the table lacks shows up as '' rather than passing.
const table = rust.slice(
  rust.indexOf("STEP_UP_COPY: &[(&str, &str)] = &["),
  rust.indexOf("];", rust.indexOf("STEP_UP_COPY: &[(&str, &str)] = &[")),
);
const copy = Object.fromEntries(
  [...table.matchAll(/\(\s*"([a-z_]+)",/g)].map((m) => [m[1], "copy:" + m[1]]),
);
assert.ok(Object.keys(copy).length > 20, "read the copy table");

// The elements the page declares, with their initial hidden/disabled state.
const declared = [...rust.matchAll(/id=\\"([a-z-]+)\\"([^>]*)>/g)].map((m) => ({
  id: m[1],
  hidden: /\bhidden\b/.test(m[2]),
  disabled: /\bdisabled\b/.test(m[2]),
}));

const SECTIONS = ["add-passkey", "remove-passkey", "change-payout"];
const flush = async () => {
  for (let i = 0; i < 50; i++) await new Promise((r) => setImmediate(r));
};
const buffer = (bytes) => new Uint8Array(bytes).buffer;
const reply = (status, body) => ({
  ok: status >= 200 && status < 300,
  status,
  type: "basic",
  // A fresh copy per read: the script edits what it is given.
  json: async () => structuredClone(body),
});

function element(id, init = {}) {
  return {
    id,
    hidden: false,
    disabled: false,
    textContent: "",
    value: "",
    children: [],
    handlers: {},
    ...init,
    addEventListener(kind, fn) {
      this.handlers[kind] = fn;
    },
    replaceChildren(...xs) {
      this.children = xs;
    },
    append(...xs) {
      this.children.push(...xs);
    },
    // Everything this element shows, children included.
    get text() {
      return [
        this.textContent,
        ...this.children.map((c) => (typeof c === "string" ? c : c.text)),
      ].join("");
    },
  };
}

// `routes` maps "METHOD path" to a reply, a function returning one, or an
// Error to throw. Unlisted calls answer 200 with an empty object.
function page(options = {}) {
  const els = {};
  for (const d of declared) els[d.id] = element(d.id, d);
  els["tc-copy"].textContent = JSON.stringify(copy);
  // What the account line said at the moment each action section appeared.
  const reveals = [];
  for (const id of SECTIONS) {
    let hidden = els[id].hidden;
    Object.defineProperty(els[id], "hidden", {
      get: () => hidden,
      set: (value) => {
        if (hidden && !value) {
          reveals.push(
            els.account && !els.account.hidden ? els["account-passkey"].textContent : "",
          );
        }
        hidden = value;
      },
    });
  }
  const calls = [];
  const routes = {
    "POST /account/passkey/login/start?purpose=step_up": reply(200, {
      publicKey: { challenge: "AAAA", allowCredentials: [] },
    }),
    "POST /account/passkey/login/finish": {
      ok: false,
      status: 0,
      type: "opaqueredirect",
    },
    "GET /v1/account/passkeys": reply(200, {
      passkeys: [
        { credential_id: "CRED-SECRET-ID", label: "Laptop", this_device: true },
        { credential_id: "CRED-OTHER-ID", label: "Phone", this_device: false },
      ],
    }),
    "GET /v1/account/near-identities": reply(200, {
      near_identities: [
        {
          near_account_id: "alice.near",
          public_key: "ed25519:PUBKEY-SECRET",
          is_payout: true,
        },
      ],
    }),
    "POST /v1/account/passkeys/register/start": reply(200, {
      publicKey: {
        challenge: "AAAA",
        user: { id: "AQID", name: "x", displayName: "x" },
        excludeCredentials: [],
      },
    }),
    "POST /v1/account/passkeys/register/finish": reply(200, {}),
    "POST /v1/account/logout": reply(200, {}),
    ...options.routes,
  };
  const fetch = async (url, opts) => {
    const key = `${opts.method} ${url}`;
    calls.push({ key, body: opts.body === undefined ? undefined : JSON.parse(opts.body) });
    let r = key in routes ? routes[key] : reply(200, {});
    if (typeof r === "function") r = r(calls);
    if (r instanceof Error) throw r;
    return r;
  };
  const credentials = {
    get: async () => ({
      id: "AQIDBA",
      rawId: buffer([1, 2, 3, 4]),
      type: "public-key",
      response: {
        authenticatorData: buffer([0, 1, 2]),
        clientDataJSON: buffer([123, 125]),
        signature: buffer([48, 69, 2]),
        userHandle: options.noUserHandle ? null : buffer([...Array(16).keys()]),
      },
    }),
    create: async () => ({
      id: "AQIDBA",
      rawId: buffer([1, 2, 3, 4]),
      type: "public-key",
      response: {
        attestationObject: buffer([163, 99, 102]),
        clientDataJSON: buffer([123, 125]),
      },
    }),
  };
  vm.runInNewContext(source, {
    window: { PublicKeyCredential: function () {}, confirm: () => true },
    navigator: { credentials },
    document: {
      getElementById: (id) => els[id] ?? null,
      createElement: (tag) => element(tag),
      body: { dataset: { action: options.action ?? "" } },
    },
    fetch,
    atob,
    btoa,
  });
  const $ = (id) => {
    assert.ok(els[id], `the page declares #${id}`);
    return els[id];
  };
  return { els, $, calls, routes, reveals, status: () => els.status.textContent };
}

async function signIn(p) {
  await p.$("sign-in").handlers.click();
  await flush();
}

// A value's structure: the keys at every level and whether each leaf is a
// string or null. Values themselves are not compared.
const shape = (v) =>
  v === null
    ? "null"
    : typeof v === "object"
      ? Object.fromEntries(
          Object.keys(v)
            .sort()
            .map((k) => [k, shape(v[k])]),
        )
      : typeof v;

const tests = [];
const test = (name, fn) => tests.push({ name, fn });

// The session-ended state: nothing to act on, sign-in offered again.
function assertSessionEnded(p, what) {
  for (const id of [...SECTIONS, "finish"]) assert.equal(p.$(id).hidden, true, `${what}: #${id} hidden`);
  assert.equal(p.$("account").hidden, true, `${what}: account hidden`);
  assert.equal(p.$("sign-in-section").hidden, false, `${what}: sign-in shown`);
  assert.equal(p.$("sign-in").disabled, false, `${what}: sign-in enabled`);
  assert.equal(p.status(), copy.session_ended, `${what}: says the session ended`);
}

test("sign-in reveals the actions and posts the finish body in the fixture's shape", async () => {
  for (const [noUserHandle, fixture] of [
    [false, "login_finish"],
    [true, "login_finish_without_user_handle"],
  ]) {
    const p = page({ noUserHandle });
    assert.equal(p.$("sign-in").disabled, false);
    await signIn(p);
    const finish = p.calls.find((c) => c.key === "POST /account/passkey/login/finish");
    assert.deepEqual(shape(finish.body), shape(finishBodies[fixture]), fixture);
    assert.deepEqual(finish.body, finishBodies[fixture], `${fixture}: same bytes`);
    assert.equal(p.$("sign-in-section").hidden, true);
    for (const id of [...SECTIONS, "finish"]) assert.equal(p.$(id).hidden, false, id);
  }
});

test("add-passkey posts the register body in the fixture's shape", async () => {
  for (const [label, fixture] of [
    ["Phone", "register_finish"],
    ["", "register_finish_without_label"],
  ]) {
    const p = page({ action: "add-passkey" });
    await signIn(p);
    p.$("passkey-label").value = label;
    await p.$("add-passkey-button").handlers.click();
    await flush();
    const finish = p.calls.find((c) => c.key === "POST /v1/account/passkeys/register/finish");
    assert.deepEqual(shape(finish.body), shape(finishBodies[fixture]), fixture);
    assert.deepEqual(finish.body, finishBodies[fixture], `${fixture}: same bytes`);
    assert.equal(p.status(), copy.action_add_done);
  }
});

test("a 401 from an action ends the session on the page and offers sign-in again", async () => {
  const p = page({
    action: "add-passkey",
    routes: { "POST /v1/account/passkeys/register/start": reply(401, {}) },
  });
  await signIn(p);
  await p.$("add-passkey-button").handlers.click();
  await flush();
  assertSessionEnded(p, "register/start 401");

  // Signing in again works from there.
  p.routes["POST /v1/account/passkeys/register/start"] = reply(200, {
    publicKey: { challenge: "AAAA", user: { id: "AQID" }, excludeCredentials: [] },
  });
  await signIn(p);
  assert.equal(p.$("add-passkey").hidden, false);
  assert.equal(p.$("sign-in-section").hidden, true);
});

test("a 401 on every other call ends the session the same way", async () => {
  const cases = [
    ["remove-passkey", "DELETE /v1/account/passkeys/CRED-OTHER-ID", async (p) => {
      const row = p.$("passkey-list").children[1];
      await row.children.at(-1).handlers.click();
    }],
    ["change-payout", "PATCH /v1/account/near-identities/ed25519%3AOTHER/payout", async (p) => {
      const row = p.$("payout-list").children[1];
      await row.children.at(-1).handlers.click();
    }],
    ["add-passkey", "POST /v1/account/passkeys/register/finish", async (p) => {
      await p.$("add-passkey-button").handlers.click();
    }],
  ];
  for (const [action, key, act] of cases) {
    const p = page({
      action,
      routes: {
        "GET /v1/account/near-identities": reply(200, {
          near_identities: [
            { near_account_id: "alice.near", public_key: "ed25519:PUBKEY-SECRET", is_payout: true },
            { near_account_id: "bob.near", public_key: "ed25519:OTHER", is_payout: false },
          ],
        }),
        [key]: reply(401, {}),
      },
    });
    await signIn(p);
    await act(p);
    await flush();
    assertSessionEnded(p, key);
  }
  // A list read that meets an ended session.
  const p = page({
    action: "remove-passkey",
    routes: {
      "GET /v1/account/passkeys": (calls) =>
        calls.filter((c) => c.key === "GET /v1/account/passkeys").length > 1
          ? reply(401, {})
          : reply(200, { passkeys: [{ credential_id: "C", label: "Laptop", this_device: true }] }),
    },
  });
  await signIn(p);
  assertSessionEnded(p, "passkey list 401");
});

test("a 403 is still the refusal, not a session end", async () => {
  const p = page({
    action: "add-passkey",
    routes: { "POST /v1/account/passkeys/register/start": reply(403, {}) },
  });
  await signIn(p);
  await p.$("add-passkey-button").handlers.click();
  await flush();
  assert.equal(p.status(), copy.action_refused);
  assert.equal(p.$("add-passkey").hidden, false);
});

test("sign-out reports what logout did", async () => {
  // Success: signed out, nothing left to act on.
  let p = page();
  await signIn(p);
  await p.$("sign-out").handlers.click();
  await flush();
  assert.equal(p.status(), copy.signed_out);
  for (const id of [...SECTIONS, "finish", "account"]) assert.equal(p.$(id).hidden, true, id);

  // A server error or a network failure: the session may be live, so the page
  // says so and keeps the sign-out button to retry.
  for (const failure of [reply(500, {}), reply(503, {}), new Error("network")]) {
    p = page({ routes: { "POST /v1/account/logout": failure } });
    await signIn(p);
    await p.$("sign-out").handlers.click();
    await flush();
    assert.equal(p.status(), copy.sign_out_failed, String(failure.status ?? failure));
    assert.equal(p.$("finish").hidden, false);
    assert.equal(p.$("sign-out").disabled, false);
  }

  // A 401: the session had already ended.
  p = page({ routes: { "POST /v1/account/logout": reply(401, {}) } });
  await signIn(p);
  await p.$("sign-out").handlers.click();
  await flush();
  assertSessionEnded(p, "logout 401");
});

test("the page shows which account is signed in, and nothing secret", async () => {
  for (const action of ["", ...SECTIONS]) {
    const p = page({ action });
    await signIn(p);
    assert.equal(p.$("account").hidden, false, `${action}: account shown`);
    assert.equal(p.$("account-passkey").textContent, `${copy.account_passkey} Laptop`);
    assert.equal(p.$("account-near").textContent, `${copy.account_near} alice.near`);
    const shown = p.$("account").text + p.$("account-passkey").text + p.$("account-near").text;
    for (const secret of ["CRED-SECRET-ID", "PUBKEY-SECRET", "ed25519", "Phone"]) {
      assert.ok(!shown.includes(secret), `${action}: shows ${secret}`);
    }
    // Already on screen when each action it is there to check appeared.
    assert.ok(p.reveals.length > 0, `${action}: an action appeared`);
    for (const shownThen of p.reveals) {
      assert.equal(shownThen, `${copy.account_passkey} Laptop`, `${action}: account shown first`);
    }
  }

  // Unnamed passkey, several NEAR accounts.
  let p = page({
    routes: {
      "GET /v1/account/passkeys": reply(200, {
        passkeys: [{ credential_id: "C", this_device: true }],
      }),
      "GET /v1/account/near-identities": reply(200, {
        near_identities: [
          { near_account_id: "alice.near", public_key: "ed25519:A", is_payout: false },
          { near_account_id: "bob.near", public_key: "ed25519:B", is_payout: true },
        ],
      }),
    },
  });
  await signIn(p);
  assert.equal(p.$("account-passkey").textContent, copy.account_passkey_unnamed);
  assert.equal(p.$("account-near").textContent, `${copy.account_near} alice.near, bob.near`);

  // No NEAR account: that line is empty and hidden.
  p = page({ routes: { "GET /v1/account/near-identities": reply(200, { near_identities: [] }) } });
  await signIn(p);
  assert.equal(p.$("account-near").hidden, true);
  assert.equal(p.$("account-passkey").textContent, `${copy.account_passkey} Laptop`);
});

test("an account that cannot be named is offered no changes, only a retry", async () => {
  const failures = [
    ["passkeys 500", { "GET /v1/account/passkeys": reply(500, {}) }],
    ["near-identities 500", { "GET /v1/account/near-identities": reply(500, {}) }],
    ["passkeys network error", { "GET /v1/account/passkeys": new Error("network") }],
    [
      "no passkey marked this_device",
      {
        "GET /v1/account/passkeys": reply(200, {
          passkeys: [{ credential_id: "C", label: "Laptop", this_device: false }],
        }),
      },
    ],
  ];
  for (const [what, routes] of failures) {
    for (const action of ["", ...SECTIONS]) {
      const p = page({ action, routes: { ...routes } });
      await signIn(p);
      for (const id of SECTIONS) assert.equal(p.$(id).hidden, true, `${what} ${action}: #${id} hidden`);
      assert.equal(p.reveals.length, 0, `${what} ${action}: no action ever appeared`);
      assert.equal(p.$("account").hidden, false, `${what}: account block shown`);
      assert.equal(p.$("account-passkey").textContent, copy.account_unknown, what);
      assert.equal(p.$("account-retry").hidden, false, `${what}: retry offered`);
      assert.equal(p.$("finish").hidden, false, `${what}: sign-out still offered`);
      assert.equal(p.$("sign-in-section").hidden, true, `${what}: still signed in`);
      // Only reads were made: nothing was changed.
      assert.ok(
        p.calls.every((c) => c.key.startsWith("GET ") || c.key.includes("/login/")),
        `${what}: made a change`,
      );
    }
  }

  // The retry succeeds once the reads do: the account is named, then the
  // actions appear.
  const p = page({ routes: { "GET /v1/account/passkeys": reply(500, {}) } });
  await signIn(p);
  delete p.routes["GET /v1/account/passkeys"];
  p.routes["GET /v1/account/passkeys"] = reply(200, {
    passkeys: [{ credential_id: "C", label: "Laptop", this_device: true }],
  });
  await p.$("account-retry").handlers.click();
  await flush();
  assert.equal(p.$("account-retry").hidden, true, "retry gone");
  assert.equal(p.$("account-passkey").textContent, `${copy.account_passkey} Laptop`);
  for (const id of SECTIONS) assert.equal(p.$(id).hidden, false, `after retry: #${id} shown`);
  for (const shownThen of p.reveals) assert.equal(shownThen, `${copy.account_passkey} Laptop`);

  // A retry that meets an ended session takes the session-ended path.
  const q = page({ routes: { "GET /v1/account/passkeys": reply(500, {}) } });
  await signIn(q);
  q.routes["GET /v1/account/passkeys"] = reply(401, {});
  await q.$("account-retry").handlers.click();
  await flush();
  assertSessionEnded(q, "retry 401");
  assert.equal(q.$("account-retry").hidden, true, "retry 401: retry hidden");
});

(async () => {
  let failed = 0;
  for (const { name, fn } of tests) {
    try {
      await fn();
      console.log(`PASS ${name}`);
    } catch (error) {
      failed += 1;
      console.log(`FAIL ${name}\n  ${error.message.split("\n").join("\n  ")}`);
    }
  }
  console.log(`${tests.length - failed} passed, ${failed} failed`);
  process.exit(failed === 0 ? 0 : 1);
})();

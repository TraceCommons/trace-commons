import assert from "node:assert/strict";
import { execFile, spawnSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { renderAasa } from "./render-aasa.mjs";

const scriptsDir = fileURLToPath(new URL(".", import.meta.url));
const renderScript = join(scriptsDir, "render-aasa.mjs");
const checkScript = join(scriptsDir, "..", "..", "scripts", "check-aasa.sh");
const TEAM = "ABCDE12345";

test("renders webcredentials.apps only, with the default bundle id", () => {
  const body = renderAasa({ TC_APPLE_TEAM_ID: TEAM });
  assert.deepEqual(JSON.parse(body), { webcredentials: { apps: [`${TEAM}.ai.tracecommons.shell`] } });
});

test("bundle id can be overridden", () => {
  const body = renderAasa({ TC_APPLE_TEAM_ID: TEAM, TC_MACOS_BUNDLE_ID: "ai.example.app" });
  assert.deepEqual(JSON.parse(body).webcredentials.apps, [`${TEAM}.ai.example.app`]);
});

for (const bad of [undefined, "", "abcde12345", "ABCDE1234", "ABCDE123456", "ABCDE-2345", "TEAMID_HERE", "XXXXXXXXXX ", "ABCDE1234\n"]) {
  test(`unset or bad Team ID is refused: ${JSON.stringify(bad)}`, () => {
    assert.throws(() => renderAasa(bad === undefined ? {} : { TC_APPLE_TEAM_ID: bad }), /TC_APPLE_TEAM_ID/);
  });
}

test("the render step exits non-zero and writes nothing without a Team ID", () => {
  const env = { ...process.env };
  delete env.TC_APPLE_TEAM_ID;
  const result = spawnSync(process.execPath, [renderScript], { env, encoding: "utf8" });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /TC_APPLE_TEAM_ID/);
  const bad = spawnSync(process.execPath, [renderScript], { env: { ...env, TC_APPLE_TEAM_ID: "nope" }, encoding: "utf8" });
  assert.notEqual(bad.status, 0);
});

async function loadWorker() {
  const source = await readFile(join(scriptsDir, "..", "public", "_worker.js"), "utf8");
  const mod = await import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);
  return mod.default;
}

// A stand-in for Pages' ASSETS binding: serves only the files in `files`,
// answering 404 for everything else, like the real asset layer.
function assetsFor(files) {
  return {
    async fetch(request) {
      const path = new URL(request.url).pathname;
      if (path in files) {
        return new Response(files[path], { status: 200, headers: { "content-type": "application/octet-stream" } });
      }
      return new Response("missing", { status: 404 });
    },
  };
}

const AASA_URL = "https://tracecommons.ai/.well-known/apple-app-site-association";

test("worker serves the rendered AASA as application/json with a 200", async () => {
  const worker = await loadWorker();
  const body = renderAasa({ TC_APPLE_TEAM_ID: TEAM });
  const env = { ASSETS: assetsFor({ "/.well-known/apple-app-site-association": body, "/": "<html>index</html>" }) };
  const response = await worker.fetch(new Request(AASA_URL), env);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-type"), "application/json");
  assert.equal(response.headers.get("location"), null);
  assert.equal(await response.text(), body);
});

test("worker never returns the index fallback for a missing AASA", async () => {
  const worker = await loadWorker();
  const env = { ASSETS: assetsFor({ "/": "<html>index</html>" }) };
  for (const path of ["/.well-known/apple-app-site-association", "/.well-known/anything", "/.well-known/"]) {
    const response = await worker.fetch(new Request(`https://tracecommons.ai${path}`), env);
    assert.equal(response.status, 404, path);
    assert.doesNotMatch(await response.text(), /index/, path);
  }
  // The SPA fallback for ordinary routes is unchanged.
  const spa = await worker.fetch(new Request("https://tracecommons.ai/leaderboard"), env);
  assert.equal(spa.status, 200);
  assert.match(await spa.text(), /index/);
});

test("worker refuses to relay a redirect from the asset layer", async () => {
  const worker = await loadWorker();
  const env = { ASSETS: { fetch: async () => new Response(null, { status: 308, headers: { location: "/elsewhere" } }) } };
  const response = await worker.fetch(new Request(AASA_URL), env);
  assert.equal(response.status, 404);
  assert.equal(response.headers.get("location"), null);
});

function toolAvailable(tool) {
  return spawnSync("sh", ["-c", `command -v ${tool}`]).status === 0;
}

async function withServer(handler, run) {
  const server = createServer(handler);
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    return await run(`http://127.0.0.1:${server.address().port}`);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}

// Async on purpose: the test server lives in this process, so a blocking
// spawnSync would stop it from answering curl.
function runCheck(base, ...args) {
  return new Promise((resolve) => {
    execFile("bash", [checkScript, base, ...args], { encoding: "utf8" }, (error, stdout, stderr) => {
      resolve({ status: error ? (error.code ?? 1) : 0, stdout, stderr });
    });
  });
}

test("check-aasa.sh accepts the worker's answer and rejects bad ones", { skip: !(toolAvailable("curl") && toolAvailable("jq")) }, async () => {
  const worker = await loadWorker();
  const good = renderAasa({ TC_APPLE_TEAM_ID: TEAM });
  const serveWith = (files) => async (req, res) => {
    const response = await worker.fetch(new Request(`http://x${req.url}`), { ASSETS: assetsFor(files) });
    res.writeHead(response.status, Object.fromEntries(response.headers));
    res.end(Buffer.from(await response.arrayBuffer()));
  };
  const aasaPath = "/.well-known/apple-app-site-association";

  await withServer(serveWith({ [aasaPath]: good, "/": "<html>index</html>" }), async (base) => {
    const ok = await runCheck(base);
    assert.equal(ok.status, 0, ok.stderr);
    assert.match(ok.stdout, /OK/);
    assert.equal((await runCheck(base, `${TEAM}.ai.tracecommons.shell`)).status, 0);
    assert.notEqual((await runCheck(base, "ZZZZZZZZZZ.ai.tracecommons.shell")).status, 0);
  });
  await withServer(serveWith({ "/": "<html>index</html>" }), async (base) => {
    assert.notEqual((await runCheck(base)).status, 0);
  });
  await withServer(serveWith({ [aasaPath]: '{"webcredentials":{"apps":["lowercase.ai.tracecommons.shell"]}}' }), async (base) => {
    assert.notEqual((await runCheck(base)).status, 0);
  });
  await withServer(serveWith({ [aasaPath]: '{"webcredentials":{"apps":["ABCDE12345.ai.tracecommons.shell"]},"applinks":{}}' }), async (base) => {
    assert.notEqual((await runCheck(base)).status, 0);
  });
  // A raw server that answers text/html, and one that redirects.
  await withServer((req, res) => { res.writeHead(200, { "content-type": "text/html" }); res.end(good); }, async (base) => {
    assert.notEqual((await runCheck(base)).status, 0);
  });
  await withServer((req, res) => { res.writeHead(301, { location: "/x" }); res.end(); }, async (base) => {
    assert.notEqual((await runCheck(base)).status, 0);
  });
});

// Tests for scripts/check-aasa.sh against local stand-ins for the origin and
// Apple's CDN. Run: node --test scripts/ci/test-check-aasa.mjs (needs curl, jq).
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createServer } from "node:http";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const checkScript = join(fileURLToPath(new URL(".", import.meta.url)), "..", "check-aasa.sh");
const APP = "ABCDE12345.ai.tracecommons.shell";
const GOOD = `{"webcredentials":{"apps":["${APP}"]}}\n`;
const PATH = "/.well-known/apple-app-site-association";

// A response is [status, headers, body].
const json = (body) => [200, { "content-type": "application/json" }, body];

async function withServer(respond, run) {
  const server = createServer((req, res) => {
    const [status, headers, body] = respond(req.url);
    res.writeHead(status, headers);
    res.end(body);
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    return await run(`http://127.0.0.1:${server.address().port}`);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}

// Serves `origin` at the AASA path and `cdn` at /a/v1/127.0.0.1, on one port.
function site(origin, cdn = json(GOOD)) {
  return (url) => {
    if (url === PATH) return origin;
    if (url === "/a/v1/127.0.0.1") return cdn;
    return [404, { "content-type": "text/html" }, "<html>404</html>"];
  };
}

function runCheck(base, { args = [], env = {} } = {}) {
  return new Promise((resolve) => {
    execFile(
      "bash",
      [checkScript, base, ...args],
      { encoding: "utf8", env: { ...process.env, TC_AASA_CDN: "1", TC_AASA_CDN_BASE: `${base}/a/v1`, ...env } },
      (error, stdout, stderr) => resolve({ status: error ? (error.code ?? 1) : 0, stdout, stderr }),
    );
  });
}

async function check(respond, options) {
  return withServer(respond, (base) => runCheck(base, options));
}

test("origin and CDN both good: exit 0 and both reported", async () => {
  const result = await check(site(json(GOOD)));
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /OK http:\/\/127\.0\.0\.1:\d+\/\.well-known\/apple-app-site-association/);
  assert.match(result.stdout, /OK http:\/\/127\.0\.0\.1:\d+\/a\/v1\/127\.0\.0\.1/);
});

test("a pinned app id must match exactly", async () => {
  assert.equal((await check(site(json(GOOD)), { args: [APP] })).status, 0);
  assert.equal((await check(site(json(GOOD)), { args: ["ZZZZZZZZZZ.ai.tracecommons.shell"] })).status, 1);
});

for (const [name, origin] of [
  ["missing (HTML 404)", [404, { "content-type": "text/html" }, "<html>404</html>"]],
  ["redirect", [301, { location: "/elsewhere" }, ""]],
  ["right body, wrong content type", [200, { "content-type": "text/html" }, GOOD]],
  ["HTML relabelled as JSON", json("<!doctype html><html>index</html>")],
  ["JSON without webcredentials.apps", json('{"applinks":{}}')],
  ["extra top-level key", json(`{"webcredentials":{"apps":["${APP}"]},"applinks":{}}`)],
  ["lowercase team id", json('{"webcredentials":{"apps":["abcde12345.ai.tracecommons.shell"]}}')],
  ["wrong bundle id", json('{"webcredentials":{"apps":["ABCDE12345.ai.example.app"]}}')],
]) {
  test(`origin fails with exit 1: ${name}`, async () => {
    const result = await check(site(origin));
    assert.equal(result.status, 1, `${name}: ${result.stdout}${result.stderr}`);
    assert.match(result.stderr, /FAIL .*\/\.well-known\/apple-app-site-association/);
  });
}

test("CDN not yet serving the file: exit 3, with Apple's failure reason", async () => {
  const cdn = [404, { "content-type": "text/plain", "apple-failure-reason": "SWCERR00101 Bad HTTP Response: 404 Not Found", "cache-control": "max-age=3600,public", age: "120" }, "Not Found"];
  const result = await check(site(json(GOOD), cdn));
  assert.equal(result.status, 3, result.stderr);
  assert.match(result.stdout, /OK .*apple-app-site-association/);
  assert.match(result.stderr, /Apple-Failure-Reason: SWCERR00101/);
  assert.match(result.stderr, /Age: 120/);
});

test("CDN serving a stale file for another app id: exit 3", async () => {
  const stale = json('{"webcredentials":{"apps":["ZZZZZZZZZZ.ai.tracecommons.shell"]}}');
  const result = await check(site(json(GOOD), stale));
  assert.equal(result.status, 3);
  assert.match(result.stderr, /differs from the origin/);
});

test("CDN body is checked whatever its content type says", async () => {
  assert.equal((await check(site(json(GOOD), [200, { "content-type": "application/octet-stream" }, GOOD]))).status, 0);
  assert.equal((await check(site(json(GOOD), json("<html>cdn error</html>")))).status, 3);
});

test("the CDN check is skipped for http unless forced, and TC_AASA_CDN=0 skips it", async () => {
  const brokenCdn = [404, {}, "Not Found"];
  const unforced = await check(site(json(GOOD), brokenCdn), { env: { TC_AASA_CDN: "" } });
  assert.equal(unforced.status, 0, unforced.stderr);
  assert.match(unforced.stdout, /SKIP Apple CDN check/);
  assert.equal((await check(site(json(GOOD), brokenCdn), { env: { TC_AASA_CDN: "0" } })).status, 0);
});

test("usage error without a base URL", async () => {
  const result = await new Promise((resolve) => {
    execFile("bash", [checkScript], (error) => resolve(error?.code ?? 0));
  });
  assert.equal(result, 2);
});

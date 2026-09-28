const INGEST_ORIGIN = "https://ingest.tracecommons.ai";
const API_PREFIX = "/api";
const COMMUNITY_PREFIX = "/api/v1/community/";
const COMMUNITY_PROFILE = "/api/v1/community/profile";
const ALLOWED_METHODS = new Set(["GET", "HEAD", "PUT", "DELETE"]);
const AASA_PATH = "/.well-known/apple-app-site-association";

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (isCommunityApi(url.pathname)) {
      return proxyCommunityRequest(request, url);
    }
    if (url.pathname.startsWith("/.well-known/")) {
      return serveWellKnown(request, env, url);
    }
    return serveAsset(request, env);
  },
};

// /.well-known/* never reaches the SPA fallback in serveAsset: the AASA file
// has no dot in its last segment, so a missing file would otherwise come back
// as index.html with a 200, which Apple's fetcher would read as a broken
// association. Anything that is not a plain 200 asset is a real 404 (a
// redirect from the asset layer is refused too: AASA must not redirect), and
// the AASA content type is set here rather than trusting _headers to apply to
// a response from an advanced-mode worker.
async function serveWellKnown(request, env, url) {
  if (request.method !== "GET" && request.method !== "HEAD") {
    return new Response("method not allowed", { status: 405, headers: { allow: "GET, HEAD" } });
  }
  const asset = await env.ASSETS.fetch(request);
  if (asset.status !== 200) {
    return new Response("not found", {
      status: 404,
      headers: { "content-type": "text/plain; charset=utf-8", "cache-control": "no-store" },
    });
  }
  const headers = new Headers(asset.headers);
  if (url.pathname === AASA_PATH) {
    headers.set("content-type", "application/json");
    headers.set("cache-control", "public, max-age=3600");
  }
  headers.set("x-content-type-options", "nosniff");
  return new Response(request.method === "HEAD" ? null : asset.body, { status: 200, headers });
}

function isCommunityApi(pathname) {
  return pathname === COMMUNITY_PROFILE || pathname.startsWith(COMMUNITY_PREFIX);
}

async function proxyCommunityRequest(request, url) {
  if (!ALLOWED_METHODS.has(request.method)) {
    return new Response("method not allowed", {
      status: 405,
      headers: { allow: "GET, HEAD, PUT, DELETE" },
    });
  }

  const target = new URL(`${url.pathname.slice(API_PREFIX.length)}${url.search}`, INGEST_ORIGIN);

  const headers = new Headers(request.headers);
  headers.delete("host");
  headers.delete("origin");
  headers.delete("referer");

  const upstream = await fetch(target, {
    method: request.method,
    headers,
    body: request.method === "GET" || request.method === "HEAD" ? undefined : request.body,
    redirect: "manual",
  });

  const responseHeaders = new Headers(upstream.headers);
  responseHeaders.delete("set-cookie");
  responseHeaders.set("cache-control", cacheControlFor(url.pathname, request.method));
  responseHeaders.set("x-tracecommons-proxy", "community");

  return new Response(upstream.body, {
    status: upstream.status,
    statusText: upstream.statusText,
    headers: responseHeaders,
  });
}

async function serveAsset(request, env) {
  const response = await env.ASSETS.fetch(request);
  if (response.status !== 404 || request.method !== "GET") {
    return response;
  }

  const url = new URL(request.url);
  const lastSegment = url.pathname.split("/").pop() || "";
  if (lastSegment.includes(".")) {
    return response;
  }

  return env.ASSETS.fetch(new Request(new URL("/", request.url), request));
}

function cacheControlFor(pathname, method) {
  if (method !== "GET" && method !== "HEAD") {
    return "no-store";
  }
  return pathname.includes("/profile") ? "no-store" : "public, max-age=30";
}

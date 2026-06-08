const INGEST_ORIGIN = "https://ingest.tracecommons.ai";
const API_PREFIX = "/api";
const COMMUNITY_PREFIX = "/api/v1/community/";
const COMMUNITY_PROFILE = "/api/v1/community/profile";
const ALLOWED_METHODS = new Set(["GET", "HEAD", "PUT", "DELETE"]);

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (isCommunityApi(url.pathname)) {
      return proxyCommunityRequest(request, url);
    }
    return serveAsset(request, env);
  },
};

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

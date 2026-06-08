import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { basename, extname, join, resolve } from "node:path";

const root = new URL("..", import.meta.url).pathname;
const publicDir = resolve(root, "public");
const port = Number.parseInt(process.env.PORT || "8788", 10);

const contentTypes = new Map([
  [".css", "text/css; charset=utf-8"],
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
  [".txt", "text/plain; charset=utf-8"],
]);

const server = createServer(async (request, response) => {
  try {
    if (request.method !== "GET" && request.method !== "HEAD") {
      response.writeHead(405, { allow: "GET, HEAD" });
      response.end();
      return;
    }

    const url = new URL(request.url || "/", `http://${request.headers.host || "localhost"}`);
    const target = await resolveTarget(url.pathname);
    const body = await readFile(target);
    response.writeHead(200, {
      "cache-control": cacheControl(target),
      "content-type": contentTypes.get(extname(target)) || "application/octet-stream",
    });
    if (request.method === "GET") response.end(body);
    else response.end();
  } catch (error) {
    response.writeHead(error.statusCode || 500, {
      "content-type": "text/plain; charset=utf-8",
    });
    response.end(error.message || "preview server error");
  }
});

server.listen(port, "127.0.0.1", () => {
  console.log(`TraceCommons community preview: http://127.0.0.1:${port}`);
});

async function resolveTarget(pathname) {
  const decoded = decodeURIComponent(pathname);
  const requested = decoded === "/" ? "/index.html" : decoded;
  const candidate = resolve(publicDir, `.${requested}`);
  if (!candidate.startsWith(`${publicDir}/`) && candidate !== publicDir) {
    throw Object.assign(new Error("path escapes public directory"), { statusCode: 400 });
  }
  if (await isFile(candidate)) return candidate;
  if (basename(candidate).includes(".")) {
    throw Object.assign(new Error("asset not found"), { statusCode: 404 });
  }
  return join(publicDir, "index.html");
}

async function isFile(path) {
  try {
    return (await stat(path)).isFile();
  } catch {
    return false;
  }
}

function cacheControl(path) {
  return basename(path) === "index.html" ? "no-cache" : "public, max-age=300";
}

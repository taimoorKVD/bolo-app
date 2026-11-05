import { createReadStream, existsSync, statSync } from "node:fs";
import { open } from "node:fs/promises";
import { join, normalize, resolve, sep, extname } from "node:path";
import type { IncomingMessage, ServerResponse } from "node:http";

const TYPES: Record<string, string> = {
  ".html": "text/html; charset=utf-8", ".htm": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8", ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8", ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml", ".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg",
  ".webp": "image/webp", ".gif": "image/gif", ".ico": "image/x-icon", ".woff2": "font/woff2",
  ".woff": "font/woff", ".ttf": "font/ttf", ".txt": "text/plain; charset=utf-8",
  ".xml": "application/xml", ".map": "application/json", ".wasm": "application/wasm",
};

/** Resolve a URL path under root; returns null on traversal or missing. */
export function safeJoin(root: string, urlPath: string): string | null {
  const decoded = decodeURIComponent(urlPath.split("?")[0].split("#")[0]);
  if (decoded.includes("\0")) return null;
  const rel = decoded.replace(/^\/+/, "").replace(/\\/g, "/");
  const full = resolve(root, normalize(rel));
  const base = resolve(root) + sep;
  if (full !== resolve(root) && !full.startsWith(base)) return null;
  return full;
}

function fileOrIndex(root: string, urlPath: string): string | null {
  const p = safeJoin(root, urlPath === "/" ? "index.html" : urlPath);
  if (!p) return null;
  try {
    const st = statSync(p);
    if (st.isFile()) return p;
    if (st.isDirectory()) {
      const idx = join(p, "index.html");
      return existsSync(idx) ? idx : null;
    }
  } catch { /* missing */ }
  return null;
}

/** Serve GET/HEAD from publicDir. SPA fallback: missing paths with no extension -> index.html. Returns true if handled. */
export async function tryStatic(req: IncomingMessage, res: ServerResponse, publicDir: string, spa = true): Promise<boolean> {
  if (!publicDir || !existsSync(publicDir)) return false;
  const method = req.method || "GET";
  if (method !== "GET" && method !== "HEAD") return false;
  const url = req.url || "/";
  let file = fileOrIndex(publicDir, url.split("?")[0]);
  if (!file && spa) {
    const pathOnly = url.split("?")[0];
    if (!extname(pathOnly) || pathOnly.endsWith("/")) file = fileOrIndex(publicDir, "/index.html");
  }
  if (!file) return false;
  const st = statSync(file);
  const type = TYPES[extname(file).toLowerCase()] || "application/octet-stream";
  res.statusCode = 200;
  res.setHeader("content-type", type);
  res.setHeader("content-length", st.size);
  res.setHeader("x-content-type-options", "nosniff");
  if (method === "HEAD") { res.end(); return true; }
  await new Promise<void>((resolveP, reject) => {
    const stream = createReadStream(file!);
    stream.on("error", reject);
    stream.on("end", () => resolveP());
    stream.pipe(res);
  });
  return true;
}

export { open };

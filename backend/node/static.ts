import { createReadStream, existsSync, statSync } from "node:fs";
import { open } from "node:fs/promises";
import { request as httpsRequest } from "node:https";
import { join, normalize, resolve, sep, extname } from "node:path";
import type { IncomingHttpHeaders, IncomingMessage, ServerResponse } from "node:http";

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

/**
 * Apache serves /contact via index.php. If Passenger still receives /contact,
 * proxy that PHP page instead of answering with the homepage.
 * The x-bolo-contact-proxy header stops a loop if the PHP URL comes back here.
 */
export async function boloProxyContactPhp(req: IncomingMessage, res: ServerResponse): Promise<boolean> {
  if (req.headers["x-bolo-contact-proxy"]) return false;
  try {
    const q = String(req.url || "").split("?")[1];
    const url = "https://bolo.thebetawebsite.com/contact/index.php" + (q ? "?" + q : "");
    const headers: Record<string, string> = {
      host: "bolo.thebetawebsite.com",
      accept: typeof req.headers.accept === "string" ? req.headers.accept : "text/html",
      "x-bolo-contact-proxy": "1",
      "x-forwarded-proto": "https",
    };
    if (typeof req.headers.cookie === "string") headers.cookie = req.headers.cookie;
    const lang = req.headers["accept-language"];
    if (typeof lang === "string") headers["accept-language"] = lang;
    const upstream = await new Promise<{ status: number; headers: IncomingHttpHeaders; body: Buffer } | null>((resolveUp) => {
      const preq = httpsRequest(url, { method: req.method || "GET", headers, timeout: 8000 }, (pres) => {
        const chunks: Buffer[] = [];
        pres.on("data", (c: Buffer) => chunks.push(c));
        pres.on("end", () => resolveUp({ status: pres.statusCode || 0, headers: pres.headers, body: Buffer.concat(chunks) }));
      });
      preq.on("error", () => resolveUp(null));
      preq.on("timeout", () => { preq.destroy(); resolveUp(null); });
      preq.end();
    });
    if (!upstream || upstream.status !== 200) return false;
    const html = upstream.body.toString("utf8");
    if (!html.includes('id="contact-form"') && !html.includes("id='contact-form'")) return false;
    res.statusCode = 200;
    res.setHeader("content-type", "text/html; charset=utf-8");
    res.setHeader("content-length", Buffer.byteLength(html));
    res.setHeader("x-content-type-options", "nosniff");
    const setCookie = upstream.headers["set-cookie"];
    if (setCookie) res.setHeader("set-cookie", setCookie);
    if ((req.method || "GET") === "HEAD") { res.end(); return true; }
    res.end(html);
    return true;
  } catch { return false; }
}

/** Serve GET/HEAD from publicDir. Unknown paths are not the homepage. Returns true if handled. */
export async function tryStatic(req: IncomingMessage, res: ServerResponse, publicDir: string, spa = true): Promise<boolean> {
  if (!publicDir || !existsSync(publicDir)) return false;
  const method = req.method || "GET";
  if (method !== "GET" && method !== "HEAD") return false;
  const url = req.url || "/";
  let file = fileOrIndex(publicDir, url.split("?")[0]);
  if (!file && spa) {
    const pathOnly = url.split("?")[0];
    // Homepage only for a real directory with no index.php, or for /downloads and /.well-known.
    if (!extname(pathOnly) || pathOnly.endsWith("/")) {
      const boloKeep = pathOnly === "/downloads" || pathOnly === "/downloads/" || pathOnly.startsWith("/downloads/")
        || pathOnly === "/.well-known" || pathOnly === "/.well-known/" || pathOnly.startsWith("/.well-known/");
      let boloDir = false, boloPhp = false;
      try {
        const boloFull = safeJoin(publicDir, pathOnly);
        boloDir = !!(boloFull && existsSync(boloFull) && statSync(boloFull).isDirectory());
        const boloSlash = pathOnly.endsWith("/") ? pathOnly : pathOnly + "/";
        const boloIdx = safeJoin(publicDir, boloSlash + "index.php");
        boloPhp = !!(boloIdx && existsSync(boloIdx));
      } catch { /* missing */ }
      if ((boloKeep || boloDir) && !boloPhp) file = fileOrIndex(publicDir, "/index.html");
    }
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

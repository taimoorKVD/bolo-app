/** Host routing for Bolo Office on its own subdomain. Pure functions, no I/O. */

export const DEFAULT_OFFICE_HOST = "officebolo.thebetawebsite.com";
export const ROBOTS_TXT = "User-agent: *\nDisallow: /\n";

export function normalizeHost(host: string | undefined | null): string {
  if (!host) return "";
  let h = String(host).trim().toLowerCase();
  h = h.replace(/^https?:\/\//, "").split("/")[0] ?? "";
  if (h.startsWith("[")) return h;
  const colon = h.lastIndexOf(":");
  if (colon > 0 && /^\d+$/.test(h.slice(colon + 1))) h = h.slice(0, colon);
  return h;
}

export function officeHostFromEnv(e: Record<string, string | undefined>): string {
  return normalizeHost(e.OFFICE_HOST || DEFAULT_OFFICE_HOST) || DEFAULT_OFFICE_HOST;
}

/** Same-origin check. A missing Origin is allowed (non-browser clients). */
export function originAllowed(origin: string | undefined, host: string | undefined): boolean {
  if (!origin) return true;
  try {
    return new URL(origin).host.toLowerCase() === String(host || "").trim().toLowerCase();
  } catch {
    return false;
  }
}

export type OfficeDecision =
  | { type: "robots" }
  | { type: "redirect"; location: string }
  | { type: "office"; internalPath: string; apiBase: string }
  | { type: "next" };

function splitUrl(url: string): { path: string; search: string } {
  const q = url.indexOf("?");
  const path = (q === -1 ? url : url.slice(0, q)) || "/";
  const search = q === -1 ? "" : url.slice(q);
  return { path, search };
}

/** /office, /office/setup, /office/api/x → /, /setup, /api/x. Null when not an office path. */
export function fromOfficePrefix(path: string): string | null {
  if (path === "/office" || path === "/office/") return "/";
  if (!path.startsWith("/office/")) return null;
  const rest = path.slice("/office".length);
  return rest || "/";
}

function safeAbsolute(officeHost: string, publicPath: string, search: string): string {
  const path = publicPath.startsWith("/") ? publicPath : `/${publicPath}`;
  try {
    const u = new URL(`https://${officeHost}${path}${search}`);
    if (u.hostname !== officeHost) return `https://${officeHost}/`;
    return u.toString();
  } catch {
    return `https://${officeHost}/`;
  }
}

function badPath(path: string): boolean {
  try {
    return decodeURIComponent(path).includes("..") || path.includes("\\");
  } catch {
    return true;
  }
}

/**
 * Office host: UI at /, setup at /setup, API at /api.
 * Any other host: /office and /admin 301 to the matching path on the office host.
 * /v1 is never redirected.
 */
export function routeRequest(opts: {
  host: string | undefined;
  url: string;
  method: string;
  officeHost: string;
}): OfficeDecision {
  const host = normalizeHost(opts.host);
  const officeHost = normalizeHost(opts.officeHost);
  const { path, search } = splitUrl(opts.url || "/");
  const method = (opts.method || "GET").toUpperCase();
  const onOffice = Boolean(officeHost) && host === officeHost;
  const read = method === "GET" || method === "HEAD";

  if (badPath(path)) return { type: "next" };

  if (onOffice) {
    if (path === "/robots.txt") return { type: "robots" };
    const legacy = fromOfficePrefix(path);
    if (legacy) {
      if (read) return { type: "redirect", location: `${legacy}${search}` };
      const internal = legacy === "/" ? "/admin" : `/admin${legacy}`;
      return { type: "office", internalPath: `${internal}${search}`, apiBase: "/api" };
    }
    if (path === "/admin" || path === "/admin/") {
      if (read) return { type: "redirect", location: `/${search}`.replace("//", "/") };
      return { type: "office", internalPath: `/admin${search}`, apiBase: "/api" };
    }
    if (path.startsWith("/admin/")) {
      const rest = path.slice("/admin".length);
      if (read) return { type: "redirect", location: `${rest}${search}` };
      return { type: "office", internalPath: `${path}${search}`, apiBase: "/api" };
    }
    let internal: string | null = null;
    if (path === "/" || path === "") internal = "/admin";
    else if (path === "/index.html") internal = "/admin";
    else if (path === "/setup" || path === "/setup/") internal = "/admin/setup";
    else if (path === "/api" || path.startsWith("/api/")) internal = `/admin${path}`;
    if (!internal) return { type: "next" };
    return { type: "office", internalPath: `${internal}${search}`, apiBase: "/api" };
  }

  const mapped = fromOfficePrefix(path);
  if (mapped !== null) return { type: "redirect", location: safeAbsolute(officeHost, mapped, search) };
  if (path === "/admin" || path === "/admin/") return { type: "redirect", location: safeAbsolute(officeHost, "/", search) };
  if (path.startsWith("/admin/")) {
    const rest = path.slice("/admin".length);
    return { type: "redirect", location: safeAbsolute(officeHost, rest, search) };
  }
  return { type: "next" };
}

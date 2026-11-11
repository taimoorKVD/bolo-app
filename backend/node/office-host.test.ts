import { describe, expect, it } from "vitest";
import { ROBOTS_TXT, normalizeHost, officeHostFromEnv, originAllowed, routeRequest } from "./office-host";

const HOST = "officebolo.thebetawebsite.com";

function route(host: string, url: string, method = "GET") {
  return routeRequest({ host, url, method, officeHost: HOST });
}

describe("office host routing", () => {
  it("serves Office at / with setup and /api on the office host", () => {
    expect(route(HOST, "/")).toEqual({ type: "office", internalPath: "/admin", apiBase: "/api" });
    expect(route(HOST, "/setup")).toEqual({ type: "office", internalPath: "/admin/setup", apiBase: "/api" });
    expect(route(HOST, "/api/login?x=1", "POST")).toEqual({ type: "office", internalPath: "/admin/api/login?x=1", apiBase: "/api" });
    expect(route("officebolo.thebetawebsite.com:443", "/")).toMatchObject({ type: "office" });
  });

  it("redirects legacy /office paths on the office host and serves robots", () => {
    expect(route(HOST, "/office")).toEqual({ type: "redirect", location: "/" });
    expect(route(HOST, "/office/setup")).toEqual({ type: "redirect", location: "/setup" });
    expect(route(HOST, "/office/api/login", "POST")).toEqual({ type: "office", internalPath: "/admin/api/login", apiBase: "/api" });
    expect(route(HOST, "/robots.txt")).toEqual({ type: "robots" });
    expect(ROBOTS_TXT.startsWith("User-agent: *\nDisallow: /")).toBe(true);
  });

  it("301s /office on the bolo host and leaves /v1 alone", () => {
    expect(route("bolo.thebetawebsite.com", "/office")).toEqual({
      type: "redirect",
      location: "https://officebolo.thebetawebsite.com/",
    });
    expect(route("bolo.thebetawebsite.com", "/office/setup?from=old")).toEqual({
      type: "redirect",
      location: "https://officebolo.thebetawebsite.com/setup?from=old",
    });
    expect(route("bolo.thebetawebsite.com", "/office/api/login", "POST").type).toBe("redirect");
    expect(route("bolo.thebetawebsite.com", "/v1/dictation/transcribe", "POST")).toEqual({ type: "next" });
    expect(route("bolo.thebetawebsite.com", "/")).toEqual({ type: "next" });
    expect(route("bolo.thebetawebsite.com", "/office/../v1")).toEqual({ type: "next" });
  });

  it("accepts the office origin and rejects a different one", () => {
    expect(originAllowed(undefined, HOST)).toBe(true);
    expect(originAllowed("https://officebolo.thebetawebsite.com", HOST)).toBe(true);
    expect(originAllowed("https://bolo.thebetawebsite.com", HOST)).toBe(false);
    expect(originAllowed("not a url", HOST)).toBe(false);
    expect(normalizeHost("OFFICEBOLO.thebetawebsite.com:8443")).toBe(HOST);
    expect(officeHostFromEnv({})).toBe(HOST);
    expect(officeHostFromEnv({ OFFICE_HOST: "https://Office.Example/" })).toBe("office.example");
  });
});

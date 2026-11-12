import { describe, expect, it } from "vitest";
import { FAVICON_ICO, writeFavicon } from "./favicon";

describe("office favicon", () => {
  it("is a real icon, not an empty response", () => {
    expect(FAVICON_ICO[0]).toBe(0);
    expect(FAVICON_ICO[1]).toBe(0);
    expect(FAVICON_ICO[2]).toBe(1);
    expect(FAVICON_ICO[3]).toBe(0);
    expect(FAVICON_ICO.length).toBeGreaterThan(100);
    expect(FAVICON_ICO.subarray(22, 26).toString("hex")).toBe("89504e47");
  });

  it("answers GET /favicon.ico and ignores other paths", () => {
    const headers = new Map<string, string>();
    let body: Buffer | undefined;
    const res = {
      statusCode: 0,
      setHeader: (name: string, value: string) => { headers.set(name, value); },
      end: (b?: Buffer) => { body = b; },
    };
    expect(writeFavicon("GET", "/favicon.ico", res)).toBe(true);
    expect(res.statusCode).toBe(200);
    expect(headers.get("content-type")).toBe("image/x-icon");
    expect(body?.length).toBe(FAVICON_ICO.length);
    expect(writeFavicon("GET", "/admin", res)).toBe(false);
  });
});

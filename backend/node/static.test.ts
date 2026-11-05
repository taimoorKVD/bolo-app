import { describe, expect, it, beforeAll, afterAll } from "vitest";
import { mkdtempSync, writeFileSync, mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { safeJoin, tryStatic } from "./static";
import { loadDotEnv } from "./load-env";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";

describe("static + env", () => {
  let dir: string;
  beforeAll(() => {
    dir = mkdtempSync(join(tmpdir(), "bolo-pub-"));
    writeFileSync(join(dir, "index.html"), "<h1>home</h1>");
    mkdirSync(join(dir, "assets"));
    writeFileSync(join(dir, "assets", "a.css"), "body{}");
    writeFileSync(join(dir, ".env"), "ONLY_IN_FILE=yes\nPRESET=fromfile\n");
  });
  afterAll(() => rmSync(dir, { recursive: true, force: true }));

  it("blocks path traversal", () => {
    expect(safeJoin(dir, "/../etc/passwd")).toBeNull();
    expect(safeJoin(dir, "/assets/../../etc/passwd")).toBeNull();
    expect(safeJoin(dir, "/assets/a.css")).toContain("a.css");
  });

  it("serves files and SPA fallback", async () => {
    const hit = async (url: string) => {
      const server = createServer(async (req, res) => {
        if (!(await tryStatic(req, res, dir))) { res.statusCode = 404; res.end("miss"); }
      });
      await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
      const port = (server.address() as { port: number }).port;
      const r = await fetch(`http://127.0.0.1:${port}${url}`);
      const text = await r.text();
      server.close();
      return { status: r.status, text, type: r.headers.get("content-type") || "" };
    };
    expect((await hit("/")).text).toContain("home");
    expect((await hit("/assets/a.css")).type).toContain("text/css");
    expect((await hit("/missing-page")).text).toContain("home"); // SPA
    expect((await hit("/no.css")).status).toBe(404);
  });

  it("loadDotEnv does not overwrite existing env", () => {
    process.env.PRESET = "keep";
    delete process.env.ONLY_IN_FILE;
    const loaded = loadDotEnv(dir);
    expect(loaded).toContain(".env");
    expect(process.env.ONLY_IN_FILE).toBe("yes");
    expect(process.env.PRESET).toBe("keep");
    delete process.env.ONLY_IN_FILE;
  });
});

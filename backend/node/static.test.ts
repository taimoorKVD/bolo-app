import { describe, expect, it, beforeAll, afterAll } from "vitest";
import { mkdtempSync, writeFileSync, mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { boloProxyContactPhp, safeJoin, tryStatic } from "./static";
import { loadDotEnv } from "./load-env";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";

describe("static + env", () => {
  let dir: string;
  beforeAll(() => {
    dir = mkdtempSync(join(tmpdir(), "bolo-pub-"));
    writeFileSync(join(dir, "index.html"), "<h1>home</h1>");
    mkdirSync(join(dir, "assets"));
    writeFileSync(join(dir, "assets", "a.css"), "body{}");
    mkdirSync(join(dir, "downloads"));
    writeFileSync(join(dir, "downloads", "Bolo_0.1.11_x64-setup.exe"), "installer");
    mkdirSync(join(dir, ".well-known"));
    mkdirSync(join(dir, "contact"));
    writeFileSync(join(dir, "contact", "index.php"), "<?php echo 'contact';");
    mkdirSync(join(dir, "urdu-voice-typing-for-pc"));
    writeFileSync(join(dir, "urdu-voice-typing-for-pc", "index.html"), "<h1>guide</h1>");
    mkdirSync(join(dir, "empty-dir"));
    writeFileSync(join(dir, ".env"), "ONLY_IN_FILE=yes\nPRESET=fromfile\n");
  });
  afterAll(() => rmSync(dir, { recursive: true, force: true }));

  it("blocks path traversal", () => {
    expect(safeJoin(dir, "/../etc/passwd")).toBeNull();
    expect(safeJoin(dir, "/assets/../../etc/passwd")).toBeNull();
    expect(safeJoin(dir, "/assets/a.css")).toContain("a.css");
  });

  it("serves files, directory fallback, and 404 for unknown paths", async () => {
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
    expect((await hit("/downloads/")).text).toContain("home");
    expect((await hit("/downloads")).text).toContain("home");
    expect((await hit("/downloads/missing")).text).toContain("home");
    expect((await hit("/downloads/Bolo_0.1.11_x64-setup.exe")).text).toBe("installer");
    expect((await hit("/.well-known/")).text).toContain("home");
    expect((await hit("/.well-known")).text).toContain("home");
    expect((await hit("/empty-dir/")).text).toContain("home");
    expect((await hit("/urdu-voice-typing-for-pc/")).text).toContain("guide");
    expect((await hit("/contact/")).status).toBe(404);
    expect((await hit("/contact")).status).toBe(404);
    expect((await hit("/blog")).status).toBe(404);
    expect((await hit("/foo/bar")).status).toBe(404);
    expect((await hit("/missing-page")).status).toBe(404);
    expect((await hit("/no.css")).status).toBe(404);
  });

  it("does not proxy contact when the loop header is set", async () => {
    let ended = false;
    const res = { end() { ended = true; }, setHeader() {}, statusCode: 0 } as unknown as import("node:http").ServerResponse;
    const req = { headers: { "x-bolo-contact-proxy": "1" }, url: "/contact/", method: "GET" } as unknown as import("node:http").IncomingMessage;
    expect(await boloProxyContactPhp(req, res)).toBe(false);
    expect(ended).toBe(false);
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

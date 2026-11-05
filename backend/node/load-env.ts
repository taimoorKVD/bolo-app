import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * Load .env from cwd or app root into process.env without overwriting existing vars.
 * Uses Node 20.12+/22 process.loadEnvFile when available.
 */
export function loadDotEnv(appRoot = process.cwd()): string | null {
  const candidates = [join(appRoot, ".env"), join(process.cwd(), ".env")];
  for (const p of [...new Set(candidates)]) {
    if (!existsSync(p)) continue;
    const load = (process as NodeJS.Process & { loadEnvFile?: (path?: string) => void }).loadEnvFile;
    if (typeof load === "function") {
      load(p);
      return p;
    }
    for (const line of readFileSync(p, "utf8").split(/\r?\n/)) {
      const t = line.trim();
      if (!t || t.startsWith("#")) continue;
      const i = t.indexOf("=");
      if (i <= 0) continue;
      const k = t.slice(0, i).trim();
      let v = t.slice(i + 1).trim();
      if ((v.startsWith('"') && v.endsWith('"')) || (v.startsWith("'") && v.endsWith("'"))) v = v.slice(1, -1);
      if (process.env[k] === undefined) process.env[k] = v;
    }
    return p;
  }
  return null;
}

import { readFileSync } from "node:fs";
import * as esbuild from "esbuild";

// Passenger starts `node dist/server.mjs`. Keep node_modules external so the
// server install (nodemailer and the rest) stays the one next to the bundle.
await esbuild.build({
  entryPoints: ["node/server.ts"],
  bundle: true,
  platform: "node",
  format: "esm",
  outfile: "dist/server.mjs",
  packages: "external",
});

const out = readFileSync("dist/server.mjs", "utf8");
const oldFallback = /if\s*\(\s*!extname\(\s*\w+\s*\)\s*\|\|\s*\w+\.endsWith\(\s*["']\/["']\s*\)\s*\)\s*\w+\s*=\s*\w+\(\s*\w+\s*,\s*["']\/index\.html["']\s*\)/;
if (!out.includes("boloKeep") || !out.includes("async function boloProxyContactPhp")) {
  console.error("dist/server.mjs is missing the route markers the website deploy looks for");
  process.exit(1);
}
if (oldFallback.test(out)) {
  console.error("dist/server.mjs still falls back to the homepage for unknown paths");
  process.exit(1);
}
console.log("dist/server.mjs");

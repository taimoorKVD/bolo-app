import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import * as esbuild from "esbuild";

// Compile Office CSS with the Tailwind CLI (no CDN script) and inline it.
const tw = spawnSync(
  process.execPath,
  ["node_modules/@tailwindcss/cli/dist/index.mjs", "-i", "office/input.css", "-o", "office/.generated.css", "--minify"],
  { stdio: "inherit" },
);
if (tw.status !== 0) process.exit(tw.status ?? 1);
const officeCss = readFileSync("office/.generated.css", "utf8");
if (officeCss.includes("cdn.tailwindcss.com")) {
  console.error("Office CSS must not load the Tailwind CDN");
  process.exit(1);
}
writeFileSync("office/office-css.ts", `export const OFFICE_CSS = ${JSON.stringify(officeCss)};\n`);

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

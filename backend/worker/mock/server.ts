import { createServer } from "node:http";
import { createLocalStack } from "../src/app";

const port = Number(process.env.PORT || 8787);
const { handle, backend, ready } = createLocalStack(undefined, { seed: process.env.BOLO_SEED !== "0" });

const server = createServer(async (req, res) => {
  await ready;
  const url = `http://127.0.0.1:${port}${req.url || "/"}`;
  const chunks: Buffer[] = [];
  for await (const chunk of req) chunks.push(Buffer.from(chunk));
  const body = Buffer.concat(chunks);
  const request = new Request(url, {
    method: req.method,
    headers: req.headers as HeadersInit,
    body: ["GET", "HEAD"].includes(req.method || "GET") ? undefined : body,
  });
  const response = await handle(request);
  res.statusCode = response.status;
  response.headers.forEach((value, key) => res.setHeader(key, value));
  res.end(Buffer.from(await response.arrayBuffer()));
});

server.listen(port, "127.0.0.1", () => {
  console.log(`Bolo mock API on http://127.0.0.1:${port}`);
  console.log(`Admin portal (EXAMPLE data, mock auth) on http://127.0.0.1:${port}/admin`);
  console.log("Kill switch:", backend.flags.killSwitch, "| OpenRouter:", backend.flags.openRouterEnabled);
});

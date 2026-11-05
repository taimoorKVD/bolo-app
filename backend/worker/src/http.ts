import type { ApiErrorBody } from "./types";

export function json(data: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: {
      "content-type": "application/json; charset=utf-8",
      "cache-control": "no-store",
      ...headers,
    },
  });
}

export function error(status: number, code: string, message: string, extra: Partial<ApiErrorBody> = {}): Response {
  const body: ApiErrorBody = { error: message, code, ...extra };
  return json(body, status);
}

export function cors(req: Request, res: Response): Response {
  const headers = new Headers(res.headers);
  headers.set("access-control-allow-origin", "*");
  headers.set("access-control-allow-headers", "authorization, content-type, x-bolo-timestamp, x-bolo-signature");
  headers.set("access-control-allow-methods", "GET, POST, OPTIONS");
  if (req.method === "OPTIONS") {
    return new Response(null, { status: 204, headers });
  }
  return new Response(res.body, { status: res.status, headers });
}

export function bearer(req: Request): string | null {
  const h = req.headers.get("authorization");
  if (!h) return null;
  const m = /^Bearer\s+(.+)$/i.exec(h.trim());
  return m ? m[1] : null;
}

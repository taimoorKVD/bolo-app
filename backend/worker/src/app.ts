import { MockAiGateway, WorkersAiGateway, sanitizeTranscribeOptions, type AiGateway, type TranscribeOptions } from "./ai";
import { bearer, cors, error, json } from "./http";
import type { WorkerEnv } from "./types";
import { MockBackend } from "../mock/store";
import { AdminService } from "./admin";
import { handleAdmin } from "./admin-routes";
import { seedExampleData } from "../mock/seed";

/**
 * Shared request handler used by wrangler (with real AI binding) and the
 * local mock server / vitest harness (in-memory Supabase stand-in).
 */
export function createApp(backend: MockBackend, ai: AiGateway, env: WorkerEnv, admin: AdminService = new AdminService(backend)) {
  async function poolFullFor(access: string): Promise<boolean> {
    if (admin.poolFullIds.size === 0) return false;
    const auth = await backend.authenticateAccess(access);
    return auth.ok && admin.poolFullIds.has(admin.poolFor(auth.install.id));
  }

  return async function handle(req: Request): Promise<Response> {
    if (req.method === "OPTIONS") {
      return cors(req, new Response(null, { status: 204 }));
    }

    const url = new URL(req.url);
    const path = url.pathname.replace(/\/+$/, "") || "/";

    try {
      if (req.method === "GET" && (path === "/" || path === "/health" || path === "/v1/health")) {
        return cors(req, json({ ok: true, service: "bolo-api", phase: 4 }));
      }

      if (path === "/admin" || path.startsWith("/admin/")) {
        const res = await handleAdmin(req, path, admin);
        // Admin API is same-origin only: no CORS headers.
        if (res) return res;
      }

      if (req.method === "GET" && path === "/v1/status") {
        const status = (await backend.statusFor(bearer(req))) as Record<string, unknown>;
        const installId = typeof status.installId === "string" ? status.installId : undefined;
        const userId = installId ? backend.installs.get(installId)?.userId : undefined;
        const code = status.code as string | undefined;
        return cors(
          req,
          json({
            ...status,
            reconnectRequired: Boolean(status.reconnectRequired) || code === "reissued",
            signInRequired: code === "revoked" || code === "refresh_reuse" || code === "suspended",
            ...admin.statusExtras(installId, userId),
          }),
        );
      }

      if (req.method === "POST" && path === "/v1/auth/request-otp") {
        const body = (await req.json()) as { email?: string };
        const result = await backend.requestOtp(body.email ?? "");
        if (!result.ok) return cors(req, error(400, result.code, "Could not start sign-in."));
        // Never return the OTP code in API responses.
        return cors(req, json({ ok: true }));
      }

      if (req.method === "POST" && path === "/v1/auth/verify-otp") {
        const body = (await req.json()) as { email?: string; code?: string };
        const result = await backend.verifyOtp(body.email ?? "", body.code ?? "");
        if (!result.ok) return cors(req, error(401, result.code, "That code did not work."));
        return cors(
          req,
          json({
            ok: true,
            userId: result.profile.userId,
            email: result.profile.email,
            plan: result.profile.plan,
            // Single-use, 10 minutes, bound to this user; required to add a new device.
            registrationTicket: await backend.issueRegistrationTicket(result.profile.userId),
          }),
        );
      }

      if (req.method === "POST" && path === "/v1/installs/register") {
        const body = (await req.json()) as {
          userId?: string;
          deviceName?: string;
          appVersion?: string;
          publicKey?: string;
          registrationTicket?: string;
          signature?: string;
          signedAtMs?: number;
        };
        if (!body.userId || !body.publicKey) {
          return cors(req, error(400, "invalid_body", "userId and publicKey are required."));
        }
        const result = await backend.registerInstall({
          userId: body.userId,
          deviceName: String(body.deviceName ?? "Windows PC").slice(0, 80),
          appVersion: body.appVersion,
          publicKey: body.publicKey,
          registrationTicket: typeof body.registrationTicket === "string" ? body.registrationTicket : undefined,
          signature: typeof body.signature === "string" ? body.signature : undefined,
          signedAtMs: typeof body.signedAtMs === "number" ? body.signedAtMs : undefined,
        });
        if (!result.ok) {
          const proof = result.code.startsWith("device_proof") || result.code.startsWith("registration_ticket");
          return cors(req, error(proof ? 401 : 403, result.code, "Could not register this PC."));
        }
        return cors(
          req,
          json({
            ok: true,
            installId: result.install.id,
            session: result.session,
          }),
        );
      }

      if (req.method === "POST" && path === "/v1/auth/refresh") {
        const body = (await req.json()) as { refreshToken?: string };
        const result = await backend.refresh(body.refreshToken ?? "");
        if (!result.ok) {
          const status = result.code === "refresh_reuse" ? 401 : 401;
          return cors(req, error(status, result.code, "Please sign in again."));
        }
        return cors(req, json({ ok: true, session: result.session }));
      }

      if (req.method === "POST" && path === "/v1/dictation/transcribe") {
        const access = bearer(req);
        if (!access) return cors(req, error(401, "missing_token", "Sign in to Bolo first."));
        const contentType = req.headers.get("content-type") ?? "";
        let audioMinutes = 0.1;
        let audio: ArrayBuffer | null = null;
        let opts: TranscribeOptions = { task: "transcribe" };
        if (contentType.includes("application/json")) {
          const body = (await req.json()) as { audioMinutes?: number; audioBase64?: string; task?: string; language?: string; prompt?: string };
          opts = sanitizeTranscribeOptions(body);
          audioMinutes = Number(body.audioMinutes ?? 0.1);
          if (body.audioBase64) {
            const bin = atob(body.audioBase64);
            const bytes = new Uint8Array(bin.length);
            for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
            audio = bytes.buffer;
          }
        } else {
          audio = await req.arrayBuffer();
          // Rough estimate: 16-bit mono 16kHz PCM minutes if raw; JSON clients should send audioMinutes.
          const declared = Number(req.headers.get("x-bolo-audio-minutes") ?? "");
          audioMinutes = Number.isFinite(declared) && declared > 0 ? declared : Math.max(0.01, audio.byteLength / (16000 * 2 * 60));
        }
        if (await poolFullFor(access)) {
          return cors(req, error(429, "pool_full", "Bolo's free pool is busy. Using your backup for now.", { retryAfterMs: 15 * 60 * 1000 }));
        }
        // Cap check before calling AI.
        const gated = await backend.dictate({
          accessToken: access,
          kind: "transcribe",
          audioMinutes,
        });
        if (!gated.ok) {
          const status =
            gated.code === "kill_switch" || gated.code === "pool_full" || gated.code.startsWith("fair_use")
              ? 429
              : gated.code === "reissued" || gated.code === "revoked"
                ? 401
                : 403;
          return cors(req, error(status, gated.code, "Managed dictation is unavailable right now."));
        }
        // For the mock path, dictate() already produced text. When a real AI
        // binding exists and audio was provided, prefer the model output.
        let text = gated.text;
        let language: string | undefined = opts.language;
        let avgLogprob: number | undefined;
        let noSpeechProb: number | undefined;
        if (audio) {
          const out = await ai.transcribe(audio, env.WHISPER_MODEL || "@cf/openai/whisper-large-v3-turbo", opts);
          text = out.text;
          language = out.language ?? language;
          avgLogprob = out.avgLogprob;
          noSpeechProb = out.noSpeechProb;
        }
        return cors(
          req,
          json({
            ok: true,
            text,
            language,
            avgLogprob,
            noSpeechProb,
            provider: "workers_ai",
            remainingDailyMinutes: gated.remainingDailyMinutes,
          }),
        );
      }

      if (req.method === "POST" && path === "/v1/dictation/cleanup") {
        const access = bearer(req);
        if (!access) return cors(req, error(401, "missing_token", "Sign in to Bolo first."));
        const body = (await req.json()) as { text?: string; audioMinutes?: number; mode?: string; tone?: string };
        if (await poolFullFor(access)) {
          return cors(req, error(429, "pool_full", "Bolo's free pool is busy. Using your backup for now.", { retryAfterMs: 15 * 60 * 1000 }));
        }
        const gated = await backend.dictate({
          accessToken: access,
          kind: "cleanup",
          audioMinutes: Number(body.audioMinutes ?? 0),
        });
        if (!gated.ok) {
          const status =
            gated.code === "kill_switch" || gated.code === "pool_full" || gated.code.startsWith("fair_use")
              ? 429
              : 401;
          return cors(req, error(status, gated.code, "Managed cleanup is unavailable right now."));
        }
        const out = await ai.cleanup(body.text ?? "", env.CLEANUP_MODEL || "@cf/openai/gpt-oss-20b", {
          mode: body.mode === "translate" ? "translate" : "cleanup",
          tone: typeof body.tone === "string" ? body.tone.slice(0, 16) : undefined,
        });
        return cors(
          req,
          json({
            ok: true,
            text: out.text,
            provider: "workers_ai",
            remainingDailyMinutes: gated.remainingDailyMinutes,
          }),
        );
      }

      return cors(req, error(404, "not_found", "Unknown endpoint."));
    } catch (err) {
      const message = err instanceof Error ? err.message : "unknown";
      return cors(req, error(500, "internal", message));
    }
  };
}

export function createMockEnv(overrides: Partial<WorkerEnv> = {}): WorkerEnv {
  return {
    OPENROUTER_ENABLED: "false",
    PAID_USAGE_ENABLED: "false",
    KILL_SWITCH: "false",
    FAIR_USE_DAILY_MINUTES: "20",
    FAIR_USE_MONTHLY_MINUTES: "300",
    FAIR_USE_NEW_INSTALL_DAILY_MINUTES: "10",
    MAX_INSTALLS_PER_USER: "3",
    ACCESS_TOKEN_TTL_SECONDS: "900",
    GLOBAL_DAILY_AUDIO_MINUTES: "180",
    WHISPER_MODEL: "@cf/openai/whisper-large-v3-turbo",
    CLEANUP_MODEL: "@cf/openai/gpt-oss-20b",
    // Local mock convenience only. The Worker entry forces proof on in production.
    REQUIRE_DEVICE_PROOF: "false",
    ...overrides,
  };
}

export function createLocalStack(env: WorkerEnv = createMockEnv(), opts: { seed?: boolean } = {}) {
  const backend = new MockBackend(env);
  const admin = new AdminService(backend, "mock");
  const ai = env.AI ? new WorkersAiGateway(env.AI) : new MockAiGateway();
  const ready = opts.seed ? seedExampleData(backend, admin) : Promise.resolve();
  return { backend, admin, ai, handle: createApp(backend, ai, env, admin), env, ready };
}

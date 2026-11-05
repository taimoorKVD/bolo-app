import { describe, expect, it } from "vitest";
import { createApp, createMockEnv } from "../src/app";
import { MockBackend } from "../mock/store";
import { AdminService } from "../src/admin";
import { MockAiGateway, cleanupSystemPrompt, sanitizeTranscribeOptions } from "../src/ai";

async function stack() {
  const env = createMockEnv();
  const backend = new MockBackend(env);
  const ai = new MockAiGateway();
  const handle = createApp(backend, ai, env, new AdminService(backend, "mock"));
  await backend.requestOtp("s@example.com");
  const v = await backend.verifyOtp("s@example.com", backend.lastOtpByEmail.get("s@example.com")!);
  if (!v.ok) throw new Error("otp");
  const reg = await backend.registerInstall({ userId: v.profile.userId, deviceName: "PC", publicKey: "pk-speech" });
  if (!reg.ok) throw new Error("reg");
  return { handle, ai, token: reg.session.accessToken };
}

const post = (path: string, body: unknown, token: string) =>
  new Request(`http://local${path}`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
    body: JSON.stringify(body),
  });

describe("managed speech parity", () => {
  it("forwards vocabulary prompt, pinned language and task, and returns detected language + confidence", async () => {
    const { handle, ai, token } = await stack();
    const prompt = "Bolo, Supabase, Taimoor";
    const res = await handle(post("/v1/dictation/transcribe", { audioMinutes: 0.1, audioBase64: btoa("x"), language: "ur", prompt }, token));
    expect(res.status).toBe(200);
    const body = (await res.json()) as { language: string; avgLogprob: number; noSpeechProb: number };
    expect(ai.lastTranscribe).toEqual({ task: "transcribe", language: "ur", prompt });
    expect(body.language).toBe("ur");
    expect(typeof body.avgLogprob).toBe("number");
    expect(typeof body.noSpeechProb).toBe("number");

    await handle(post("/v1/dictation/transcribe", { audioMinutes: 0.1, audioBase64: btoa("x"), task: "translate" }, token));
    expect(ai.lastTranscribe?.task).toBe("translate");
    expect(ai.lastTranscribe?.language).toBeUndefined();
  });

  it("sanitizes options", () => {
    expect(sanitizeTranscribeOptions({ language: "auto", task: "evil", prompt: 5 })).toEqual({ task: "transcribe" });
    expect(sanitizeTranscribeOptions({ language: "EN" }).language).toBe("en");
    expect(sanitizeTranscribeOptions({ prompt: "a".repeat(5000) }).prompt!.length).toBe(1200);
  });

  it("cleanup supports translate mode and tone", async () => {
    const { handle, ai, token } = await stack();
    const res = await handle(post("/v1/dictation/cleanup", { text: "kal milte hain", mode: "translate" }, token));
    expect(res.status).toBe(200);
    expect(ai.lastCleanup?.mode).toBe("translate");
    expect(cleanupSystemPrompt({ mode: "translate" })).toMatch(/Translate/);
    expect(cleanupSystemPrompt({ tone: "formal" })).toMatch(/professional/);
  });
});

describe("health", () => {
  it("answers on /health and /v1/health", async () => {
    const env = createMockEnv();
    const backend = new MockBackend(env);
    const handle = createApp(backend, new MockAiGateway(), env, new AdminService(backend, "mock"));
    for (const path of ["/health", "/v1/health"]) {
      const res = await handle(new Request(`http://local${path}`));
      expect(res.status).toBe(200);
      expect(((await res.json()) as { ok: boolean }).ok).toBe(true);
    }
  });
});

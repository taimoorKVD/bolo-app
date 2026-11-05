/**
 * Workers AI adapters. In production the Worker binds `env.AI`.
 * Local mock harness never calls Cloudflare; tests inject a stub.
 * Audio and text pass straight to the model binding and are never logged.
 */

export interface TranscribeOptions {
  /** "translate" turns speech into English (Whisper task). */
  task?: "transcribe" | "translate";
  /** ISO code ("en", "ur"); omitted means auto-detect. */
  language?: string;
  /** Custom vocabulary / Urdu-script prompt from the desktop, same as the Groq path. */
  prompt?: string;
}

export interface TranscribeResult {
  text: string;
  provider: "workers_ai";
  /** Detected (or pinned) language, so the desktop can run its Urdu/translate routing. */
  language?: string;
  avgLogprob?: number;
  noSpeechProb?: number;
}

export interface CleanupOptions {
  mode?: "cleanup" | "translate";
  tone?: string;
}

export interface CleanupResult {
  text: string;
  provider: "workers_ai";
}

export interface AiGateway {
  transcribe(audioBytes: ArrayBuffer, model: string, opts?: TranscribeOptions): Promise<TranscribeResult>;
  cleanup(text: string, model: string, opts?: CleanupOptions): Promise<CleanupResult>;
}

const LANG_RE = /^[a-z]{2,3}$/;

/** Accept only well-formed options from clients. */
export function sanitizeTranscribeOptions(body: { task?: unknown; language?: unknown; prompt?: unknown }): TranscribeOptions {
  const out: TranscribeOptions = {};
  if (body.task === "translate") out.task = "translate";
  else out.task = "transcribe";
  const lang = typeof body.language === "string" ? body.language.trim().toLowerCase() : "";
  if (LANG_RE.test(lang)) out.language = lang;
  if (typeof body.prompt === "string" && body.prompt.trim()) out.prompt = body.prompt.slice(0, 1200);
  return out;
}

const TONE_HINT: Record<string, string> = {
  casual: "Keep a casual chat tone.",
  formal: "Use a clear, professional tone for mail or documents.",
  plain: "Keep the speaker's tone.",
};

export function cleanupSystemPrompt(opts: CleanupOptions = {}): string {
  if (opts.mode === "translate") {
    return "Translate the dictated text into natural English. Keep names, numbers and technical terms exactly. Do not add or drop content. Return only the translation.";
  }
  const tone = TONE_HINT[opts.tone ?? ""] ?? "";
  return `Clean up dictated text. Fix punctuation and remove filler words. Do not invent content. ${tone} Return only the cleaned text.`.replace(/\s+/g, " ").trim();
}

function toBase64(bytes: Uint8Array): string {
  let bin = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) bin += String.fromCharCode(...bytes.subarray(i, i + chunk));
  return btoa(bin);
}

export class WorkersAiGateway implements AiGateway {
  constructor(private ai: Ai) {}

  async transcribe(audioBytes: ArrayBuffer, model: string, opts: TranscribeOptions = {}): Promise<TranscribeResult> {
    const input: Record<string, unknown> = { audio: toBase64(new Uint8Array(audioBytes)), task: opts.task ?? "transcribe" };
    if (opts.language) input.language = opts.language;
    if (opts.prompt) input.initial_prompt = opts.prompt;
    const result = (await this.ai.run(model as keyof AiModels, input as never)) as {
      text?: string;
      transcription_info?: { language?: string };
      segments?: { avg_logprob?: number; no_speech_prob?: number }[];
    };
    const segs = result.segments ?? [];
    const avg = (pick: (s: { avg_logprob?: number; no_speech_prob?: number }) => number | undefined) => {
      const vals = segs.map(pick).filter((v): v is number => typeof v === "number");
      return vals.length ? vals.reduce((a, b) => a + b, 0) / vals.length : undefined;
    };
    return {
      text: (result.text ?? "").trim(),
      provider: "workers_ai",
      language: result.transcription_info?.language ?? opts.language,
      avgLogprob: avg((s) => s.avg_logprob),
      noSpeechProb: avg((s) => s.no_speech_prob),
    };
  }

  async cleanup(text: string, model: string, opts: CleanupOptions = {}): Promise<CleanupResult> {
    const result = (await this.ai.run(model as keyof AiModels, {
      messages: [
        { role: "system", content: cleanupSystemPrompt(opts) },
        { role: "user", content: text },
      ],
    } as never)) as { response?: string; text?: string };
    const out = (result.response ?? result.text ?? text).trim();
    return { text: out, provider: "workers_ai" };
  }
}

export class MockAiGateway implements AiGateway {
  /** Last options seen, so tests can prove vocabulary/language pass through. */
  lastTranscribe: TranscribeOptions | null = null;
  lastCleanup: CleanupOptions | null = null;
  async transcribe(_audio?: ArrayBuffer, _model?: string, opts: TranscribeOptions = {}): Promise<TranscribeResult> {
    this.lastTranscribe = opts;
    if (opts.task === "translate") return { text: "hello in english", provider: "workers_ai", language: opts.language ?? "en" };
    return { text: "hello from bolo managed whisper", provider: "workers_ai", language: opts.language ?? "en", avgLogprob: -0.2, noSpeechProb: 0.01 };
  }
  async cleanup(text: string, _model?: string, opts: CleanupOptions = {}): Promise<CleanupResult> {
    this.lastCleanup = opts;
    return { text: text.trim() || "Hello from Bolo managed cleanup.", provider: "workers_ai" };
  }
}

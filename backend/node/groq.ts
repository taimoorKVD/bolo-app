import type { AiGateway, CleanupOptions, CleanupResult, TranscribeOptions, TranscribeResult } from "../worker/src/ai";
import { cleanupSystemPrompt } from "../worker/src/ai";

/** Groq adapter for the self-hosted service. Audio/text are forwarded and never logged or stored. */
export class GroqGateway implements AiGateway {
  constructor(private keys: string[], private base = "https://api.groq.com/openai/v1") {
    if (!keys.length) throw new Error("GROQ_API_KEYS is empty");
  }
  /** When set, Office key failover chooses the secret. Falls back to the env list. */
  keyPicker?: () => string | null;
  private i = 0;
  private key() { return this.keyPicker?.() || this.keys[this.i++ % this.keys.length]; }

  async transcribe(audio: ArrayBuffer, model: string, opts: TranscribeOptions = {}): Promise<TranscribeResult> {
    const form = new FormData();
    form.append("file", new Blob([audio], { type: "audio/wav" }), "audio.wav");
    form.append("model", model.startsWith("@cf/") ? "whisper-large-v3" : model);
    form.append("response_format", "verbose_json");
    if (opts.language && opts.task !== "translate") form.append("language", opts.language);
    if (opts.prompt) form.append("prompt", opts.prompt.slice(0, 896));
    const url = `${this.base}/audio/${opts.task === "translate" ? "translations" : "transcriptions"}`;
    const r = await fetch(url, { method: "POST", headers: { authorization: `Bearer ${this.key()}` }, body: form, signal: AbortSignal.timeout(60_000) });
    if (!r.ok) throw Object.assign(new Error(`groq_${r.status}`), { status: r.status });
    const j = (await r.json()) as { text?: string; language?: string; segments?: { avg_logprob?: number; no_speech_prob?: number }[] };
    const segs = j.segments ?? [];
    const avg = (f: (s: (typeof segs)[number]) => number | undefined) => {
      const v = segs.map(f).filter((x): x is number => typeof x === "number");
      return v.length ? v.reduce((a, b) => a + b, 0) / v.length : undefined;
    };
    return { text: (j.text ?? "").trim(), provider: "workers_ai", language: normLang(j.language) ?? opts.language, avgLogprob: avg((s) => s.avg_logprob), noSpeechProb: avg((s) => s.no_speech_prob) };
  }

  async cleanup(text: string, model: string, opts: CleanupOptions = {}): Promise<CleanupResult> {
    const r = await fetch(`${this.base}/chat/completions`, {
      method: "POST",
      headers: { authorization: `Bearer ${this.key()}`, "content-type": "application/json" },
      body: JSON.stringify({ model: model.startsWith("@cf/") ? "llama-3.3-70b-versatile" : model, temperature: 0.2, messages: [{ role: "system", content: cleanupSystemPrompt(opts) }, { role: "user", content: text }] }),
      signal: AbortSignal.timeout(60_000),
    });
    if (!r.ok) throw Object.assign(new Error(`groq_${r.status}`), { status: r.status });
    const j = (await r.json()) as { choices?: { message?: { content?: string } }[] };
    return { text: (j.choices?.[0]?.message?.content ?? text).trim(), provider: "workers_ai" };
  }
}

const NAMES: Record<string, string> = { english: "en", urdu: "ur", hindi: "hi", arabic: "ar", punjabi: "pa" };
function normLang(l?: string) { if (!l) return undefined; const k = l.toLowerCase(); return NAMES[k] ?? (k.length <= 3 ? k : undefined); }

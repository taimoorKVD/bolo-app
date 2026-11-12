import { sha256Hex } from "./crypto";

/** Unset OFFICE_CODE_REDIRECT keeps the current owner-code delivery address. */
export const DEFAULT_OFFICE_CODE_REDIRECT = "info@kingdomvision.com=taimoor.kingdomvision@gmail.com";

export function codeRecipient(accountEmail: string, spec?: string | null): string {
  const raw = (spec == null || spec.trim() === "" ? DEFAULT_OFFICE_CODE_REDIRECT : spec).trim();
  const email = accountEmail.trim().toLowerCase();
  for (const part of raw.split(";")) {
    const i = part.indexOf("=");
    if (i <= 0) continue;
    const from = part.slice(0, i).trim().toLowerCase();
    const to = part.slice(i + 1).trim();
    if (from === email && to.includes("@") && !/[\r\n]/.test(to)) return to;
  }
  return email;
}

export type SpeechKind = "transcribe" | "cleanup";
export type SpeechOutcome = "ok" | "rejected_key" | "rate_limit" | "unavailable" | "offline_fallback";

export interface SpeechSample {
  at: number;
  kind: SpeechKind;
  outcome: SpeechOutcome;
  latencyMs?: number;
  version?: string;
}

export interface PoolKeyRecord {
  id: string;
  poolId: string;
  fingerprint: string;
  secret: string;
  status: "active" | "backup" | "retired";
  dailyMinuteCap: number;
  minutesToday: number;
  day: string;
}

export interface DeviceMeta { installId: string; engine?: "online" | "on_device"; blocked?: boolean }

export interface ErrorReport {
  id: string;
  at: number;
  kind: string;
  version: string;
  line: string;
  deviceId: string;
  fixedIn?: string;
  assignee?: string;
  note?: string;
}

export interface FeedbackItem {
  id: string;
  at: number;
  source: "app" | "website";
  issueType: string;
  text: string;
  deviceId?: string;
  resolved?: boolean;
  assignee?: string;
  note?: string;
}

export interface OfficeSettings {
  retentionDays: number;
  privacyPolicyVersion: string;
  requireEmail2fa: boolean;
  ipAllowlist: string[];
  /** Stored updater document. Served only when paused and this object was saved. */
  pinnedLatest: Record<string, unknown> | null;
  paused: boolean;
  channels: { website: string; updater: string; store: string };
  announcement: string;
}

export interface OfficeState {
  speech: SpeechSample[];
  keys: PoolKeyRecord[];
  limits: { freeMinutesPerDay: number; softWarnRatio: number };
  devices: DeviceMeta[];
  errors: ErrorReport[];
  feedback: FeedbackItem[];
  settings: OfficeSettings;
  alertedAt: number;
}

const ISSUE = new Set(["accuracy", "bug", "billing", "other", "contact"]);
const ONLINE_FAIL = new Set<SpeechOutcome>(["rejected_key", "rate_limit", "unavailable"]);

export function emptyOfficeState(): OfficeState {
  return {
    speech: [],
    keys: [],
    limits: { freeMinutesPerDay: 20, softWarnRatio: 0.8 },
    devices: [],
    errors: [],
    feedback: [],
    settings: {
      retentionDays: 40,
      privacyPolicyVersion: "",
      requireEmail2fa: false,
      ipAllowlist: [],
      pinnedLatest: null,
      paused: false,
      channels: { website: "", updater: "", store: "" },
      announcement: "",
    },
    alertedAt: 0,
  };
}

export function sanitizeReportLine(line: string): string | null {
  if (typeof line !== "string") return null;
  if (/data:audio|transcript|BEGIN CERT/i.test(line)) return null;
  const cleaned = line.replace(/gsk_[A-Za-z0-9_-]+/g, "[key]").replace(/\bsk-[A-Za-z0-9_-]+/g, "[key]").trim();
  if (!cleaned) return null;
  return cleaned.slice(0, 240);
}

function dayKey(now: number) { return new Date(now).toISOString().slice(0, 10); }

function percentile(values: number[], p: number): number | null {
  if (!values.length) return null;
  const s = [...values].sort((a, b) => a - b);
  const idx = Math.min(s.length - 1, Math.max(0, Math.ceil((p / 100) * s.length) - 1));
  return s[idx];
}

export class OfficeOps {
  state: OfficeState = emptyOfficeState();
  constructor(private now: () => number = () => Date.now()) {}
  setClock(fn: () => number) { this.now = fn; }

  exportState(): OfficeState { return structuredClone(this.state); }
  importState(raw: OfficeState | null | undefined) {
    if (!raw || !Array.isArray(raw.speech) || !raw.settings) return;
    this.state = raw;
    this.state.keys ||= [];
    this.state.devices ||= [];
    this.state.errors ||= [];
    this.state.feedback ||= [];
    this.state.limits ||= { freeMinutesPerDay: 20, softWarnRatio: 0.8 };
    this.state.settings.announcement ??= "";
    this.purge();
  }

  private purge() {
    const cut = this.now() - this.state.settings.retentionDays * 864e5;
    this.state.speech = this.state.speech.filter((s) => s.at >= cut);
    this.state.errors = this.state.errors.filter((e) => e.at >= cut);
    this.state.feedback = this.state.feedback.filter((f) => f.at >= cut);
  }

  recordSpeech(sample: Omit<SpeechSample, "at"> & { at?: number }) {
    this.purge();
    const row: SpeechSample = {
      at: sample.at ?? this.now(),
      kind: sample.kind,
      outcome: sample.outcome,
      latencyMs: typeof sample.latencyMs === "number" && sample.latencyMs >= 0 ? Math.round(sample.latencyMs) : undefined,
      version: sample.version ? String(sample.version).slice(0, 32) : undefined,
    };
    this.state.speech.push(row);
    return row;
  }

  /** True when a fresh alert should be emailed. One alert per hour. */
  consumeAlert(): boolean {
    const h = this.health();
    if (!h.alert) return false;
    if (this.now() - this.state.alertedAt < 60 * 60 * 1000) return false;
    this.state.alertedAt = this.now();
    return true;
  }

  health() {
    this.purge();
    const since = this.now() - 60 * 60 * 1000;
    const rows = this.state.speech.filter((s) => s.at >= since);
    const byKind: Record<string, { ok: number; fail: number }> = {};
    const byVersion: Record<string, number> = {};
    const lat: number[] = [];
    let online = 0;
    let onlineFail = 0;
    for (const s of rows) {
      const slot = byKind[s.kind] ||= { ok: 0, fail: 0 };
      if (s.outcome === "ok") slot.ok += 1;
      else slot.fail += 1;
      if (s.version) byVersion[s.version] = (byVersion[s.version] ?? 0) + 1;
      if (s.outcome !== "offline_fallback") {
        online += 1;
        if (ONLINE_FAIL.has(s.outcome)) onlineFail += 1;
      }
      if (s.outcome === "ok" && typeof s.latencyMs === "number") lat.push(s.latencyMs);
    }
    const rate = online ? onlineFail / online : 0;
    return {
      samples: rows.length,
      byKind,
      byVersion,
      online,
      onlineFail,
      failureRate: online ? Math.round(rate * 1000) / 1000 : null,
      p50: percentile(lat, 50),
      p95: percentile(lat, 95),
      alert: online >= 10 && rate >= 0.5 && this.now() >= this.state.alertedAt,
    };
  }

  async ensureEnvKey(poolId: string, secret: string) {
    const fingerprint = (await sha256Hex(secret)).slice(0, 12);
    if (this.state.keys.some((k) => k.fingerprint === fingerprint && k.status !== "retired")) return fingerprint;
    const active = this.state.keys.some((k) => k.poolId === poolId && k.status === "active");
    this.state.keys.push({
      id: "key_" + fingerprint,
      poolId,
      fingerprint,
      secret,
      status: active ? "backup" : "active",
      dailyMinuteCap: 0,
      minutesToday: 0,
      day: dayKey(this.now()),
    });
    return fingerprint;
  }

  async addKey(poolId: string, secret: string, opts: { backup?: boolean; dailyMinuteCap?: number } = {}) {
    if (!secret || secret.length < 8) return null;
    const fingerprint = (await sha256Hex(secret)).slice(0, 12);
    const id = "key_" + fingerprint + "_" + this.state.keys.length;
    this.state.keys.push({
      id,
      poolId: poolId.slice(0, 64),
      fingerprint,
      secret,
      status: opts.backup ? "backup" : "active",
      dailyMinuteCap: Math.max(0, Number(opts.dailyMinuteCap) || 0),
      minutesToday: 0,
      day: dayKey(this.now()),
    });
    return this.publicKey(this.state.keys[this.state.keys.length - 1]);
  }

  publicKey(k: PoolKeyRecord) {
    return {
      id: k.id, poolId: k.poolId, fingerprint: k.fingerprint, status: k.status,
      dailyMinuteCap: k.dailyMinuteCap, minutesToday: this.minutesToday(k),
    };
  }

  private minutesToday(k: PoolKeyRecord) {
    return k.day === dayKey(this.now()) ? k.minutesToday : 0;
  }

  listKeys() { return this.state.keys.filter((k) => k.status !== "retired").map((k) => this.publicKey(k)); }

  retireKey(id: string) {
    return Boolean(this.setKeyMode(id, "retired"));
  }

  /** primary makes this key the live one and demotes other live keys in the pool. */
  setKeyMode(id: string, mode: "active" | "backup" | "retired" | "primary") {
    const k = this.state.keys.find((x) => x.id === id);
    if (!k) return null;
    if (mode === "retired") { k.status = "retired"; k.secret = ""; return this.publicKey(k); }
    if (mode === "primary") {
      for (const o of this.state.keys) if (o.poolId === k.poolId && o.id !== k.id && o.status === "active") o.status = "backup";
      k.status = "active";
      return this.publicKey(k);
    }
    k.status = mode;
    return this.publicKey(k);
  }

  async rotateKey(id: string, secret: string) {
    const k = this.state.keys.find((x) => x.id === id && x.status !== "retired");
    if (!k || !secret) return null;
    k.secret = secret;
    k.fingerprint = (await sha256Hex(secret)).slice(0, 12);
    k.minutesToday = 0;
    k.day = dayKey(this.now());
    return this.publicKey(k);
  }

  /** Secret for the live call. Switches to a backup when the active key is over its cap. */
  selectSecret(): string | null {
    const day = dayKey(this.now());
    const usable = (k: PoolKeyRecord) => k.status !== "retired" && k.secret && (k.dailyMinuteCap <= 0 || (k.day === day ? k.minutesToday : 0) < k.dailyMinuteCap);
    const active = this.state.keys.find((k) => k.status === "active" && usable(k));
    if (active) return active.secret;
    const backup = this.state.keys.find((k) => k.status === "backup" && usable(k));
    return backup?.secret ?? null;
  }

  noteMinutes(fingerprint: string, minutes: number) {
    const day = dayKey(this.now());
    const k = this.state.keys.find((x) => x.fingerprint === fingerprint && x.status !== "retired");
    if (!k) return;
    if (k.day !== day) { k.day = day; k.minutesToday = 0; }
    k.minutesToday = Math.round((k.minutesToday + Math.max(0, minutes)) * 100) / 100;
  }

  limits() { return { ...this.state.limits }; }
  setLimits(freeMinutesPerDay: number, softWarnRatio: number) {
    if (freeMinutesPerDay > 0) this.state.limits.freeMinutesPerDay = freeMinutesPerDay;
    if (softWarnRatio > 0 && softWarnRatio < 1) this.state.limits.softWarnRatio = softWarnRatio;
    return this.limits();
  }

  /** Soft warning when used minutes cross the ratio. Null when under it. */
  warnAt(usedMinutes: number) {
    const cap = this.state.limits.freeMinutesPerDay;
    const at = cap * this.state.limits.softWarnRatio;
    return usedMinutes >= at ? { cap, at } : null;
  }

  deviceBlocked(installId: string) { return this.state.devices.some((d) => d.installId === installId && d.blocked); }

  setDevice(installId: string, patch: Partial<DeviceMeta>) {
    let row = this.state.devices.find((d) => d.installId === installId);
    if (!row) { row = { installId }; this.state.devices.push(row); }
    if (patch.engine) row.engine = patch.engine;
    if (typeof patch.blocked === "boolean") row.blocked = patch.blocked;
    return row;
  }

  setChannel(channel: "website" | "updater" | "store", version: string) {
    this.state.settings.channels[channel] = version.slice(0, 32);
  }

  setPinnedLatest(doc: Record<string, unknown> | null, paused: boolean) {
    this.state.settings.pinnedLatest = doc;
    this.state.settings.paused = paused && !!doc;
  }

  /** Document to serve, or null so the static updater file stays in place. */
  pinnedLatest(): Record<string, unknown> | null {
    if (!this.state.settings.paused || !this.state.settings.pinnedLatest) return null;
    return this.state.settings.pinnedLatest;
  }

  addError(input: { kind: string; version: string; line: string; deviceId: string }) {
    const line = sanitizeReportLine(input.line);
    if (!line) return null;
    const row: ErrorReport = {
      id: "err_" + this.state.errors.length + "_" + (this.now() % 1e6),
      at: this.now(),
      kind: input.kind.slice(0, 40) || "unknown",
      version: input.version.slice(0, 32),
      line,
      deviceId: input.deviceId.slice(0, 64),
    };
    this.state.errors.push(row);
    return row;
  }

  markFixed(id: string, version: string) {
    const row = this.state.errors.find((e) => e.id === id);
    if (!row) return null;
    row.fixedIn = version.trim() ? version.slice(0, 32) : undefined;
    return row;
  }

  noteError(id: string, patch: { assignee?: string; note?: string }) {
    const row = this.state.errors.find((e) => e.id === id);
    if (!row) return null;
    if (patch.assignee != null) row.assignee = patch.assignee.slice(0, 80);
    if (patch.note != null) row.note = patch.note.slice(0, 280);
    return row;
  }

  deleteError(id: string) {
    const n = this.state.errors.length;
    this.state.errors = this.state.errors.filter((e) => e.id !== id);
    return this.state.errors.length < n;
  }

  addFeedback(input: { source: "app" | "website"; issueType: string; text: string; deviceId?: string }) {
    const text = sanitizeReportLine(input.text);
    if (!text) return null;
    const row: FeedbackItem = {
      id: "fb_" + this.state.feedback.length + "_" + (this.now() % 1e6),
      at: this.now(),
      source: input.source === "website" ? "website" : "app",
      issueType: ISSUE.has(input.issueType) ? input.issueType : "other",
      text,
      deviceId: input.deviceId?.slice(0, 64),
    };
    this.state.feedback.push(row);
    return row;
  }

  updateFeedback(id: string, patch: { resolved?: boolean; assignee?: string; note?: string }) {
    const row = this.state.feedback.find((f) => f.id === id);
    if (!row) return null;
    if (typeof patch.resolved === "boolean") row.resolved = patch.resolved;
    if (patch.assignee != null) row.assignee = patch.assignee.slice(0, 80);
    if (patch.note != null) row.note = patch.note.slice(0, 280);
    return row;
  }

  deleteFeedback(id: string) {
    const n = this.state.feedback.length;
    this.state.feedback = this.state.feedback.filter((f) => f.id !== id);
    return this.state.feedback.length < n;
  }

  dismissAlert() { this.state.alertedAt = this.now() + 60 * 60 * 1000; }

  forgetDevice(installId: string) {
    this.state.devices = this.state.devices.filter((d) => d.installId !== installId);
    this.state.errors = this.state.errors.filter((e) => e.deviceId !== installId);
    this.state.feedback = this.state.feedback.filter((f) => f.deviceId !== installId);
  }
}

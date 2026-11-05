import { readFile, writeFile, rename } from "node:fs/promises";
import type { MockBackend } from "../worker/mock/store";
import type { AdminService } from "../worker/src/admin";

/**
 * Durable state for the single-process Plesk service. The tested in-memory
 * backend stays authoritative; its metadata (never audio/text) is snapshotted
 * to Supabase (table public.bolo_state) or a local JSON file and reloaded on boot.
 */
export interface StateStore { load(): Promise<unknown | null>; save(data: unknown): Promise<void> }

export class SupabaseStateStore implements StateStore {
  constructor(private url: string, private key: string, private id = "main") {}
  private h() { return { apikey: this.key, authorization: `Bearer ${this.key}`, "content-type": "application/json" }; }
  async load() {
    const r = await fetch(`${this.url}/rest/v1/bolo_state?id=eq.${this.id}&select=data`, { headers: this.h() });
    if (!r.ok) throw new Error(`state load ${r.status}`);
    const rows = (await r.json()) as { data: unknown }[];
    return rows[0]?.data ?? null;
  }
  async save(data: unknown) {
    const r = await fetch(`${this.url}/rest/v1/bolo_state?on_conflict=id`, {
      method: "POST", headers: { ...this.h(), prefer: "resolution=merge-duplicates,return=minimal" },
      body: JSON.stringify({ id: this.id, data, updated_at: new Date().toISOString() }),
    });
    if (!r.ok) throw new Error(`state save ${r.status}`);
  }
}

export class FileStateStore implements StateStore {
  constructor(private path: string) {}
  async load() { try { return JSON.parse(await readFile(this.path, "utf8")); } catch { return null; } }
  async save(data: unknown) { await writeFile(this.path + ".tmp", JSON.stringify(data), { mode: 0o600 }); await rename(this.path + ".tmp", this.path); }
}

const EVENT_DAYS = 40;
type Any = Record<string, any>;

export function snapshot(b: MockBackend, a: AdminService) {
  const ad = a as unknown as Any; const now = Date.now();
  return {
    v: 1,
    profiles: [...b.profiles.entries()], installs: [...b.installs.entries()], tokens: b.tokens,
    events: b.events.filter((e) => e.createdAt > now - EVENT_DAYS * 864e5),
    flags: b.flags, tickets: [...b.tickets.entries()].filter(([, t]) => t.expiresAt > now),
    usedProofs: [...b.usedProofs.entries()].filter(([, t]) => t > now - 864e5),
    audit: ad.audit, installPool: [...ad.installPool.entries()], notices: ad.notices, poolFullIds: [...ad.poolFullIds],
    staff: a.exportStaff(),
  };
}

export function restore(b: MockBackend, a: AdminService, s: Any | null) {
  if (!s || s.v !== 1) return;
  const ad = a as unknown as Any;
  b.profiles = new Map(s.profiles); b.installs = new Map(s.installs); b.tokens = s.tokens; b.events = s.events;
  Object.assign(b.flags, s.flags); b.tickets = new Map(s.tickets); b.usedProofs = new Map(s.usedProofs);
  ad.audit.push(...(s.audit || []).map((e: object) => Object.freeze(e)));
  for (const [k, v] of (s.installPool || [])) ad.installPool.set(k, v);
  ad.notices.push(...(s.notices || [])); for (const id of (s.poolFullIds || [])) ad.poolFullIds.add(id);
  a.importStaff(s.staff);
}

/** Debounced writer; flush() on shutdown. */
export function autosave(store: StateStore, take: () => unknown, ms = 1500) {
  let timer: NodeJS.Timeout | null = null; let chain = Promise.resolve();
  const flush = () => { if (timer) clearTimeout(timer); timer = null; chain = chain.then(() => store.save(take())).catch((e) => console.error("state save failed:", (e as Error).message)); return chain; };
  return { touch() { if (!timer) timer = setTimeout(flush, ms); }, flush };
}

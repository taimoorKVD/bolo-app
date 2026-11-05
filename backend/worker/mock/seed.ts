import type { MockBackend } from "./store";
import { AdminService } from "../src/admin";

/**
 * Clearly-labelled EXAMPLE data for the local admin portal. No real users,
 * emails or credentials. Pool "secrets" below are placeholder strings used
 * only to compute a display fingerprint.
 */
export async function seedExampleData(backend: MockBackend, admin: AdminService) {
  admin.addAdmin({ email: "owner@example.com", role: "owner", label: "Example Owner" });
  admin.addAdmin({ email: "support@example.com", role: "support", label: "Example Support" });
  admin.addAdmin({ email: "viewer@example.com", role: "viewer", label: "Example Viewer" });
  await admin.addPool({ id: "pool_wai_a", label: "Workers AI · Pool A", provider: "workers_ai", status: "active" }, "EXAMPLE-PLACEHOLDER-A");
  await admin.addPool({ id: "pool_wai_b", label: "Workers AI · Pool B", provider: "workers_ai", status: "active" }, "EXAMPLE-PLACEHOLDER-B");
  await admin.addPool({ id: "pool_groq_1", label: "Groq · Pool 1", provider: "groq", status: "active" }, "EXAMPLE-PLACEHOLDER-G1");

  const people = [
    { email: "ayesha.example@example.com", plan: "free", devices: [["Ayesha laptop (example)", "0.1.8"]], minutes: [3.2, 1.1, 0.8] },
    { email: "bilal.example@example.com", plan: "paid", devices: [["Bilal desktop (example)", "0.1.8"], ["Bilal tablet (example)", "0.1.7"]], minutes: [6.5, 4.2, 2.0, 1.4] },
    { email: "chen.example@example.com", plan: "free", devices: [["Chen PC (example)", "0.1.6"]], minutes: [9.6, 0.4] },
    { email: "dana.example@example.com", plan: "free", devices: [["Dana laptop (example)", "0.1.8"]], minutes: [] },
  ] as const;

  const day = 24 * 60 * 60 * 1000;
  for (const [idx, p] of people.entries()) {
    await backend.requestOtp(p.email);
    const v = await backend.verifyOtp(p.email, backend.lastOtpByEmail.get(p.email)!);
    if (!v.ok) continue;
    v.profile.plan = p.plan;
    v.profile.fairUseDailyMinutes = p.plan === "paid" ? 60 : 20;
    for (const [n, [deviceName, appVersion]] of p.devices.entries()) {
      const r = await backend.registerInstall({ userId: v.profile.userId, deviceName, appVersion, publicKey: `EXAMPLE-DEVICE-KEY-${idx}-${n}` });
      if (!r.ok) continue;
      admin.setPool(r.install.id, idx === 2 ? "pool_groq_1" : n === 0 ? "pool_wai_a" : "pool_wai_b");
      for (const [k, m] of p.minutes.entries()) {
        const at = Date.now() - k * day * 3 - 60_000;
        backend.events.push({ userId: v.profile.userId, installId: r.install.id, provider: idx === 2 ? "groq" : "workers_ai", kind: "transcribe", audioMinutes: m, statusClass: "ok", httpStatus: 200, latencyMs: 300 + k * 40, createdAt: at });
      }
      if (idx === 2) backend.events.push({ userId: v.profile.userId, installId: r.install.id, provider: "groq", kind: "transcribe", audioMinutes: 0, statusClass: "quota", httpStatus: 429, createdAt: Date.now() - 30_000 });
    }
  }
}

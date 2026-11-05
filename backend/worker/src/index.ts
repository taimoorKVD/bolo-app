import { createApp, createMockEnv } from "./app";
import { MockBackend } from "../mock/store";
import { seedExampleData } from "../mock/seed";
import { AdminService } from "./admin";
import { MockAiGateway, WorkersAiGateway } from "./ai";
import type { WorkerEnv } from "./types";

/**
 * Cloudflare Worker entry. Until Supabase + secrets are configured, this uses
 * the in-memory MockBackend so `wrangler dev` still boots for route checks.
 * Admin sign-in is "supabase" (MFA) unless ADMIN_AUTH_MODE="mock" is set for
 * local dev, in which case clearly-labelled EXAMPLE data is seeded.
 */
const localBackend = new MockBackend(createMockEnv());
let admin: AdminService | null = null;
let seeded: Promise<void> = Promise.resolve();

export default {
  async fetch(request: Request, env: WorkerEnv): Promise<Response> {
    const merged: WorkerEnv = { ...createMockEnv(), ...env };
    const on = (v: unknown) => ["1", "true", "yes", "on"].includes(String(v || "").toLowerCase());
    if (!admin) {
      const mode = String(merged.ADMIN_AUTH_MODE || "supabase") === "mock" ? "mock" : "supabase";
      admin = new AdminService(localBackend, mode);
      // Device proof is always required unless this is explicit local mock dev.
      const allowUnsigned = mode === "mock" && String(merged.REQUIRE_DEVICE_PROOF).toLowerCase() === "false";
      localBackend.requireDeviceProof = false;
      seeded = (mode === "mock" ? seedExampleData(localBackend, admin) : Promise.resolve()).then(() => {
        localBackend.requireDeviceProof = !allowUnsigned;
      });
      // Env kill switch seeds the initial state; admin portal can change it afterwards.
      localBackend.flags.killSwitch = on(merged.KILL_SWITCH);
    }
    await seeded;
    localBackend.flags.openRouterEnabled = on(merged.OPENROUTER_ENABLED);
    localBackend.flags.paidUsageEnabled = on(merged.PAID_USAGE_ENABLED);
    const ai = env.AI ? new WorkersAiGateway(env.AI) : new MockAiGateway();
    return createApp(localBackend, ai, merged, admin)(request);
  },
};

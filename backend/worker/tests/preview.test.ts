import { describe, expect, it } from "vitest";
import { createApp, createLocalStack, createMockEnv } from "../src/app";
import { MockBackend } from "../mock/store";
import { AdminService } from "../src/admin";
import { MockAiGateway } from "../src/ai";

describe("admin example preview", () => {
  it("works only in mock mode and is labelled", async () => {
    const { handle, ready } = createLocalStack(createMockEnv(), { seed: true });
    await ready;
    const res = await handle(new Request("http://local/admin/preview?view=user"));
    expect(res.status).toBe(200);
    expect(await res.text()).toContain("LOCAL EXAMPLE PREVIEW");
  });
  it("is 404 when admin auth is production", async () => {
    const env = createMockEnv();
    const backend = new MockBackend(env);
    const handle = createApp(backend, new MockAiGateway(), env, new AdminService(backend, "supabase"));
    const res = await handle(new Request("http://local/admin/preview"));
    expect(res.status).toBe(404);
  });
});

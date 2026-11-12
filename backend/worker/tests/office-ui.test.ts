import { describe, expect, it } from "vitest";
import { ADMIN_HTML } from "../src/admin-ui";

describe("office shell", () => {
  it("is self-contained and still exposes the owner actions", () => {
    expect(ADMIN_HTML).toContain('name="robots" content="noindex, nofollow"');
    expect(ADMIN_HTML).toContain('id="shell" hidden');
    expect(ADMIN_HTML).toContain('id="palette"');
    expect(ADMIN_HTML).toContain('id="drawer"');
    expect(ADMIN_HTML).toContain("Office sign-in");
    expect(ADMIN_HTML).toContain("Revoke tokens");
    expect(ADMIN_HTML).toContain("Add key");
    expect(ADMIN_HTML).toContain("/ops/feedback");
    expect(ADMIN_HTML).toContain("/ops/settings");
    expect(ADMIN_HTML).not.toContain("googletagmanager");
    expect(ADMIN_HTML).not.toContain("fonts.googleapis");
    expect(ADMIN_HTML).not.toContain("cdn.tailwindcss.com");
    expect(ADMIN_HTML).toContain('rel="icon" href="/favicon.ico"');
    expect(ADMIN_HTML).toContain("position:fixed");
    expect(ADMIN_HTML).toContain("sessionCheck");
    expect(ADMIN_HTML).toContain('id="navbtn"');
    new Function(ADMIN_HTML.split("<script>").pop()!.split("</script>")[0]);
  });
});

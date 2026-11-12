import { bearer, error, json } from "./http";
import { AdminService, can, type AdminAction, type AdminRole } from "./admin";
import { ADMIN_HTML } from "./admin-ui";
import { SETUP_HTML } from "./setup-ui";
import type { Plan } from "./types";

async function body(req: Request): Promise<Record<string, unknown>> {
  try { return (await req.json()) as Record<string, unknown>; } catch { return {}; }
}
const str = (v: unknown) => (typeof v === "string" ? v : "");

function htmlHeaders(): HeadersInit {
  return {
    "content-type": "text/html; charset=utf-8",
    "cache-control": "no-store",
    "x-frame-options": "DENY",
    "referrer-policy": "no-referrer",
    "content-security-policy": "default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'",
  };
}

function bootScript(admin: AdminService): string {
  const boot = { mock: admin.isMock(), apiBase: "/admin/api", home: "/admin" };
  return `<script>window.__BOLO_OFFICE__=${JSON.stringify(boot)};</script>`;
}

function page(admin: AdminService, extraHead = ""): Response {
  return new Response(ADMIN_HTML.replace("</head>", `${bootScript(admin)}${extraHead}</head>`), { headers: htmlHeaders() });
}

/** Admin routes under /admin (UI) and /admin/api/* (JSON). Returns null when not an admin path. */
export async function handleAdmin(req: Request, path: string, admin: AdminService): Promise<Response | null> {
  if (req.method === "GET" && (path === "/admin" || path === "/admin/index.html")) {
    return page(admin);
  }
  // LOCAL EXAMPLE PREVIEW (mock mode only): signs in the example read-only
  // viewer so a headless browser can screenshot the portal. Production admin
  // login returns 404 here.
  if (req.method === "GET" && path === "/admin/preview") {
    const login = await admin.login("viewer@example.com", "", "123456");
    if (!login.ok) return error(404, "not_found", "Not found.");
    const view = new URL(req.url).searchParams.get("view") === "user" ? "user" : "dash";
    const inject = `<script>
sessionStorage.setItem("bolo_admin", ${JSON.stringify(login.value.token)});
location.hash = "dash";
window.addEventListener("load", function () {
  var b = document.createElement("div");
  b.textContent = "LOCAL EXAMPLE PREVIEW - mock data only - read-only viewer";
  b.style.cssText = "position:fixed;top:0;left:0;right:0;z-index:99;background:#c6a15a;color:#0b0a08;font:600 12px system-ui;text-align:center;padding:3px";
  document.body.appendChild(b);
  if (${JSON.stringify(view)} === "user") {
    fetch("/admin/api/users?q=bilal", { headers: { authorization: "Bearer " + sessionStorage.getItem("bolo_admin") } })
      .then(function (r) { return r.json(); })
      .then(function (j) { if (j.users && j.users[0]) show("user/" + j.users[0].userId); });
  }
});
</script>`;
    return page(admin, inject);
  }
  if (req.method === "GET" && path === "/admin/setup") {
    if (!admin.setupAvailable()) return error(404, "not_found", "Not found.");
    return new Response(SETUP_HTML.replace("</head>", `${bootScript(admin)}</head>`), { headers: htmlHeaders() });
  }

  if (!path.startsWith("/admin/api/")) return null;
  const route = path.slice("/admin/api".length);
  const token = bearer(req);
  const out = (r: { ok: true; value: unknown } | { ok: false; status: number; code: string; message: string }) =>
    r.ok ? json({ ok: true, ...(r.value as object) }) : error(r.status, r.code, r.message);

  if (req.method === "POST" && route === "/setup") {
    const b = await body(req);
    return out(await admin.completeOwnerSetup(str(b.token), str(b.password), str(b.confirm)));
  }
  if (req.method === "GET" && route === "/setup") {
    // Do not advertise setup details; only confirm availability with a boolean.
    if (!admin.setupAvailable()) return error(404, "not_found", "Not found.");
    return json({ ok: true, available: true });
  }

  if (req.method === "POST" && route === "/login") {
    const b = await body(req);
    return out(await admin.login(str(b.email), str(b.password), str(b.code)));
  }
  if (req.method === "POST" && route === "/login/code") {
    const b = await body(req);
    return out(await admin.completeEmailLogin(str(b.challenge), str(b.code)));
  }
  if (req.method === "POST" && route === "/login/resend") {
    const b = await body(req);
    return out(await admin.resendEmailLogin(str(b.challenge)));
  }
  if (req.method === "POST" && route === "/reauth/send") {
    return out(await admin.sendReauthCode(token));
  }
  if (req.method === "POST" && route === "/reauth") {
    const b = await body(req);
    return out(await admin.reauth(token, str(b.code), str(b.password)));
  }
  if (req.method === "POST" && route === "/me/email-2fa") {
    return out(await admin.requestOwnEmail2fa(token));
  }
  if (req.method === "POST" && route === "/me/email-2fa/confirm") {
    const b = await body(req);
    return out(await admin.confirmOwnEmail2fa(token, str(b.code)));
  }
  if (req.method === "POST" && route === "/enroll") {
    const b = await body(req);
    return out(await admin.confirmEnrollment(token, str(b.code)));
  }
  if (req.method === "GET" && route === "/enroll") {
    return out(await admin.enrollmentFor(token));
  }

  const gate = async (action: AdminAction, target = "", reason = "") => admin.authorize(token, action, target, reason);

  if (req.method === "GET") {
    const g = await gate("view");
    if (!g.ok) return out(g);
    const s = g.value;
    if (route === "/me") return json({ ok: true, ...admin.me(s) });
    // Enrollment-gated sessions may only hit /me and /enroll.
    if (admin.me(s).needsEnrollment) return error(403, "enrollment_required", "Finish authenticator setup before using Office.");
    if (route === "/dashboard") return json({ ok: true, ...admin.dashboard() });
    if (route === "/ops/health") return json({ ok: true, ...admin.ops.health() });
    if (route === "/ops/limits") return json({ ok: true, ...admin.ops.limits() });
    if (route === "/ops/releases") return json({ ok: true, channels: admin.ops.state.settings.channels, paused: admin.ops.state.settings.paused, pinned: Boolean(admin.ops.pinnedLatest()) });
    if (route === "/ops/devices") return json({ ok: true, ...admin.devicesView() });
    if (route === "/ops/keys") {
      if (!can(s.role, "view_keys")) return error(403, "forbidden_role", "Your role cannot view provider keys.");
      return json({ ok: true, keys: admin.ops.listKeys() });
    }
    if (route === "/ops/errors") {
      if (!can(s.role, "manage_feedback")) return error(403, "forbidden_role", "Your role cannot view error reports.");
      return json({ ok: true, reports: admin.ops.state.errors });
    }
    if (route === "/ops/feedback") {
      if (!can(s.role, "manage_feedback")) return error(403, "forbidden_role", "Your role cannot view feedback.");
      return json({ ok: true, items: admin.ops.state.feedback });
    }
    if (route === "/ops/settings") {
      if (!can(s.role, "manage_settings") && !can(s.role, "owner_settings")) return error(403, "forbidden_role", "Your role cannot view Office settings.");
      const st = admin.ops.state.settings;
      return json({ ok: true, retentionDays: st.retentionDays, privacyPolicyVersion: st.privacyPolicyVersion, requireEmail2fa: st.requireEmail2fa, ipAllowlist: st.ipAllowlist, sessions: can(s.role, "owner_settings") ? admin.listSessions() : [] });
    }
    if (route === "/users") return json({ ok: true, users: admin.listUsers(s, new URL(req.url).searchParams.get("q") ?? "") });
    if (route === "/audit") return json({ ok: true, entries: admin.auditLog().slice(0, 200) });
    if (route === "/team") {
      if (!can(s.role, "manage_team")) return error(403, "forbidden_role", "Your role (owner only) cannot manage the team.");
      return json({ ok: true, staff: admin.listTeam(s) });
    }
    const m = /^\/users\/([\w-]+)$/.exec(route);
    if (m) {
      const d = admin.userDetail(s, m[1]);
      return d ? json({ ok: true, user: d }) : error(404, "user_missing", "User not found.");
    }
    return error(404, "not_found", "Unknown admin endpoint.");
  }

  if (req.method === "POST") {
    const b = await body(req);
    const reason = str(b.reason);

    if (route === "/team") {
      const g = await admin.authorizeTeamMutation(token, "team", reason);
      if (!g.ok) return out(g);
      return out(await admin.createStaff(g.value, {
        email: str(b.email),
        label: str(b.label) || str(b.name),
        role: str(b.role) as AdminRole,
        password: str(b.password),
      }, reason));
    }
    let tm = /^\/team\/([^/]+)\/(role|enabled|email-2fa)$/.exec(route);
    if (tm) {
      const email = decodeURIComponent(tm[1]);
      const verb = tm[2];
      const g = await admin.authorizeTeamMutation(token, `staff:${email}`, reason);
      if (!g.ok) return out(g);
      if (verb === "role") return out(admin.setStaffRole(g.value, email, str(b.role) as AdminRole, reason));
      if (verb === "email-2fa") return out(admin.setStaffEmail2fa(g.value, email, b.enabled === true, reason));
      return out(admin.setStaffEnabled(g.value, email, b.enabled !== false, reason));
    }

    let m = /^\/installs\/([\w-]+)\/(revoke|reissue|move-pool)$/.exec(route);
    if (m) {
      const [, id, verb] = m;
      const action: AdminAction = verb === "move-pool" ? "move_pool" : (verb as AdminAction);
      const g = await gate(action, `install:${id}`, reason);
      if (!g.ok) return out(g);
      if (verb === "revoke") return out(admin.revoke(g.value, id, reason));
      if (verb === "reissue") return out(admin.reissue(g.value, id, reason));
      return out(admin.movePool(g.value, id, str(b.poolId), reason));
    }
    m = /^\/users\/([\w-]+)\/(plan|suspend|notice)$/.exec(route);
    if (m) {
      const [, id, verb] = m;
      const action: AdminAction = verb === "plan" ? "set_plan" : verb === "suspend" ? "suspend" : "notice";
      const g = await gate(action, `user:${id}`, reason);
      if (!g.ok) return out(g);
      if (verb === "plan") return out(admin.setPlan(g.value, id, str(b.plan) as Plan, reason));
      if (verb === "suspend") return out(admin.setSuspended(g.value, id, b.suspended !== false, reason));
      return out(admin.addNotice(g.value, id, str(b.message)));
    }
    if (route === "/ops/keys") {
      const g = await gate("manage_keys", "keys", reason);
      if (!g.ok) return out(g);
      if (str(b.op) === "retire") return admin.ops.retireKey(str(b.id)) ? json({ ok: true, keys: admin.ops.listKeys() }) : error(404, "not_found", "Key not found.");
      if (str(b.op) === "rotate") {
        const row = await admin.ops.rotateKey(str(b.id), str(b.secret));
        return row ? json({ ok: true, key: row }) : error(400, "invalid_body", "That key was not updated.");
      }
      const row = await admin.ops.addKey(str(b.poolId), str(b.secret), { backup: b.backup === true, dailyMinuteCap: Number(b.dailyMinuteCap) || 0 });
      return row ? json({ ok: true, key: row }) : error(400, "invalid_body", "A key value is required.");
    }
    if (route === "/ops/limits") {
      const g = await gate("manage_limits", "limits", reason);
      if (!g.ok) return out(g);
      return json({ ok: true, ...admin.ops.setLimits(Number(b.freeMinutesPerDay), Number(b.softWarnRatio)) });
    }
    if (route === "/ops/releases") {
      const g = await gate("manage_releases", "releases", reason);
      if (!g.ok) return out(g);
      const channel = str(b.channel);
      if (channel === "website" || channel === "updater" || channel === "store") admin.ops.setChannel(channel, str(b.version));
      if (b.pinned && typeof b.pinned === "object") admin.ops.setPinnedLatest(b.pinned as Record<string, unknown>, b.paused !== false);
      else if (b.paused === false) admin.ops.setPinnedLatest(admin.ops.state.settings.pinnedLatest, false);
      return json({ ok: true, channels: admin.ops.state.settings.channels, paused: admin.ops.state.settings.paused });
    }
    {
      const dev = /^\/ops\/devices\/([\w-]+)\/block$/.exec(route);
      if (dev) {
        const g = await gate("revoke", `install:${dev[1]}`, reason);
        if (!g.ok) return out(g);
        return admin.blockInstall(dev[1], b.blocked !== false) ? json({ ok: true }) : error(404, "not_found", "Device not found.");
      }
    }
    {
      const fix = /^\/ops\/errors\/([\w-]+)\/fixed$/.exec(route);
      if (fix) {
        const g = await gate("manage_feedback", `error:${fix[1]}`, reason);
        if (!g.ok) return out(g);
        const row = admin.ops.markFixed(fix[1], str(b.version));
        return row ? json({ ok: true, report: row }) : error(404, "not_found", "Report not found.");
      }
    }
    if (route === "/ops/feedback") {
      const g = await gate("manage_feedback", "feedback", reason);
      if (!g.ok) return out(g);
      const row = admin.ops.addFeedback({ source: str(b.source) === "website" ? "website" : "app", issueType: str(b.issueType), text: str(b.text) });
      return row ? json({ ok: true, item: row }) : error(400, "invalid_body", "That note could not be saved.");
    }
    if (route === "/ops/settings") {
      const ownerOnly = "requireEmail2fa" in b || "ipAllowlist" in b || str(b.op) === "revoke-session";
      const g = await gate(ownerOnly ? "owner_settings" : "manage_settings", "settings", reason);
      if (!g.ok) return out(g);
      const st = admin.ops.state.settings;
      if (typeof b.retentionDays === "number" && b.retentionDays >= 1) st.retentionDays = Math.min(400, b.retentionDays);
      if (typeof b.privacyPolicyVersion === "string") st.privacyPolicyVersion = b.privacyPolicyVersion.slice(0, 40);
      if (typeof b.requireEmail2fa === "boolean") st.requireEmail2fa = b.requireEmail2fa;
      if (Array.isArray(b.ipAllowlist)) st.ipAllowlist = b.ipAllowlist.map((x) => String(x).trim()).filter(Boolean).slice(0, 50);
      if (str(b.op) === "revoke-session") return admin.revokeSession(str(b.id)) ? json({ ok: true, sessions: admin.listSessions() }) : error(404, "not_found", "Session not found.");
      return json({ ok: true, retentionDays: st.retentionDays, privacyPolicyVersion: st.privacyPolicyVersion, requireEmail2fa: st.requireEmail2fa, ipAllowlist: st.ipAllowlist });
    }
    {
      const userOp = /^\/ops\/users\/([\w-]+)\/(export|delete)$/.exec(route);
      if (userOp) {
        const g = await gate("manage_settings", `user:${userOp[1]}`, reason);
        if (!g.ok) return out(g);
        if (userOp[2] === "export") {
          const data = admin.exportUser(userOp[1]);
          return data ? json({ ok: true, user: data }) : error(404, "user_missing", "User not found.");
        }
        return admin.deleteUserData(userOp[1]) ? json({ ok: true }) : error(404, "user_missing", "User not found.");
      }
    }
    if (route === "/kill-switch") {
      const g = await gate("kill_switch", "global", reason);
      if (!g.ok) return out(g);
      return out(admin.setKillSwitch(g.value, b.on === true, reason));
    }
  }
  return error(404, "not_found", "Unknown admin endpoint.");
}

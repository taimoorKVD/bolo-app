import { describe, expect, it } from "vitest";
import { buildOtpMessage, SmtpMailer, smtpConfigFromEnv } from "./mailer";

describe("mailer", () => {
  it("builds the OTP message", () => {
    const m = buildOtpMessage("Bolo <no-reply@example.com>", "u@example.com", "123456");
    expect(m.to).toBe("u@example.com");
    expect(m.subject).toContain("123456");
    expect(m.text).toContain("123456");
  });
  it("rejects header injection and bad codes", () => {
    expect(() => buildOtpMessage("a@x.com", "u@x.com\r\nBcc: e@x.com", "123456")).toThrow();
    expect(() => buildOtpMessage("a@x.com", "u@x.com", "12ab")).toThrow();
  });
  it("reads SMTP env", () => {
    expect(smtpConfigFromEnv({})).toBeNull();
    expect(smtpConfigFromEnv({ SMTP_HOST: "mail.example.com" })).toEqual({ host: "mail.example.com", port: 587, secure: false, user: undefined, pass: undefined });
    expect(smtpConfigFromEnv({ SMTP_HOST: "h", SMTP_PORT: "465" })?.secure).toBe(true);
    expect(smtpConfigFromEnv({ SMTP_HOST: "h", SMTP_PORT: "2525", SMTP_SECURE: "false", SMTP_USER: "u", SMTP_PASS: "p" })).toMatchObject({ port: 2525, secure: false, user: "u" });
  });
  it("sends through the transport without network", async () => {
    const sent: any[] = [];
    const m = new SmtpMailer({ host: "h", port: 587, secure: false }, "a@x.com", { sendMail: async (x: object) => { sent.push(x); return { accepted: ["u@x.com"] }; } });
    expect(await m.sendOtp("u@x.com", "654321")).toBe(true);
    expect(sent[0].subject).toContain("654321");
    const bad = new SmtpMailer({ host: "h", port: 587, secure: false }, "a@x.com", { sendMail: async () => { throw new Error("down"); } });
    expect(await bad.sendOtp("u@x.com", "654321")).toBe(false);
  });
});

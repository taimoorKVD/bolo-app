import { describe, expect, it } from "vitest";
import { buildOfficeCodeMessage, buildOtpMessage, missingOfficeMailEnv, SmtpMailer, smtpConfigFromEnv } from "./mailer";

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
  it("names missing Office mail env without values", () => {
    expect(missingOfficeMailEnv({})).toEqual(["SMTP2GO_SENDER", "SMTP_HOST", "SMTP2GO_API_KEY"]);
    expect(missingOfficeMailEnv({ SMTP_HOST: "mail.example.com", SMTP2GO_SENDER: "Bolo <a@b.c>", SMTP_PASS: "secret" })).toEqual([]);
    expect(JSON.stringify(missingOfficeMailEnv({ SMTP_PASS: "secret" }))).not.toContain("secret");
    const office = buildOfficeCodeMessage("Bolo <a@b.c>", "u@b.c", "654321");
    expect(office.subject).toContain("Office");
    expect(office.text).toContain("654321");
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

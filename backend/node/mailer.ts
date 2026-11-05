import nodemailer from "nodemailer";

/** Sign-in code senders. The code is never logged. */
export interface Mailer { sendOtp(email: string, code: string): Promise<boolean> }

export function buildOtpMessage(sender: string, email: string, code: string) {
  if (!/^\d{6}$/.test(code)) throw new Error("bad code");
  if (/[\r\n]/.test(email + sender) || !email.includes("@")) throw new Error("bad address");
  return {
    from: sender,
    to: email,
    subject: `Your Bolo sign-in code: ${code}`,
    text: `Your Bolo sign-in code is ${code}\n\nIt expires in 10 minutes. If you did not ask for it, ignore this email.`,
    html: `<p>Your Bolo sign-in code is</p><p style="font-size:24px;font-weight:bold;letter-spacing:4px">${code}</p><p>It expires in 10 minutes. If you did not ask for it, ignore this email.</p>`,
  };
}

export interface SmtpConfig { host: string; port: number; secure: boolean; user?: string; pass?: string }

export function smtpConfigFromEnv(e: Record<string, string | undefined>): SmtpConfig | null {
  if (!e.SMTP_HOST) return null;
  const port = Number(e.SMTP_PORT || 587);
  const secure = e.SMTP_SECURE ? ["1", "true", "yes", "ssl"].includes(e.SMTP_SECURE.toLowerCase()) : port === 465;
  return { host: e.SMTP_HOST, port, secure, user: e.SMTP_USER || undefined, pass: e.SMTP_PASS || undefined };
}

/** SMTP (e.g. mail.smtp2go.com:2525/587 STARTTLS or 465 TLS). */
export class SmtpMailer implements Mailer {
  private t: { sendMail(m: object): Promise<{ accepted?: unknown[] }> };
  constructor(c: SmtpConfig, private sender: string, transport?: SmtpMailer["t"]) {
    this.t = transport ?? nodemailer.createTransport({
      host: c.host, port: c.port, secure: c.secure, requireTLS: !c.secure,
      auth: c.user ? { user: c.user, pass: c.pass } : undefined,
      connectionTimeout: 15_000, greetingTimeout: 15_000, socketTimeout: 20_000,
      tls: { minVersion: "TLSv1.2" },
    });
  }
  async sendOtp(email: string, code: string): Promise<boolean> {
    try { const r = await this.t.sendMail(buildOtpMessage(this.sender, email, code)); return (r.accepted?.length ?? 0) > 0; }
    catch { return false; }
  }
}

export class Smtp2GoMailer implements Mailer {
  constructor(private apiKey: string, private sender: string, private base = "https://api.smtp2go.com/v3") {}
  async sendOtp(email: string, code: string): Promise<boolean> {
    try {
      const r = await fetch(`${this.base}/email/send`, {
        method: "POST",
        headers: { "content-type": "application/json", "X-Smtp2go-Api-Key": this.apiKey },
        body: JSON.stringify((({ from, to, subject, text, html }) => ({ sender: from, to: [to], subject, text_body: text, html_body: html }))(buildOtpMessage(this.sender, email, code))),
        signal: AbortSignal.timeout(15_000),
      });
      if (!r.ok) return false;
      const j = (await r.json()) as { data?: { succeeded?: number } };
      return (j.data?.succeeded ?? 0) > 0;
    } catch { return false; }
  }
}

export class MemoryMailer implements Mailer {
  sent: { email: string; code: string }[] = [];
  ok = true;
  async sendOtp(email: string, code: string) { if (this.ok) this.sent.push({ email, code }); return this.ok; }
}

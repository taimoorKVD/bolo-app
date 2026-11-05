/** Hashing and token helpers for install access. Device signatures use Ed25519. */

const textEncoder = new TextEncoder();

export function bytesToHex(bytes: ArrayBuffer | Uint8Array): string {
  const arr = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  return [...arr].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export function hexToBytes(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) throw new Error("invalid hex");
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

export async function sha256Hex(input: string | Uint8Array): Promise<string> {
  const data = typeof input === "string" ? textEncoder.encode(input) : input;
  const digest = await crypto.subtle.digest("SHA-256", data as BufferSource);
  return bytesToHex(digest);
}

export function randomToken(bytes = 32): string {
  const buf = new Uint8Array(bytes);
  crypto.getRandomValues(buf);
  return bytesToHex(buf);
}

export function publicTokenId(): string {
  return `tok_${randomToken(4)}`;
}

export function last4(token: string): string {
  return token.slice(-4);
}

export async function fingerprintPublicKey(publicKeyPemOrRaw: string): Promise<string> {
  return (await sha256Hex(publicKeyPemOrRaw.trim())).slice(0, 16);
}

/** Verify an Ed25519 signature over canonical request bytes. */
export async function verifyEd25519(
  publicKeySpkiBase64: string,
  message: string,
  signatureBase64: string,
): Promise<boolean> {
  try {
    const keyBytes = Uint8Array.from(atob(publicKeySpkiBase64), (c) => c.charCodeAt(0));
    const key = await crypto.subtle.importKey(
      "spki",
      keyBytes as BufferSource,
      { name: "Ed25519" },
      false,
      ["verify"],
    );
    const sig = Uint8Array.from(atob(signatureBase64), (c) => c.charCodeAt(0));
    return await crypto.subtle.verify(
      { name: "Ed25519" },
      key,
      sig as BufferSource,
      textEncoder.encode(message),
    );
  } catch {
    return false;
  }
}

/** Dev/mock signer using Web Crypto Ed25519 when available. */
export async function generateEd25519KeyPair(): Promise<{
  publicKeySpkiBase64: string;
  privateKeyPkcs8Base64: string;
}> {
  const pair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
  const spki = await crypto.subtle.exportKey("spki", pair.publicKey);
  const pkcs8 = await crypto.subtle.exportKey("pkcs8", pair.privateKey);
  return {
    publicKeySpkiBase64: btoa(String.fromCharCode(...new Uint8Array(spki))),
    privateKeyPkcs8Base64: btoa(String.fromCharCode(...new Uint8Array(pkcs8))),
  };
}

export async function signEd25519(privateKeyPkcs8Base64: string, message: string): Promise<string> {
  const keyBytes = Uint8Array.from(atob(privateKeyPkcs8Base64), (c) => c.charCodeAt(0));
  const key = await crypto.subtle.importKey(
    "pkcs8",
    keyBytes as BufferSource,
    { name: "Ed25519" },
    false,
    ["sign"],
  );
  const sig = await crypto.subtle.sign({ name: "Ed25519" }, key, textEncoder.encode(message));
  return btoa(String.fromCharCode(...new Uint8Array(sig)));
}

export function canonicalRequest(parts: {
  method: string;
  path: string;
  timestamp: string;
  bodyHash: string;
}): string {
  return `${parts.method.toUpperCase()}\n${parts.path}\n${parts.timestamp}\n${parts.bodyHash}`;
}

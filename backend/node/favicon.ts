import { deflateSync } from "node:zlib";

function crc32(buf: Buffer): number {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i];
    for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
  }
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type: string, data: Buffer): Buffer {
  const t = Buffer.from(type);
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([t, data]);
  const sum = Buffer.alloc(4);
  sum.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, sum]);
}

/** 32px gold mark with a dark B. No external asset. */
function png(size: number): Buffer {
  const raw = Buffer.alloc((size * 4 + 1) * size);
  for (let y = 0; y < size; y++) {
    const row = y * (size * 4 + 1);
    raw[row] = 0;
    for (let x = 0; x < size; x++) {
      const dx = x - (size - 1) / 2;
      const dy = y - (size - 1) / 2;
      const inside = Math.max(Math.abs(dx), Math.abs(dy)) <= size * 0.42;
      const gx = x / size;
      const gy = y / size;
      const ink = (gx > 0.30 && gx < 0.44 && gy > 0.26 && gy < 0.74)
        || (gx > 0.30 && gx < 0.66 && gy > 0.26 && gy < 0.40)
        || (gx > 0.30 && gx < 0.62 && gy > 0.44 && gy < 0.56)
        || (gx > 0.30 && gx < 0.66 && gy > 0.60 && gy < 0.74)
        || (gy > 0.28 && gy < 0.54 && gx > 0.56 && gx < 0.70)
        || (gy > 0.46 && gy < 0.72 && gx > 0.54 && gx < 0.68);
      const i = row + 1 + x * 4;
      if (!inside) { raw[i] = raw[i + 1] = raw[i + 2] = raw[i + 3] = 0; continue; }
      if (ink) { raw[i] = 0x1a; raw[i + 1] = 0x14; raw[i + 2] = 0x08; }
      else { raw[i] = 0xe4; raw[i + 1] = 0xc2; raw[i + 2] = 0x7a; }
      raw[i + 3] = 255;
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function ico(pngBytes: Buffer, size: number): Buffer {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(1, 4);
  const entry = Buffer.alloc(16);
  entry[0] = size;
  entry[1] = size;
  entry.writeUInt16LE(1, 4);
  entry.writeUInt16LE(32, 6);
  entry.writeUInt32LE(pngBytes.length, 8);
  entry.writeUInt32LE(22, 12);
  return Buffer.concat([header, entry, pngBytes]);
}

export const FAVICON_ICO = ico(png(32), 32);

/** Answer GET/HEAD /favicon.ico. Returns false when this request is not the icon. */
export function writeFavicon(
  method: string,
  path: string,
  res: { statusCode: number; setHeader: (name: string, value: string) => void; end: (body?: Buffer) => void },
): boolean {
  if (method !== "GET" && method !== "HEAD") return false;
  if (path.split("?")[0] !== "/favicon.ico") return false;
  res.statusCode = 200;
  res.setHeader("content-type", "image/x-icon");
  res.setHeader("content-length", String(FAVICON_ICO.length));
  res.setHeader("cache-control", "public, max-age=86400");
  res.setHeader("x-content-type-options", "nosniff");
  if (method === "HEAD") res.end();
  else res.end(FAVICON_ICO);
  return true;
}

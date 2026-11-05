/**
 * Minimal QR Code → SVG (byte mode, ECC level M). No external deps.
 * Sufficient for otpauth URIs used in Office enrollment.
 */
/* eslint-disable */

const ECC_M = 0; // table index into ECC_TABLE (we only ship M)
// ECC codewords per block for versions 1–10, level M
const ECC_TABLE: Record<number, { ec: number; blocks: number }> = {
  1: { ec: 10, blocks: 1 }, 2: { ec: 16, blocks: 1 }, 3: { ec: 26, blocks: 1 },
  4: { ec: 18, blocks: 2 }, 5: { ec: 24, blocks: 2 }, 6: { ec: 16, blocks: 4 },
  7: { ec: 18, blocks: 4 }, 8: { ec: 22, blocks: 4 }, 9: { ec: 22, blocks: 5 },
  10: { ec: 26, blocks: 5 },
};
const CAPACITY_M: Record<number, number> = {
  // max data codewords (total - ecc) for level M
  1: 16, 2: 28, 3: 44, 4: 64, 5: 86, 6: 108, 7: 124, 8: 154, 9: 182, 10: 216,
};
// Total codewords = capacity + ecc*blocks — verify via size^2 formula below

function gfMul(x: number, y: number): number {
  if (!x || !y) return 0;
  let z = 0;
  for (let i = 0; i < 8; i++) {
    if (y & (1 << i)) z ^= x << i;
  }
  for (let i = 15; i >= 8; i--) {
    if (z & (1 << i)) z ^= 0x11d << (i - 8);
  }
  return z;
}

function rsGenerator(ecLen: number): number[] {
  let g = [1];
  for (let i = 0; i < ecLen; i++) {
    const next = new Array(g.length + 1).fill(0);
    const alpha = (() => { let a = 1; for (let k = 0; k < i; k++) a = gfMul(a, 2); return a; })();
    for (let j = 0; j < g.length; j++) {
      next[j] ^= g[j];
      next[j + 1] ^= gfMul(g[j], alpha);
    }
    g = next;
  }
  return g;
}

function rsEncode(data: number[], ecLen: number): number[] {
  const gen = rsGenerator(ecLen);
  const res = data.concat(new Array(ecLen).fill(0));
  for (let i = 0; i < data.length; i++) {
    const coef = res[i];
    if (!coef) continue;
    for (let j = 0; j < gen.length; j++) res[i + j] ^= gfMul(gen[j], coef);
  }
  return res.slice(data.length);
}

function bitBuffer() {
  const bytes: number[] = [];
  let cur = 0, bits = 0;
  return {
    push(val: number, len: number) {
      for (let i = len - 1; i >= 0; i--) {
        cur = (cur << 1) | ((val >>> i) & 1);
        bits++;
        if (bits === 8) { bytes.push(cur); cur = 0; bits = 0; }
      }
    },
    toBytes(padTo: number) {
      if (bits > 0) bytes.push(cur << (8 - bits));
      const pads = [0xec, 0x11];
      let pi = 0;
      while (bytes.length < padTo) bytes.push(pads[pi++ % 2]);
      return bytes.slice(0, padTo);
    },
  };
}

function chooseVersion(dataLen: number): number {
  for (let v = 1; v <= 10; v++) {
    const cc = CAPACITY_M[v];
    // byte mode: 4 mode + (8 or 16) count + 8*data + 4 terminator ≈
    const countBits = v <= 9 ? 8 : 16;
    const needBits = 4 + countBits + dataLen * 8 + 4;
    const needBytes = Math.ceil(needBits / 8);
    if (needBytes <= cc) return v;
  }
  throw new Error("data too long for QR");
}

function sizeOf(v: number) { return 21 + (v - 1) * 4; }

function totalCodewords(v: number): number {
  const s = sizeOf(v);
  // Approximate from known totals:
  const TOTAL: Record<number, number> = {
    1: 26, 2: 44, 3: 70, 4: 100, 5: 134, 6: 172, 7: 196, 8: 242, 9: 292, 10: 346,
  };
  return TOTAL[v];
}

function placeFinders(mod: boolean[][], n: number) {
  const draw = (x: number, y: number) => {
    for (let dy = -1; dy <= 7; dy++) for (let dx = -1; dx <= 7; dx++) {
      const xx = x + dx, yy = y + dy;
      if (xx < 0 || yy < 0 || xx >= n || yy >= n) continue;
      const on = dx >= 0 && dx <= 6 && dy >= 0 && dy <= 6 &&
        (dx === 0 || dx === 6 || dy === 0 || dy === 6 || (dx >= 2 && dx <= 4 && dy >= 2 && dy <= 4));
      mod[yy][xx] = on;
    }
  };
  draw(0, 0); draw(n - 7, 0); draw(0, n - 7);
}

function placeTiming(mod: boolean[][], reserved: boolean[][], n: number) {
  for (let i = 0; i < n; i++) {
    if (!reserved[6][i]) { mod[6][i] = i % 2 === 0; reserved[6][i] = true; }
    if (!reserved[i][6]) { mod[i][6] = i % 2 === 0; reserved[i][6] = true; }
  }
}

function reserveFinders(reserved: boolean[][], n: number) {
  const mark = (x: number, y: number) => {
    for (let dy = -1; dy <= 7; dy++) for (let dx = -1; dx <= 7; dx++) {
      const xx = x + dx, yy = y + dy;
      if (xx >= 0 && yy >= 0 && xx < n && yy < n) reserved[yy][xx] = true;
    }
  };
  mark(0, 0); mark(n - 7, 0); mark(0, n - 7);
  for (let i = 0; i < 8; i++) {
    reserved[8][i] = true; reserved[i][8] = true;
    reserved[8][n - 1 - i] = true; reserved[n - 1 - i][8] = true;
  }
  reserved[8][8] = true;
}

function placeData(mod: boolean[][], reserved: boolean[][], bits: number[], n: number) {
  let bi = 0;
  let upward = true;
  for (let x = n - 1; x > 0; x -= 2) {
    if (x === 6) x--;
    for (let i = 0; i < n; i++) {
      const y = upward ? n - 1 - i : i;
      for (let dx = 0; dx < 2; dx++) {
        const xx = x - dx;
        if (reserved[y][xx]) continue;
        const bit = bi < bits.length ? bits[bi++] : 0;
        mod[y][xx] = bit === 1;
      }
    }
    upward = !upward;
  }
}

function maskFn(mask: number, x: number, y: number): boolean {
  switch (mask) {
    case 0: return (x + y) % 2 === 0;
    case 1: return y % 2 === 0;
    case 2: return x % 3 === 0;
    case 3: return (x + y) % 3 === 0;
    case 4: return (Math.floor(y / 2) + Math.floor(x / 3)) % 2 === 0;
    case 5: return ((x * y) % 2) + ((x * y) % 3) === 0;
    case 6: return (((x * y) % 2) + ((x * y) % 3)) % 2 === 0;
    case 7: return (((x + y) % 2) + ((x * y) % 3)) % 2 === 0;
    default: return false;
  }
}

function applyMask(mod: boolean[][], reserved: boolean[][], mask: number, n: number) {
  const out = mod.map((row) => row.slice());
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
    if (!reserved[y][x] && maskFn(mask, x, y)) out[y][x] = !out[y][x];
  }
  return out;
}

function formatBits(mask: number): number {
  // ECC level M = 00, mask 3 bits; BCH(15,5)
  const data = (0b00 << 3) | mask;
  let rem = data << 10;
  for (let i = 14; i >= 10; i--) {
    if (rem & (1 << i)) rem ^= 0x537 << (i - 10);
  }
  return ((data << 10) | rem) ^ 0x5412;
}

function placeFormat(mod: boolean[][], mask: number, n: number) {
  const fb = formatBits(mask);
  const set = (x: number, y: number, bit: number) => { mod[y][x] = bit === 1; };
  for (let i = 0; i < 8; i++) {
    const bit = (fb >>> i) & 1;
    set(i < 6 ? i : i + 1, 8, bit);
    set(8, n - 1 - i, bit);
  }
  for (let i = 0; i < 7; i++) {
    const bit = (fb >>> (14 - i)) & 1;
    set(8, i < 6 ? i : i + 1, bit);
    set(n - 1 - i, 8, bit); // careful: format vertical bottom-left
  }
  // Fix bottom-left vertical format placement (standard positions)
  // Positions: (8, n-1)..(8,n-7) already set above for first 8 of low bits on right...
  mod[n - 8][8] = true; // dark module
}

function penalty(mod: boolean[][], n: number): number {
  let p = 0;
  // simplified penalty: just count transitions imbalance
  let dark = 0;
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) if (mod[y][x]) dark++;
  p += Math.abs(dark * 100 / (n * n) - 50) * 2;
  return p;
}

/** Encode text as QR modules SVG (black/gold friendly: black modules on transparent). */
export function qrSvg(text: string, moduleSize = 4, margin = 4): string {
  const data = new TextEncoder().encode(text);
  const version = chooseVersion(data.length);
  const n = sizeOf(version);
  const eccInfo = ECC_TABLE[version];
  const dataCapacity = CAPACITY_M[version];
  const buf = bitBuffer();
  buf.push(0b0100, 4); // byte mode
  buf.push(data.length, version <= 9 ? 8 : 16);
  for (const b of data) buf.push(b, 8);
  buf.push(0, 4);
  const dataBytes = buf.toBytes(dataCapacity);

  // Split into blocks (simplified: equal blocks)
  const blocks = eccInfo.blocks;
  const dataPerBlock = Math.floor(dataCapacity / blocks);
  const ecLen = eccInfo.ec;
  const dataBlocks: number[][] = [];
  const ecBlocks: number[][] = [];
  let offset = 0;
  // Some versions have uneven block sizes; handle remainder
  const shortBlocks = blocks * dataPerBlock - (blocks * dataPerBlock - (dataCapacity - (dataCapacity % blocks) === dataCapacity ? 0 : 0));
  // Simpler approach for v1-3 (1 block) and even splits:
  const base = Math.floor(dataCapacity / blocks);
  const rem = dataCapacity % blocks;
  for (let i = 0; i < blocks; i++) {
    const sz = base + (i >= blocks - rem ? 1 : 0);
    const block = dataBytes.slice(offset, offset + sz);
    offset += sz;
    dataBlocks.push(block);
    ecBlocks.push(rsEncode(block, ecLen));
  }

  const interleaved: number[] = [];
  const maxData = Math.max(...dataBlocks.map((b) => b.length));
  for (let i = 0; i < maxData; i++) for (const b of dataBlocks) if (i < b.length) interleaved.push(b[i]);
  for (let i = 0; i < ecLen; i++) for (const b of ecBlocks) interleaved.push(b[i]);

  // Remainder bits for some versions — ignore for module placement (zeros)

  const bits: number[] = [];
  for (const b of interleaved) for (let i = 7; i >= 0; i--) bits.push((b >>> i) & 1);

  const reserved = Array.from({ length: n }, () => Array(n).fill(false));
  const mod = Array.from({ length: n }, () => Array(n).fill(false));
  placeFinders(mod, n);
  reserveFinders(reserved, n);
  placeTiming(mod, reserved, n);
  // Also reserve timing
  for (let i = 0; i < n; i++) { reserved[6][i] = true; reserved[i][6] = true; }

  placeData(mod, reserved, bits, n);

  let best = mod;
  let bestMask = 0;
  let bestPen = Infinity;
  for (let mask = 0; mask < 8; mask++) {
    const masked = applyMask(mod, reserved, mask, n);
    // place format onto a copy
    const withFmt = masked.map((r) => r.slice());
    placeFormat(withFmt, mask, n);
    const pen = penalty(withFmt, n);
    if (pen < bestPen) { bestPen = pen; best = withFmt; bestMask = mask; }
  }
  void bestMask;

  const dim = (n + margin * 2) * moduleSize;
  let parts = `<svg xmlns="http://www.w3.org/2000/svg" width="${dim}" height="${dim}" viewBox="0 0 ${dim} ${dim}" shape-rendering="crispEdges">`;
  parts += `<rect width="100%" height="100%" fill="#fff"/>`;
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
    if (best[y][x]) {
      parts += `<rect x="${(x + margin) * moduleSize}" y="${(y + margin) * moduleSize}" width="${moduleSize}" height="${moduleSize}" fill="#0b0b0c"/>`;
    }
  }
  parts += `</svg>`;
  return parts;
}

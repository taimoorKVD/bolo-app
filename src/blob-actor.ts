import {
  clamp,
  poseImpulse,
  type CharacterState,
  type Voice,
} from "./blob-state";

/** Bolo Blob's size in CSS pixels. The curl, arms, and hop fit in the overlay canvas. */
export const BLOB_UNIT = 50;
export const BLOB_BLUSH = 0.36;

export class Spring {
  x: number;
  v = 0;
  t: number;
  k: number;
  z: number;

  constructor(x: number, k = 140, z = 0.8) {
    this.x = x;
    this.t = x;
    this.k = k;
    this.z = z;
  }

  step(dt: number, reduced: boolean): number {
    if (reduced) {
      this.x = this.t;
      this.v = 0;
      return this.x;
    }
    const c = 2 * Math.sqrt(this.k) * this.z;
    const n = Math.max(1, Math.ceil(dt * 240));
    const h = dt / n;
    for (let i = 0; i < n; i++) {
      const a = this.k * (this.t - this.x) - c * this.v;
      this.v += a * h;
      this.x += this.v * h;
    }
    return this.x;
  }

  snap(x: number): void {
    this.x = this.t = x;
    this.v = 0;
  }
}

export type BlobUniforms = {
  shw: number;
  uSq: [number, number];
  uTilt: number;
  uLift: number;
  uHover: number;
  uLook: [number, number];
  uPup: [number, number];
  uLid: [number, number];
  uLow: [number, number];
  uSlope: number;
  uHappy: number;
  uBrow: [number, number, number, number];
  uMouth: [number, number, number, number];
  uMo2: [number, number];
  uTip: [number, number];
  uTip2: [number, number];
  uArm: [number, number];
  uWob: [number, number];
  uHand: [number, number];
  uGlow: number;
  uErr: number;
  uSpark: number;
  uLv: number;
  uListen: number;
  uThink: number;
  uDone: number;
  uFaceY: number;
  uCheek: number;
  uBlush: number;
  uBeat: number;
  uGlitch: number;
  uAlpha: number;
  uFast: number;
  uT: number;
};

const ZERO_VOICE: Voice = { lv: 0, bands: [0, 0, 0, 0], vo: 0, vw: 1, vr: 0, cl: 0 };

function rng(seed: number): () => number {
  let a = seed | 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function sm(x: number): number {
  const c = clamp(x);
  return c * c * (3 - 2 * c);
}

/** Up and slightly left: the user's text sits above the overlay. */
function caretGaze(): [number, number] {
  return [-0.42, 0.7];
}

/**
 * Bolo Blob's acting, ported from the v4 mockup.
 * Reduced motion snaps springs and skips the hop, wink, and fidgets.
 * The mouth still follows `voice` while listening.
 */
export class BlobActor {
  t = 0;
  state: CharacterState = "hidden";
  since = 0;
  hidden = true;
  hideT = -9;
  reduced: boolean;
  blink = 0;
  alpha = 0;
  lv = 0;
  bands: [number, number, number, number] = [0, 0, 0, 0];
  vis = { vo: 0, vw: 1, vr: 0, cl: 0 };
  lift = 0;
  liftV = 0;
  prevLiftV = 0;
  hop: { t0: number; stage: number } | null = null;
  beats: Record<string, number> = {};
  gaze: [number, number] = [0, 0.05];
  micro: [number, number] = [0, 0];
  nextBlink = 1.3;
  blinkT = -9;
  blinkAmp = 1;
  nextSac = 0.4;
  nextMicro = 0.2;
  nextFidget = 2.4;
  lastHi = -9;
  lastNod = -9;
  prevLv = 0;
  readonly sq = new Spring(1, 260, 0.32);
  readonly tilt = new Spring(0, 80, 0.5);
  readonly lean = new Spring(0, 70, 0.85);
  readonly gx = new Spring(0, 300, 0.95);
  readonly gy = new Spring(0, 300, 0.95);
  readonly px = new Spring(0, 700, 0.9);
  readonly py = new Spring(0, 700, 0.9);
  readonly lidL = new Spring(1, 260, 0.9);
  readonly lidR = new Spring(1, 260, 0.9);
  readonly lowL = new Spring(0, 200, 0.9);
  readonly lowR = new Spring(0, 200, 0.9);
  readonly slope = new Spring(0, 120, 0.9);
  readonly happy = new Spring(0, 150, 0.78);
  readonly bLh = new Spring(0, 160, 0.7);
  readonly bLa = new Spring(0, 160, 0.7);
  readonly bRh = new Spring(0, 160, 0.7);
  readonly bRa = new Spring(0, 160, 0.7);
  readonly cheek = new Spring(0, 140, 0.8);
  readonly blush = new Spring(0.3, 60, 1);
  readonly mo = new Spring(0, 520, 0.82);
  readonly mw = new Spring(0.13, 420, 0.82);
  readonly ms = new Spring(0.7, 200, 0.75);
  readonly mr = new Spring(0, 380, 0.85);
  readonly mx = new Spring(0, 120, 0.8);
  readonly mt = new Spring(0, 120, 0.8);
  readonly armL = new Spring(0, 170, 0.45);
  readonly armR = new Spring(0, 170, 0.45);
  readonly jx = new Spring(0, 70, 0.16);
  readonly jy = new Spring(0, 70, 0.16);
  readonly wa = new Spring(0, 110, 0.12);
  readonly wb = new Spring(0, 90, 0.12);
  readonly tx = new Spring(0, 55, 0.18);
  readonly ty = new Spring(0, 55, 0.18);
  readonly glow = new Spring(0.2, 60, 1);
  readonly err = new Spring(0, 90, 1);
  readonly spark = new Spring(0, 60, 1);
  readonly faceY = new Spring(0, 180, 0.45);
  readonly listen = new Spring(0, 80, 1);
  readonly think = new Spring(0, 80, 1);
  readonly done = new Spring(0, 80, 1);
  readonly pop = new Spring(0, 210, 0.5);
  readonly pill = new Spring(0, 170, 0.82);
  readonly qm = new Spring(0, 180, 0.5);
  private random = rng(11);
  private springs: Spring[];

  constructor(reduced: boolean) {
    this.reduced = reduced;
    this.springs = [
      this.sq, this.tilt, this.lean, this.gx, this.gy, this.px, this.py,
      this.lidL, this.lidR, this.lowL, this.lowR, this.slope, this.happy,
      this.bLh, this.bLa, this.bRh, this.bRa, this.cheek, this.blush,
      this.mo, this.mw, this.ms, this.mr, this.mx, this.mt,
      this.armL, this.armR, this.jx, this.jy, this.wa, this.wb, this.tx, this.ty,
      this.glow, this.err, this.spark, this.faceY, this.listen, this.think, this.done,
      this.pop, this.pill, this.qm,
    ];
    this.pop.snap(0);
    this.pill.snap(0);
  }

  setReduced(reduced: boolean): void {
    this.reduced = reduced;
  }

  setState(state: CharacterState): void {
    if (state === this.state) return;
    const prev = this.state;
    this.state = state;
    this.since = this.t;
    this.beats = {};
    this.blinkT = this.t + 0.03;
    this.blinkAmp = 1;
    this.nextSac = this.t;
    if (state === "hidden") {
      this.hide();
      return;
    }
    if (prev === "hidden" || this.hidden) this.show();
    const impulse = poseImpulse(state, this.reduced);
    if (impulse.hop) this.hop = { t0: this.t, stage: 0 };
    if (impulse.shrug) this.tilt.v += 2.2;
    if (state === "listening" && !this.reduced) {
      this.sq.v += 1.6;
      this.jy.v += 1.5;
    }
  }

  show(): void {
    this.hidden = false;
    this.hideT = -9;
    if (this.reduced) {
      this.pop.snap(1);
      this.pill.snap(this.state === "hidden" || this.state === "fix" ? 0 : 1);
      return;
    }
    this.pop.z = 0.5;
    this.pop.t = 1;
    this.sq.v += 3.2;
    this.jy.v += 2.5;
    this.tx.v += 2;
    this.blinkT = this.t + 0.25;
  }

  /** Tuck. The pose stays so the face doesn't change while it shrinks. */
  hide(): void {
    if (this.hidden) return;
    this.hidden = true;
    this.hideT = this.t;
    if (this.reduced) {
      this.pop.snap(0);
      this.pill.snap(0);
      return;
    }
    this.sq.v += 2.2;
    this.jy.v += 2;
  }

  /** Start the tuck while the done face is still showing. */
  tuck(): void {
    this.hide();
  }

  step(dt: number, voice: Voice | null, fast: boolean): BlobUniforms {
    this.t += dt;
    const target = this.state === "listening" && voice ? voice : ZERO_VOICE;
    const ka = 1 - Math.exp(-dt * 32);
    const kr = 1 - Math.exp(-dt * 9);
    const kv = 1 - Math.exp(-dt * 22);
    this.lv += (target.lv - this.lv) * (target.lv > this.lv ? ka : kr);
    for (let i = 0; i < 4; i++) {
      const next = target.bands[i] ?? 0;
      this.bands[i] += (next - this.bands[i]) * (next > this.bands[i] ? ka : kr);
    }
    this.vis.vo += (target.vo - this.vis.vo) * kv;
    this.vis.vw += (target.vw - this.vis.vw) * kv;
    this.vis.vr += (target.vr - this.vis.vr) * kv;
    this.vis.cl += (target.cl - this.vis.cl) * (1 - Math.exp(-dt * 35));
    this.act(dt);
    const uniforms = this.uniforms(fast);
    this.alpha = uniforms.uAlpha;
    return uniforms;
  }

  private beat(name: string, at: number, ts: number): boolean {
    if (ts >= at && !this.beats[name]) {
      this.beats[name] = 1;
      return true;
    }
    return false;
  }

  private act(dt: number): void {
    const t = this.t;
    const s = this.state === "hidden" ? "idle" : this.state === "fix" ? "done" : this.state;
    const ts = t - this.since;
    const R = this.random;
    if (t >= this.nextSac) {
      if (s === "idle") {
        const r = R();
        this.gaze = r < 0.2 ? caretGaze() : r < 0.32 ? [0.85, -0.15] : [(R() * 2 - 1) * 0.75, (R() * 2 - 1) * 0.35 + 0.05];
        this.nextSac = t + 0.9 + 1.7 * R();
      } else if (s === "listening") {
        this.gaze = R() < 0.28 ? [0.9, -0.2] : [(R() * 2 - 1) * 0.1, 0.14 + (R() * 2 - 1) * 0.05];
        this.nextSac = t + 0.5 + 0.8 * R();
      } else if (s === "thinking") {
        this.gaze = [0.55 + (R() * 2 - 1) * 0.18, 0.75 + R() * 0.12];
        this.nextSac = t + 0.45 + 0.5 * R();
      } else if (s === "done") {
        this.gaze = ts < 0.95 ? caretGaze() : [(R() * 2 - 1) * 0.08, 0.06];
        this.nextSac = ts < 0.95 ? this.since + 0.95 : t + 0.8 + R();
      } else {
        this.gaze = [(R() * 2 - 1) * 0.15, 0.08];
        this.nextSac = t + 0.6 + 0.6 * R();
      }
    }
    if (!this.reduced && t >= this.nextMicro) {
      this.micro = [(R() * 2 - 1) * 0.06, (R() * 2 - 1) * 0.05];
      this.nextMicro = t + 0.16 + 0.35 * R();
    }
    this.gx.t = this.gaze[0];
    this.gy.t = this.gaze[1];
    this.px.t = this.gaze[0] + this.micro[0];
    this.py.t = this.gaze[1] + this.micro[1];

    if (!this.reduced && t >= this.nextBlink) {
      this.blinkT = t;
      this.blinkAmp = R() < 0.25 ? 0.55 + 0.2 * R() : 1;
      this.nextBlink = t + (R() < 0.18 ? 0.34 : 2.3 + 2.6 * R());
    }
    const bt = t - this.blinkT;
    let blink = 0;
    if (!this.reduced && bt >= 0 && bt < 0.24) {
      blink = bt < 0.07 ? sm(bt / 0.07) : bt < 0.1 ? 1 : 1 - sm((bt - 0.1) / 0.14);
      blink *= this.blinkAmp;
    }
    const lv = this.lv;
    if (s === "listening" && !this.reduced) {
      if (lv > 0.3) this.lastHi = t;
      if (lv < 0.12 && t - this.lastHi > 0.07 && t - this.lastHi < 0.35 && t - this.lastNod > 0.75) {
        this.lastNod = t;
        this.faceY.v -= 1.5;
        this.sq.v -= 0.7;
        this.jy.v -= 1.2;
        this.wa.v += 1.5;
        this.ty.v -= 1;
      }
      if (lv > 0.45 && this.prevLv <= 0.45) {
        this.sq.v -= 0.55;
        this.wb.v += 0.55;
      }
    }
    this.prevLv = lv;

    const vis = this.vis;
    const lo = (0.035 + 0.36 * (0.35 + 0.65 * vis.vo) * clamp(lv * 1.7)) * (1 - vis.cl * 0.9) * 0.8;
    const wave = this.reduced ? 0 : 1;
    const pose = {
      idle: {
        lid: [1, 1], low: [0.1, 0.1], sl: 0, bh: [0, 0], ba: [0, 0],
        m: [0, 0.13, 0.75, 0, 0, 0], tilt: 0.05 * Math.sin(t * 0.55) * wave, lean: 0, arm: [0, 0],
        glow: 0.16, cheek: 0.3, blush: BLOB_BLUSH * 0.85,
      },
      listening: {
        lid: [1.12, 1.12], low: [0, 0], sl: -0.06, bh: [0.07, 0.07], ba: [-0.08, -0.08],
        m: [lo, 0.115 * (0.72 + 0.42 * vis.vw), 0.38, clamp(vis.vr), 0, 0],
        tilt: 0.06 + 0.05 * Math.sin(t * 0.8) * wave, lean: 1, arm: [0.12, 0.12],
        glow: 0.32 + lv * 0.9, cheek: 0.15, blush: BLOB_BLUSH * 0.8,
      },
      thinking: {
        lid: [0.84, 0.74], low: [0.16, 0.3], sl: 0.1, bh: [0.1, -0.03], ba: [-0.12, 0.28],
        m: [0, 0.085, -0.12, 0, 0.06, 0.2], tilt: -0.13 + 0.02 * Math.sin(t * 1.3) * wave,
        lean: 0, arm: [0, 0.5], glow: 0.32, cheek: 0.1, blush: BLOB_BLUSH * 0.7,
      },
      done: {
        lid: [1, 1], low: [0.3, 0.3], sl: 0, bh: [0.08, 0.08], ba: [0, 0],
        m: [0.055, 0.135, 1.05, 0, 0, 0], tilt: 0, lean: 0.3,
        arm: ts > 0.25 && ts < 1.05 ? [0.8, 0.8] : [0.05, 0.05],
        glow: 0.45, cheek: 1, blush: BLOB_BLUSH * 1.45,
      },
      error: {
        lid: [1.06, 1.06], low: [0.05, 0.05], sl: -0.22, bh: [0.09, 0.06], ba: [-0.34, -0.34],
        m: [0.018, 0.085, -0.65, 0, -0.04, -0.24], tilt: 0.24, lean: 0,
        arm: ts > 0.1 && ts < 0.85 ? [1, 1] : [0.2, 0.2],
        glow: 0.22, cheek: 0, blush: BLOB_BLUSH * 0.6,
      },
    }[s];

    const lid = pose.lid.slice();
    let smile = pose.m[2] ?? 0;
    if (!this.reduced && s === "idle" && ts > 0.9 && ts < 1.35) {
      lid[1] = 0;
      smile = 1.2;
      if (this.beat("wink", 0.9, ts)) {
        this.sq.v -= 0.8;
        this.jx.v += 1.2;
      }
    }
    this.lidL.t = lid[0] ?? 1;
    this.lidR.t = lid[1] ?? 1;
    this.lowL.t = pose.low[0] ?? 0;
    this.lowR.t = pose.low[1] ?? 0;
    this.slope.t = pose.sl;
    this.bLh.t = pose.bh[0] ?? 0;
    this.bRh.t = pose.bh[1] ?? 0;
    this.bLa.t = pose.ba[0] ?? 0;
    this.bRa.t = pose.ba[1] ?? 0;
    this.mo.t = (pose.m[0] ?? 0);
    this.mw.t = pose.m[1] ?? 0.13;
    this.ms.t = smile;
    this.mr.t = pose.m[3] ?? 0;
    this.mx.t = pose.m[4] ?? 0;
    this.mt.t = pose.m[5] ?? 0;
    this.tilt.t = pose.tilt;
    this.lean.t = pose.lean;
    this.armL.t = pose.arm[0] ?? 0;
    this.armR.t = pose.arm[1] ?? 0;
    this.glow.t = pose.glow;
    this.cheek.t = pose.cheek;
    this.blush.t = pose.blush;
    this.happy.t = s === "done" && ts > 0.12 ? 1 : 0;
    this.err.t = s === "error" ? 1 : 0;
    this.spark.t = !this.reduced && s === "thinking" ? 1 : 0;
    this.listen.t = s === "listening" ? 1 : 0;
    this.think.t = s === "thinking" ? 1 : 0;
    this.done.t = s === "done" ? 1 : 0;

    let sqT = 1 + (this.reduced ? 0 : 0.022 * Math.sin((t * 2 * Math.PI) / 3.3));
    if (!this.reduced && s === "error" && ts > 0.1 && ts < 0.85) sqT = 1.07;
    if (!this.reduced && s === "idle" && t >= this.nextFidget) {
      const r = R();
      if (r < 0.35) this.tilt.v += (R() < 0.5 ? -1 : 1) * 1.3;
      else if (r < 0.65) {
        this.sq.v -= 1.1;
        this.jy.v -= 1;
      } else {
        this.armR.v += 4;
        this.tx.v += 1.5;
      }
      this.nextFidget = t + 2.6 + 2.6 * R();
    }
    if (!this.reduced && this.hop) {
      const hop = this.hop;
      const ht = t - hop.t0;
      if (hop.stage === 0) {
        sqT = 0.84;
        if (ht > 0.13) {
          hop.stage = 1;
          this.liftV = 3.6;
          this.sq.v += 2.2;
        }
      } else if (hop.stage === 1) {
        sqT = this.liftV > 0 ? 1.09 : 1.02;
      }
    }
    if (this.hidden) {
      const ht = t - this.hideT;
      this.pill.t = 0;
      if (ht > 0.14) {
        this.pop.z = 0.95;
        this.pop.t = 0;
      } else sqT = 1.1;
    } else {
      this.pop.t = 1;
      this.pill.t = s === "idle" || this.state === "fix" ? 0 : 1;
    }
    this.sq.t = sqT;
    this.qm.t = s === "error" && ts > 0.25 && !this.hidden ? 1 : 0;

    this.prevLiftV = this.liftV;
    if (!this.reduced && (this.lift > 0 || this.liftV > 0)) {
      this.liftV -= 16 * dt;
      this.lift += this.liftV * dt;
      if (this.lift <= 0) {
        this.lift = 0;
        if (this.hop && this.hop.stage === 1) {
          this.sq.v -= 3.4;
          this.wa.v += 4;
          this.wb.v -= 3;
          this.jy.v -= 3.5;
          this.ty.v -= 3;
          this.hop.stage = 2;
        }
        this.liftV = 0;
      }
    } else if (this.reduced) {
      this.lift = 0;
      this.liftV = 0;
    }
    if (!this.reduced) {
      const accY = (this.liftV - this.prevLiftV) / Math.max(dt, 1e-4);
      this.jy.v += (-accY * 0.0045 - this.sq.v * 0.06) * dt * 30;
      this.jx.v += -this.tilt.v * 0.05 * dt * 30;
      this.ty.v += (-accY * 0.004 - this.sq.v * 0.05) * dt * 30;
      this.tx.v += -this.tilt.v * 0.07 * dt * 30;
    }
    for (const spring of this.springs) spring.step(dt, this.reduced);
    this.blink = blink;
  }

  private uniforms(fast: boolean): BlobUniforms {
    const t = this.t;
    const lean = this.lean.x;
    const sc = 1 + 0.05 * lean;
    const sy = this.sq.x * sc;
    const sx = (1 + (1 - this.sq.x) * 0.75) * sc;
    const perk = this.listen.x;
    const wave = this.reduced ? 0 : Math.sin(t * 0.9);
    const tip: [number, number] = [
      0.05 + 0.04 * wave - 0.04 * perk + this.jx.x * 0.6,
      0.3 + 0.08 * perk + this.jy.x * 0.5,
    ];
    const lid: [number, number] = [
      Math.max(0, this.lidL.x * (1 - this.blink)),
      Math.max(0, this.lidR.x * (1 - this.blink)),
    ];
    const shw = Math.max(0.001, this.pop.x);
    return {
      shw,
      uSq: [sx, sy],
      uTilt: this.tilt.x,
      uLift: this.lift,
      uHover: 0,
      uLook: [this.gx.x, this.gy.x],
      uPup: [this.px.x, this.py.x],
      uLid: lid,
      uLow: [this.lowL.x, this.lowR.x],
      uSlope: this.slope.x,
      uHappy: this.happy.x,
      uBrow: [this.bLa.x, this.bLh.x, this.bRa.x, this.bRh.x],
      uMouth: [Math.max(0, this.mo.x), Math.max(0.02, this.mw.x), this.ms.x, clamp(this.mr.x)],
      uMo2: [this.mx.x, this.mt.x],
      uTip: tip,
      uTip2: [0, 0],
      uArm: [this.armL.x, this.armR.x],
      uWob: [
        this.wa.x * 0.5 + 0.06 * this.lv * perk * (this.reduced ? 0 : Math.sin(t * 9)),
        this.wb.x * 0.5 + 0.05 * this.bands[1] * perk * (this.reduced ? 0 : Math.sin(t * 7)),
      ],
      uHand: [0, 0],
      uGlow: this.glow.x,
      uErr: this.err.x,
      uSpark: this.reduced ? 0 : this.spark.x,
      uLv: this.lv * perk,
      uListen: perk,
      uThink: this.think.x,
      uDone: this.done.x,
      uFaceY: this.faceY.x - 0.025 * lean,
      uCheek: this.cheek.x,
      uBlush: this.blush.x,
      uBeat: 0,
      uGlitch: 0,
      uAlpha: sm(shw / 0.35),
      uFast: fast ? 1 : 0,
      uT: t,
    };
  }
}

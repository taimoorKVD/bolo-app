import { BLOB_UNIT, type BlobUniforms } from "./blob-actor";
import { BLOB_VS, characterFragment, type CharacterKey } from "./blob-shaders";

export type BlobProgram = {
  program: WebGLProgram;
  uniforms: Record<string, WebGLUniformLocation | null>;
};

export function createBlobGl(
  canvas: HTMLCanvasElement,
  key: CharacterKey = "blob",
): { gl: WebGL2RenderingContext; program: BlobProgram } | null {
  const gl = canvas.getContext("webgl2", {
    alpha: true,
    premultipliedAlpha: true,
    antialias: false,
    depth: false,
    stencil: false,
    powerPreference: "low-power",
  });
  if (!gl) return null;
  const program = link(gl, BLOB_VS, characterFragment(key));
  if (!program) return null;
  gl.bindVertexArray(gl.createVertexArray());
  gl.disable(gl.DEPTH_TEST);
  gl.disable(gl.BLEND);
  return { gl, program };
}

function link(gl: WebGL2RenderingContext, vs: string, fs: string): BlobProgram | null {
  const program = gl.createProgram();
  if (!program) return null;
  const vert = compile(gl, gl.VERTEX_SHADER, vs);
  const frag = compile(gl, gl.FRAGMENT_SHADER, fs);
  if (!vert || !frag) return null;
  gl.attachShader(program, vert);
  gl.attachShader(program, frag);
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    console.warn("blob shader", gl.getProgramInfoLog(program));
    return null;
  }
  const uniforms: Record<string, WebGLUniformLocation | null> = {};
  const count = gl.getProgramParameter(program, gl.ACTIVE_UNIFORMS) as number;
  for (let i = 0; i < count; i++) {
    const info = gl.getActiveUniform(program, i);
    if (!info) continue;
    uniforms[info.name] = gl.getUniformLocation(program, info.name);
  }
  return { program, uniforms };
}

function compile(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader | null {
  const shader = gl.createShader(type);
  if (!shader) return null;
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    console.warn("blob shader", gl.getShaderInfoLog(shader));
    gl.deleteShader(shader);
    return null;
  }
  return shader;
}

export function resizeCanvas(canvas: HTMLCanvasElement, dpr: number): void {
  const width = Math.max(1, Math.round(canvas.clientWidth * dpr));
  const height = Math.max(1, Math.round(canvas.clientHeight * dpr));
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
}

/** Draw one frame. Returns immediately when the character is fully tucked. */
export function drawBlob(
  gl: WebGL2RenderingContext,
  program: BlobProgram,
  canvas: HTMLCanvasElement,
  uniforms: BlobUniforms,
  unit = BLOB_UNIT,
  drift = false,
  at?: { x: number; y: number },
): void {
  const w = canvas.width;
  const h = canvas.height;
  gl.viewport(0, 0, w, h);
  gl.disable(gl.SCISSOR_TEST);
  gl.clearColor(0, 0, 0, 0);
  gl.clear(gl.COLOR_BUFFER_BIT);
  if (uniforms.uAlpha <= 0.001 || w < 2 || h < 2) return;

  const cssW = canvas.clientWidth || w;
  const dpr = w / cssW;
  const show = Math.max(0.001, uniforms.shw);
  const unitPx = unit * dpr * show;
  if (at) {
    // Same placement and scissor as the v4 mockup's render():
    // `at.x` is the character's centre and `at.y` its ground line, in CSS px from the top-left.
    const cxm = at.x * dpr + (drift ? Math.sin(uniforms.uT * 0.7) * 0.18 * unitPx : 0);
    const cym = h - at.y * dpr;
    const bu = unit * dpr * Math.max(1, show);
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(Math.max(0, (at.x * dpr - 3.3 * bu) | 0), Math.max(0, (cym - 0.8 * bu) | 0), Math.ceil(6.6 * bu), Math.ceil(5 * bu));
    gl.useProgram(program.program);
    setUniform(gl, program, "uRes", [w, h]);
    setUniform(gl, program, "uC", [cxm, cym]);
    setUniform(gl, program, "uU", unitPx);
    for (const key of Object.keys(uniforms) as Array<keyof BlobUniforms>) {
      if (key === "shw") continue;
      setUniform(gl, program, key, uniforms[key]);
    }
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    gl.disable(gl.SCISSOR_TEST);
    return;
  }
  const cx = w * 0.48 + (drift ? Math.sin(uniforms.uT * 0.7) * 0.18 * unitPx : 0);
  const cy = h * 0.2;
  const bound = unit * dpr * Math.max(1, show);
  const sx = Math.max(0, Math.floor(cx - 2.15 * bound));
  const sy = Math.max(0, Math.floor(cy - 0.65 * bound));
  const sw = Math.min(w - sx, Math.ceil(4.3 * bound));
  const sh = Math.min(h - sy, Math.ceil(3.5 * bound));
  gl.enable(gl.SCISSOR_TEST);
  gl.scissor(sx, sy, Math.max(1, sw), Math.max(1, sh));
  gl.useProgram(program.program);
  setUniform(gl, program, "uRes", [w, h]);
  setUniform(gl, program, "uC", [cx, cy]);
  setUniform(gl, program, "uU", unitPx);
  for (const key of Object.keys(uniforms) as Array<keyof BlobUniforms>) {
    if (key === "shw") continue;
    setUniform(gl, program, key, uniforms[key]);
  }
  gl.drawArrays(gl.TRIANGLES, 0, 3);
  gl.disable(gl.SCISSOR_TEST);
}

function setUniform(
  gl: WebGL2RenderingContext,
  program: BlobProgram,
  name: string,
  value: number | number[],
): void {
  const loc = program.uniforms[name];
  if (!loc) return;
  if (typeof value === "number") {
    gl.uniform1f(loc, value);
    return;
  }
  if (value.length === 2) gl.uniform2f(loc, value[0] ?? 0, value[1] ?? 0);
  else if (value.length === 4) gl.uniform4f(loc, value[0] ?? 0, value[1] ?? 0, value[2] ?? 0, value[3] ?? 0);
}

export function releaseBlobGl(gl: WebGL2RenderingContext): void {
  const lose = gl.getExtension("WEBGL_lose_context");
  lose?.loseContext();
}

import { drawBlob, createBlobGl, releaseBlobGl } from "./blob-gl";
import { BlobActor } from "./blob-actor";
import type { CharacterKey } from "./blob-shaders";
import { dprFor, nextQuality, prefersReducedMotion, shapeVoice, type Quality } from "./blob-state";

/**
 * A small looping preview of one listening character.
 * It draws only while the canvas is on screen, caps at 60 fps, and drops
 * to the cheap shader if a frame runs long.
 */
export function mountLivePreview(canvas: HTMLCanvasElement, key: CharacterKey): () => void {
  const bundle = createBlobGl(canvas, key);
  if (!bundle) return () => {};
  const reduced = prefersReducedMotion();
  const actor = new BlobActor(reduced, key);
  actor.setState("listening");
  let quality: Quality = "full";
  let slow = 0;
  let fast = 0;
  let running = false;
  let raf = 0;
  let last = 0;
  let released = false;

  const visible = () => canvas.isConnected && canvas.getClientRects().length > 0 && !document.hidden;

  const stop = () => {
    running = false;
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
  };

  const release = () => {
    if (released) return;
    released = true;
    stop();
    releaseBlobGl(bundle.gl);
  };

  const frame = (now: number) => {
    if (!running || released) return;
    if (!visible()) {
      stop();
      return;
    }
    if (now - last < 15.5) {
      raf = requestAnimationFrame(frame);
      return;
    }
    const dt = Math.min(0.05, Math.max(0.001, (now - last) / 1000));
    last = now;
    const t0 = performance.now();
    const dpr = dprFor(quality, 1);
    const width = Math.max(1, Math.round(canvas.clientWidth * dpr));
    const height = Math.max(1, Math.round(canvas.clientHeight * dpr));
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }
    const level = 0.28 + 0.42 * (0.5 + 0.5 * Math.sin(actor.t * 6.2));
    const voice = shapeVoice(level, [level * 0.4, level * 0.9, level * 0.35, level * 0.1]);
    const uniforms = actor.step(dt, voice, quality === "low");
    drawBlob(bundle.gl, bundle.program, canvas, uniforms, actor.rig.unit, key === "spirit" && !reduced);
    const budget = nextQuality(quality, performance.now() - t0, slow, fast);
    quality = budget.quality;
    slow = budget.slow;
    fast = budget.fast;
    raf = requestAnimationFrame(frame);
  };

  const start = () => {
    if (running || released || !visible()) return;
    running = true;
    last = performance.now();
    raf = requestAnimationFrame(frame);
  };

  const poke = () => {
    if (released) return;
    if (visible()) start();
    else stop();
  };
  const observer = new MutationObserver(() => {
    requestAnimationFrame(poke);
  });
  const onVis = () => requestAnimationFrame(poke);
  document.addEventListener("visibilitychange", onVis);
  // The canvas is mounted before it joins the page, so wait a frame to find its panel.
  requestAnimationFrame(() => {
    const panel = canvas.closest("[data-panel]");
    if (panel) observer.observe(panel, { attributes: true, attributeFilter: ["hidden"] });
    poke();
  });

  return () => {
    observer.disconnect();
    document.removeEventListener("visibilitychange", onVis);
    release();
  };
}

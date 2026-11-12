import { resolvePack, soundUrl, type SoundCue, type SoundPackId } from "./characters";
import type { Settings } from "./types";

/** Plays the start, done, and dismiss cues for the chosen pack. */
export class CuePlayer {
  private pack: SoundPackId | null = null;
  private volume = 0.6;
  private audio = new Map<SoundCue, HTMLAudioElement>();
  private tokens = new WeakMap<HTMLAudioElement, number>();

  configure(pack: SoundPackId, volume: number): void {
    this.volume = Math.max(0, Math.min(1, Number.isFinite(volume) ? volume : 0.6));
    if (pack === this.pack) return;
    this.pack = pack;
    this.audio.clear();
    for (const cue of ["start", "done", "dismiss"] as SoundCue[]) {
      const url = soundUrl(pack, cue);
      if (!url) continue;
      const node = new Audio(url);
      node.preload = "auto";
      node.load();
      this.audio.set(cue, node);
      this.warm(node);
    }
  }

  configureSettings(settings: Partial<Settings>, characterId: string): void {
    this.configure(resolvePack(settings.soundPack, characterId), settings.soundVolume ?? 0.6);
  }

  play(cue: SoundCue): void {
    const node = this.audio.get(cue);
    if (!node || this.volume <= 0) return;
    this.tokens.set(node, (this.tokens.get(node) ?? 0) + 1);
    node.volume = cue === "start" ? this.volume * 0.45 : this.volume;
    try {
      node.pause();
      node.currentTime = 0;
    } catch {
      // The element may not be seekable until the first bytes arrive.
    }
    const pending = node.play();
    if (pending) void pending.catch(() => {});
  }

  stop(): void {
    for (const node of this.audio.values()) node.pause();
  }

  /** Decode once silently so the first real cue is not late. */
  private warm(node: HTMLAudioElement): void {
    const mine = (this.tokens.get(node) ?? 0) + 1;
    this.tokens.set(node, mine);
    node.volume = 0;
    const reset = () => {
      if (mine !== this.tokens.get(node)) return;
      node.pause();
      try {
        node.currentTime = 0;
      } catch {
        // Ignore a seek that races the first play.
      }
    };
    const pending = node.play();
    if (pending) void pending.then(reset).catch(reset);
    else reset();
  }
}

/// <reference types="vite/client" />
// Character and sound pack catalog shared by the overlay and Settings.
import type { Settings } from "./types";

const listenDefault = new URL("./assets/listen.webm", import.meta.url).href;
const writeDefault = new URL("./assets/write.webm", import.meta.url).href;
const dismissDefault = new URL("./assets/dismiss.wav", import.meta.url).href;

const clipFiles = import.meta.glob("./assets/characters/*/*.{webm,jpg}", {
  eager: true,
  query: "?url",
  import: "default",
}) as Record<string, string>;
const soundFiles = import.meta.glob("./assets/sounds/*/*.wav", {
  eager: true,
  query: "?url",
  import: "default",
}) as Record<string, string>;

export type CharacterGroup = "characters" | "outfits" | "styles" | "smart";
export type SoundPackId =
  | "soft-chime"
  | "chiptune"
  | "robot"
  | "magic"
  | "wood"
  | "paper"
  | "neon"
  | "off";
export type SoundCue = "start" | "done" | "dismiss";

export type Character = { id: string; name: string; group: CharacterGroup; pack: SoundPackId };

export const DEFAULT_CHARACTER = "classic";
export const DEFAULT_PACK = "match";

const RAW: Array<[number, string, CharacterGroup, SoundPackId]> = [
  [1, "Classic Plush", "characters", "soft-chime"],
  [2, "Headphones DJ", "outfits", "neon"],
  [3, "Scholar · Typewriter", "outfits", "wood"],
  [4, "Wizard · Magic Quill", "outfits", "magic"],
  [5, "Kitten", "characters", "soft-chime"],
  [6, "Owl", "characters", "wood"],
  [7, "Robot Buddy", "characters", "robot"],
  [8, "Fox Cub", "characters", "wood"],
  [9, "Panda", "characters", "soft-chime"],
  [10, "Aura Orb", "styles", "magic"],
  [11, "Retro Mic", "characters", "neon"],
  [12, "Dream Cloud", "characters", "magic"],
  [13, "Detective", "outfits", "paper"],
  [14, "Astronaut", "outfits", "robot"],
  [15, "Chef", "outfits", "wood"],
  [16, "Pilot", "outfits", "robot"],
  [17, "Night Owl Mode", "outfits", "soft-chime"],
  [18, "Reporter", "outfits", "paper"],
  [19, "Pixel-Art 8-bit", "styles", "chiptune"],
  [20, "Paper Cutout", "styles", "paper"],
  [21, "Neon Outline", "styles", "neon"],
  [22, "Claymation", "styles", "wood"],
  [23, "Mood Mirror", "smart", "soft-chime"],
  [24, "Language Costume", "smart", "soft-chime"],
  [25, "Long-Talk Tea", "smart", "wood"],
  [26, "Time of Day", "smart", "soft-chime"],
  [27, "Eid Skin", "smart", "magic"],
  [28, "Laptop + Paper Plane", "styles", "paper"],
];

export const CHARACTERS: Character[] = RAW.map(([n, name, group, pack]) => ({
  id: n === 1 ? DEFAULT_CHARACTER : `c${String(n).padStart(2, "0")}`,
  name,
  group,
  pack,
}));

export const GROUPS: Array<[CharacterGroup, string]> = [
  ["characters", "Characters"],
  ["outfits", "Outfits"],
  ["styles", "Styles"],
  ["smart", "Smart & Seasonal"],
];

export const SOUND_PACKS: Array<[SoundPackId | "match", string, string]> = [
  ["match", "Match character", "Each character brings its own sounds"],
  ["soft-chime", "Soft Chime", "Warm and quiet. The classic Bolo bell"],
  ["chiptune", "Chiptune", "Tiny 8-bit arpeggios"],
  ["robot", "Robot Beeps", "Short friendly beeps"],
  ["magic", "Magic Sparkle", "High glassy twinkles"],
  ["wood", "Wooden Tap", "Soft marimba knocks"],
  ["paper", "Paper Rustle", "Light paper flicks"],
  ["neon", "Neon Synth", "Mellow retro synth"],
  ["off", "Off", "No sounds"],
];

export function findCharacter(id: string | undefined | null): Character {
  return CHARACTERS.find((c) => c.id === id) ?? CHARACTERS[0]!;
}

export function clipUrls(id: string): { listen: string; write: string; thumb: string } {
  const thumb = clipFiles[`./assets/characters/${id === DEFAULT_CHARACTER ? "classic" : id}/thumb.jpg`] ?? "";
  if (id === DEFAULT_CHARACTER) return { listen: listenDefault, write: writeDefault, thumb };
  const listen = clipFiles[`./assets/characters/${id}/listen.webm`];
  const write = clipFiles[`./assets/characters/${id}/write.webm`];
  if (!listen || !write) return { listen: listenDefault, write: writeDefault, thumb };
  return { listen, write, thumb };
}

export function resolvePack(pack: string | undefined, characterId: string): SoundPackId {
  if (!pack || pack === "match") return findCharacter(characterId).pack;
  return (SOUND_PACKS.some(([id]) => id === pack) ? pack : "soft-chime") as SoundPackId;
}

export function soundUrl(pack: SoundPackId, cue: SoundCue): string | null {
  if (pack === "off") return null;
  if (pack === "soft-chime" && cue === "dismiss") return dismissDefault;
  return soundFiles[`./assets/sounds/${pack}/${cue}.wav`] ?? null;
}

// Eid al-Fitr and Eid al-Adha (first day, approximate; local calendars can shift by a day).
const EID_DAYS = [
  "2026-03-20", "2026-05-27", "2027-03-10", "2027-05-17",
  "2028-02-27", "2028-05-05", "2029-02-15", "2029-04-24",
];

export function isEid(now: Date): boolean {
  const day = Date.UTC(now.getFullYear(), now.getMonth(), now.getDate());
  return EID_DAYS.some((d) => {
    const start = Date.parse(`${d}T00:00:00Z`);
    return day >= start && day < start + 3 * 86_400_000;
  });
}

export function isNight(now: Date): boolean {
  const h = now.getHours();
  return h >= 21 || h < 6;
}

/** The character the overlay should show right now, after smart rules. */
export function activeCharacter(settings: Partial<Settings>, now = new Date()): string {
  if (settings.smartHoliday && isEid(now)) return "c27";
  if (settings.smartTimeOfDay && isNight(now)) return "c17";
  return findCharacter(settings.character).id;
}

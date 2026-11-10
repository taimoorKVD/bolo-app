export const SCRIPT_SETS: Array<{ id: string; label: string }> = [
  { id: "original", label: "Original 30 — English" },
  { id: "languages", label: "Urdu, Hindi, and mixed (32)" },
];

export const BUCKET_LABEL: Record<string, string> = {
  en: "English",
  ur: "Urdu",
  mixed: "Mixed",
  hi: "Hindi",
};

export function scriptSetId(value: string | null | undefined): string {
  return SCRIPT_SETS.some((set) => set.id === value) ? (value as string) : "original";
}

export function sentenceCue(bucket: string | null | undefined, cue: string): string {
  const label = bucket ? BUCKET_LABEL[bucket] ?? bucket : "";
  const note = cue.trim();
  if (label && note) return `${label} · ${note}`;
  return label || note;
}

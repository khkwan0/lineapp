const STORAGE_KEY = "lineapp.appearance";

const SCRIPT =
  '"Noto Sans Thai", "Noto Sans CJK JP", "Noto Sans CJK KR", "Noto Sans CJK SC", "Noto Sans CJK TC", "Noto Naskh Arabic", "Noto Sans Arabic", "Noto Sans Devanagari"';

export const FONT_IDS = ["noto", "system", "serif", "mono"] as const;

export type FontId = (typeof FONT_IDS)[number];

const STACKS: Record<FontId, string> = {
  noto: `"Noto Sans", ${SCRIPT}, "Segoe UI", sans-serif`,
  system: `system-ui, "Segoe UI", Ubuntu, Cantarell, ${SCRIPT}, sans-serif`,
  serif: `"Noto Serif", "Noto Serif CJK JP", "Noto Serif CJK KR", "Noto Serif CJK SC", "Noto Serif CJK TC", "Noto Naskh Arabic", Georgia, "Times New Roman", serif`,
  mono: `ui-monospace, "Cascadia Mono", "Sarasa Mono", "Noto Sans Mono", ${SCRIPT}, monospace`,
};

export const TEXT_SCALES = [0.875, 1, 1.125, 1.25, 1.5] as const;

export type Appearance = {
  font: FontId;
  scaleIndex: number;
};

const DEFAULTS: Appearance = { font: "noto", scaleIndex: 1 };

function isFontId(value: unknown): value is FontId {
  return typeof value === "string" && (FONT_IDS as readonly string[]).includes(value);
}

export function fontStack(font: FontId): string {
  return STACKS[font];
}

export function readAppearance(): Appearance {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) {
      return { ...DEFAULTS };
    }
    const parsed = JSON.parse(raw) as { font?: unknown; scaleIndex?: unknown };
    const font = isFontId(parsed.font) ? parsed.font : DEFAULTS.font;
    const scaleIndex =
      typeof parsed.scaleIndex === "number" && parsed.scaleIndex >= 0 && parsed.scaleIndex < TEXT_SCALES.length
        ? Math.round(parsed.scaleIndex)
        : DEFAULTS.scaleIndex;
    return { font, scaleIndex };
  } catch {
    return { ...DEFAULTS };
  }
}

export function applyAppearance(prefs: Appearance = readAppearance()): void {
  const root = document.documentElement;
  root.style.setProperty("--font-family", STACKS[prefs.font]);
  root.style.setProperty("--text-scale", String(TEXT_SCALES[prefs.scaleIndex]));
}

export function saveAppearance(prefs: Appearance): void {
  const font = isFontId(prefs.font) ? prefs.font : DEFAULTS.font;
  const scaleIndex = Math.min(TEXT_SCALES.length - 1, Math.max(0, Math.round(prefs.scaleIndex)));
  const next = { font, scaleIndex };
  localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  applyAppearance(next);
}

applyAppearance();

window.addEventListener("storage", (event) => {
  if (event.key === STORAGE_KEY) {
    applyAppearance();
  }
});

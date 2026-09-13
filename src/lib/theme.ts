// Never hardcode a hex value (CLAUDE.md non-negotiable). design/tokens.json is
// the machine-readable source of truth; two themes ship and skins are
// user-loadable, so a literal in a component is a bug waiting for v0.5.
import tokens from "../../design/tokens.json";
import { legible, rampFrom } from "./palette";
import type { Token } from "./skin";

export type ThemeName = keyof typeof tokens.themes;

/**
 * The themes a person can pick (#147). Cone (#85) is Eyewall's colours with
 * the radar behind the classic windows.
 */
export const WEARABLE = ["eyewall", "purricane", "cone"] as const satisfies readonly ThemeName[];
export type Wearable = (typeof WEARABLE)[number];

export function isWearable(name: unknown): name is Wearable {
  return typeof name === "string" && (WEARABLE as readonly string[]).includes(name);
}

/** A theme's own name for itself, for the picker. */
export function themeLabel(name: ThemeName): string {
  return (tokens.themes as Record<string, { name?: string }>)[name]?.name ?? name;
}

/** Resolve a theme's colors, following `extends` (Cone builds on Eyewall). */
export function colorsFor(name: ThemeName): Record<string, string> {
  const themes = tokens.themes as Record<string, any>;
  const theme = themes[name];
  const base = theme.extends ? colorsFor(theme.extends as ThemeName) : {};
  return { ...base, ...(theme.colors ?? {}) };
}

/** Resolve a theme's typefaces (`type.chrome`, `type.ui`), following
 * `extends` like the colours. */
export function typeFor(name: ThemeName): Record<string, string> {
  const themes = tokens.themes as Record<string, any>;
  const theme = themes[name];
  const base = theme.extends ? typeFor(theme.extends as ThemeName) : {};
  const own: Record<string, string> = {};
  for (const [role, family] of Object.entries(theme.type ?? {})) {
    if (role !== "$comment" && typeof family === "string") own[role] = family;
  }
  return { ...base, ...own };
}

/**
 * The six a theme is worn in: its colours as written, made legible (#147).
 * The tokens keep what was designed; a window paints this.
 */
export function colorsShown(name: ThemeName): Record<Token, string> {
  return legible(colorsFor(name) as Record<Token, string>);
}

/** How much larger than written a theme sets the classic windows' system
 * fonts (`type.scale`): 1 unless it says. Comic Sans sits small beside
 * Iosevka at the same size (`docs/purricane.md`). */
export function typeScale(name: ThemeName): number {
  const themes = tokens.themes as Record<string, any>;
  let t = themes[name];
  while (t && typeof t.type?.scale !== "number" && t.extends) t = themes[t.extends];
  const s = t?.type?.scale;
  return typeof s === "number" && s > 0.5 && s < 2 ? s : 1;
}

/** Push a theme onto :root: --token custom properties for the six colours,
 * --type-chrome / --type-ui for its typefaces, so a skin's `system` font is
 * the theme's face and never names one itself (skin-manifest.md, D92), and
 * --type-scale for how large it sits. */
export function applyTheme(name: ThemeName = "eyewall") {
  const c = colorsShown(name);
  for (const [token, hex] of Object.entries(c)) {
    document.documentElement.style.setProperty(`--${token}`, hex);
  }
  for (const [role, family] of Object.entries(typeFor(name))) {
    document.documentElement.style.setProperty(`--type-${role}`, JSON.stringify(family));
  }
  document.documentElement.style.setProperty("--type-scale", String(typeScale(name)));
}

/**
 * The six a classic window paints while it wears this skin (D101). The theme
 * owns the app's palette; **a skin whose art is `final` is the exception**,
 * because its pixels cannot be tinted, so the colours that sit beside them —
 * the playlist's rows, the seam, the words these windows draw — are its own.
 * A `mask` skin's palette stays validated and ignored, which is what lets one
 * grey sheet wear whichever theme is on.
 */
export function colorsWorn(
  skin: { art: string; colors?: "own" | "theme"; palette: Record<string, string> },
  name: ThemeName = "eyewall",
): Record<string, string> {
  return ownsColours(skin) ? { ...skin.palette } : colorsShown(name);
}

/** Whether a skin paints its own colours or the theme's. `colors` says it
 * outright (D122); without it, D101's rule is the answer. */
export function ownsColours(skin: { art: string; colors?: "own" | "theme" }): boolean {
  return skin.colors ? skin.colors === "own" : skin.art === "final";
}

/**
 * The analyser's ramp a window draws while it wears this skin (#147): the
 * skin's own for a skin that paints its own colours, and the theme's
 * otherwise. A theme with no ramp in the tokens, as Purricane has none, gets
 * one made from its colours the way a made skin's is.
 */
export function rampWorn(
  skin: { art: string; colors?: "own" | "theme"; viscolor: string[] },
  name: ThemeName = "eyewall",
): string[] {
  if (ownsColours(skin)) return skin.viscolor;
  const own = viscolor(name);
  return own.length === 24 ? own : rampFrom(colorsShown(name));
}

/**
 * Which analyser Main draws in its bars position (#147): the skin's
 * `visualizer.component` for a skin that paints its own colours, and the
 * theme's for one that follows the theme. The analyser goes with the colours
 * because it is drawn in them: Purricane's kaleidoscope on a skin that wears
 * Purricane, a made skin's bars in the picture's ramp.
 */
export function visualizerFor(
  skin: { art: string; colors?: "own" | "theme"; visualizer: { component: string } },
  name: ThemeName = "eyewall",
): string {
  if (ownsColours(skin)) return skin.visualizer.component;
  return themeVisualizer(name);
}

/** The analyser a theme names, `spectrum-bars` when it names none. */
export function themeVisualizer(name: ThemeName): string {
  const themes = tokens.themes as Record<string, any>;
  let t = themes[name];
  while (t && !t.visualizer?.component && t.extends) t = themes[t.extends];
  return t?.visualizer?.component ?? "spectrum-bars";
}

/** The kaleidoscope's settings from a theme's tokens, with the app's caps
 * already applied where the tokens could ask for more (`docs/purricane.md`:
 * the clamps are the app's, not the theme's). */
export function kaleidoscopeFor(name: ThemeName): { segments: 6 | 8; degPerSec: number; maxBloomHz: number } {
  const v = (tokens.themes as Record<string, any>)[name]?.visualizer ?? {};
  const segments = v.segments === 8 ? 8 : 6;
  const degPerSec = Math.min(10, Math.max(0, Number(v.rotationDegPerSec) || 4));
  const maxBloomHz = Math.min(3, Math.max(0.1, Number(v.accessibility?.maxBloomHz) || 3));
  return { segments, degPerSec, maxBloomHz };
}

/** The 24-step radar reflectivity ramp. One array, several consumers. */
export function viscolor(name: ThemeName = "eyewall"): string[] {
  const themes = tokens.themes as Record<string, any>;
  let t = themes[name];
  while (t && !t.visualizer?.palette && t.extends) t = themes[t.extends];
  return t?.visualizer?.palette ?? [];
}

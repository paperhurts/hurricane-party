// Never hardcode a hex value (CLAUDE.md non-negotiable). design/tokens.json is
// the machine-readable source of truth; two themes ship and skins are
// user-loadable, so a literal in a component is a bug waiting for v0.5.
import tokens from "../../design/tokens.json";

export type ThemeName = keyof typeof tokens.themes;

/** Resolve a theme's colors, following `extends` (Cone builds on Eyewall). */
export function colorsFor(name: ThemeName): Record<string, string> {
  const themes = tokens.themes as Record<string, any>;
  const theme = themes[name];
  const base = theme.extends ? colorsFor(theme.extends as ThemeName) : {};
  return { ...base, ...(theme.colors ?? {}) };
}

/** Push a theme onto :root as --token custom properties. */
export function applyTheme(name: ThemeName = "eyewall") {
  const c = colorsFor(name);
  for (const [token, hex] of Object.entries(c)) {
    document.documentElement.style.setProperty(`--${token}`, hex);
  }
}

/** The 24-step radar reflectivity ramp. One array, several consumers. */
export function viscolor(name: ThemeName = "eyewall"): string[] {
  const themes = tokens.themes as Record<string, any>;
  let t = themes[name];
  while (t && !t.visualizer?.palette && t.extends) t = themes[t.extends];
  return t?.visualizer?.palette ?? [];
}

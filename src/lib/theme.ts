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

/** Push a theme onto :root: --token custom properties for the six colours,
 * and --type-chrome / --type-ui for its typefaces, so a skin's `system` font
 * is the theme's face and never names one itself (skin-manifest.md, D92). */
export function applyTheme(name: ThemeName = "eyewall") {
  const c = colorsFor(name);
  for (const [token, hex] of Object.entries(c)) {
    document.documentElement.style.setProperty(`--${token}`, hex);
  }
  for (const [role, family] of Object.entries(typeFor(name))) {
    document.documentElement.style.setProperty(`--type-${role}`, JSON.stringify(family));
  }
}

/** The 24-step radar reflectivity ramp. One array, several consumers. */
export function viscolor(name: ThemeName = "eyewall"): string[] {
  const themes = tokens.themes as Record<string, any>;
  let t = themes[name];
  while (t && !t.visualizer?.palette && t.extends) t = themes[t.extends];
  return t?.visualizer?.palette ?? [];
}

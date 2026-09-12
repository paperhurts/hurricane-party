// Six roles out of a picture (#131). The skin maker's whole palette step: a
// person picks an image, and this finds a ground, a recessed surface, text, an
// accent, an alert and a warning that belong to that image and can still be
// read.
//
// Legible, not dark. Dark-only is about what this app ships, not what people
// make (owner, 2026-09-11): a bright photo gets a bright skin, and the only
// thing enforced is that the words and the live edge stand off their ground.
//
// Everything is done in OKLab, where distance tracks what an eye sees and a
// lightness change does not drag the hue with it — which is what lets a colour
// be nudged until it is readable without turning into a different colour.
// Pure: pixels in, colours out. Decoding the picture is the caller's.

import type { Token } from "./skin";

/** An OKLab colour: lightness 0..1, and two opponent axes around 0. */
type Lab = [number, number, number];

/** Contrast this module guarantees against the ground (WCAG 2 ratios). */
export const TEXT_CONTRAST = 4.5;
export const EDGE_CONTRAST = 3;

// ---- colour spaces ----

const toLinear = (c: number) => {
  const v = c / 255;
  return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
};
const fromLinear = (c: number) => {
  const v = c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055;
  return Math.round(Math.min(1, Math.max(0, v)) * 255);
};

function toLab(r: number, g: number, b: number): Lab {
  const R = toLinear(r);
  const G = toLinear(g);
  const B = toLinear(b);
  const l = Math.cbrt(0.4122214708 * R + 0.5363325363 * G + 0.0514459929 * B);
  const m = Math.cbrt(0.2119034982 * R + 0.6806995451 * G + 0.1073969566 * B);
  const s = Math.cbrt(0.0883024619 * R + 0.2817188376 * G + 0.6299787005 * B);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

function toRgb([L, a, b]: Lab): [number, number, number] {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    fromLinear(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    fromLinear(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    fromLinear(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
  ];
}

export function hex(lab: Lab): string {
  return "#" + toRgb(lab).map((c) => c.toString(16).padStart(2, "0").toUpperCase()).join(""); // tokens-exempt: formats a computed colour as the hex a manifest stores
}

function labOf(hexColour: string): Lab {
  const n = parseInt(hexColour.slice(1), 16);
  return toLab((n >> 16) & 255, (n >> 8) & 255, n & 255);
}

const chroma = ([, a, b]: Lab) => Math.hypot(a, b);

/** How colourful a hex colour is, in OKLab chroma: about 0 for greys, 0.3 for
 * the most saturated colours a screen shows. */
export const chromaOf = (h: string) => chroma(labOf(h));
const hueOf = ([, a, b]: Lab) => (Math.atan2(b, a) * 180) / Math.PI;
const hueGap = (x: Lab, y: Lab) => {
  const d = Math.abs(hueOf(x) - hueOf(y)) % 360;
  return d > 180 ? 360 - d : d;
};

/** WCAG 2 relative luminance of a hex colour, 0..1. */
function luminance(h: string): number {
  const n = parseInt(h.slice(1), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map(toLinear);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG 2 contrast ratio between two colours, 1..21. */
export function contrast(x: string, y: string): number {
  const [a, b] = [luminance(x), luminance(y)].sort((p, q) => q - p);
  return (a + 0.05) / (b + 0.05);
}

/**
 * Whether colours should get lighter to stand off this ground, rather than
 * darker. Not "is the ground dark": a mid grey reaches only 3.9:1 against
 * pure white but 5.3:1 against black, so lighter words could never be read on
 * it however far they went. Lighter wins exactly when white out-contrasts
 * black, which is when the ground's luminance is at or below 0.179.
 */
const lighterWins = (ground: Lab) => luminance(hex(ground)) <= 0.1791;

// ---- finding the picture's colours ----

/** The picture's distinct colours, most common first: k-means in OKLab over a
 * sample of its opaque pixels. Seeded from evenly spaced lightness so the
 * same picture always gives the same answer. */
function clusters(pixels: Uint8ClampedArray, k: number): { lab: Lab; weight: number }[] {
  const samples: Lab[] = [];
  const stride = Math.max(1, Math.floor(pixels.length / 4 / 4096)) * 4;
  for (let i = 0; i + 3 < pixels.length; i += stride) {
    if (pixels[i + 3] < 128) continue; // a transparent pixel is not a colour the picture shows
    samples.push(toLab(pixels[i], pixels[i + 1], pixels[i + 2]));
  }
  if (samples.length === 0) return [];
  const sorted = [...samples].sort((p, q) => p[0] - q[0]);
  const n = Math.min(k, sorted.length);
  let centres: Lab[] = Array.from({ length: n }, (_, i) => [...sorted[Math.floor(((i + 0.5) / n) * sorted.length)]] as Lab);
  let owner = new Int32Array(samples.length);
  for (let round = 0; round < 16; round++) {
    let moved = false;
    samples.forEach((s, i) => {
      let best = 0;
      let bestD = Infinity;
      centres.forEach((c, j) => {
        const d = (s[0] - c[0]) ** 2 + (s[1] - c[1]) ** 2 + (s[2] - c[2]) ** 2;
        if (d < bestD) {
          bestD = d;
          best = j;
        }
      });
      if (owner[i] !== best) moved = true;
      owner[i] = best;
    });
    const sums = centres.map(() => [0, 0, 0, 0]);
    samples.forEach((s, i) => {
      const t = sums[owner[i]];
      t[0] += s[0];
      t[1] += s[1];
      t[2] += s[2];
      t[3] += 1;
    });
    centres = centres.map((c, j) => (sums[j][3] ? [sums[j][0] / sums[j][3], sums[j][1] / sums[j][3], sums[j][2] / sums[j][3]] : c)) as Lab[];
    if (!moved && round > 0) break;
  }
  const weights = centres.map(() => 0);
  owner.forEach((j) => (weights[j] += 1));
  return centres
    .map((lab, j) => ({ lab, weight: weights[j] / samples.length }))
    .filter((c) => c.weight > 0)
    .sort((p, q) => q.weight - p.weight);
}

/** Move a colour's lightness away from its grounds, keeping its hue and most
 * of its chroma, until it reaches `ratio` against every one of them. A colour
 * already there is returned as it is: the picture's own colours are the point.
 * The direction is fixed by the main ground, so two grounds can never pull the
 * same colour opposite ways. */
function standOff(c: Lab, grounds: Lab[], ratio: number): Lab {
  const gs = grounds.map(hex);
  const worst = (x: Lab) => Math.min(...gs.map((g) => contrast(hex(x), g)));
  if (worst(c) >= ratio) return c;
  const up = lighterWins(grounds[0]);
  const out: Lab = [...c];
  for (let step = 0; step < 60 && worst(out) < ratio; step++) {
    out[0] = Math.min(0.99, Math.max(0.02, out[0] + (up ? 0.02 : -0.02)));
    // Very light and very dark colours cannot hold much chroma; let it fall
    // rather than clip into a different hue.
    const room = Math.min(out[0], 1 - out[0]) * 0.6;
    const cNow = chroma(out);
    if (cNow > room) {
      out[1] *= room / cNow;
      out[2] *= room / cNow;
    }
  }
  return out;
}

/** A colour at a given OKLab hue (degrees), lightness and chroma. */
const atHue = (L: number, C: number, deg: number): Lab => [L, C * Math.cos((deg * Math.PI) / 180), C * Math.sin((deg * Math.PI) / 180)];

export type MadePalette = { palette: Record<Token, string>; viscolor: string[] };

/**
 * The six roles, and a 24-step analyser ramp, for a picture's pixels (RGBA,
 * as `getImageData` hands them over).
 *
 * - **ground** is the colour the picture has most of, so the chrome sits on the
 *   picture's own field.
 * - **surface** is the ground one step recessed, the wells and troughs.
 * - **text** is the picture's colour that stands furthest off the ground, made
 *   readable at 4.5:1 if none already is.
 * - **accent** is the most vivid colour that is not the ground, at 3:1.
 * - **alert** is the next vivid colour a clear hue away from the accent, or the
 *   accent's opposite when the picture has only one.
 * - **warn** is the picture's warm colour, or an amber made to fit.
 */
export function paletteFromPixels(pixels: Uint8ClampedArray): MadePalette {
  const found = clusters(pixels, 8);
  // The ground is a calm field the picture actually has — near black, near
  // white, or greyed — when it has a real one. The most common colour is not
  // enough: a subject on a transparent background makes its own orange coat
  // the most common opaque colour, and a loud ground forces every accent to
  // go dark and muddy to stand off it (tried on the repo's capybara art).
  // Failing a field, the ground is made: the picture's dominant hue, dark and
  // barely tinted, so the picture's vivid colours can stay vivid on it.
  const calm = (c: Lab) => chroma(c) < 0.06 || c[0] < 0.25 || c[0] > 0.92;
  const field = found.find((c) => calm(c.lab) && c.weight >= 0.2);
  const lead = found[0]?.lab;
  const ground: Lab = field
    ? field.lab
    : lead
      ? atHue(0.2, Math.min(0.035, chroma(lead)), hueOf(lead))
      : [0.16, 0.01, -0.02];
  const rest = found.filter((c) => c !== field).map((c) => c.lab);

  // The well sits a step away from the words: darker under light text, as
  // Eyewall's does, and lighter under dark text — on a mid-tone ground a
  // darker well under dark words leaves no text that can read on both. A
  // ground too near white to lighten gets the darker step, which dark words
  // still stand well off.
  const wordsUp = lighterWins(ground);
  const wellL = wordsUp ? ground[0] - 0.04 : ground[0] + 0.04 <= 0.97 ? ground[0] + 0.04 : ground[0] - 0.04;
  const surface: Lab = [Math.min(0.99, Math.max(0.02, wellL)), ground[1] * 0.8, ground[2] * 0.8];

  // Each picture colour plays one role. Without this the cooler capybara's
  // cream jacket was both the words and the alert, two roles in one colour.
  const dist = (x: Lab, y: Lab) => Math.hypot(x[0] - y[0], x[1] - y[1], x[2] - y[2]);
  const apart = (c: Lab, taken: Lab[]) => taken.every((t) => dist(c, t) >= 0.1);
  const onGround = (c: Lab) => contrast(hex(c), hex(ground));

  // Words want the picture's quiet colour that stands furthest off the ground,
  // so its vivid ones are left for the roles that are meant to be seen. A
  // picture with no second colour still gets words: the ground's own hue at
  // the far end of the lightness scale.
  const quiet = rest.filter((c) => chroma(c) < 0.08).sort((p, q) => onGround(q) - onGround(p));
  const loud = [...rest].sort((p, q) => onGround(q) - onGround(p));
  const textSeed: Lab = quiet[0] ?? loud[0] ?? [lighterWins(ground) ? 0.95 : 0.2, ground[1] * 0.3, ground[2] * 0.3];
  // On the ground and on the recessed surface both: the rows sit on the latter.
  const text = standOff(textSeed, [ground, surface], TEXT_CONTRAST);

  const vivid = rest.filter((c) => c !== textSeed).sort((p, q) => chroma(q) - chroma(p));
  const accentSeed: Lab =
    vivid.find((c) => chroma(c) >= 0.03 && apart(c, [text])) ??
    atHue(lighterWins(ground) ? 0.8 : 0.45, 0.13, hueOf(ground) + 180);
  const accent = standOff(accentSeed, [ground], EDGE_CONTRAST);

  const alertSeed: Lab =
    vivid.find((c) => c !== accentSeed && chroma(c) > 0.05 && hueGap(c, accent) >= 50 && apart(c, [text, accent])) ??
    atHue(accent[0], Math.max(0.1, chroma(accent)), hueOf(accent) + 150);
  const alert = standOff(alertSeed, [ground], EDGE_CONTRAST);

  // Warm: OKLab puts red near 30 degrees, orange near 60 and yellow near 100.
  // A warning that is the accent's twin warns of nothing, so it has to stand
  // apart from every role already given out.
  const warmSeed: Lab =
    vivid.find(
      (c) => c !== accentSeed && c !== alertSeed && chroma(c) > 0.05 && hueOf(c) >= 25 && hueOf(c) <= 110 && apart(c, [text, accent, alert]),
    ) ?? atHue(lighterWins(ground) ? 0.8 : 0.45, 0.14, hueGap(accent, atHue(0.5, 0.1, 65)) < 30 ? 95 : 65);
  const warn = standOff(warmSeed, [ground], EDGE_CONTRAST);

  const palette: Record<Token, string> = {
    ground: hex(ground),
    surface: hex(surface),
    text: hex(text),
    accent: hex(accent),
    alert: hex(alert),
    warn: hex(warn),
  };
  return { palette, viscolor: rampFrom(palette) };
}

/** 24 steps from the recessed surface up through the accent to the alert,
 * with lightness rising to the peak, so the analyser's quiet bars sit in the
 * window and its loud ones reach for the picture's brightest colour. */
export function rampFrom(p: Record<Token, string>): string[] {
  const stops = [labOf(p.surface), labOf(p.accent), labOf(p.alert)];
  const mix = (x: Lab, y: Lab, t: number): Lab => [x[0] + (y[0] - x[0]) * t, x[1] + (y[1] - x[1]) * t, x[2] + (y[2] - x[2]) * t];
  return Array.from({ length: 24 }, (_, i) => {
    const t = i / 23;
    const lab = t < 0.5 ? mix(stops[0], stops[1], t * 2) : mix(stops[1], stops[2], (t - 0.5) * 2);
    // The first steps stay near the surface; lift them enough to show.
    const floor = stops[0][0] + 0.08;
    return hex([Math.max(floor * (1 - t) + lab[0] * t, lab[0]), lab[1], lab[2]]);
  });
}

// A companion to paint (#208, D163): the companion's sheet, blank, in the
// format's layout, and a guide the same size that names every row. The
// sibling of the skin template (template.ts, D129), for true pixel art: a
// cell is 64 px, which is the companion's own size, so a painter in Aseprite
// draws every pixel at 1:1 and the app doubles it for 2x windows (D160).
//
// Nothing is decided here about which frames a companion has. The template's
// companion.json says `"painted": true`, and the import reads the painted
// cells off the sheet (painted.rs).

import type { Token } from "./skin";

/** The format's states, in the sheet's row order (purricane.md). */
export const COMPANION_STATES = ["idle", "sleep", "dance", "walk", "startle", "pet", "carry"] as const;
export const COMPANION_CELL = 64;
export const COMPANION_COLUMNS = 8;

/** How many frames each state wants (companion-art.md), shown on the guide. */
export const SUGGESTED: Record<(typeof COMPANION_STATES)[number], number> = {
  idle: 2,
  sleep: 2,
  dance: 4,
  walk: 4,
  startle: 2,
  pet: 2,
  carry: 2,
};

/** What each row is, for the README. */
const DOES: Record<(typeof COMPANION_STATES)[number], string> = {
  idle: "standing still; a second frame is a blink or a small shift",
  sleep: "curled up asleep; two frames breathe",
  dance: "a pose on each beat of the music",
  walk: "a walk cycle, facing right",
  startle: "jumping when the window under it moves: in the air, then landing",
  pet: "leaning into a click",
  carry: "hanging from the pointer when it is dragged",
};

/** The template's manifest: a name to change, and the flag the import reads. */
export function companionTemplateManifest(name = "My companion"): Record<string, unknown> {
  return { format: "hp-companion/1", name, sprite: "sheet.png", painted: true };
}

export function companionTemplateReadme(): string {
  const w = COMPANION_COLUMNS * COMPANION_CELL;
  const h = COMPANION_STATES.length * COMPANION_CELL;
  const rows = COMPANION_STATES.map(
    (s, i) => `  ${i + 1}  ${s.padEnd(8)} ${DOES[s]} (${SUGGESTED[s]} suggested)`,
  );
  const lines = [
    "A companion to paint for hurricane-party",
    "",
    `sheet.png is your companion: ${COMPANION_COLUMNS} cells across and ${COMPANION_STATES.length} rows down, each cell`,
    `${COMPANION_CELL} x ${COMPANION_CELL} pixels (${w} x ${h} in all). Each row is one thing it does:`,
    "",
    ...rows,
    "",
    "Only idle is needed: a row left empty uses idle instead. A row's frames are",
    "its painted cells, from the left, so paint left to right with no gaps.",
    "",
    "One pose per cell. Face right (it turns around by itself to walk left),",
    "stand on the bottom row of the cell (the ground line in guide.png), and",
    "keep to the middle mark. Leave the background transparent.",
    "",
    "guide.png is the same size, with the rows named, the suggested cells shaded,",
    "and the ground line and the middle marked. Use it as a layer under your",
    "painting; the app never reads it.",
    "",
    "In Aseprite: open sheet.png. View > Grid > Grid Settings, set 64 x 64, and",
    "turn on View > Snap to Grid. Add the guide with Layer > New > New Reference",
    "Layer from File, and pick guide.png. Draw at 1:1: every pixel you place is",
    "one pixel of the companion. Beside the player's 2x windows it is drawn twice",
    "as big, pixel for pixel, so it stays crisp.",
    "",
    `Painting somewhere else, bigger? A sheet of ${2 * w} x ${2 * h} (${2 * COMPANION_CELL} px cells) works too.`,
    "",
    'Give it a name: change "name" in companion.json.',
    "",
    "When it is ready, open hurricane-party's library, press Import companion...,",
    "and pick this folder's companion.json. It becomes the one the companion box starts.",
    "To share it, zip this folder; a zip imports the same way.",
    "",
  ];
  return lines.join("\r\n");
}

// ---- drawing, in the webview ----

async function png(c: OffscreenCanvas): Promise<Uint8Array> {
  return new Uint8Array(await (await c.convertToBlob({ type: "image/png" })).arrayBuffer());
}

/** The sheet to paint: the template's size, every pixel transparent. */
export async function blankCompanionSheet(): Promise<Uint8Array> {
  const c = new OffscreenCanvas(COMPANION_COLUMNS * COMPANION_CELL, COMPANION_STATES.length * COMPANION_CELL);
  // A canvas with no context yet refuses convertToBlob.
  c.getContext("2d");
  return png(c);
}

/**
 * The guide, the sheet's own size so it can sit under it as a layer: every
 * cell outlined, the suggested cells shaded, the ground line along each
 * cell's bottom row and a mark at its middle, and each row named in its
 * first cell.
 */
export async function companionGuide(palette: Record<Token, string>): Promise<Uint8Array> {
  const cell = COMPANION_CELL;
  const c = new OffscreenCanvas(COMPANION_COLUMNS * cell, COMPANION_STATES.length * cell);
  const g = c.getContext("2d")!;
  g.fillStyle = palette.ground;
  g.fillRect(0, 0, c.width, c.height);
  g.font = "9px ui-monospace, Consolas, monospace";
  g.textBaseline = "top";
  COMPANION_STATES.forEach((state, r) => {
    const y = r * cell;
    for (let col = 0; col < COMPANION_COLUMNS; col++) {
      const x = col * cell;
      const suggested = col < SUGGESTED[state];
      // A suggested cell is lifted and outlined in the accent; the rest are
      // only outlined, faintly. (A theme's surface can be darker than its
      // ground, so a lift is drawn with the text colour, not the surface.)
      if (suggested) {
        g.fillStyle = palette.text;
        g.globalAlpha = 0.08;
        g.fillRect(x, y, cell, cell);
      }
      g.strokeStyle = suggested ? palette.accent : palette.text;
      g.globalAlpha = suggested ? 0.6 : 0.2;
      g.strokeRect(x + 0.5, y + 0.5, cell - 1, cell - 1);
      g.globalAlpha = 1;
      // The ground line: the cell's bottom row, where the feet go.
      g.fillStyle = palette.accent;
      g.fillRect(x, y + cell - 1, cell, 1);
      // The middle: where the feet are centred.
      g.fillRect(x + cell / 2, y + cell - 8, 1, 7);
    }
    g.fillStyle = palette.text;
    g.fillText(`${state.toUpperCase()} ${SUGGESTED[state]}`, 3, y + 3);
  });
  return png(c);
}

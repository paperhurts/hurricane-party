# hurricane-party — Making the art for a companion pack

How a list of poses becomes a sprite sheet the app can load, whether the frames come out of an image model or off a friend's desk. The pack format itself is in `purricane.md` (`hp-companion/1`); this is the part before it, and it applies to the captain as much as to the kittens.

---

## What the sheet has to be

- **One PNG, transparent background**, a grid of square cells. `frameSize` is the cell (32 for the kittens; the captain with the jacket, the boombox and the seagull wants **64**, which shows at 128 px on a 2x desktop, about the height of Main's clock).
- **One row per state, eight cells per row**, in the format's order: `idle`, `sleep`, `dance`, `walk`, `startle`, `pet`, `carry`. A frame's index is `row × 8 + column`, which is why the example manifest counts `0`, `8`, `16`, `24`, `32`, `40`, `48`.
- **Feet on the cell's bottom edge**, character centred. `anchor` is that point; it is how a 64 px captain and a 32 px kitten both perch on a bond seam.
- The character's own colours, drawn in (`"palette": "fixed"`). Only a pack that wants to drift with the kaleidoscope's hue draws in greys for `"theme"`.

Seven states, and the frames each one wants. Fewer is fine; a state with no frames falls back to `idle`, and `idle` is the only one the app refuses to go without.

| State | Frames | What the frames are |
|---|---|---|
| `idle` | 2–4 | Standing. Frame 2 is a blink, frame 3 a small shift: an ear, the tail, the seagull looking the other way |
| `sleep` | 2 | Curled up; the second frame is the breath, one pixel taller |
| `dance` | 4 | Two bounce heights and a head-bob each way. These advance on the beat, so each frame must read as a *position*, not a motion blur |
| `walk` | 4 | The animator's four: contact (front foot down, back foot up), down (weight on it, body lowest), passing (legs together, body highest), up (pushing off) |
| `startle` | 2–3 | Jump, in the air with everything out, land |
| `pet` | 2–3 | Eyes closed, leaning in; a heart or two |
| `carry` | 2 | Held up, limbs dangling, one frame kicking |

## The pipeline, whichever way the frames are made

**One image per pose, never one sheet.** An image model does not count pixels and does not keep eight frames the same size or on the same baseline; asked for a strip it returns eight slightly different capybaras at eight sizes. The reliable way is a single, large, centred pose per image, then a script to size and place them.

1. **Make each pose** as a square image, character centred, on a flat colour that appears nowhere on him (`#00FF00` green; his palette is brown, yellow, grey and white). Or, from a pencil: line art on white, scanned or photographed flat, colour filled without gradients.
2. **Key the background out**: `tools\keyout.ps1 -In pose.png -Out idle-0.png -Size 1024 -NoCrop`. It flood-fills in from the border, so a grey boombox on a green field survives, and it takes the green cast out of the outline pixels that were anti-aliased against the screen (`-Despill`, 3 px by default), which is what stops green dots appearing around him at sprite size. A model that can output a transparent PNG skips this step, though the flat green with keyout has given cleaner edges than the models' own transparency so far. `-NoCrop` keeps the canvas rather than cropping to the art: the poses were all drawn on the same square, and that is the only record of their size relative to each other. Cropped, every pose fills its square, the squat comes out as tall as the stand, and the packer's one factor has nothing left to keep.
3. **Name the frames** `<state>-<n>.png` in one folder: `idle-0.png`, `idle-1.png`, `walk-0.png` … `walk-3.png`.
4. **Pack them**: `tools\sheet.ps1 -In <folder> -Out <packdir> -Frame 64 -Name "Cap'n Capy"`. It finds the tallest pose, scales every frame by that one factor (so a crouch does not grow to fill the cell), puts the feet on the bottom edge, and writes `sheet.png` plus a `companion.json` with every frame it placed. The default filter averages the area each cell pixel covers, which comes out even across frames; `-Filter nearest` is only for pixel art drawn at the cell size or an integer multiple of it.
5. **Look at the sheet at 1x.** Sixty-four pixels is the truth; the 1024 px source is not. If a pose reads as a blob, the fix is in the drawing (thicker outline, fewer details, a bigger silhouette change between frames), not in the scaling.

## Prompting an image model for the frames

Two rules do most of the work: **the prompt never changes except for one sentence**, and **every pose is generated with the approved idle frame attached as the reference**. Models that take a reference image (a character reference, an edit-with-image mode, a seed) hold the character steady; without one, each image reinvents him.

The fixed part, worded for a 64-cell:

> Pixel art sprite of the attached character, exactly the same character, same proportions, same colours, same outfit: a capybara in a yellow storm jacket with a boombox on his shoulder and a seagull standing on his head. Chunky 16-bit pixel art, 2-pixel dark outline, flat colours, no gradients, no anti-aliasing. Full body, side view facing right, feet on the ground line at the bottom, centred, filling the frame. Solid bright green background (#00FF00), no shadow, no ground, no text, no border, nothing else in the image. Square.

Then one sentence for the pose, swapped per frame:

- `idle-0`: standing still, relaxed, eyes open.
- `idle-1`: identical to the previous image with the eyes closed for a blink.
- `walk-0`: mid-step, contact pose: front foot planted forward, back foot lifting off.
- `walk-1`: down pose: weight on the front foot, body at its lowest.
- `walk-2`: passing pose: legs together under the body, body at its highest.
- `walk-3`: up pose: pushing off the back foot, front leg swinging forward.
- `dance-0` … `dance-3`: bouncing to music, body low / body high with the head tilted left / body low / body high with the head tilted right.
- `sleep-0`: curled up asleep on the ground, eyes closed, seagull asleep too.
- `startle-1`: leaping straight up, startled, all four legs out, seagull flapping.
- `pet-0`: eyes closed, leaning into a hand, content, one small heart above.
- `carry-0`: held up from above by the scruff, legs dangling, unimpressed.

What to expect, and what to do about it:

- **Scale drifts** between images even with a reference. `sheet.ps1` normalises on the tallest frame, so a slightly smaller walk frame is harmless; a pose that came out at half size will lose detail. Regenerate it rather than accept it. The first sixteen captain frames kept their canvas heights within a few percent of each other, and the walk's down frame was drawn lowest and its passing frame highest, so the drawn scale is worth keeping; that is what `-NoCrop` is for.
- **Facing flips.** Say "facing right" every time, and mirror in the script's input folder if one still comes out wrong; the app walks him both ways by flipping, so only one facing is drawn.
- **"Pixel art" is a look, not a resolution.** The model draws big fake pixels at 1024. That is exactly what you want: the downscale to 64 snaps them to real ones. Ask for a pixel look even if the final art will be smoothed, because it forces the simple silhouettes that survive 64 px.
- **Frames that get blockier one to the next are the packer, not the model.** The model draws at roughly 10 px per fake pixel, and a 64-cell samples one pixel per 15 of source, so nearest-neighbour keeps or drops whole fake pixels by where the grid happens to fall, differently in every frame. Measured on the first three idles: the sources were all within a pixel of the same grid while the packed frames climbed from fine to chunky. The packer's default filter averages instead, which is even across frames. If the pixel look is wanted back, it comes from the drawing (ask for a coarser grid), not from the filter.
- **The seagull is the tell.** If he is missing or has moved to the shoulder, the model has lost the reference; reattach it and regenerate before doing more frames.
- **Do not ask for two frames in one image**, not even a before/after. See the first rule above.
- **A near-identical frame (the blink, the breath) is an edit, not a generation.** A fresh generation from the reference moves the boombox, the arm and the seagull a little every time, and at 64 px two "idle" frames that differ that much read as a jump, not a blink. Use the model's edit mode on the approved frame ("close the eyes; change nothing else") so everything but the change is carried over pixel for pixel.
- **Branch the chat from the approved idle for every new pose.** A long chat accumulates drift; each pose started from the same message starts from the same captain. Found the first time he went off the rails on idle.
- **A pose described as a small change comes back as idle.** Asked for "body high with the head tilted left", the model returned the reference standing straight, twice. The tilt has to be the whole sentence, and an edit of the idle frame does it more reliably than a generation.
- **The seagull grows.** On the first walk and dance frames he came back half again as large as on the idle, the same within a state and different between states, so a loop that crosses into idle pops. Compare him against the idle before accepting a state's first frame.
- **Carry comes with a hand** unless told otherwise, and in the app the cursor is the hand. Say the grip is off the top of the frame; a jacket stretched to the top edge reads as the pinch point at sprite size.

A friend with a pencil follows the same list with the same poses and skips the prompt. Line art on white, one pose per sheet of paper, photographed flat: `keyout.ps1` handles white as well as green.

## Not this: the chrome sheet

The Eyewall skin's sprite sheet (#3, D73) is a different job and a generator is the wrong tool for it. Those sprites are **shapes** at one and two pixels of stroke: title bar, buttons in their four states, slider tracks and thumbs, drawn as alpha masks the renderer tints from the palette and glows from `--arc`. The session produces it, derived from the CSS chrome that already ships so the look is the one already accepted (D90), from the rectangles `skin-manifest.md` fixes, and it goes under `skins/eyewall/`. Nobody draws it by hand; "hand-drawn" in #37 and D73 only ever meant committed PNGs rather than a build-time script. The companion is the one that gets to be an illustration.

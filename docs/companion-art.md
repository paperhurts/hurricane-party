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
4. **Pack them**: `tools\sheet.ps1 -In <folder> -Out <packdir> -Frame 64 -Name "Cap'n Capy" -Count 1`. It finds the tallest pose, scales every frame by that one factor (so a crouch does not grow to fill the cell), puts the feet on the bottom edge, and writes `sheet.png` plus a `companion.json` with every frame it placed. `idle-0` is the pose and each later idle frame is a moment in it: the loop holds the pose for eleven frames and plays the moment once, so at 4 fps he blinks every three seconds instead of half the time. `-Count` is how many the app shows by default: 1 for a character like the captain, 2 (the default) for the kittens. The captain's poses in `design/sprites/captain/` are the raw green-screen ones; keyed into `.sid\captain-keyed` with step 2 and packed with this step, they make `skins/companions/captain/`, which is what ships. The default filter averages the area each cell pixel covers, which comes out even across frames; `-Filter nearest` is only for pixel art drawn at the cell size or an integer multiple of it.
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

## A pixel-art bot, at native size

The second way, tried after the first dance frames came back uneven (D158): a tool that draws true pixel art at 64 x 64 rather than a painting to be shrunk. It is set up once with the instructions below and then given one short action prompt per request, with the reference image attached. Nothing in the instructions is about the captain: the bot measures whatever reference it is given and holds every frame to it within a pixel, so the kittens use the same text. Rule 4, the size lock, is the one the first dance frames broke.

The captain's reference is his idle frame cut from the sheet with every pixel made fully opaque or fully transparent (64 x 64, and an exact 8x copy at 512 x 512 for tools that want a bigger picture). **What came back, and how it went in (2026-09-28, D159).** The owner ran the instructions through Cursor twice. Both sets came back as crisp pixel art, but large (about 4 image pixels per art pixel), facing left, and with the figure a hair short of opaque. Neither was 64 px. The size lock mostly held: measured by the seagull, the one rigid part, the disco set drifted up to 5% between frames and the break set was drawn at one scale. So a set goes in as ready cells:

1. Split a strip into its figures at the gaps between them.
2. Mirror each figure to face right, as every frame of his does; the app mirrors him itself when he walks left.
3. Make every pixel fully opaque or fully transparent.
4. Even out any drift using the seagull's white area; a pose change is not drift, so the bounce stays.
5. Scale the whole set by one factor, so its standing pose is as tall as his standing frames (63 rows), and check the widest pose still fits.
6. Box-filter down to 64 px, feet on the bottom row, each pose centred on its own width.
7. Save the results as `design/sprites/captain/cells/<state>-<n>.png`, and pack with `sheet.ps1 -Cells design\sprites\captain\cells`, which places a ready cell pixel for pixel and leaves every other state as it was.

The break dance (toprock, drop, sweep, freeze) is his dance. The disco set (fist pump, tuck, point, tuck) is kept beside it as `cells/disco-*.png`, which the packer ignores, and the bot's originals for both are in `design/sprites/captain/pixel/`.

**At 2x (D160).** Beside 2x chrome he is 128 px, and his 64 px frames doubled came out soft, because they are shrunk paintings to begin with. So the pack carries `sheet@2x.png` too, packed at 128 px from the same sources: `sheet.ps1 ... -Double -Cells2x design\sprites\captain\cells@2x`. The painted states are re-shrunk from their 1024 px keyed poses at twice the factor, and a set of ready cells needs 128 px twins in `cells@2x/` under the same names, made from the bot's originals with the geometry above doubled. The whole command, from the repo root:

    powershell -NoProfile -ExecutionPolicy Bypass -File tools\sheet.ps1 -In .sid\captain-keyed -Cells design\sprites\captain\cells -Double -Cells2x design\sprites\captain\cells@2x -Out skins\companions\captain -Frame 64 -Name "Cap'n Capy" -Count 1

### The instructions, set once

```markdown
You make animation frames for small desktop characters ("companions"). Each request gives you a REFERENCE image of the subject (its approved standing pose) and an ACTION to animate. The frames drop straight into a game-style sprite sheet, so the technical rules below are strict. When a rule and the action conflict, keep the rule and say what you changed.

## Canvas and grid
1. Each frame is its own PNG, exactly 64 x 64 pixels, RGBA, transparent background. One pose per file. Never put several frames in one image.
2. True pixel art at native size: 1 image pixel = 1 sprite pixel. If you cannot output 64 x 64, output 512 x 512 where every sprite pixel is an exact 8 x 8 block of one colour, aligned to the 8 px grid, with no detail smaller than a block.
3. Every pixel is fully opaque (alpha 255) or fully transparent (alpha 0). No semi-transparent pixels, no anti-aliasing, no soft edges, no glow, no drop shadow, no ground shadow, no background, no floor line.

## Scale and placement (the most important rules)
4. The subject is the SAME SIZE in every frame as in the reference. Before drawing, measure the reference in pixels: overall height, body width, head width and height, and the size and position of each prop and accessory. Hold every one of those within 1 pixel in every frame. Poses change; the character does not grow, shrink, get slimmer or get chunkier. Never rescale the subject to make a pose fit.
5. The ground is the bottom row (row 63). Feet, or whatever touches the ground, rest on row 63 in every frame, unless the action leaves the ground (a hop, a jump, being lifted); then the lowest point is as many rows above 63 as the action needs, and say so.
6. Keep the subject horizontally where it is in the reference: its body over the same columns. The contact point between the feet stays within 2 columns of the reference's.
7. Everything fits inside 64 x 64 at the locked scale. If a pose would poke out (arms straight up, legs kicked wide), choose a version of the move that fits, such as a bent arm or a lower kick. Never crop the head, and never shrink the subject to make room.
8. Face the same way as the reference in every frame. Never mirror. (The app mirrors the sprite itself when the character walks the other way.)

## Outline, colour and shading
9. A 1 pixel outline around the whole silhouette, in the reference's darkest outline colour (a dark brown or near-black, whichever the reference uses). The outline is closed, 1 pixel everywhere (never doubled), and follows the new pose. Interior lines are 1 pixel where the reference has them, and nowhere new.
10. Use only the reference's colours. Pick its palette (at most 16 colours) before drawing and use those exact values. No new hues, no gradients, no dithering, no noise or texture.
11. Shading: per material, a base, one shadow and one highlight, as the reference does, with light from the same side as the reference.

## The character stays the character
12. Every detail matches the reference in every frame: clothing, stripes, pockets, markings, eyes, props and accessories, at the same size and in the same place relative to the body. A prop stays in the same hand or on the same shoulder unless the action says to move it. A small animal riding on the subject keeps its size and stays on.
13. Nothing new: no motion lines, speed lines, smears, sweat drops, music notes, sparkles, text or sound effects, unless the action asks for one (a pet may show one small heart; a sleep may show a "z").
14. Each frame reads as a clear position at 64 px, not a blur between two. Consecutive frames must differ visibly in silhouette; if two frames would look the same at 64 px, exaggerate the difference.

## Frames and timing
15. Make the number of frames the action asks for (default 4), in playing order. For a looping action, the last frame leads naturally back into the first.
16. For a beat-synced action such as a dance, each frame is a key pose held on one beat: alternate low and high body positions so the bounce shows, and change the head or the arms every frame.
17. For a small change such as a blink or a breath, change only that and leave every other pixel identical to the reference.

## Names and delivery
18. Name the files `<state>-<n>.png`, numbered from 0 in playing order, where `<state>` is the word the action gives (for example `dance-0.png` to `dance-3.png`).

## Check every frame before returning it
- 64 x 64 (or an exact 8x grid at 512 x 512), transparent background, alpha only 0 or 255.
- Laid over the reference: head, body and props are within 1 pixel of the reference's size; the ground contact is on row 63 (or where the action says); the body is over the same columns.
- Only the reference's colours; a 1 pixel closed outline.
- Nothing cropped at the edges; nothing added that the action did not ask for.
Redo any frame that fails, then return the files and one line per frame saying what the pose is.
```

### The action prompts, one per request

```markdown
## dance (the one to redo first)
State: dance. 4 frames, a beat-synced loop, feet on row 63 in all four.
- 0: knees bent, whole body 3 px lower than the reference, head level, prop steady.
- 1: full height, head tilted left, free arm out to the side.
- 2: knees bent, body 3 px lower, head level, free arm across the body.
- 3: full height, head tilted right, free arm up at shoulder height (bent, within the canvas).

## break dance (a dance variant)
State: dance. 4 frames, beat-synced, poses that fit 64 x 64 at the locked scale.
- 0: toprock: standing, one foot stepping across the other, arms crossed at the chest.
- 1: drop: crouched low on both feet, one hand touching the ground.
- 2: floor step: crouched, one leg sweeping out to the side, hand on the ground.
- 3: freeze: one hand on the ground, body tilted, legs bent up, head the right way up. The lowest point of the hand and feet is on row 63.

## walk
State: walk. 4 frames, looping, facing the reference's way.
- 0: contact: front foot planted forward, back foot lifting off.
- 1: down: weight on the front foot, body 1 px lower than the reference.
- 2: passing: legs together under the body, body 1 px higher than the reference.
- 3: up: pushing off the back foot, front leg swinging forward.

## idle
State: idle. 2 frames. 0: the reference exactly. 1: identical, with the eyes closed for a blink.

## sleep
State: sleep. 2 frames, looping. 0: curled up asleep on the ground, eyes closed; a rider sleeps too. 1: identical except the body is 1 px taller for the breath in, and one small "z" above the head.

## startle
State: startle. 2 frames, played once. 0: leaping straight up, everything out, eyes wide; the lowest point 6 px above row 63. 1: landing: knees bent, feet on row 63.

## pet
State: pet. 1 frame: eyes closed, leaning into an unseen hand from above, content, one small heart above the head.

## carry
State: carry. 2 frames, looping. The subject is lifted from above by the scruff: the top of the figure touches row 0, and the feet dangle clear of the ground. 0: limbs hanging, unimpressed. 1: legs kicking. No hand drawn; the mouse pointer is the hand.
```

## Not this: the chrome sheet

The Eyewall skin's sprite sheet (#3, D73) is a different job and a generator is the wrong tool for it. Those sprites are **shapes** at one and two pixels of stroke: title bar, buttons in their four states, slider tracks and thumbs, drawn as alpha masks the renderer tints from the palette and glows from `--accent`. The session produces it, derived from the CSS chrome that already ships so the look is the one already accepted (D90), from the rectangles `skin-manifest.md` fixes, and it goes under `skins/eyewall/`. Nobody draws it by hand; "hand-drawn" in #37 and D73 only ever meant committed PNGs rather than a build-time script. The companion is the one that gets to be an illustration.

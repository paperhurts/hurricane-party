<script lang="ts">
  import type { Snippet } from "svelte";
  import type { Element } from "../lib/skin";
  import { actionTitle, placeRect } from "../lib/skin";
  import { type LoadedSkin, nineSliceRefs, ninePieceStyle, type Slice } from "../lib/skinsheet";

  // One element of an hp-skin/1 window, drawn from its sprites (#3, D73, D93).
  //
  // A sprite is a data: URL of its own rectangle. For `art: mask` it is an
  // alpha mask over a token colour, so the palette reaches the chrome live
  // through the custom property; for `art: final` it is the picture. Each
  // state's slice and tint go into custom properties on the element, and
  // chrome.css picks the one for :hover, :active and the group's focus.
  // Nothing here is a colour: a tint is the name of a token, and `opacity`
  // is how strong that token is drawn — the way a sheet's own alpha would
  // say it, for the sprites several elements share.
  let {
    el,
    skin,
    base,
    current,
    on = false,
    binds = {},
    glowing = true,
    slot,
    onpointerdown,
    onclick,
    onslide,
  }: {
    el: Element;
    skin: LoadedSkin;
    /** The element set's base size and the window's current size, logical px. */
    base: [number, number];
    current: [number, number];
    /** A toggle's state, when the shell rather than a bind decides it. */
    on?: boolean;
    /** What the window is showing, by binding name. */
    binds?: Record<string, unknown>;
    /** The person's glow toggle (#108, D100). Off, the renderer adds no
     * halo: no filter on a button, no glow on a text. */
    glowing?: boolean;
    /** Rendered inside this element's box, above its art: the analyser in the
     * visualizer, the windowshade strip in the title, an error affordance
     * over the title bar. The skin positions it; the window fills it. */
    slot?: Snippet;
    onpointerdown?: (e: PointerEvent) => void;
    onclick?: (e: MouseEvent) => void;
    /** A slider was pressed or dragged, 0..1 along its length. */
    onslide?: (frac: number) => void;
  } = $props();

  // A mask skin tints its art, except a sheet that says it is final (D122): a
  // made skin's picture is drawn as the picture, not as a silhouette of it.
  let mask = $derived(
    skin.skin.art === "mask" && !("sprite" in el && skin.skin.finalSheets.includes(el.sprite.sheet)),
  );
  // The renderer's halo (D73): only for a skin that leaves the glow to it,
  // and only while the person has it on (D100). A `baked` skin's halo is in
  // its pixels either way.
  let glow = $derived(skin.skin.glow === "renderer" && glowing);

  function vars(slices: { n: Slice; h?: Slice; a?: Slice; i?: Slice }): string {
    const parts = [`--sp:url("${slices.n.url}")`, `--tc:var(--${slices.n.tint})`];
    if (slices.h) parts.push(`--sp-h:url("${slices.h.url}")`, `--tc-h:var(--${slices.h.tint})`);
    if (slices.a) parts.push(`--sp-a:url("${slices.a.url}")`, `--tc-a:var(--${slices.a.tint})`);
    if (slices.i) parts.push(`--sp-i:url("${slices.i.url}")`, `--tc-i:var(--${slices.i.tint})`);
    return parts.join(";");
  }

  let box = $derived.by(() => {
    if (el.type === "nineslice" && el.fill) return "inset:0";
    const r = placeRect(el, base, current);
    return `left:${r.x}px;top:${r.y}px;width:${r.w}px;height:${r.h}px`;
  });

  // Which sprite set a toggle shows: its own, or the `on` set. A toggle with
  // a bind reads its state from the window instead of from the shell, which
  // is what makes a transport button light while that is what is happening.
  let lit = $derived(
    el.type === "toggle" && el.bind && el.when !== null ? String(binds[el.bind] ?? "") === el.when : on,
  );

  let states = $derived.by(() => {
    if (el.type === "button") {
      return {
        n: skin.slice(el.sprite),
        h: el.hover && skin.slice(el.hover),
        a: el.active && skin.slice(el.active),
        i: el.inactive && skin.slice(el.inactive),
      };
    }
    if (el.type === "toggle") {
      const s = lit ? el.on : el;
      return {
        n: skin.slice(s.sprite),
        h: s.hover && skin.slice(s.hover),
        a: s.active && skin.slice(s.active),
        i: s.inactive && skin.slice(s.inactive),
      };
    }
    if (el.type === "image") {
      return { n: skin.slice(el.sprite), i: el.inactive && skin.slice(el.inactive) };
    }
    return null;
  });

  let font = $derived(el.type === "text" || el.type === "list" ? skin.skin.fonts[el.font] : null);

  /** A font's size and tracking, as style. A bitmap font is not drawn yet
   * (v0.5, with the importers); until it is, its text is the theme's face at
   * the glyphs' height, not the page's 14 px in a 10 px row. */
  function fontStyle(name: string): string {
    const f = skin.skin.fonts[name];
    if (f?.type === "system") return `font-size:${f.size}px;letter-spacing:${f.tracking}em`;
    if (f?.type === "bitmap") return `font-size:${f.glyphSize[1]}px`;
    return "";
  }
  const isUpper = (name: string) => {
    const f = skin.skin.fonts[name];
    return f?.type === "system" && f.case === "upper";
  };

  /** A token at a strength, as a colour. */
  const tone = (t: string, opacity: number) =>
    opacity >= 1 ? `var(--${t})` : `color-mix(in srgb, var(--${t}) ${Math.round(opacity * 100)}%, transparent)`;

  /** A literal with `{}` for the bound value, the bound value alone, or the
   * literal alone. */
  function wording(value: string | null, bind: string | null): string {
    const bound = bind === null ? "" : String(binds[bind] ?? "");
    if (value === null) return bound;
    return value.includes("{}") ? value.replace("{}", bound) : value;
  }

  // A text element's look: its own, or the `lit` one while the binding it
  // names holds the value it names (the PLAY tag while the transport plays).
  let look = $derived.by(() => {
    if (el.type !== "text") return null;
    const l = el.lit;
    if (l && l.bind && String(binds[l.bind] ?? "") === l.when) return l;
    return { tint: el.tint, opacity: el.opacity, glow: el.glow };
  });

  let shown = $derived(el.type === "text" ? wording(el.value, el.bind) : "");

  // ---- bitmap text (D104) ----
  //
  // A classic skin's words are art: a grid of glyphs on a sheet, and a string
  // is drawn by cutting one per character. How many fit across the grid is
  // the sheet's own width. A character the sheet has no glyph for leaves its
  // box empty, which is what the classic drew too; `tracking` is the gap
  // between boxes, 3 on the clock whose colon is painted into the window.
  let glyphs = $derived.by(() => {
    if (el.type !== "text" || font?.type !== "bitmap" || !look) return null;
    const [gw, gh] = font.glyphSize;
    const cols = Math.max(1, Math.floor(skin.sheetSize(font.sheet).w / gw));
    const cells = [...shown].map((ch) => {
      const i = font.map.indexOf(ch.toLowerCase());
      if (i < 0) return null;
      const rect: [number, number, number, number] = [(i % cols) * gw, Math.floor(i / cols) * gh, gw, gh];
      return skin.slice({ sheet: font.sheet, rect, tint: look.tint });
    });
    const width = cells.length * (gw + font.tracking) - font.tracking;
    return { cells, w: gw, h: gh, tracking: font.tracking, width };
  });
  /** A bitmap line too long for its box scrolls, and its width is arithmetic
   * rather than a measurement: every glyph is the same size. */
  let glyphRoll = $derived.by(() => {
    if (!glyphs || el.type !== "text" || el.overflow !== "scroll") return false;
    return glyphs.width > placeRect(el, base, current).w;
  });

  // ---- a button's words and its disabled state (D99) ----

  let label = $derived(el.type === "button" || el.type === "toggle" ? (el.label ?? null) : null);
  // The words answer the pointer the way the art does: the hover colour while
  // it is over the button, the `on` colour while the toggle is on.
  let labelStyle = $derived.by(() => {
    if (!label) return "";
    const rest = lit && label.on ? `var(--${label.on})` : tone(label.tint, label.opacity);
    const hover = label.hover ? `var(--${label.hover})` : rest;
    return `--lc:${rest};--lc-h:${hover};${fontStyle(label.font)}`;
  });
  let off = $derived.by(() => {
    if (el.type !== "button" && el.type !== "toggle") return false;
    const d = el.disabled;
    return d && d.bind ? String(binds[d.bind] ?? "") === d.when : false;
  });

  // ---- a list's box (D99) ----
  //
  // The skin says where the rows go, how tall each is, their face and their
  // three colours; the window draws the rows, and reads all of it from these
  // properties, so a skin that wants taller rows or another colour for the
  // playing one changes the manifest and nothing else.
  let listStyle = $derived.by(() => {
    if (el.type !== "list") return "";
    return [
      `--list-fg:${tone(el.tint, el.opacity)}`,
      `--list-hi:var(--${el.tint})`,
      `--list-now:var(--${el.current})`,
      `--list-sel:var(--${el.selected})`,
      `--list-row:${el.rowHeight}px`,
      fontStyle(el.font),
    ].join(";");
  });

  // A title longer than its box scrolls (the manifest's `overflow: "scroll"`).
  // Measured rather than guessed from a character count: the box is the
  // skin's to size, and a 2x chrome or another font changes what fits. The
  // reset-then-measure is what keeps it honest when the text gets shorter.
  let textBox = $state<HTMLElement | undefined>(undefined);
  let roll = $state(false);
  // The text last measured. Plain, not $state, and that is the fix: this
  // effect re-runs whenever the window's bindings change, which is every
  // clock tick while a track plays, and resetting `roll` on each of those
  // restarted the animation four times a second — a title that jinked a
  // pixel back and forth and only scrolled once the clock stopped. Now a
  // re-run with the same text leaves a running marquee alone.
  let measured: string | null = null;
  // The one measurement in flight. Also plain: it is cancelled only when the
  // text changes, never by a re-run. Unshading rebuilds this element while
  // several bindings land in the same instant, and when each re-run cancelled
  // the last one's frame, the re-run that followed saw the text already
  // "measured" and scheduled nothing, so the title sat behind an ellipsis.
  let pending = 0;
  $effect(() => {
    const text = shown;
    // Read before any early return, so a `bind:this` that lands after the
    // first run is a dependency and brings this back to measure.
    const host = textBox;
    if (el.type !== "text" || el.overflow !== "scroll" || !host) return;
    if (text === measured) return;
    measured = text;
    roll = false;
    cancelAnimationFrame(pending);
    if (text === "") return;
    pending = requestAnimationFrame(() => {
      pending = 0;
      const span = host.querySelector(".t");
      if (span) roll = span.scrollWidth > host.clientWidth + 1;
    });
  });
  // …and the frame dies with the element, and only then.
  $effect(() => () => cancelAnimationFrame(pending));

  // ---- slider ----

  let frac = $derived.by(() => {
    if (el.type !== "slider" || el.bind === null) return 0;
    const v = Number(binds[el.bind]);
    return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0;
  });

  let vertical = $derived(el.type === "slider" && el.orientation === "vertical");
  // Where the fill starts: the origin for a centred control (the EQ's 0 dB),
  // else the start of the track. The fill spans origin..value, whichever is
  // lower first, so a cut hangs below the line and a boost rises above it.
  let origin = $derived(el.type === "slider" ? (el.origin ?? 0) : 0);

  /** A length in CSS px, moved to the nearest whole device pixel. */
  const snap = (v: number) => Math.round(v * skin.dpr) / skin.dpr;

  // The fill's and thumb's boxes, in px, snapped to the device grid. Placing
  // them by percentage and a -50% transform put an odd-sized thumb on half
  // pixels, soft at 1x, and let a 3-pixel core sit half a pixel off the 0 dB
  // tick it should cover. Worked out here instead, so both land whole.
  let geom = $derived.by(() => {
    if (el.type !== "slider") return null;
    const r = placeRect(el, base, current);
    const t = el.thumb ? skin.slice(el.thumb) : null;
    const tw = t?.w ?? 0;
    const th = t?.h ?? 0;
    if (vertical) {
      // Top is 1: the value is measured up from the bottom.
      const y0 = (1 - origin) * r.h;
      const y1 = (1 - frac) * r.h;
      const top = snap(Math.min(y0, y1));
      return {
        fill: `left:0;width:${r.w}px;top:${top}px;height:${snap(Math.max(y0, y1)) - top}px`,
        thumb: `left:${snap((r.w - tw) / 2)}px;top:${snap(y1 - th / 2)}px`,
      };
    }
    const x0 = origin * r.w;
    const x1 = frac * r.w;
    const left = snap(Math.min(x0, x1));
    return {
      fill: `top:0;height:${r.h}px;left:${left}px;width:${snap(Math.max(x0, x1)) - left}px`,
      thumb: `top:${snap((r.h - th) / 2)}px;left:${snap(x1 - tw / 2)}px`,
    };
  });
  // A second look while its binding holds (the EQ dims while off), and the
  // thumb's hot tint past `beyond`. The dim wins: an off EQ is off however
  // far a band is pushed.
  let sliderLit = $derived(
    el.type === "slider" && el.lit && el.lit.bind ? String(binds[el.lit.bind] ?? "") === el.lit.when : false,
  );
  let hot = $derived(el.type === "slider" && el.hot ? Math.abs(frac - origin) > el.hot.beyond : false);

  /** The tint and strength a slider's fill or thumb is drawn at right now,
   * as custom properties that override the sprite's own. */
  function sliderLook(thumb: boolean): string {
    if (el.type !== "slider") return "";
    if (sliderLit && el.lit) return `;--tc:var(--${el.lit.tint});opacity:${el.lit.opacity}`;
    if (thumb && hot && el.hot) return `;--tc:var(--${el.hot.tint})`;
    return "";
  }

  // A centred control takes the wheel: one notch is 1/48 of the range, half a
  // dB on the EQ's 24. A seek bar and a level do not; they have no centre.
  function slideWheel(e: WheelEvent) {
    if (el.type !== "slider" || el.origin === null || !onslide) return;
    // A sideways swipe, or Shift and the wheel, has no vertical part: it is
    // not a nudge, and falling through read it as one notch down.
    if (e.deltaY === 0) return;
    e.preventDefault();
    onslide(Math.min(1, Math.max(0, frac + (e.deltaY < 0 ? 1 : -1) / 48)));
  }

  // The last press, for the double press: when, where, and whether it became
  // a drag. Plain, not $state; nothing renders from it.
  let lastDown = { at: 0, x: 0, y: 0, moved: false };
  /** Windows' own rule for a double click: the second within a few pixels. */
  const NEAR = 4;

  // Press or drag anywhere along it, the way the CSS sliders behaved. The
  // pointer is captured on <html>, not on this element: a state push
  // re-renders the sprite mid-drag and capture dies with the element it was
  // taken on — the same lesson the title-bar drag records in Classic.
  function slideDown(e: PointerEvent, node: HTMLElement) {
    if (e.button !== 0 || !onslide) return;
    e.stopPropagation();
    // A double press on a centred control returns it to the origin. Timed
    // from pointerdown rather than the DOM's dblclick: the drag below captures
    // the pointer on <html>, and a captured pointer's dblclick lands there,
    // not here — why Classic times its title-bar double-click the same way.
    // Only a real double click counts: the first press stayed put rather than
    // becoming a drag, and the second lands where the first did. Without both,
    // a quick drag and regrab, or a tap at +6 then one at -6, reset the band.
    const now = Date.now();
    const near = Math.abs(e.clientX - lastDown.x) <= NEAR && Math.abs(e.clientY - lastDown.y) <= NEAR;
    if (el.type === "slider" && el.origin !== null && now - lastDown.at < 400 && near && !lastDown.moved) {
      lastDown.at = 0;
      onslide(el.origin);
      return;
    }
    lastDown = { at: now, x: e.clientX, y: e.clientY, moved: false };
    const at = (ev: PointerEvent) => {
      const r = node.getBoundingClientRect();
      const t =
        el.type === "slider" && el.orientation === "vertical"
          ? 1 - (ev.clientY - r.top) / r.height
          : (ev.clientX - r.left) / r.width;
      onslide!(Math.min(1, Math.max(0, t)));
    };
    const root = document.documentElement;
    const move = (ev: PointerEvent) => {
      // Past a few pixels this press is a drag, and cannot be the first half
      // of a double click.
      if (Math.abs(ev.clientX - lastDown.x) > NEAR || Math.abs(ev.clientY - lastDown.y) > NEAR) lastDown.moved = true;
      at(ev);
    };
    const up = () => {
      root.removeEventListener("pointermove", move);
      root.removeEventListener("pointerup", up);
      root.removeEventListener("pointercancel", up);
      try {
        root.releasePointerCapture(e.pointerId);
      } catch {
        // The capture is gone already; nothing to release.
      }
    };
    at(e);
    root.setPointerCapture(e.pointerId);
    root.addEventListener("pointermove", move);
    root.addEventListener("pointerup", up);
    root.addEventListener("pointercancel", up);
  }
</script>

{#if el.type === "nineslice"}
  <!-- Nine boxes filling the element's box: corners at their size, edges
       stretched along one axis, the centre along both. `rect: "fill"` tracks
       the whole window; a rect of its own edges a control. -->
  <div class="sp-frame" aria-hidden="true" style="{box};opacity:{el.opacity}">
    {#each nineSliceRefs(el.sprite, el.insets) as { piece, ref } (piece)}
      {@const s = skin.slice(ref)}
      <div class="sp" class:mask class:final={!mask} style="{ninePieceStyle(piece, el.insets)};{vars({ n: s })}"></div>
    {/each}
  </div>
{:else if el.type === "image" && states}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="sp sp-image"
    class:mask
    class:final={!mask}
    class:drag={el.role === "drag"}
    style="{box};opacity:{el.opacity};{vars(states)}"
    {onpointerdown}
  ></div>
  {#if slot}
    <!-- Beside the art, not inside it. The art is a mask, and a mask clips its
         children to the sprite's box: the EQ's preset menu lost its glow on
         three sides there, and anything taller than the curve would have
         been cut off. Same box, drawn over the art, no mask. -->
    <div class="sp-slot" style={box}>{@render slot()}</div>
  {/if}
{:else if (el.type === "button" || el.type === "toggle") && states}
  <!-- The glow, when the skin leaves it to the renderer (D73), sits on a
       wrapper: a filter is applied before a mask, so on the masked element
       itself the halo would be cut away with everything else outside the
       shape. Never on the visualizer's ancestors; this is a sibling. -->
  <!-- A toggle with a binding and no action is an indicator (D93): the clip
       lamp. It shows its state and takes nothing, so it offers nothing — no
       hand cursor, no hover ring, no halo. -->
  {@const indicator = el.type === "toggle" && el.action === null}
  <!-- A button that cannot be pressed right now (REM with nothing selected)
       is drawn dim and offers nothing: no hover art, no halo, no click. -->
  <div class="sp-glow" class:glow={glow && !indicator && !off} class:off class:indicator style={box}>
    <button
      class="sp sp-button"
      class:indicator
      class:mask
      class:final={!mask}
      style={vars(states)}
      title={actionTitle(el.action, lit)}
      data-el={el.name}
      disabled={off}
      onpointerdown={(e) => {
        // Neither a drag nor a double-tap on the title bar underneath.
        e.stopPropagation();
        onpointerdown?.(e);
      }}
      {onclick}
    ></button>
    {#if label}
      <!-- Inside the wrapper, so the halo takes the words with the box and
           the hover that lights the box lights them. -->
      <span class="sp-label" class:upper={isUpper(label.font)} style={labelStyle}
        >{wording(label.value, label.bind)}</span
      >
    {/if}
  </div>
{:else if el.type === "text" && font && look}
  <div
    bind:this={textBox}
    class="sp-text"
    class:upper={font.type === "system" && font.case === "upper"}
    class:scroll={el.overflow === "scroll"}
    class:lit={look.glow && glow}
    style="{box};text-align:{el.align};--tc:var(--{look.tint});--tc-i:var(--{el.inactive?.tint ??
      look.tint});opacity:{look.opacity};{fontStyle(el.font)}"
  >
    {#if slot}
      {@render slot()}
    {:else if glyphs}
      <!-- Cut from the skin's own sheet, so `mask` art takes the text's tint
           and `final` art is the picture the author drew. -->
      {#snippet run()}
        <span class="sp-run" style="gap:{glyphs.tracking}px">
          {#each glyphs.cells as cell, i (i)}
            {#if cell}
              <span
                class="sp sp-glyph"
                class:mask
                class:final={!mask}
                style="width:{glyphs.w}px;height:{glyphs.h}px;{vars({ n: cell })}"
              ></span>
            {:else}
              <span class="sp-glyph" style="width:{glyphs.w}px;height:{glyphs.h}px"></span>
            {/if}
          {/each}
        </span>
      {/snippet}
      {#if glyphRoll}
        <span class="rolling" style="animation-duration:{Math.max(6, glyphs.cells.length * 0.35)}s">
          {@render run()}<span class="sp-gap" style="width:{glyphs.w * 3}px"></span>{@render run()}<span
            class="sp-gap"
            style="width:{glyphs.w * 3}px"
          ></span>
        </span>
      {:else}
        {@render run()}
      {/if}
    {:else if roll}
      <!-- Not `.t`: that class clips and ellipsizes, which is right for a
           title that fits and wrong for one that moves. -->
      <span class="rolling" style="animation-duration:{Math.max(6, shown.length * 0.35)}s">
        {shown}&nbsp;&nbsp;&nbsp;///&nbsp;&nbsp;&nbsp;{shown}&nbsp;&nbsp;&nbsp;///&nbsp;&nbsp;&nbsp;
      </span>
    {:else}
      <span class="t">{shown}</span>
    {/if}
  </div>
{:else if el.type === "slider"}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="sp-slider"
    class:v={vertical}
    style={box}
    onpointerdown={(e) => slideDown(e, e.currentTarget as HTMLElement)}
    onwheel={slideWheel}
  >
    {#if el.track}
      {@const s = skin.slice(el.track)}
      <div class="sp sp-track" class:mask class:final={!mask} style={vars({ n: s })}></div>
    {/if}
    {#if el.fill}
      {@const s = skin.slice(el.fill)}
      <!-- From the origin to the value along the slider's axis, the full
           breadth across it, on whole device pixels (`geom`). -->
      <div
        class="sp sp-fill"
        class:mask
        class:final={!mask}
        style="{vars({ n: s })}{sliderLook(false)};{geom?.fill}"
      ></div>
    {/if}
    {#if el.thumb}
      {@const s = skin.slice(el.thumb)}
      <div
        class="sp sp-thumb"
        class:mask
        class:final={!mask}
        style="{vars({ n: s })}{sliderLook(true)};width:{s.w}px;height:{s.h}px;{geom?.thumb}"
      ></div>
    {/if}
  </div>
{:else if el.type === "visualizer"}
  <div class="sp-vis" style={box}>
    {#if slot}{@render slot()}{/if}
  </div>
{:else if el.type === "list"}
  <div class="sp-list" class:upper={font?.type === "system" && font.case === "upper"} style="{box};{listStyle}">
    {#if slot}{@render slot()}{/if}
  </div>
{:else if el.type === "slot"}
  <!-- A box and nothing else: where the window puts something the skin has
       no element for, the playlist's count and its link field. -->
  <div class="sp-slot" style={box}>
    {#if slot}{@render slot()}{/if}
  </div>
{/if}

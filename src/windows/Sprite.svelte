<script lang="ts">
  import type { Snippet } from "svelte";
  import type { Element } from "../lib/skin";
  import { placeRect } from "../lib/skin";
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
    /** Rendered inside this element's box, above its art: the analyser in the
     * visualizer, the windowshade strip in the title, an error affordance
     * over the title bar. The skin positions it; the window fills it. */
    slot?: Snippet;
    onpointerdown?: (e: PointerEvent) => void;
    onclick?: (e: MouseEvent) => void;
    /** A slider was pressed or dragged, 0..1 along its length. */
    onslide?: (frac: number) => void;
  } = $props();

  let mask = $derived(skin.skin.art === "mask");
  let glow = $derived(skin.skin.glow === "renderer");

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

  let font = $derived(el.type === "text" ? skin.skin.fonts[el.font] : null);

  // A text element's look: its own, or the `lit` one while the binding it
  // names holds the value it names (the PLAY tag while the transport plays).
  let look = $derived.by(() => {
    if (el.type !== "text") return null;
    const l = el.lit;
    if (l && l.bind && String(binds[l.bind] ?? "") === l.when) return l;
    return { tint: el.tint, opacity: el.opacity, glow: el.glow };
  });

  let shown = $derived.by(() => {
    if (el.type !== "text") return "";
    const bound = el.bind === null ? "" : String(binds[el.bind] ?? "");
    if (el.value === null) return bound;
    return el.value.includes("{}") ? el.value.replace("{}", bound) : el.value;
  });

  // A title longer than its box scrolls (the manifest's `overflow: "scroll"`).
  // Measured rather than guessed from a character count: the box is the
  // skin's to size, and a 2x chrome or another font changes what fits. The
  // reset-then-measure is what keeps it honest when the text gets shorter.
  let textBox = $state<HTMLElement | undefined>(undefined);
  let roll = $state(false);
  $effect(() => {
    const text = shown;
    if (el.type !== "text" || el.overflow !== "scroll") return;
    roll = false;
    const host = textBox;
    if (!host || text === "") return;
    const id = requestAnimationFrame(() => {
      const span = host.querySelector(".t");
      if (span) roll = span.scrollWidth > host.clientWidth + 1;
    });
    return () => cancelAnimationFrame(id);
  });

  // ---- slider ----

  let frac = $derived.by(() => {
    if (el.type !== "slider" || el.bind === null) return 0;
    const v = Number(binds[el.bind]);
    return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0;
  });

  // Press or drag anywhere along it, the way the CSS sliders behaved. The
  // pointer is captured on <html>, not on this element: a state push
  // re-renders the sprite mid-drag and capture dies with the element it was
  // taken on — the same lesson the title-bar drag records in Classic.
  function slideDown(e: PointerEvent, node: HTMLElement) {
    if (e.button !== 0 || !onslide) return;
    e.stopPropagation();
    const at = (ev: PointerEvent) => {
      const r = node.getBoundingClientRect();
      const t =
        el.type === "slider" && el.orientation === "vertical"
          ? 1 - (ev.clientY - r.top) / r.height
          : (ev.clientX - r.left) / r.width;
      onslide!(Math.min(1, Math.max(0, t)));
    };
    const root = document.documentElement;
    const move = (ev: PointerEvent) => at(ev);
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
  >
    {#if slot}{@render slot()}{/if}
  </div>
{:else if (el.type === "button" || el.type === "toggle") && states}
  <!-- The glow, when the skin leaves it to the renderer (D73), sits on a
       wrapper: a filter is applied before a mask, so on the masked element
       itself the halo would be cut away with everything else outside the
       shape. Never on the visualizer's ancestors; this is a sibling. -->
  <div class="sp-glow" class:glow style={box}>
    <button
      class="sp sp-button"
      class:mask
      class:final={!mask}
      style={vars(states)}
      title={el.name}
      onpointerdown={(e) => {
        // Neither a drag nor a double-tap on the title bar underneath.
        e.stopPropagation();
        onpointerdown?.(e);
      }}
      {onclick}
    ></button>
  </div>
{:else if el.type === "text" && font && look}
  <div
    bind:this={textBox}
    class="sp-text"
    class:upper={font.type === "system" && font.case === "upper"}
    class:scroll={el.overflow === "scroll"}
    class:lit={look.glow}
    style="{box};--tc:var(--{look.tint});--tc-i:var(--{el.inactive?.tint ?? look.tint});opacity:{look.opacity};{font.type ===
    'system'
      ? `font-size:${font.size}px;letter-spacing:${font.tracking}em`
      : ''}"
  >
    {#if slot}
      {@render slot()}
    {:else if roll}
      <span class="t rolling" style="animation-duration:{Math.max(6, shown.length * 0.35)}s">
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
    style={box}
    onpointerdown={(e) => slideDown(e, e.currentTarget as HTMLElement)}
  >
    {#if el.track}
      {@const s = skin.slice(el.track)}
      <div class="sp sp-track" class:mask class:final={!mask} style={vars({ n: s })}></div>
    {/if}
    {#if el.fill}
      {@const s = skin.slice(el.fill)}
      <div
        class="sp sp-fill"
        class:mask
        class:final={!mask}
        style="{vars({ n: s })};{el.orientation === 'vertical'
          ? `height:${frac * 100}%`
          : `width:${frac * 100}%`}"
      ></div>
    {/if}
    {#if el.thumb}
      {@const s = skin.slice(el.thumb)}
      <div
        class="sp sp-thumb"
        class:mask
        class:final={!mask}
        style="{vars({ n: s })};width:{s.w}px;height:{s.h}px;{el.orientation === 'vertical'
          ? `bottom:${frac * 100}%`
          : `left:${frac * 100}%`}"
      ></div>
    {/if}
  </div>
{:else if el.type === "visualizer"}
  <div class="sp-vis" style={box}>
    {#if slot}{@render slot()}{/if}
  </div>
{/if}

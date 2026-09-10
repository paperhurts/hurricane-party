<script lang="ts">
  import type { Snippet } from "svelte";
  import type { Element } from "../lib/skin";
  import { placeRect } from "../lib/skin";
  import { type LoadedSkin, nineSliceRefs, ninePieceStyle, type Slice } from "../lib/skinsheet";

  // One element of an hp-skin/1 window, drawn from its sprites (#3, D73).
  //
  // A sprite is a data: URL of its own rectangle. For `art: mask` it is an
  // alpha mask over a token colour, so the palette reaches the chrome live
  // through the custom property; for `art: final` it is the picture. Each
  // state's slice and tint go into custom properties on the element, and
  // chrome.css picks the one for :hover, :active and the group's focus.
  // Nothing here is a colour: a tint is the name of a token.
  let {
    el,
    skin,
    base,
    current,
    on = false,
    text = "",
    children,
    onpointerdown,
    onclick,
  }: {
    el: Element;
    skin: LoadedSkin;
    /** The element set's base size and the window's current size, logical px. */
    base: [number, number];
    current: [number, number];
    /** A toggle's state. */
    on?: boolean;
    /** What a text element shows. */
    text?: string;
    /** Rendered inside a text element instead of `text`. The windowshade
     * snippet goes here, so it sits where the title sits. */
    children?: Snippet;
    onpointerdown?: (e: PointerEvent) => void;
    onclick?: (e: MouseEvent) => void;
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

  function box(): string {
    if (el.type === "nineslice") return "";
    const r = placeRect(el, base, current);
    return `left:${r.x}px;top:${r.y}px;width:${r.w}px;height:${r.h}px`;
  }

  // Which sprite set a toggle shows: its own, or the `on` set.
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
      const s = on ? el.on : el;
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
</script>

{#if el.type === "nineslice"}
  <!-- Nine boxes filling the window: corners at their size, edges stretched
       along one axis, the centre along both. Below every other element. -->
  <div class="sp-frame" aria-hidden="true">
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
    style="{box()};{vars(states)}"
    {onpointerdown}
  ></div>
{:else if (el.type === "button" || el.type === "toggle") && states}
  <!-- The glow, when the skin leaves it to the renderer (D73), sits on a
       wrapper: a filter is applied before a mask, so on the masked element
       itself the halo would be cut away with everything else outside the
       shape. Never on the visualizer's ancestors; this is a sibling. -->
  <div class="sp-glow" class:glow style={box()}>
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
{:else if el.type === "text" && font}
  <div
    class="sp-text"
    class:upper={font.type === "system" && font.case === "upper"}
    class:scroll={el.overflow === "scroll"}
    style="{box()};--tc:var(--{el.tint});--tc-i:var(--{el.inactive?.tint ?? el.tint});{font.type === 'system'
      ? `font-size:${font.size}px;letter-spacing:${font.tracking}em`
      : ''}"
  >
    {#if children}
      {@render children()}
    {:else}
      <span class="t">{text}</span>
    {/if}
  </div>
{/if}

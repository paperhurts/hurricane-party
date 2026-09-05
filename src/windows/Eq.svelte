<script lang="ts">
  // The equalizer window (D21). Sliders live here; the audio graph lives in
  // Main, so every change is sent over as the whole state and Main applies
  // it. Persisted in localStorage, which all the app's windows share, so both
  // sides read the same saved state at mount and only this window writes.
  import { emitTo, listen } from "@tauri-apps/api/event";
  import Classic from "./Classic.svelte";
  import {
    applyPreset,
    BANDS,
    clampDb,
    DB_MAX,
    DB_MIN,
    LABELS,
    loadEq,
    PRESETS,
    presetName,
    saveEq,
    shadeBarPx,
    trimDb,
    type EqState,
  } from "../lib/eq";

  let eq = $state<EqState>(loadEq(localStorage));
  let preset = $derived(presetName(eq));
  let trim = $derived(trimDb(eq));

  // The lamp stays lit a beat after the last clip Main reported, so a burst
  // of clipped blocks reads as one steady light rather than a flicker.
  let clip = $state(false);
  let clipTimer = 0;

  function commit() {
    saveEq(localStorage, eq);
    emitTo("main", "eq:set", $state.snapshot(eq)).catch(() => {});
  }

  // Half-dB steps: fine enough to be smooth, coarse enough that a preset can
  // be matched exactly after a nudge back.
  const snap = (db: number) => clampDb(Math.round(db * 2) / 2);

  function setBand(i: number, db: number) {
    eq.bands[i] = snap(db);
    commit();
  }
  function setPre(db: number) {
    eq.preamp = snap(db);
    commit();
  }
  function toggleOn() {
    eq.on = !eq.on;
    commit();
  }
  // The preset button opens a menu. It used to cycle, which is not what a ▼
  // promises.
  let menuOpen = $state(false);
  const presetNames = Object.keys(PRESETS);

  function pick(name: string) {
    eq = applyPreset(eq, name);
    menuOpen = false;
    commit();
  }

  $effect(() => {
    const sub = listen("eq:clip", () => {
      clip = true;
      clearTimeout(clipTimer);
      clipTimer = window.setTimeout(() => (clip = false), 400);
    });
    return () => {
      sub.then((off) => off());
      clearTimeout(clipTimer);
    };
  });

  /** Where a dB value sits on a track, as a percentage from the top. */
  const pct = (db: number) => ((DB_MAX - db) / (DB_MAX - DB_MIN)) * 100;

  // The response curve: one point per band, drawn in a stretched viewBox.
  let curve = $derived(
    eq.bands.map((v, i) => `${(i / (BANDS.length - 1)) * 100},${pct(eq.on ? v : 0)}`).join(" "),
  );

  // A vertical slider: press or drag sets the value from the pointer's
  // height, the wheel nudges by half a dB, a double-click returns to 0.
  function vslider(node: HTMLElement, set: (db: number) => void) {
    let fn = set;
    const at = (e: PointerEvent) => {
      const r = node.getBoundingClientRect();
      const frac = Math.min(1, Math.max(0, (e.clientY - r.top) / r.height));
      fn(DB_MAX - frac * (DB_MAX - DB_MIN));
    };
    const move = (e: PointerEvent) => at(e);
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    const down = (e: PointerEvent) => {
      if (e.button !== 0) return;
      at(e);
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    };
    const dbl = () => fn(0);
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      const cur = Number(node.dataset.db ?? 0);
      fn(cur + (e.deltaY < 0 ? 0.5 : -0.5));
    };
    node.addEventListener("pointerdown", down);
    node.addEventListener("dblclick", dbl);
    node.addEventListener("wheel", wheel, { passive: false });
    return {
      update(next: (db: number) => void) {
        fn = next;
      },
      destroy() {
        node.removeEventListener("pointerdown", down);
        node.removeEventListener("dblclick", dbl);
        node.removeEventListener("wheel", wheel);
        up();
      },
    };
  }
</script>

{#snippet slider(db: number, label: string, set: (db: number) => void, strong: boolean)}
  <div class="col" class:strong>
    <div class="track" use:vslider={set} data-db={db} title="{db > 0 ? '+' : ''}{db} dB">
      <div
        class="fill"
        style:top="{Math.min(pct(db), 50)}%"
        style:height="{Math.abs(pct(db) - 50)}%"
      ></div>
      <div class="thumb" class:hot={Math.abs(db) > 8} class:off={!eq.on} style:top="{pct(db)}%"></div>
    </div>
    <div class="lbl">{label}</div>
  </div>
{/snippet}

<svelte:window
  onpointerdown={() => (menuOpen = false)}
  onkeydown={(e) => {
    if (e.key === "Escape") menuOpen = false;
  }}
/>

<!-- The EQ's shade: on or off, the curve as ten bars, the preset (D79). -->
{#snippet shade()}
  <div class="shade">
    <span class="stag">EQ</span>
    <span class="stag" class:lit={eq.on}>{eq.on ? "ON" : "OFF"}</span>
    <div class="sbars" class:off={!eq.on} title={preset}>
      {#each eq.bands as db, i (i)}
        <i style:height="{shadeBarPx(db)}px"></i>
      {/each}
    </div>
    <span class="stag">{preset}</span>
  </div>
{/snippet}

<Classic label="eq" title="EQUALIZER" {shade}>
  <div class="eqw" class:off={!eq.on}>
    <div class="top">
      <div class="side">
        <button class="tg" class:lit={eq.on} onclick={toggleOn}>{eq.on ? "EQ ON" : "EQ OFF"}</button>
        <!-- Pointerdowns inside stay inside, so the window-level close does
             not fire under a click on one of the menu's own items. -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="presetwrap" onpointerdown={(e) => e.stopPropagation()}>
          <button
            class="tg preset"
            class:open={menuOpen}
            onclick={() => (menuOpen = !menuOpen)}
            title="Choose a preset"
          >
            <span class="name">{preset}</span><span class="arrow">▼</span>
          </button>
          {#if menuOpen}
            <div class="pmenu" role="menu">
              {#each presetNames as name (name)}
                <button role="menuitem" class:on={name === preset} onclick={() => pick(name)}>{name}</button>
              {/each}
            </div>
          {/if}
        </div>
        <div class="curve">
          <svg viewBox="0 0 100 100" preserveAspectRatio="none">
            <line x1="0" y1="50" x2="100" y2="50" class="zero" />
            <polyline points={curve} class="halo" />
            <polyline points={curve} class="line" />
          </svg>
        </div>
      </div>
      <div class="sliders">
        {@render slider(eq.preamp, "PRE", setPre, true)}
        {#each LABELS as label, i (label)}
          {@render slider(eq.bands[i], label, (db) => setBand(i, db), false)}
        {/each}
      </div>
    </div>
    <div class="bottom">
      <div class="trim" title="Automatic: pulled down by the largest band boost">
        TRIM {trim > 0 ? "+" : ""}{trim.toFixed(1)} dB
      </div>
      <div class="clip" class:lit={clip}>CLIP</div>
    </div>
  </div>
</Classic>

<style>
  .eqw {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 3px 4px;
    font-size: 7px;
    letter-spacing: 0.06em;
  }

  .top {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    gap: 5px;
  }

  /* ---- left column: switch, preset, response curve ---- */
  .side {
    flex: 0 0 60px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tg {
    height: 11px;
    padding: 0 3px;
    border: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 3px;
    font-size: 6px;
    letter-spacing: 0.1em;
    line-height: 1;
    color: color-mix(in srgb, var(--filament) 55%, transparent);
    background: color-mix(in srgb, var(--void) 70%, var(--well));
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--arc) 22%, transparent);
    cursor: pointer;
  }
  .tg:hover {
    color: var(--arc);
    background: color-mix(in srgb, var(--void) 70%, var(--well));
    box-shadow: inset 0 0 0 1px var(--arc);
  }
  .tg.lit {
    color: var(--arc);
    background: color-mix(in srgb, var(--arc) 14%, var(--void));
    box-shadow:
      inset 0 0 0 1px var(--arc),
      0 0 8px color-mix(in srgb, var(--arc) 45%, transparent);
  }
  .tg.preset {
    justify-content: space-between;
    color: var(--filament);
    background: var(--well);
  }
  .tg.preset .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tg.preset .arrow {
    color: var(--arc);
    font-size: 5px;
  }
  .presetwrap {
    position: relative;
    display: flex;
    flex-direction: column;
  }
  .tg.preset.open {
    box-shadow: inset 0 0 0 1px var(--arc);
  }
  /* The menu overlays the curve box below the button; the window is small. */
  .pmenu {
    position: absolute;
    left: 0;
    right: 0;
    top: 12px;
    z-index: 5;
    display: flex;
    flex-direction: column;
    padding: 2px 0;
    background: var(--void);
    box-shadow:
      inset 0 0 0 1px var(--arc),
      0 0 8px color-mix(in srgb, var(--arc) 35%, transparent);
  }
  .pmenu button {
    height: 11px;
    padding: 0 4px;
    border: 0;
    text-align: left;
    font: inherit;
    font-size: 6px;
    letter-spacing: 0.1em;
    line-height: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--filament);
    background: transparent;
    cursor: pointer;
  }
  .pmenu button:hover,
  .pmenu button.on {
    color: var(--arc);
    background: color-mix(in srgb, var(--arc) 14%, transparent);
  }
  .curve {
    flex: 1 1 auto;
    min-height: 0;
    background: var(--well);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--arc) 14%, transparent);
  }
  .curve svg {
    display: block;
    width: 100%;
    height: 100%;
  }
  .zero {
    stroke: color-mix(in srgb, var(--filament) 15%, transparent);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  /* A sharp core over a soft halo (theme.md): glow that reads as glow. */
  .halo {
    fill: none;
    stroke: color-mix(in srgb, var(--arc) 35%, transparent);
    stroke-width: 3;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
  }
  .line {
    fill: none;
    stroke: var(--arc);
    stroke-width: 1;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
  }
  .off .line {
    stroke: color-mix(in srgb, var(--filament) 35%, transparent);
  }
  .off .halo {
    stroke: transparent;
  }

  /* ---- sliders ---- */
  .sliders {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    gap: 2px;
  }
  .col {
    flex: 1 1 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }
  .track {
    position: relative;
    flex: 1 1 auto;
    width: 100%;
    cursor: ns-resize;
  }
  /* The rail: one centred hairline, with the centre tick for 0 dB. */
  .track::before {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: 50%;
    width: 1px;
    margin-left: -0.5px;
    background: color-mix(in srgb, var(--arc) 22%, transparent);
  }
  .track::after {
    content: "";
    position: absolute;
    top: 50%;
    left: 50%;
    width: 5px;
    height: 1px;
    margin: -0.5px 0 0 -2.5px;
    background: color-mix(in srgb, var(--filament) 22%, transparent);
  }
  /* The lit part of the rail from 0 dB to the thumb. */
  .fill {
    position: absolute;
    left: 50%;
    width: 1px;
    margin-left: -0.5px;
    background: var(--arc);
  }
  .off .fill {
    background: color-mix(in srgb, var(--filament) 30%, transparent);
  }
  .thumb {
    position: absolute;
    left: 50%;
    width: 9px;
    height: 3px;
    margin: -1.5px 0 0 -4.5px;
    background: var(--arc);
    box-shadow: 0 0 4px color-mix(in srgb, var(--arc) 70%, transparent);
  }
  .thumb.hot {
    background: var(--strike);
    box-shadow: 0 0 4px color-mix(in srgb, var(--strike) 80%, transparent);
  }
  .thumb.off {
    background: color-mix(in srgb, var(--filament) 45%, transparent);
    box-shadow: none;
  }
  .lbl {
    font-size: 6px;
    letter-spacing: 0.05em;
    color: color-mix(in srgb, var(--filament) 40%, transparent);
    white-space: nowrap;
  }
  .col.strong .lbl {
    color: color-mix(in srgb, var(--filament) 65%, transparent);
  }

  /* ---- bottom: the automatic trim, and the clip lamp ---- */
  .bottom {
    flex: 0 0 13px;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .trim {
    font-size: 6px;
    letter-spacing: 0.1em;
    color: color-mix(in srgb, var(--filament) 40%, transparent);
  }
  .clip {
    height: 13px;
    padding: 0 5px;
    display: flex;
    align-items: center;
    font-size: 6px;
    letter-spacing: 0.12em;
    color: color-mix(in srgb, var(--filament) 30%, transparent);
    background: color-mix(in srgb, var(--void) 70%, var(--well));
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--arc) 18%, transparent);
  }
  .clip.lit {
    color: var(--strike);
    background: color-mix(in srgb, var(--strike) 14%, var(--void));
    box-shadow:
      inset 0 0 0 1px var(--strike),
      0 0 8px color-mix(in srgb, var(--strike) 45%, transparent);
  }
</style>

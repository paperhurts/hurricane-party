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

  // ---- what the skin draws (#3) ----
  //
  // The manifest says where the switch, the preset button, the curve, the
  // eleven sliders and the lamp are, and how each is drawn; this window says
  // what they read. A gain crosses as a 0..1 fraction, 0.5 being 0 dB.
  const frac = (db: number) => (db - DB_MIN) / (DB_MAX - DB_MIN);
  const dbOf = (f: number) => DB_MIN + f * (DB_MAX - DB_MIN);

  let binds = $derived({
    eqOn: eq.on ? "on" : "off",
    eqPreset: preset,
    eqMenu: menuOpen ? "open" : "closed",
    eqTrim: `${trim > 0 ? "+" : ""}${trim.toFixed(1)} dB`,
    eqClip: clip ? "on" : "off",
    eqPre: frac(eq.preamp),
    ...Object.fromEntries(eq.bands.map((db, i) => [`eqBand${i + 1}`, frac(db)])),
  });

  function action(name: string) {
    if (name === "eqOn") toggleOn();
    else if (name === "eqPresets") menuOpen = !menuOpen;
  }

  // A drag, a wheel notch, or a double press back to 0 dB all arrive as a
  // fraction; the half-dB snap happens where it always did.
  function slide(bind: string, f: number) {
    if (bind === "eqPre") setPre(dbOf(f));
    else if (bind.startsWith("eqBand")) setBand(Number(bind.slice(6)) - 1, dbOf(f));
  }
</script>

<!-- Any press outside the menu closes it. In the capture phase, because the
     skin's sliders and buttons stop their pointerdown from bubbling (so a
     press is neither a drag nor a double-tap on the title bar), and a
     bubbling listener here never heard a press on a slider, the switch or
     shade. The preset button is spared: its own click toggles the menu. -->
<svelte:window
  onpointerdowncapture={(e) => {
    const t = e.target as Element | null;
    if (t?.closest?.(".pmenu") || t?.closest?.('[data-el="eqPresetButton"]')) return;
    menuOpen = false;
  }}
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

<!-- The response curve, in whatever box the skin gave it, and the preset
     menu over it: the menu drops from the button straight onto the curve,
     which is where it always opened. Both are this window's to draw. -->
{#snippet curveBox()}
  <svg class="curve" class:off={!eq.on} viewBox="0 0 100 100" preserveAspectRatio="none">
    <line x1="0" y1="50" x2="100" y2="50" class="zero" />
    <polyline points={curve} class="halo" />
    <polyline points={curve} class="line" />
  </svg>
  {#if menuOpen}
    <div class="pmenu" role="menu">
      {#each presetNames as name (name)}
        <button role="menuitem" class:on={name === preset} onclick={() => pick(name)}>{name}</button>
      {/each}
    </div>
  {/if}
{/snippet}

<Classic
  label="eq"
  title="EQUALIZER"
  {shade}
  {binds}
  slots={{ eqCurveWell: curveBox }}
  onaction={action}
  onslide={slide}
/>

<style>
  /* The curve: the app's, drawn on the skin's well. A sharp core over a soft
     halo (theme.md), and grey while the EQ is off. */
  .curve {
    display: block;
    width: 100%;
    height: 100%;
  }
  .zero {
    stroke: color-mix(in srgb, var(--filament) 15%, transparent);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
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
  .curve.off .line {
    stroke: color-mix(in srgb, var(--filament) 35%, transparent);
  }
  .curve.off .halo {
    stroke: transparent;
  }
  /* The glow toggle off (D100): the curve's core without its halo, and the
     menu's edge without its bloom. */
  :global(.chrome[data-glow="off"]) .halo {
    stroke: transparent;
  }
  :global(.chrome[data-glow="off"]) .pmenu {
    box-shadow: inset 0 0 0 1px var(--arc);
  }

  /* The preset menu, over the curve. The skin's boxes let the pointer
     through; the menu takes it back. */
  .pmenu {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    z-index: 5;
    display: flex;
    flex-direction: column;
    padding: 2px 0;
    pointer-events: auto;
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
</style>

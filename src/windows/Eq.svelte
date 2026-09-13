<script lang="ts">
  // The equalizer window (D21). Sliders live here; the audio graph lives in
  // Main, so every change is sent over as the whole state and Main applies
  // it. Persisted in localStorage, which all the app's windows share, so both
  // sides read the same saved state at mount and only this window writes.
  import { invoke } from "@tauri-apps/api/core";
  import { emitTo, listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { tick } from "svelte";
  import Classic from "./Classic.svelte";
  import {
    applyPreset,
    BANDS,
    clampDb,
    CUSTOM,
    DB_MAX,
    DB_MIN,
    loadEq,
    presetName,
    saveEq,
    shadeBarPx,
    SHIPPED,
    shipsAs,
    trimDb,
    type EqState,
    type Preset,
  } from "../lib/eq";

  let eq = $state<EqState>(loadEq(localStorage));
  // The person's own presets (#145), from the database; the four that ship
  // come first and cannot be removed.
  let mine = $state<Preset[]>([]);
  let presets = $derived([...SHIPPED, ...mine]);
  let preset = $derived(presetName(eq, presets));
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

  function pick(p: Preset) {
    eq = applyPreset(eq, p);
    menuOpen = false;
    commit();
  }

  async function loadMine() {
    try {
      mine = await invoke<Preset[]>("eq_presets");
    } catch {
      // No backend (a browser, a test): the four that ship are all there is.
    }
  }
  loadMine();

  // A line where the preset's name goes, for a moment: what an import or a
  // save did. The EQ window has nowhere else to say it.
  let said = $state<string | null>(null);
  let saidTimer = 0;
  function say(line: string) {
    said = line;
    clearTimeout(saidTimer);
    saidTimer = window.setTimeout(() => (said = null), 2500);
  }

  // SAVE: the menu's last row becomes a name field. Enter keeps it, Esc
  // or a press elsewhere lets it go.
  let naming = $state(false);
  let newName = $state("");
  let nameEl = $state<HTMLInputElement | null>(null);
  async function startNaming() {
    naming = true;
    newName = preset === CUSTOM ? "" : preset;
    await tick();
    nameEl?.focus();
    nameEl?.select();
  }
  async function saveAs() {
    const name = newName.trim();
    if (!name) return;
    if (shipsAs(name)) {
      say("THAT NAME SHIPS");
      return;
    }
    try {
      await invoke("save_eq_preset", { name, preamp: eq.preamp, bands: $state.snapshot(eq.bands) });
      await loadMine();
      naming = false;
      menuOpen = false;
      say(`SAVED ${name}`);
    } catch (e) {
      say(String(e).toUpperCase());
    }
  }
  // A menu that closes takes an unfinished name with it; one that opens
  // shows the preset in use, which can be below the list's fold.
  let menuEl = $state<HTMLElement | null>(null);
  $effect(() => {
    if (!menuOpen) naming = false;
    else menuEl?.querySelector(".plist .on")?.scrollIntoView({ block: "nearest" });
  });
  function nameKey(e: KeyboardEvent) {
    if (e.key === "Enter") saveAs();
    else if (e.key === "Escape") naming = false;
  }

  async function remove(p: Preset) {
    if (p.id === undefined) return;
    await invoke("delete_eq_preset", { id: p.id }).catch(() => {});
    await loadMine();
  }

  // IMPORT: Winamp `.eqf` files, one preset or a library of them (D31).
  async function importEqf() {
    menuOpen = false;
    const picked = await openDialog({
      multiple: true,
      title: "Import EQ presets",
      filters: [{ name: "Winamp EQ preset", extensions: ["eqf"] }],
    });
    const paths = picked === null ? [] : Array.isArray(picked) ? picked : [picked];
    if (!paths.length) return;
    const got = await invoke<{ saved: number; refused: [string, string][]; cut_short: string[] }>("import_eqf", {
      paths,
    }).catch((e) => ({ saved: 0, refused: [["", String(e)]] as [string, string][], cut_short: [] }));
    await loadMine();
    for (const [file, why] of got.refused) console.warn(`EQ import: ${file}: ${why}`);
    if (got.saved === 0) say(got.refused.length ? "NOT AN EQF" : "NO PRESETS");
    else say(`+${got.saved} PRESET${got.saved === 1 ? "" : "S"}${got.refused.length || got.cut_short.length ? ", SOME NOT" : ""}`);
    if (got.saved) menuOpen = true;
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
    eqPreset: said ?? preset,
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
    if (e.key === "Escape") {
      menuOpen = false;
      naming = false;
    }
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
    <div class="pmenu" role="menu" bind:this={menuEl}>
      <!-- The list scrolls; SAVE and IMPORT stay at the bottom where they can
           be reached, however many presets the person has. -->
      <div class="plist">
        {#each SHIPPED as p (p.name)}
          <button role="menuitem" class:on={p.name === preset} onclick={() => pick(p)}>{p.name}</button>
        {/each}
        {#if mine.length}
          <hr />
          {#each mine as p (p.id)}
            <div class="mine">
              <button role="menuitem" class:on={p.name === preset} title={p.name} onclick={() => pick(p)}
                >{p.name}</button
              >
              <button class="x" title="Remove {p.name}" aria-label="Remove {p.name}" onclick={() => remove(p)}
                >×</button
              >
            </div>
          {/each}
        {/if}
      </div>
      <hr />
      {#if naming}
        <input
          class="pname"
          bind:this={nameEl}
          bind:value={newName}
          maxlength="40"
          placeholder="NAME, THEN ENTER"
          onkeydown={nameKey}
        />
      {:else}
        <div class="acts">
          <button role="menuitem" title="Keep the EQ as it is now under a name" onclick={startNaming}>SAVE…</button>
          <button role="menuitem" title="Presets from Winamp .eqf files" onclick={importEqf}>IMPORT…</button>
        </div>
      {/if}
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
    stroke: color-mix(in srgb, var(--text) 15%, transparent);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  .halo {
    fill: none;
    stroke: color-mix(in srgb, var(--accent) 35%, transparent);
    stroke-width: 3;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
  }
  .line {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
  }
  .curve.off .line {
    stroke: color-mix(in srgb, var(--text) 35%, transparent);
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
    box-shadow: inset 0 0 0 1px var(--accent);
  }

  /* The preset menu, over the curve. The skin's boxes let the pointer
     through; the menu takes it back. */
  /* Wider than the curve box, over the first sliders while it is open, so a
     name from a Winamp file reads; as tall as the window leaves below the
     curve's top, with the list scrolling past that. */
  .pmenu {
    position: absolute;
    left: 0;
    top: 0;
    z-index: 5;
    display: flex;
    flex-direction: column;
    width: 110px;
    max-height: 70px;
    padding: 2px 0;
    pointer-events: auto;
    background: var(--ground);
    box-shadow:
      inset 0 0 0 1px var(--accent),
      0 0 8px color-mix(in srgb, var(--accent) 35%, transparent);
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
    color: var(--text);
    background: transparent;
    cursor: pointer;
  }
  .pmenu button:hover,
  .pmenu button.on {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .pmenu button,
  .pname {
    flex: none;
    text-transform: uppercase;
  }
  .pmenu hr {
    flex: none;
    height: 1px;
    margin: 2px 0;
    border: 0;
    background: color-mix(in srgb, var(--accent) 25%, transparent);
  }
  .plist {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: color-mix(in srgb, var(--accent) 50%, transparent) transparent;
  }
  /* One of the person's: the name, and the × that removes it. */
  .mine {
    display: flex;
  }
  .mine button:first-child {
    flex: 1;
    min-width: 0;
  }
  .pmenu .x {
    width: 11px;
    padding: 0;
    text-align: center;
    color: color-mix(in srgb, var(--text) 50%, transparent);
  }
  .pmenu .x:hover {
    color: var(--warn);
    background: color-mix(in srgb, var(--warn) 14%, transparent);
  }
  .acts {
    display: flex;
  }
  .acts button {
    flex: 1;
  }
  .pname {
    height: 11px;
    margin: 0 2px;
    padding: 0 2px;
    border: 0;
    outline: 1px solid color-mix(in srgb, var(--accent) 50%, transparent);
    font: inherit;
    font-size: 6px;
    letter-spacing: 0.1em;
    color: var(--text);
    background: var(--surface);
  }
  .pname::placeholder {
    color: color-mix(in srgb, var(--text) 40%, transparent);
  }
</style>

<script lang="ts">
  // The visuals window (#167, D169): a window of its own for the visual that
  // fills a room, fed sixty times a second from the analyser Main already has.
  // The style changes only when the person picks another; full screen on a
  // double-click or F11, and Esc leaves it.
  import { onMount } from "svelte";
  import { Channel, invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { applyTheme, isWearable, rampWorn, type ThemeName } from "./lib/theme";
  import { currentSkin } from "./lib/skins";
  import { Flow } from "./lib/flow";
  import { Fractal } from "./lib/fractal";
  import type { Visual } from "./lib/gl";
  import { parseVisualsFrame, silentFrame, type VisualsFrame } from "./lib/visualsframe";

  const LABEL = "visuals";
  const target = { target: { kind: "WebviewWindow" as const, label: LABEL } };

  /** The styles to pick from, in the order the bar shows them. */
  const STYLES = [
    { id: "flow", name: "Flow" },
    { id: "fractal", name: "Fractal" },
  ] as const;
  type StyleId = (typeof STYLES)[number]["id"];
  const STYLE_KEY = "hp.visuals.style";

  function savedStyle(): StyleId {
    try {
      const s = localStorage.getItem(STYLE_KEY);
      if (STYLES.some((x) => x.id === s)) return s as StyleId;
    } catch {
      // No storage: the first style.
    }
    return STYLES[0].id;
  }

  let canvas: HTMLCanvasElement;
  let style = $state<StyleId>(savedStyle());
  let full = $state(false);
  let bar = $state(true);
  /** Why a style could not start, by style: shown while it is picked. */
  let broken = $state<Partial<Record<StyleId, string>>>({});

  let frame: VisualsFrame = silentFrame();
  const visuals: Partial<Record<StyleId, Visual>> = {};
  let calmSetting = false;
  let reduced = false;

  function pick(id: StyleId) {
    style = id;
    try {
      localStorage.setItem(STYLE_KEY, id);
    } catch {
      // Remembered for this window only.
    }
  }

  function applyCalm() {
    for (const v of Object.values(visuals)) v.calm = calmSetting || reduced;
  }

  async function toggleFull(on?: boolean) {
    try {
      full = await invoke<boolean>("visuals_fullscreen", { on: on ?? null });
    } catch (e) {
      console.error(e);
    }
  }

  // The bar shows while the mouse moves and fades after a moment's rest,
  // with the pointer, so nothing sits over the visual.
  let restTimer: ReturnType<typeof setTimeout> | null = null;
  function woke() {
    bar = true;
    if (restTimer) clearTimeout(restTimer);
    restTimer = setTimeout(() => (bar = false), 2500);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "F11") {
      e.preventDefault();
      toggleFull();
    } else if (e.key === "Escape" && full) {
      toggleFull(false);
    }
  }

  function loadColours() {
    const asked = invoke<string>("get_theme").catch(() => "eyewall");
    Promise.all([currentSkin(), asked]).then(([w, t]) => {
      const theme: ThemeName = isWearable(t) ? t : "eyewall";
      applyTheme(theme);
      const ramp = rampWorn(w.skin, theme);
      for (const v of Object.values(visuals)) v.setRamp(ramp);
    });
  }

  onMount(() => {
    // Every style shares the canvas's one context; one that cannot start
    // says why when it is picked, and the others still work.
    const makers: Record<StyleId, (c: HTMLCanvasElement) => Visual> = {
      flow: (c) => new Flow(c),
      fractal: (c) => new Fractal(c),
    };
    for (const { id } of STYLES) {
      try {
        visuals[id] = makers[id](canvas);
      } catch (e) {
        broken[id] = e instanceof Error ? e.message : String(e);
      }
    }
    loadColours();
    invoke<boolean>("get_calm").then((c) => {
      calmSetting = c;
      applyCalm();
    }, () => {});
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    const onMotion = () => {
      reduced = motion.matches;
      applyCalm();
    };
    onMotion();
    motion.addEventListener("change", onMotion);

    // The frames: newest wins, drawn on the next animation frame.
    const channel = new Channel<ArrayBuffer>((buf) => {
      const f = parseVisualsFrame(buf);
      if (f) frame = f;
    });
    invoke("visuals_subscribe", { frames: channel }).catch((e) => console.error(e));

    // The canvas in physical pixels, so the trails are as fine as the screen.
    const sizer = new ResizeObserver((entries) => {
      const e = entries[0];
      const box = e.devicePixelContentBoxSize?.[0];
      const w = Math.max(1, Math.round(box ? box.inlineSize : e.contentRect.width * devicePixelRatio));
      const h = Math.max(1, Math.round(box ? box.blockSize : e.contentRect.height * devicePixelRatio));
      canvas.width = w;
      canvas.height = h;
      for (const v of Object.values(visuals)) v.resize(w, h);
    });
    try {
      sizer.observe(canvas, { box: "device-pixel-content-box" });
    } catch {
      sizer.observe(canvas);
    }

    let raf = 0;
    let last = performance.now();
    const loop = (now: number) => {
      const dt = Math.min(0.1, (now - last) / 1000);
      last = now;
      visuals[style]?.render(frame, now / 1000, dt);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);

    const subs = [
      listen("skin:changed", () => loadColours(), target),
      listen("theme:changed", () => loadColours(), target),
      listen<boolean>("vis:calm", (e) => {
        calmSetting = e.payload;
        applyCalm();
      }, target),
    ];
    woke();

    return () => {
      cancelAnimationFrame(raf);
      sizer.disconnect();
      motion.removeEventListener("change", onMotion);
      subs.forEach((s) => s.then((un) => un()));
      for (const v of Object.values(visuals)) v.dispose();
    };
  });
</script>

<svelte:window onkeydown={onKey} onmousemove={woke} />

<main class:rest={!bar}>
  <canvas bind:this={canvas} ondblclick={() => toggleFull()}></canvas>
  {#if broken[style]}
    <p class="broken">This style would not start on this machine's graphics: {broken[style]}</p>
  {/if}
  <nav class:hidden={!bar} aria-label="Visuals">
    {#each STYLES as s (s.id)}
      <button class:on={style === s.id} aria-pressed={style === s.id} onclick={() => pick(s.id)}>{s.name}</button>
    {/each}
    <span class="gap"></span>
    <button onclick={() => toggleFull()} title="Full screen: double-click or F11; Esc leaves it">
      {full ? "Leave full screen" : "Full screen"}
    </button>
  </nav>
</main>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    height: 100%;
    overflow: hidden;
    background: var(--ground);
  }
  main {
    position: fixed;
    inset: 0;
  }
  main.rest {
    cursor: none;
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
  nav {
    position: absolute;
    left: 12px;
    right: 12px;
    bottom: 12px;
    display: flex;
    gap: 6px;
    align-items: center;
    transition: opacity 400ms;
  }
  nav.hidden {
    opacity: 0;
    pointer-events: none;
  }
  .gap {
    flex: 1;
  }
  button {
    font: 12px var(--mono, ui-monospace, monospace);
    color: var(--text);
    background: color-mix(in srgb, var(--surface) 80%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 50%, transparent);
    border-radius: 4px;
    padding: 4px 10px;
    cursor: pointer;
  }
  button:hover,
  button.on {
    border-color: var(--accent);
    color: var(--accent);
  }
  .broken {
    position: absolute;
    inset: 40% 10% auto;
    color: var(--alert);
    font: 14px var(--mono, ui-monospace, monospace);
    text-align: center;
  }
</style>

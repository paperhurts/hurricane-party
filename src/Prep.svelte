<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { applyTheme, isWearable } from "./lib/theme";
  import { headline, plan, remaining, type Line, type Picks, type Read, type ReadState } from "./lib/prep";
  import { pressure, sayPressure, size, used, type StorageStatus } from "./lib/storage";
  import type { RadarSite, RadarStatus } from "./lib/radar";
  import pockets from "./assets/capybara-seagull-pockets.png";

  // Hurricane Party Planning (#163, D140). Not a wizard (O5): paste, look,
  // press. Everything it queues goes through the same queue as the library.

  type Progress = {
    id: number;
    want_video: boolean;
    created_at: number;
    total: number;
    done: number;
    running: number;
    queued: number;
    paused: number;
    failed: number;
    offline: number;
    written: number;
    expected: number;
    unknown: number;
    failures: { id: number; title: string; error: string }[];
  };

  const target = { target: { kind: "WebviewWindow" as const, label: "prep" } };
  /** How long a list that could not be read for want of a connection waits before it is read again. */
  const READ_RETRY_MS = 30_000;
  /** Lists read at once: each is a yt-dlp run. */
  const READS_AT_ONCE = 2;

  let text = $state("");
  let lines = $state<Line[]>([]);
  const reads = new SvelteMap<string, ReadState>();
  let picks = $state<Picks>({});
  const open = new SvelteSet<string>();
  let wantVideo = $state(false);
  let storage = $state<StorageStatus | null>(null);
  let radar = $state<RadarStatus | null>(null);
  let sites = $state<RadarSite[]>([]);
  let progress = $state<Progress | null>(null);
  let hidden = $state<number | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let said = $state<string | null>(null);

  let p = $derived(plan(lines, reads, picks, wantVideo, storage));
  let chosen = $derived(wantVideo ? p.video : p.audio);
  let press = $derived(storage && chosen.bytes > 0 ? pressure(storage, chosen.bytes) : null);
  let states = $derived([...new Set(sites.map((s) => s.state))].sort());
  let showProgress = $derived(progress != null && progress.id !== hidden);
  let inFlight = $derived(progress != null && progress.running + progress.queued + progress.offline > 0);

  // The pasted text is kept as it is typed, and sorted into lines a moment
  // after the typing stops.
  let sortTimer: ReturnType<typeof setTimeout> | undefined;
  function edited() {
    clearTimeout(sortTimer);
    sortTimer = setTimeout(sort, 350);
  }
  async function sort() {
    const now = text;
    invoke("set_prep_draft", { text: now }).catch(() => {});
    lines = await invoke<Line[]>("prep_lines", { text: now });
    readWhatIsNew();
  }

  let readingNow = 0;
  /** Read every list line not read yet, a couple at a time. */
  function readWhatIsNew() {
    for (const line of lines) {
      if (readingNow >= READS_AT_ONCE) return;
      if (line.kind !== "list" || !line.url || reads.has(line.url)) continue;
      readOne(line.url);
    }
  }
  async function readOne(url: string) {
    readingNow++;
    reads.set(url, "reading");
    let got: Read;
    try {
      got = await invoke<Read>("prep_read", { url });
    } catch (e) {
      got = { status: "failed", list: null, message: String(e) };
    }
    reads.set(url, got);
    readingNow--;
    if (got.status === "offline") {
      // Held and read again until it goes through (the owner's call on #163).
      setTimeout(() => {
        if (lines.some((l) => l.url === url) && reads.get(url) !== "reading") {
          reads.delete(url);
          readWhatIsNew();
        }
      }, READ_RETRY_MS);
    }
    readWhatIsNew();
  }

  function toggle(key: string, now: boolean) {
    picks = { ...picks, [key]: !now };
  }

  async function refreshStorage() {
    storage = await invoke<StorageStatus>("storage_status").catch(() => storage);
  }
  async function refreshProgress() {
    progress = await invoke<Progress | null>("prep_progress").catch(() => progress);
  }

  async function go() {
    if (!p.count || busy) return;
    busy = true;
    error = null;
    const left = remaining(text, p.leftOver);
    try {
      await invoke<number>("prep_go", { run: { ...p.go, remaining: left } });
      said = `Saving ${p.count}. Closing this window stops nothing: they carry on in the library's Downloads.`;
      text = left;
      picks = {};
      hidden = null;
      await sort();
      await refreshProgress();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function act(action: "pause" | "resume" | "retry") {
    if (!progress) return;
    try {
      await invoke<number>("prep_act", { batchId: progress.id, action });
      await refreshProgress();
    } catch (e) {
      error = String(e);
    }
  }

  function hide() {
    if (!progress) return;
    hidden = progress.id;
    try {
      localStorage.setItem("prep.hidden", String(progress.id));
    } catch {
      // A preference, not state: nothing depends on it.
    }
  }

  async function pickRadar(id: string) {
    radar = await invoke<RadarStatus>("set_radar_site", { id }).catch(() => radar);
  }

  onMount(() => {
    applyTheme("eyewall");
    const wear = (t: unknown) => applyTheme(isWearable(t) ? t : "eyewall");
    invoke<string>("get_theme").then(wear, () => {});
    try {
      const h = localStorage.getItem("prep.hidden");
      if (h) hidden = Number(h);
    } catch {
      // No stored preference: show the last run.
    }
    invoke<string>("get_prep_draft").then((d) => {
      untrack(() => {
        if (!text) {
          text = d;
          sort();
        }
      });
    });
    refreshStorage();
    refreshProgress();
    invoke<RadarStatus>("get_radar")
      .then((r) => {
        radar = r;
        if (!r.site) invoke<RadarSite[]>("radar_sites").then((s) => (sites = s));
      })
      .catch(() => {});

    const subs = [
      listen<string>("theme:changed", (e) => wear(e.payload), target),
      listen("jobs-changed", () => {
        refreshProgress();
        refreshStorage();
      }, target),
      listen("library-changed", refreshStorage, target),
    ];
    // The run's progress, while it has downloads to make; the drive's room
    // follows it more slowly.
    let beat = 0;
    const tick = setInterval(() => {
      if (!inFlight) return;
      refreshProgress();
      if (++beat % 3 === 0) refreshStorage();
    }, 2000);
    return () => {
      clearInterval(tick);
      clearTimeout(sortTimer);
      subs.forEach((s) => s.then((f) => f()));
    };
  });
</script>

<main>
  <header>
    <h1>Hurricane Party Planning</h1>
    <span class="head" class:warn={p.offline > 0 || p.failed > 0}>{headline(p)}</span>
  </header>

  <textarea
    bind:value={text}
    oninput={edited}
    placeholder="Paste every link you want to keep, one per line: videos and playlists."
    spellcheck="false"
  ></textarea>

  {#if p.state === "empty" && !showProgress}
    <div class="empty">
      <img src={pockets} alt="A capybara with a seagull on its head, hands in its pockets" width="160" height="160" />
      <div>
        <div class="big">Nothing to save yet.</div>
        <div class="dim">Paste links above. Lists are read before anything downloads, and everything goes into the queue that survives a power cut.</div>
      </div>
    </div>
  {:else if p.rows.length}
    <ol class="rows">
      {#each p.rows as row (row.n)}
        {#if row.kind === "not_link"}
          <li class="row bad">
            <span class="nobox"></span>
            <span class="num">{String(row.n).padStart(2, "0")}</span>
            <span class="what">{row.text}</span>
            <span class="note warn">not a link</span>
          </li>
        {:else if row.kind === "video"}
          <li class="row" class:off={!row.picked}>
            <input type="checkbox" checked={row.picked} onchange={() => toggle(row.key, row.picked)} />
            <span class="num">{String(row.n).padStart(2, "0")}</span>
            <span class="what">{row.text}</span>
            {#if row.dupOf != null}
              <span class="note">same as line {row.dupOf}</span>
            {:else if row.held}
              <span class="note">in the library</span>
            {:else if row.otherHeld}
              <span class="note dim">{wantVideo ? "MP3" : "video"} in the library</span>
            {:else if row.fromList}
              <span class="note dim" title={row.fromList}>the video, not the list</span>
            {/if}
            {#if row.picked}<span class="size dim">size when it starts</span>{/if}
          </li>
        {:else}
          <li class="row list">
            <button class="mini ghost fold" onclick={() => (open.has(row.url) ? open.delete(row.url) : open.add(row.url))} disabled={!row.entries.length}>
              {open.has(row.url) ? "▾" : "▸"}
            </button>
            <span class="num">{String(row.n).padStart(2, "0")}</span>
            <span class="what"><strong>{row.name}</strong></span>
            {#if row.state === "reading"}
              <span class="note">reading…</span>
            {:else if row.state === "offline"}
              <span class="note warn" title={row.message ?? ""}>no connection: reading it again in a moment</span>
            {:else if row.state === "failed"}
              <span class="note warn" title={row.message ?? ""}>could not be read</span>
            {:else}
              <span class="note">
                {row.entries.filter((e) => e.picked).length} of {row.entries.length}
                {#if row.entries.some((e) => e.held)}· {row.entries.filter((e) => e.held).length} in the library{/if}
              </span>
            {/if}
          </li>
          {#if open.has(row.url)}
            {#each row.entries as e (e.key)}
              <li class="row entry" class:off={!e.picked}>
                <input type="checkbox" checked={e.picked} onchange={() => toggle(e.key, e.picked)} />
                <span class="what">{e.item.title}</span>
                {#if e.dupOf != null}
                  <span class="note">also on line {e.dupOf}</span>
                {:else if e.held}
                  <span class="note">in the library</span>
                {:else if e.otherHeld}
                  <span class="note dim">{wantVideo ? "MP3" : "video"} in the library</span>
                {:else if e.item.missing}
                  <span class="note warn">{e.item.missing}</span>
                {/if}
              </li>
            {/each}
          {/if}
        {/if}
      {/each}
    </ol>
  {/if}

  {#if storage?.drive}
    <!-- The storage budget's figures (#162): what is on the drive, and what this would add. -->
    <div class="drive" title="The drive downloads go to">
      <div class="bar">
        <div class="usedpart" style:width={`${used(storage.drive) * 100}%`}></div>
        <div class="adds" class:warn={press != null} style:width={`${Math.min(1 - used(storage.drive), chosen.bytes / storage.drive.total) * 100}%`}></div>
      </div>
      <span class="free">
        {#if p.count}about {size(chosen.bytes)}{chosen.unknown ? ` + ${chosen.unknown} not known yet` : ""} ·{/if}
        {size(storage.drive.free)} free of {size(storage.drive.total)}
      </span>
    </div>
    {#if press}<p class="warn say">{sayPressure(press, storage, chosen.bytes)}</p>{/if}
  {:else if storage}
    <p class="warn say">The download folder is not there: downloads wait until it is back.</p>
  {/if}

  {#if radar && !radar.site && sites.length}
    <!-- Optional, and never in the way of the button (the owner's call on #163). -->
    <label class="radar">
      <span class="dim">Radar loop: not saved for the outage.</span>
      <select value="" onchange={(e) => pickRadar(e.currentTarget.value)}>
        <option value="">Pick your radar…</option>
        {#each states as st (st)}
          <optgroup label={st}>
            {#each sites.filter((s) => s.state === st) as s (s.id)}
              <option value={s.id}>{s.id} — {s.name}</option>
            {/each}
          </optgroup>
        {/each}
      </select>
    </label>
  {:else if radar?.site}
    <p class="dim radar">Radar loop: {radar.site.id} fills while this runs, whatever the theme.</p>
  {/if}

  <div class="press">
    <button class="go" onclick={go} disabled={!p.count || busy}>
      <span class="big">{busy ? "Queueing…" : `Save all ${p.count}`}</span>
      {#if p.leftOver.length && p.count}<span class="small">{p.leftOver.length} {p.leftOver.length === 1 ? "line stays" : "lines stay"} here</span>{/if}
    </button>
    <div class="kind" role="radiogroup" aria-label="Audio or video">
      <button class:on={!wantVideo} role="radio" aria-checked={!wantVideo} onclick={() => (wantVideo = false)}>
        <span>Audio only</span>
        {#if p.count}<span class="small">about {size(p.audio.bytes)}</span>{/if}
      </button>
      <button class:on={wantVideo} role="radio" aria-checked={wantVideo} onclick={() => (wantVideo = true)}>
        <span>Video</span>
        {#if p.count}<span class="small">about {size(p.video.bytes)}</span>{/if}
      </button>
    </div>
  </div>

  {#if said}<p class="said">{said}</p>{/if}
  {#if error}<p class="warn say">{error}</p>{/if}

  {#if showProgress && progress}
    <section class="run">
      <div class="runhead">
        <strong>Last run</strong>
        <span class="tally">
          {progress.done} of {progress.total} saved
          {#if progress.running}· {progress.running} downloading{/if}
          {#if progress.queued}· {progress.queued} queued{/if}
          {#if progress.paused}· {progress.paused} paused{/if}
          {#if progress.offline}<span class="warn">· {progress.offline} waiting for a connection</span>{/if}
          {#if progress.failed}<span class="warn">· {progress.failed} failed</span>{/if}
        </span>
        <span class="acts">
        {#if progress.running + progress.queued + progress.offline}<button class="mini" onclick={() => act("pause")}>Pause all</button>{/if}
        {#if progress.paused}<button class="mini" onclick={() => act("resume")}>Resume all</button>{/if}
        {#if progress.failed}<button class="mini" onclick={() => act("retry")}>Retry failed</button>{/if}
        <button class="mini ghost" onclick={hide} title="Hide this run here; its downloads carry on">Hide</button>
        </span>
      </div>
      <div class="bar">
        <div class="donepart" style:width={`${progress.expected ? Math.min(1, progress.written / progress.expected) * 100 : 0}%`}></div>
      </div>
      <div class="dim small">
        {size(progress.written)} written of about {size(progress.expected)}{progress.unknown ? `, plus ${progress.unknown} not known yet` : ""}
      </div>
      {#each progress.failures.slice(0, 3) as f (f.id)}
        <p class="failure"><span class="what">{f.title}</span> <span class="dim" title={f.error}>{f.error.split("\n")[0]}</span></p>
      {/each}
      {#if progress.failures.length > 3}<p class="dim small">and {progress.failures.length - 3} more in the library's Downloads</p>{/if}
    </section>
  {/if}
</main>

<style>
  main { padding: 12px 14px; display: flex; flex-direction: column; gap: 10px; height: 100vh; }
  header { display: flex; align-items: baseline; gap: 12px; flex-wrap: wrap; }
  h1 { margin: 0; font-size: 15px; font-weight: 400; letter-spacing: 2px; text-transform: uppercase; color: var(--accent); }
  .head { font-size: 11px; letter-spacing: 1px; text-transform: uppercase; color: color-mix(in srgb, var(--text) 60%, transparent); }
  .warn { color: var(--warn); }
  .dim { color: color-mix(in srgb, var(--text) 50%, transparent); }
  .small { font-size: 10px; }
  textarea {
    font: inherit; font-size: 12px; color: var(--text); background: var(--surface); resize: vertical;
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent); padding: 8px 10px; min-height: 84px; height: 110px;
  }
  textarea:focus { outline: none; border-color: var(--accent); }
  .empty { display: flex; align-items: center; gap: 18px; padding: 8px 4px; }
  .empty .big { font-size: 15px; margin-bottom: 6px; }
  .rows { list-style: none; margin: 0; padding: 0; overflow-y: auto; flex: 1 1 auto; min-height: 60px;
          border: 1px solid color-mix(in srgb, var(--accent) 15%, transparent); }
  .row { display: flex; align-items: center; gap: 8px; padding: 3px 8px; font-size: 12px; min-width: 0; }
  .row.off .what { color: color-mix(in srgb, var(--text) 45%, transparent); }
  .row.entry { padding-left: 42px; }
  .row input { margin: 0; accent-color: var(--accent); }
  .num { font-size: 10px; color: color-mix(in srgb, var(--text) 40%, transparent); flex: 0 0 auto; }
  .what { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .note { font-size: 10px; text-transform: uppercase; letter-spacing: 0.5px; flex: 0 0 auto; color: color-mix(in srgb, var(--accent) 80%, transparent); }
  .note.warn { color: var(--warn); }
  .note.dim { color: color-mix(in srgb, var(--text) 50%, transparent); }
  .nobox { width: 13px; flex: 0 0 auto; }
  .size { font-size: 10px; flex: 0 0 auto; }
  .fold { width: 22px; padding: 0; }
  .mini { padding: 1px 6px; font-size: 10px; border-color: color-mix(in srgb, var(--accent) 30%, transparent); }
  .mini.ghost { border-color: transparent; color: color-mix(in srgb, var(--text) 55%, transparent); }
  .drive { display: flex; align-items: center; gap: 10px; }
  .bar { position: relative; flex: 1 1 auto; height: 8px; display: flex;
         background: color-mix(in srgb, var(--accent) 10%, transparent); border: 1px solid color-mix(in srgb, var(--accent) 25%, transparent); }
  .usedpart { background: color-mix(in srgb, var(--text) 30%, transparent); }
  .adds { background: var(--accent); }
  .adds.warn { background: var(--warn); }
  .donepart { background: var(--accent); }
  .free { font-size: 11px; letter-spacing: 0.5px; flex: 0 0 auto; }
  .say { margin: 0; font-size: 12px; }
  .radar { display: flex; align-items: center; gap: 8px; font-size: 11px; margin: 0; }
  .radar select { font: inherit; font-size: 11px; color: var(--text); background: var(--surface); border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent); }
  .press { display: flex; gap: 10px; }
  .go { flex: 1 1 auto; display: flex; flex-direction: column; align-items: center; gap: 2px; padding: 12px;
        background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .go .big { font-size: 18px; }
  .kind { display: flex; }
  .kind button { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 2px; min-width: 116px;
                 border-color: color-mix(in srgb, var(--accent) 30%, transparent); color: color-mix(in srgb, var(--text) 60%, transparent); }
  .kind button.on { border-color: var(--accent); color: var(--accent); background: color-mix(in srgb, var(--accent) 12%, transparent); }
  .said { margin: 0; font-size: 12px; color: var(--accent); }
  .run { display: flex; flex-direction: column; gap: 6px; padding-top: 8px;
         border-top: 1px solid color-mix(in srgb, var(--accent) 20%, transparent); }
  .runhead { display: flex; align-items: baseline; gap: 8px; font-size: 12px; }
  .runhead .tally { flex: 1 1 auto; min-width: 0; }
  .runhead strong { white-space: nowrap; }
  .runhead .acts { flex: 0 0 auto; display: flex; gap: 6px; }
  .failure { margin: 0; font-size: 11px; display: flex; gap: 8px; min-width: 0; }
  .failure .dim { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>

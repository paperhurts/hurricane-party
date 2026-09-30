<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { ask, open as openDialog } from "@tauri-apps/plugin-dialog";
  import {
    applyTheme,
    colorsFor,
    isWearable,
    themeLabel,
    themeVisualizer,
    WEARABLE,
    type Wearable,
  } from "./lib/theme";
  import { checkSheetBounds, parseSkin, type Token } from "./lib/skin";
  import { guideSheet, paintableSheet, templateManifest, templateParts, templateReadme } from "./lib/template";
  import {
    blankCompanionSheet,
    companionGuide,
    companionTemplateManifest,
    companionTemplateReadme,
  } from "./lib/companiontemplate";
  import { blindTogglesIn, eyewallFile, measureSheets, placePicture, readyPicture, sheetSizes, skinNotes } from "./lib/skins";
  import { sayBlind, wszManifest } from "./lib/wsz";
  import type { RadarSite, RadarStatus } from "./lib/radar";
  import {
    backdropPng,
    madeManifest,
    nameFrom,
    pictureFor,
    pixelsOf,
    PLACES,
    type PicturePlace,
  } from "./lib/madeskin";
  import { paletteFromPixels } from "./lib/palette";

  // The settings window (#233, D173): the set-once things that had crowded
  // the library's header, in four groups. Each one saves as it changes, as it
  // did in the header; there is no Apply. What each says back goes on this
  // window's notice line, where the person who changed it is looking.

  const target = { target: { kind: "WebviewWindow" as const, label: "settings" } };

  // What came out of a skin's zip (#107): where it went, and the two text
  // files, which are colours rather than art and so travel as strings.
  type Unpacked = {
    id: string;
    name: string;
    dir: string;
    files: string[];
    pledit: string | null;
    viscolor: string | null;
    /** A painted or zipped `hp-skin/1` skin's own manifest (#146). */
    manifest: string | null;
  };

  let notice = $state<string | null>(null);
  let error = $state<string | null>(null);

  // ---- Look ------------------------------------------------------------------

  let theme = $state<Wearable>("eyewall");
  let calm = $state(false);
  let glow = $state(true);
  // The skin the classic windows wear, and the ones there are to pick (#107).
  // Eyewall ships and is always first (D90); the rest were imported here.
  let skin = $state("eyewall");
  let skins = $state<string[]>(["eyewall"]);
  // A skin just made or imported, worn so it can be seen, and not yet kept
  // (#146): Keep leaves it, Discard throws it away and puts back the one that
  // was on. Clearing the notice keeps it.
  let pendingKeep = $state<{ id: string; name: string; was: string } | null>(null);
  // Where the worn made skin's picture sits, or null when there is no choice
  // to offer: not a made skin, or a picture that looks the same anywhere.
  let picturePlace = $state<PicturePlace | null>(null);
  let making = $state(false);
  let painting = $state(false);

  /** Wear a theme (#147): every window hears it. A theme that ships with a
   * skin puts it on, and leaving it takes it off (D132), so the picker
   * follows what Rust says is worn. */
  async function setTheme(name: string) {
    if (!isWearable(name)) return;
    theme = name;
    applyTheme(name);
    const worn = await invoke<string>("set_theme", { name });
    if (worn !== skin) {
      skin = worn;
      picturePlace = (await readyPicture(worn)).at;
    }
  }

  async function setCalm(on: boolean) {
    calm = on;
    await invoke("set_calm", { on });
  }

  async function setGlow(on: boolean) {
    glow = on;
    await invoke("set_glow", { on });
  }

  // Whether the line showing is the skin's, so a skin with nothing to say
  // clears the last skin's line without taking an offer with it.
  let skinSaid = false;
  async function setSkin(id: string, name = id) {
    skin = id;
    // Before the windows are told, so none of them reads a sheet that is
    // still being given its room (D127).
    picturePlace = (await readyPicture(id)).at;
    // Purricane's skin brings Purricane's theme (D132).
    const worn = await invoke<string>("set_skin", { id });
    if (isWearable(worn) && worn !== theme) {
      theme = worn;
      applyTheme(worn);
    }
    // What this app could not use of it, every time it is worn (D110).
    const notes = await skinNotes(id);
    if (notes.length) {
      notice = `${name} is on. ${notes.join(" ")}`;
      skinSaid = true;
    } else if (skinSaid) {
      notice = null;
      skinSaid = false;
    }
  }

  /** Move the picture behind the three windows (D127). A manifest change, so
   * it is instant and can be changed back. */
  async function movePicture(at: PicturePlace) {
    const id = skin;
    try {
      await placePicture(id, at);
      picturePlace = at;
      await invoke("set_skin", { id });
    } catch (e) {
      notice = `Couldn't move the picture: ${e instanceof Error ? e.message : String(e)}`;
    }
  }

  /**
   * Make a skin from a picture (#131, D122): pick any image, and get a skin in
   * its colours with the picture behind the chrome. Nothing is drawn — the
   * sheets are Eyewall's — so what could fail is only reading the picture or
   * writing the folder, and a skin that fails is thrown away, as an import is.
   */
  async function makeSkin() {
    if (making) return;
    const picked = await openDialog({
      multiple: false,
      title: "Make a skin from a picture",
      filters: [{ name: "Picture", extensions: ["png", "jpg", "jpeg", "gif", "webp"] }],
    });
    if (typeof picked !== "string") return;
    const name = nameFrom(picked);
    making = true;
    notice = `Making ${name} from that picture…`;
    let id: string | null = null;
    try {
      const bytes = await invoke<ArrayBuffer>("read_picture", { path: picked });
      const bitmap = await createImageBitmap(new Blob([bytes]));
      const { palette, viscolor } = paletteFromPixels(pixelsOf(bitmap));
      const [one, two] = await Promise.all([backdropPng(bitmap, 1), backdropPng(bitmap, 2)]);
      const picture = pictureFor(bitmap.width, bitmap.height);
      bitmap.close();
      id = (await invoke<{ id: string; dir: string }>("make_skin", { name })).id;
      await invoke("write_skin_picture", one, { headers: { "x-hp-skin": id, "x-hp-scale": "1" } });
      await invoke("write_skin_picture", two, { headers: { "x-hp-skin": id, "x-hp-scale": "2" } });
      const manifest = madeManifest({ name, palette, viscolor, picture });
      // The same validator every skin goes through, before anything is worn.
      parseSkin(manifest);
      await invoke("write_skin_manifest", { id, json: JSON.stringify(manifest, null, 1) });
      skins = await invoke<string[]>("list_skins");
      const was = skin;
      await setSkin(id, name);
      notice = `${name} is on: Eyewall's chrome in that picture's colours, with the picture behind it.`;
      pendingKeep = { id, name, was };
    } catch (e) {
      if (id) await invoke("discard_skin", { id }).catch(() => {});
      notice = `Couldn't make a skin from that picture: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      making = false;
    }
  }

  /**
   * Import a skin (#107, D91): a click and the OS dialog, never a watched
   * folder. Rust unpacks the zip, this window maps it into `hp-skin/1` and
   * validates it, and a skin that will not load is thrown away with the
   * reason in front of the person who chose it — never half-loaded
   * (skin-manifest.md).
   */
  async function importSkin() {
    const picked = await openDialog({
      multiple: false,
      title: "Import a skin: a .wsz, a zip, or a painted skin's manifest.json",
      filters: [{ name: "Skin", extensions: ["wsz", "zip", "json"] }],
    });
    if (typeof picked !== "string") return;
    notice = null;
    let unpacked: Unpacked | null = null;
    const was = skin;
    try {
      unpacked = await invoke<Unpacked>("import_skin", { path: picked });
      if (unpacked.manifest !== null) {
        // A painted skin, or a skin folder someone zipped (#146): its manifest
        // is its own. The validator, then its rectangles against its own art.
        const { skin: parsed } = parseSkin(JSON.parse(unpacked.manifest));
        const files = new Set(Object.values(parsed.sheets).flatMap((per) => Object.values(per)));
        const problems = checkSheetBounds(parsed, await sheetSizes(unpacked.dir, [...files]));
        if (problems.length) {
          const more = problems.length > 2 ? ` (and ${problems.length - 2} more)` : "";
          throw new Error(`${problems.slice(0, 2).join("; ")}${more}`);
        }
        skins = await invoke<string[]>("list_skins");
        await setSkin(unpacked.id, unpacked.name);
        notice ??= `${unpacked.name} is on.`;
        pendingKeep = { id: unpacked.id, name: unpacked.name, was };
        return;
      }
      // Look at the sheets before mapping them (D106): a classic skin often
      // stops a file short, and a manifest must not claim art that is not
      // there.
      const sizes = await measureSheets(unpacked.dir, unpacked.files);
      // Shuffle and repeat drawn the same on and off (#132): said the once,
      // here, since nothing about the skin changes and the tooltip tells.
      const blind = await blindTogglesIn(unpacked.dir, unpacked.files);
      const built = wszManifest({
        files: unpacked.files,
        name: unpacked.name,
        pledit: unpacked.pledit ?? undefined,
        viscolor: unpacked.viscolor ?? undefined,
        sizes,
      });
      // The same validator the shipped skin goes through.
      parseSkin(built.manifest);
      await invoke("write_skin_manifest", { id: unpacked.id, json: JSON.stringify(built.manifest, null, 1) });
      skins = await invoke<string[]>("list_skins");
      // The notice is `setSkin`'s: what it says here is what it will say
      // every later time this skin is picked, rather than a better line a
      // person sees once and never again.
      await setSkin(unpacked.id, unpacked.name);
      notice ??= `${unpacked.name} is on.`;
      if (blind.length) notice = `${notice} ${sayBlind(blind)}`;
      pendingKeep = { id: unpacked.id, name: unpacked.name, was };
    } catch (e) {
      if (unpacked) await invoke("discard_skin", { id: unpacked.id }).catch(() => {});
      notice = `That skin was refused: ${e instanceof Error ? e.message : String(e)}`;
    }
  }

  /** Keep the skin just made or imported: nothing to do but stop asking. */
  function keepSkin() {
    pendingKeep = null;
    notice = null;
  }

  /** Throw away the skin just made or imported and wear the one before it. */
  async function discardSkin() {
    const p = pendingKeep;
    if (!p) return;
    pendingKeep = null;
    await setSkin(p.was);
    await invoke("discard_skin", { id: p.id }).catch(() => {});
    skins = await invoke<string[]>("list_skins");
    notice = `${p.name} is gone; ${p.was} is back on.`;
  }

  /**
   * Paint your own (#146): a folder with Eyewall's chrome in colour to paint
   * over, a guide that names every part, the manifest, and a note on how.
   * Written where the person chooses, in a new folder, never over one.
   */
  async function paintYourOwn() {
    if (painting) return;
    const parent = await openDialog({ directory: true, title: "Where should the skin to paint go?" });
    if (typeof parent !== "string") return;
    painting = true;
    try {
      const palette = colorsFor("eyewall") as Record<Token, string>;
      const manifest = templateManifest(palette);
      // The template is a skin: the same validator before a byte is written.
      parseSkin(manifest);
      const parts = templateParts();
      const sheet = await paintableSheet(eyewallFile("chrome@2x.png"), parts, palette);
      const guide = await guideSheet(sheet.canvas, parts, palette);
      const dir = await invoke<string>("start_template", { parent });
      const text = (s: string) => new TextEncoder().encode(s);
      const files: [string, Uint8Array][] = [
        ["manifest.json", text(JSON.stringify(manifest, null, 1))],
        ["chrome.png", sheet.bytes],
        ["guide.png", guide],
        ["README.txt", text(templateReadme())],
      ];
      for (const [name, bytes] of files) {
        await invoke("write_template_file", bytes, { headers: { "x-hp-name": name } });
      }
      notice = `A skin to paint is in ${dir}. Paint chrome.png (guide.png says what every part is), then Import skin… and pick its manifest.json.`;
    } catch (e) {
      notice = `Couldn't write the skin to paint: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      painting = false;
    }
  }

  // ---- Radar -----------------------------------------------------------------

  // What the Cone loop is centred on (#161, D171): a radar, or a ZIP code.
  // Here whatever the theme, since a prep run fills the loop under any theme
  // (D140) and a ZIP code can be set before Cone is put on.
  let radarSites = $state<RadarSite[]>([]);
  let radar = $state<RadarStatus | null>(null);
  let radarStates = $derived([...new Set(radarSites.map((s) => s.state))].sort());
  // Which control shows. It follows what the loop is centred on, and a flip on
  // its own changes nothing until a radar is picked or a ZIP code entered.
  let centreBy = $state<"radar" | "zip">("radar");

  function heard(s: RadarStatus) {
    radar = s;
    if (s.centre) centreBy = s.centre.zip ? "zip" : "radar";
  }

  // A refusal says why on the notice line until a later pick is accepted.
  let refusal: string | null = null;

  async function centreOn(cmd: "set_radar_site" | "set_radar_zip", args: Record<string, string>, what: string) {
    try {
      heard(await invoke<RadarStatus>(cmd, args));
      if (notice === refusal) notice = null;
    } catch (e) {
      notice = refusal = `That ${what} was refused: ${e instanceof Error ? e.message : String(e)}`;
    }
  }
  const setRadar = (id: string) => centreOn("set_radar_site", { id }, "radar");
  const setZip = (zip: string) => centreOn("set_radar_zip", { zip }, "ZIP code");

  // ---- Companion -------------------------------------------------------------

  // Cap'n Capy's switch (#192, D157): the player starts his own little program.
  let capn = $state(false);
  // Which companion the box starts (#208, D162): one that ships (Cap'n Capy,
  // Wee Man, D164), or one imported, which can be removed again (#223).
  type Pack = { id: string; name: string; shipped: boolean };
  let companions = $state<Pack[]>([{ id: "captain", name: "Cap'n Capy", shipped: true }]);
  let companionPick = $state("captain");
  let picked = $derived(companions.find((c) => c.id === companionPick));

  /** The list and the pick as they are on disk: after a removal, and when the
   * window comes forward, since a folder deleted by hand leaves the list. */
  async function refreshCompanions() {
    companions = await invoke<Pack[]>("list_companions");
    companionPick = await invoke<string>("get_companion_pick");
  }
  let paintingCompanion = $state(false);

  /** Start or stop the companion. The box follows what happened, not what was asked. */
  async function setCapn(on: boolean) {
    capn = on;
    try {
      await invoke("set_companion", { on });
    } catch (e) {
      capn = !on;
      error = String(e);
    }
  }

  /** Pick which companion the box starts; with the box ticked, he is swapped now. */
  async function pickCompanion(id: string) {
    const was = companionPick;
    companionPick = id;
    try {
      await invoke("set_companion_pick", { id });
    } catch (e) {
      companionPick = was;
      error = String(e);
    }
  }

  /**
   * Import companion (#208): a finished companion's companion.json or a zip
   * of one, or frames the app packs (D165): any frame of a folder of
   * <state>-<n>.png, or an Aseprite export's .json. The companion's own
   * loader checks it before it is kept (D162), and it becomes the pick.
   */
  async function importCompanion() {
    const picked = await openDialog({
      multiple: false,
      title: "Import a companion: its companion.json or a zip, one frame of a folder of frames (idle-0.png), or an Aseprite export's .json",
      filters: [{ name: "Companion", extensions: ["json", "zip", "png"] }],
    });
    if (typeof picked !== "string") return;
    notice = null;
    try {
      const made = await invoke<{ id: string; name: string }>("import_companion", { path: picked });
      companions = await invoke<Pack[]>("list_companions");
      await pickCompanion(made.id);
      notice = capn ? `${made.name} is here.` : `${made.name} is in. Tick the box to meet them.`;
    } catch (e) {
      notice = `That companion was refused: ${e instanceof Error ? e.message : String(e)}`;
    }
  }

  /**
   * Remove the picked companion (#223): only one that was imported, and only
   * after asking, since it deletes the app's copy of its files. Removing the
   * pick brings Cap'n Capy back as the pick, on screen if the box is ticked.
   */
  async function removeCompanion() {
    const c = picked;
    if (!c || c.shipped) return;
    const yes = await ask(
      `Remove ${c.name}?\n\nThe app's copy of their files is deleted. Whatever you imported them from stays where it is.`,
      { title: "Remove companion", kind: "warning", okLabel: "Remove", cancelLabel: "Keep" },
    );
    if (!yes) return;
    try {
      await invoke("remove_companion", { id: c.id });
      await refreshCompanions();
      notice = capn ? `${c.name} is gone, and Cap'n Capy is back.` : `${c.name} is gone.`;
    } catch (e) {
      notice = `${c.name} could not be removed: ${e instanceof Error ? e.message : String(e)}`;
      await refreshCompanions().catch(() => {});
    }
  }

  /**
   * Paint a companion (D163): a folder with a blank sheet in the format's
   * layout, a guide the same size that names every row, a manifest that says
   * it is painted, and a note on how. Import companion… turns it into a pack.
   */
  async function paintCompanion() {
    if (paintingCompanion) return;
    const parent = await openDialog({ directory: true, title: "Where should the companion to paint go?" });
    if (typeof parent !== "string") return;
    paintingCompanion = true;
    try {
      const palette = colorsFor("eyewall") as Record<Token, string>;
      const sheet = await blankCompanionSheet();
      const guide = await companionGuide(palette);
      const dir = await invoke<string>("start_companion_template", { parent });
      const text = (s: string) => new TextEncoder().encode(s);
      const files: [string, Uint8Array][] = [
        ["companion.json", text(JSON.stringify(companionTemplateManifest(), null, 1))],
        ["sheet.png", sheet],
        ["guide.png", guide],
        ["README.txt", text(companionTemplateReadme())],
      ];
      for (const [name, bytes] of files) {
        await invoke("write_companion_template_file", bytes, { headers: { "x-hp-name": name } });
      }
      notice = `A companion to paint is in ${dir}. Paint sheet.png, a row for each thing it does (README.txt says which), then Import companion… and pick its companion.json.`;
    } catch (e) {
      notice = `Couldn't write the companion to paint: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      paintingCompanion = false;
    }
  }

  // ---- Downloads -------------------------------------------------------------

  let concurrency = $state(2);

  async function setConc(n: number) {
    concurrency = n;
    await invoke("set_concurrency", { n });
  }

  /**
   * Where downloads go (#154, D136). A folder a person picks takes the
   * downloads queued from then on; the ones already queued, and everything
   * already downloaded, stay where they were. A chosen folder that is missing
   * (a drive unplugged) holds its downloads, and says so.
   */
  let downloadDir = $state<{ path: string; chosen: boolean; present: boolean }>({
    path: "",
    chosen: false,
    present: true,
  });
  function refreshDownloadDir() {
    invoke<{ path: string; chosen: boolean; present: boolean }>("get_download_dir")
      .then((d) => (downloadDir = d))
      .catch(() => {});
  }
  async function pickDownloadDir() {
    const picked = await openDialog({ directory: true, multiple: false, title: "Where should downloads go?" });
    if (typeof picked !== "string") return;
    try {
      downloadDir = await invoke("set_download_dir", { path: picked });
      notice = `New downloads go to ${downloadDir.path}. What is already downloaded stays where it is.`;
    } catch (e) {
      notice = `That folder was refused: ${e instanceof Error ? e.message : String(e)}`;
    }
  }
  async function resetDownloadDir() {
    downloadDir = await invoke("set_download_dir", { path: "" });
    notice = "New downloads go to the app's own library folder again.";
  }

  /**
   * The signed-in session yt-dlp uses for the videos that need one (D112).
   * The app stores the path; the file is the person's own and stays where
   * they put it.
   */
  let cookies = $state("");
  async function pickCookies() {
    const picked = await openDialog({
      multiple: false,
      title: "Pick a cookies.txt",
      filters: [{ name: "Cookies", extensions: ["txt"] }],
    });
    if (typeof picked !== "string") return;
    try {
      cookies = await invoke<string>("set_cookies_file", { path: picked });
      notice = "Cookies set. Age-restricted and members-only videos you can watch will now import.";
    } catch (e) {
      notice = `That cookies file was refused: ${e instanceof Error ? e.message : String(e)}`;
    }
  }
  async function clearCookies() {
    cookies = await invoke<string>("set_cookies_file", { path: "" });
    notice = "Cookies cleared. Downloads are signed out again.";
  }

  /**
   * The same thing without the export dance (D113): yt-dlp reads the browser's
   * own cookie store and writes the jar into this app's folder. The list comes
   * from Rust so the picker cannot offer something the allowlist refuses.
   */
  type CookieSource = { browser: string; profile: string | null; label: string; spec: string };
  let browsers = $state<CookieSource[]>([]);
  let reading = $state(false);
  async function fromBrowser(spec: string) {
    if (!spec || reading) return;
    const browser = browsers.find((b) => b.spec === spec)?.label ?? spec;
    reading = true;
    notice = `Reading cookies from ${browser}…`;
    try {
      const made = await invoke<{
        path: string;
        count: number;
        youtube: boolean;
        elsewhere: string[];
        kept: boolean;
        encrypted: boolean;
      }>("export_cookies_from_browser", { browser: spec });
      const from = await invoke<string>("get_cookies_from").catch(() => "");
      cookies = made.path;
      // What to say when no sign-in came through (D115). Three cases, and
      // only one of them is "you are not signed in": Firefox's store is
      // plain, so an empty jar from it means what it says. A Chromium store
      // encrypts the sign-in where no other program can open it, and when the
      // browser is running its database is locked too, so an empty jar from
      // one is "encrypted" when the database shows the session and "cannot
      // tell" when it cannot be read — never "sign in", which the owner was
      // told three times while signed in everywhere.
      const src = browsers.find((b) => b.spec === spec);
      const chromium = src ? src.browser !== "firefox" : false;
      const reason = made.encrypted
        ? `${browser} is signed in to YouTube, but keeps that sign-in encrypted so only the browser itself can open it.`
        : chromium
          ? `No YouTube sign-in came through from ${browser}. If you are signed in there, it is encrypted so only the browser itself can open it.`
          : `${browser} has no YouTube sign-in.`;
      const advice =
        chromium || made.encrypted
          ? " Firefox is the browser Windows lets another program read: sign in to YouTube there once, then read firefox with From a browser."
          : made.elsewhere.length
            ? ` These do have one: ${made.elsewhere.join(", ")}.`
            : ` Sign in to YouTube in ${browser}, then read it again.`;
      notice = made.youtube
        ? `Read ${made.count} cookies from ${browser}, with a YouTube sign-in among them. Videos that want one will import now; read them again when they stop.`
        : made.kept
          ? // A read with no sign-in never replaces one that has it (D115).
            `${reason} The app kept the cookies it already had${from ? ` from ${from}` : ""}, so age-restricted videos still import.`
          : `${reason}${advice}`;
    } catch (e) {
      notice = e instanceof Error ? e.message : String(e);
    } finally {
      reading = false;
    }
  }

  /**
   * An ffmpeg of the person's own instead of the one that ships (D133): a
   * newer one, or one with more in it. Rust asks it what it is before keeping
   * it, and a copy that has gone since is reported while the bundled one runs.
   */
  let ffmpeg = $state<{ path: string; present: boolean }>({ path: "", present: false });
  async function pickFfmpeg() {
    const picked = await openDialog({
      multiple: false,
      title: "Pick an ffmpeg.exe",
      filters: [{ name: "ffmpeg", extensions: ["exe"] }],
    });
    if (typeof picked !== "string") return;
    try {
      const said = await invoke<string>("set_ffmpeg", { path: picked });
      ffmpeg = await invoke<{ path: string; present: boolean }>("get_ffmpeg");
      notice = `Downloads now use your ffmpeg: ${said}.`;
    } catch (e) {
      notice = `That ffmpeg was refused: ${e instanceof Error ? e.message : String(e)}`;
    }
  }
  async function clearFfmpeg() {
    await invoke<string>("set_ffmpeg", { path: "" });
    ffmpeg = { path: "", present: false };
    notice = "Downloads use the ffmpeg that ships with the app again.";
  }

  // ---- as it is now ----------------------------------------------------------

  onMount(() => {
    applyTheme("eyewall");
    invoke<string>("get_theme").then((t) => {
      theme = isWearable(t) ? t : "eyewall";
      applyTheme(theme);
    });
    invoke<boolean>("get_calm").then((on) => (calm = on));
    invoke<boolean>("get_glow").then((on) => (glow = on));
    invoke<string>("get_skin").then(async (s) => {
      skin = s;
      picturePlace = (await readyPicture(s)).at;
    });
    invoke<string[]>("list_skins").then((s) => (skins = s));
    invoke<RadarSite[]>("radar_sites").then((s) => (radarSites = s)).catch(() => {});
    invoke<RadarStatus>("get_radar").then(heard).catch(() => {});
    invoke<boolean>("get_companion").then((on) => (capn = on));
    refreshCompanions().catch(() => {});
    invoke<number>("get_concurrency").then((n) => (concurrency = n));
    refreshDownloadDir();
    invoke<string>("get_cookies_file").then((c) => (cookies = c));
    invoke<{ path: string; present: boolean }>("get_ffmpeg").then((f) => (ffmpeg = f));
    invoke<CookieSource[]>("cookie_browsers").then((b) => (browsers = b));

    // What changes elsewhere while this is open: Purricane's skin bringing its
    // theme, Main's calm pill (D132), a skin that would not load and was put
    // back to Eyewall (the library says why), a radar picked in prep mode, a
    // download folder's drive coming or going.
    const subs = [
      listen<string>("theme:changed", (e) => {
        if (!isWearable(e.payload)) return;
        theme = e.payload;
        applyTheme(theme);
      }, target),
      listen<string>("skin:changed", async (e) => {
        if (e.payload === skin) return;
        skin = e.payload;
        picturePlace = (await readyPicture(skin)).at;
      }, target),
      listen<boolean>("vis:calm", (e) => (calm = e.payload), target),
      listen<boolean>("chrome:glow", (e) => (glow = e.payload), target),
      listen<RadarStatus>("radar:updated", (e) => heard(e.payload), target),
      listen("jobs-changed", refreshDownloadDir, target),
    ];
    const onFocus = () => refreshCompanions().catch(() => {});
    window.addEventListener("focus", onFocus);
    return () => {
      subs.forEach((s) => s.then((off) => off()));
      window.removeEventListener("focus", onFocus);
    };
  });
</script>

<main>
  <div class="groups">
    <div class="col">
      <section>
        <h2>Look</h2>
        <label class="row" title="The colours and type of the library, and of every skin that wears the theme">
          <span class="k">Theme</span>
          <select value={theme} onchange={(e) => setTheme(e.currentTarget.value)}>
            {#each WEARABLE as t (t)}<option value={t}>{themeLabel(t)}</option>{/each}
          </select>
        </label>
        <label class="row" title="What the three classic windows wear">
          <span class="k">Skin</span>
          <select
            value={skin}
            onchange={(e) => {
              // Picking another skin keeps the one on trial.
              pendingKeep = null;
              setSkin(e.currentTarget.value);
            }}
          >
            {#each skins as s (s)}<option value={s}>{s}</option>{/each}
          </select>
        </label>
        {#if picturePlace}
          <label class="row" title="Which part of the picture shows behind the three windows">
            <span class="k">Picture</span>
            <select value={picturePlace} onchange={(e) => movePicture(e.currentTarget.value as PicturePlace)}>
              {#each PLACES as p (p)}<option value={p}>{p}</option>{/each}
            </select>
          </label>
        {/if}
        <label class="row check" title="The halo on the player's buttons, clock and lit rows">
          <span class="k"></span>
          <input type="checkbox" checked={glow} onchange={(e) => setGlow(e.currentTarget.checked)} />
          Glow on the player
        </label>
        {#if themeVisualizer(theme) === "kaleidoscope"}
          <label class="row check" title="A still kaleidoscope: no turning, no bloom, one colour, and only its size answers the music">
            <span class="k"></span>
            <input type="checkbox" checked={calm} onchange={(e) => setCalm(e.currentTarget.checked)} />
            Calm kaleidoscope
          </label>
        {/if}
        <div class="acts">
          <button class="link" onclick={importSkin} title="A .wsz, a zip of a skin, or a painted skin's manifest.json">Import skin…</button>
          <button class="link" onclick={makeSkin} disabled={making} title="Pick any picture and get a skin in its colours, with the picture behind the windows">
            {making ? "Making…" : "Make a skin from a picture…"}
          </button>
          <button class="link" onclick={paintYourOwn} disabled={painting} title="A folder with the windows' chrome to paint over, and a guide to every part of it">
            {painting ? "Writing…" : "Paint your own…"}
          </button>
        </div>
      </section>

      <section>
        <h2>Radar</h2>
        <div class="row">
          <span class="k">Centre on</span>
          <span class="toggle" role="group" aria-label="Centre the radar loop on">
            {#each [["radar", "Radar"], ["zip", "My ZIP"]] as [k, label] (k)}
              <button class:sel={centreBy === k} aria-pressed={centreBy === k} onclick={() => (centreBy = k as "radar" | "zip")}>{label}</button>
            {/each}
          </span>
        </div>
        <div class="row">
          <span class="k"></span>
          {#if centreBy === "radar"}
            <select
              aria-label="Radar"
              title="The NWS radar nearest you. Only its name is kept, on this PC"
              value={radar?.centre?.site ?? ""}
              onchange={(e) => setRadar(e.currentTarget.value)}
            >
              <option value="">Pick…</option>
              {#each radarStates as st (st)}
                <optgroup label={st}>
                  {#each radarSites.filter((s) => s.state === st) as s (s.id)}
                    <option value={s.id}>{s.id} — {s.name}</option>
                  {/each}
                </optgroup>
              {/each}
            </select>
          {:else}
            <input
              class="zip"
              aria-label="ZIP code"
              placeholder="ZIP code"
              inputmode="numeric"
              maxlength="10"
              title="The loop centres on your ZIP code, the rings are distance from you, and the alerts are the ones for where you are. It is kept only on this PC, and leaves it only as the point the Weather Service is asked for alerts at"
              value={radar?.centre?.zip ?? ""}
              onchange={(e) => setZip(e.currentTarget.value)}
              onkeydown={(e) => {
                if (e.key === "Enter") e.currentTarget.blur();
              }}
            />
          {/if}
        </div>
        <p class="note">The Cone theme draws it behind the player, and Hurricane Party Planning saves it for the outage.</p>
      </section>
    </div>

    <div class="col">
      <section>
        <h2>Companion</h2>
        <label class="row check" title="A companion who stands on the player's windows, dances to the music and naps when it stops. Click them, or pick them up">
          <span class="k"></span>
          <input type="checkbox" checked={capn} onchange={(e) => setCapn(e.currentTarget.checked)} />
          On the desktop
        </label>
        <div class="row">
          <span class="k">Who</span>
          <select aria-label="Which companion" value={companionPick} onchange={(e) => pickCompanion(e.currentTarget.value)}>
            {#each companions as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
          </select>
          <button
            class="quiet"
            onclick={removeCompanion}
            disabled={!picked || picked.shipped}
            title={!picked || picked.shipped
              ? "Cap'n Capy and Wee Man ship with the app and stay; one you imported can be removed"
              : `Delete ${picked.name} from the app. Asks first`}
          >Remove…</button>
        </div>
        <div class="acts">
          <button class="link" onclick={paintCompanion} disabled={paintingCompanion} title="A folder with a blank companion sheet to paint, 64 px a frame, and a guide to every row">
            {paintingCompanion ? "Writing…" : "Paint a companion…"}
          </button>
          <button class="link" onclick={importCompanion} title="A companion of your own: its companion.json or a zip, a folder of frames named idle-0.png, walk-0.png…, or an Aseprite sprite-sheet export with a tag per state">Import companion…</button>
        </div>
      </section>

      <section>
        <h2>Downloads</h2>
        <div class="row">
          <span class="k">Folder</span>
          <span class="val" class:warn={!downloadDir.present} title={downloadDir.path}>
            {downloadDir.chosen ? downloadDir.path : "The app's library folder"}
          </span>
          <button class="quiet" onclick={pickDownloadDir} title="Send downloads you queue from now on to another folder">Change…</button>
          {#if downloadDir.chosen}
            <button class="quiet" onclick={resetDownloadDir} title="Send new downloads to the app's own library folder again">&times;</button>
          {/if}
        </div>
        {#if !downloadDir.present}
          <p class="note warn">It is not there: downloads for it wait until it is back.</p>
        {/if}
        <label class="row">
          <span class="k">At once</span>
          <select class="narrow" value={concurrency} onchange={(e) => setConc(+e.currentTarget.value)}>
            {#each [1, 2, 3, 4] as n}<option value={n}>{n}</option>{/each}
          </select>
        </label>
        <div class="row">
          <span class="k">Sign-in</span>
          <button
            class="quiet"
            onclick={pickCookies}
            title={cookies
              ? `yt-dlp signs in with ${cookies}. Click to pick another.`
              : "For age-restricted and members-only videos: a cookies.txt exported from a browser you are signed in with"}
          >
            {cookies ? "Cookies ✓" : "Cookies file…"}
          </button>
          <select
            disabled={reading}
            value=""
            onchange={(e) => {
              fromBrowser(e.currentTarget.value);
              e.currentTarget.value = "";
            }}
            title="Read cookies straight out of a browser you are signed in with"
          >
            <option value="" disabled selected>{reading ? "Reading…" : "From a browser…"}</option>
            <!-- Windows lets another program read Firefox's cookie store and
               not Chromium's, so the list says which is which (D113). -->
            {#each browsers as b (b.spec)}<option value={b.spec}>{b.label}</option>{/each}
          </select>
          {#if cookies}
            <button class="quiet" onclick={clearCookies} title="Stop using that file">&times;</button>
          {/if}
        </div>
        <div class="row">
          <span class="k">ffmpeg</span>
          <span class="val" class:warn={!!ffmpeg.path && !ffmpeg.present} title={ffmpeg.path || "The ffmpeg that ships with the app"}>
            {!ffmpeg.path ? "Built in" : ffmpeg.present ? ffmpeg.path : "Yours is gone: built in"}
          </span>
          <button class="quiet" onclick={pickFfmpeg} title="Use your own copy of ffmpeg instead">Change…</button>
          {#if ffmpeg.path}
            <button class="quiet" onclick={clearFfmpeg} title="Use the ffmpeg that ships with the app">&times;</button>
          {/if}
        </div>
      </section>
    </div>
  </div>

  {#if notice || error}
    <!-- What the last change said, with the offer riding on it: a skin just
         made or imported is on trial until it is kept (#146). -->
    <footer class:err={!!error}>
      <span>{error ?? notice}</span>
      {#if pendingKeep && !error}
        <button onclick={keepSkin}>Keep it</button>
        <button class="danger" onclick={discardSkin} title="Delete it and put {pendingKeep.was} back on">Discard</button>
      {/if}
      <button class="quiet" onclick={() => ((notice = null), (error = null), (pendingKeep = null))} title="Dismiss">&times;</button>
    </footer>
  {/if}
</main>

<style>
  /* Iosevka Aile, the modern windows' register (theme.md): proportional, of
     the same family as the player's Iosevka. Named outright, as the library
     names its face (D130), so a theme's own face stays the player's. */
  main {
    font-family: "Iosevka Aile", "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
    font-size: 13px;
    height: 100vh;
    display: flex;
    flex-direction: column;
  }
  .groups {
    flex: 1;
    overflow-y: auto;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    align-content: start;
  }
  .col + .col { border-left: 1px solid color-mix(in srgb, var(--accent) 14%, transparent); }
  section { padding: 14px 18px; border-bottom: 1px solid color-mix(in srgb, var(--accent) 14%, transparent); }
  h2 { margin: 0 0 8px; font-size: 13px; font-weight: 600; color: var(--text); }
  .row { display: flex; align-items: center; gap: 8px; margin: 6px 0; min-width: 0; }
  .row > .k { flex: 0 0 76px; color: color-mix(in srgb, var(--text) 58%, transparent); }
  .row select { flex: 1 1 auto; min-width: 0; }
  .row select.narrow { flex: 0 0 64px; }
  .row.check { cursor: pointer; }
  .row input[type="checkbox"] { margin: 0; }
  .val { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
         color: color-mix(in srgb, var(--text) 58%, transparent); }
  .val.warn, .note.warn { color: var(--warn); }
  .note { margin: 6px 0 0; font-size: 12px; color: color-mix(in srgb, var(--text) 40%, transparent); }
  select, input:not([type="checkbox"]) { font: inherit; font-size: 13px; padding: 4px 8px; border-radius: 3px; background: var(--surface);
                  color: var(--text); border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent); }
  .zip { width: 12ch; }
  button { font: inherit; font-size: 12px; padding: 3px 9px; border-radius: 3px; white-space: nowrap; }
  button.quiet { border-color: transparent; color: color-mix(in srgb, var(--text) 58%, transparent); }
  button.quiet:hover:not(:disabled) { color: var(--text); }
  .acts { display: flex; flex-direction: column; align-items: flex-start; gap: 2px; margin: 8px 0 0 84px; }
  button.link { border: 0; padding: 2px 0; color: var(--accent); }
  button.link:hover:not(:disabled) { background: none; text-decoration: underline; }
  .toggle { display: inline-flex; border: 1px solid color-mix(in srgb, var(--accent) 32%, transparent); border-radius: 3px; overflow: hidden; }
  .toggle button { border: 0; border-radius: 0; color: color-mix(in srgb, var(--text) 58%, transparent); }
  .toggle button.sel { color: var(--accent); background: color-mix(in srgb, var(--accent) 12%, transparent); }
  footer { display: flex; align-items: center; gap: 8px; padding: 9px 18px; border-top: 1px solid color-mix(in srgb, var(--accent) 22%, transparent);
           background: var(--surface); font-size: 12px; }
  footer > span { flex: 1; min-width: 0; }
  footer.err > span { color: var(--warn); }
  button.danger { color: var(--warn); border-color: color-mix(in srgb, var(--warn) 50%, transparent); }
</style>

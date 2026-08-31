# hurricane-party — Control API

**Status:** back in scope, and for a much better reason than the one I deferred it for.

Transport control from doc-md was a convenience the windowshade mini-bar already solved. **A visualization data stream so someone can drive an LED wall off the spectrum analyzer is a different thing entirely** — nothing else in the app provides it, and there's no workaround.

This changes the API's nature. It's no longer a private convenience between two of your own apps. It's a **public contract that strangers will write against**, which means versioning, documentation, and the discipline of not breaking it. That's a real ongoing cost and you should sign up for it deliberately.

---

## Two channels, one entry point

| Channel | Transport | Shape | Rate |
|---|---|---|---|
| **Control** | named pipe, NDJSON | request/response + unsolicited events | on demand |
| **Viz** | separate named pipe, **binary frames** | push only | 15–60 Hz |

Why binary for viz: 32 spectrum bands as JSON floats at 60 Hz is ~240 KB/s of number formatting and parsing, for data that is natively 32 bytes. Binary frames are ~50 bytes. When someone's driving LEDs, latency and jitter are the product.

Why a separate pipe rather than interleaving on one: mixing framed binary with newline-delimited JSON on a single stream is a parsing hazard for every client that will ever be written. Keep them apart.

---

## Control channel

**Path:** `\\.\pipe\hurricane-party` (Windows) · `$XDG_RUNTIME_DIR/hurricane-party.sock` · `~/Library/Caches/hurricane-party.sock`

### Handshake — required first message

```jsonc
→ {"id":0, "cmd":"hello", "client":"led-bridge", "protocol_version":1}
← {"id":0, "ok":true, "result":{
     "protocol_version": 1,
     "app_version": "0.6.0",
     "capabilities": ["transport","library","viz","palette"]
   }}
```

Clients check `protocol_version` and refuse to proceed on mismatch. Server rejects unknown major versions rather than guessing.

### Transport

```jsonc
{"id":1, "cmd":"toggle"}
{"id":2, "cmd":"seek", "pos_s":42.5}
{"id":3, "cmd":"volume", "level":0.7}
{"id":4, "cmd":"queue_playlist", "playlist_id":12}
{"id":5, "cmd":"search", "q":"cure"}
{"id":6, "cmd":"status"}
```

Full set: `play` `pause` `toggle` `next` `prev` `stop` `seek` `volume` `status` `queue_playlist` `search`

### Events (unsolicited, no `id`)

```jsonc
{"event":"now_playing_changed", "media_id":89, "title":"…", "uploader":"…", "duration_s":240}
{"event":"state_changed", "state":"paused"}
{"event":"palette_changed", "viscolor":["#000000","#0f0f0f", "…24 entries…"]}
```

`palette_changed` is the one worth calling out. When you switch skins, the LED wall changes color scheme to match. That's a genuinely nice thing that costs almost nothing to ship, and it's the kind of detail that makes people want to build against your API. The payload is the same 24-entry `viscolor` array the skin manifest defines (`skin-manifest.md`) — one definition, three consumers: analyser, Cone backdrop, and this event.

### `layout_changed` — required by the kittens, useful to everyone

```jsonc
{"event":"layout_changed", "windows":[
  {"id":"main",     "x":420, "y":300, "w":550, "h":232},
  {"id":"playlist", "x":420, "y":532, "w":550, "h":232}
], "bonds":[
  {"a":"main", "b":"playlist", "edge":"bottom", "span":[420, 970]}
]}
```

The viz stream carries no geometry, but the kittens treat the bonded window group as **terrain** — they perch on top edges, walk the glowing seam, and have to jump off when a bond breaks under them (`purricane.md`). That needs window rectangles and the bond graph, which is this event.

It generalizes past cats: any external overlay, LED positioning rig, or second-screen tool wants to know where the windows are. Coordinates are **physical pixels**, matching the internal convention (`CLAUDE.md`) — a client compositing against these must not have to guess a scale factor.

Lands at v1.0 with the protocol freeze, since it's part of the public commitment rather than an early convenience.

---

## Viz channel

### Subscribing

```jsonc
→ {"id":7, "cmd":"subscribe_viz",
   "bands": 32,          // 8–128
   "rate_hz": 30,        // 15 | 30 | 60
   "depth": "u8",        // "u8" (LED-friendly) | "f32" (precise)
   "include": ["spectrum","level","beat"]}

← {"id":7, "ok":true, "result":{
     "stream": "\\\\.\\pipe\\hurricane-party-viz-7f3a"
   }}
```

Server allocates a per-subscriber pipe and starts pushing. Multiple subscribers at different band counts and rates is explicitly supported — the LED wall wants 32 bands at 30 Hz, someone's desktop toy wants 128 at 60.

### Frame format (little-endian)

```
offset  size  field
0       4     magic          0x48505631  ("HPV1")
4       8     timestamp_us   monotonic, source clock
12      1     n_bands
13      1     depth          0 = u8, 1 = f32
14      1     flags          bit0 = beat detected
15      1     reserved
16      1     level_peak     0–255
17      1     level_rms      0–255
18      n     spectrum       n_bands × (1 or 4 bytes)
```

Fixed header, self-describing length. A client can resync on the magic after a dropped connection.

### Backpressure

**If a subscriber can't keep up, drop frames — never buffer.** Stale visualization data is worse than missing data; a laggy LED wall reads as broken. Non-blocking writes, drop on `WouldBlock`, done.

---

## Architectural consequence worth flagging

Your audio lives in the webview (Web Audio `AnalyserNode`), not in Rust. So the spectrum path is:

```
<audio> → AnalyserNode → JS getByteFrequencyData()
        → Tauri IPC → Rust → downsample to N bands → socket
```

That's an extra hop compared to tapping audio natively in Rust. At 60 Hz with ~40-byte payloads it's fine — Tauri IPC handles that comfortably — but **measure the end-to-end latency before you publish the API**, because once someone's LED rig is calibrated against it you can't quietly change the timing.

Target: under 20 ms from speaker to socket. If it comes in worse, the fix is moving the analysis into Rust with `symphonia` decoding in parallel, which is a bigger change and better to know about early.

This is the one place my earlier "use HTML5 audio" recommendation costs you something. I still think it's right — the in-app EQ and analyser are worth far more than the hop — but I don't want to pretend the tradeoff isn't there.

---

## Security posture

Any local process can connect to the named pipe. For a personal offline media player that's the correct tradeoff — the alternative is a token dance that makes third-party integration annoying, to protect against an attacker who already has code execution on your machine.

But be aware the surface includes `queue_playlist` and `search`, not just viz. If that ever bothers you, the fix is capability scoping at handshake (`{"cmd":"hello", "want":["viz"]}`) rather than authentication. Don't build it now.

---

## Versioning discipline

Once this is public:

- **Freeze the frame format at 1.0.** Additive changes go in new optional fields on the control channel, not by reshaping frames.
- **Bump `protocol_version` only for breaking changes**, and support the previous major for one release cycle.
- **Write the doc before the second client exists.** A README section with the frame layout and a 40-line Python example client is enough, and it's the difference between people building things and people giving up.

Ship the example client. Someone with an LED strip and a Raspberry Pi should be able to get bars moving in ten minutes.

---

## Milestone placement

| Milestone | Scope |
|---|---|
| v0.3 | Control channel only: handshake, transport, events. Proves the pipe, no public commitment yet |
| v0.4 | Viz channel — lands with the analyser, since it's the same data |
| v0.5 | `palette_changed` — lands with the skin loader, since that's when palettes become dynamic |
| v1.0 | `layout_changed`. Freeze protocol v1. Publish docs + example client |

This table is the control API's internal phasing and is consistent with the canonical milestone table in `decisions.md` (D27). If they ever disagree, `decisions.md` wins.

Keep it undocumented and explicitly unstable until v1.0. That's your window to change your mind.

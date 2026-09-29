# hurricane-party — Control API, protocol 1

**Stable.** Protocol 1 was frozen with v1.0 (#184, D151). From here it changes only by adding: new commands, new optional fields, new events, new capabilities. Anything that would break a client written against this page is protocol 2 (see [Versioning](#versioning)).

A program on the same machine as the player can control it, ask what is in the library, know where its windows are, and read its analyser as a stream of frames. That last one is enough to drive an LED wall from the bass. The player never reaches out: everything here is a program the person runs, connecting in.

| Channel | Where | Shape | Rate |
|---|---|---|---|
| **Control** | a named pipe | one JSON object per line, requests and replies, plus events | on demand |
| **Viz** | a second pipe per subscriber | binary frames, push only | 15, 30 or 60 Hz |

The fastest way in is the example: [`examples/viz_bars.py`](../examples/viz_bars.py), Python's standard library only, bars in a terminal.

---

## Connecting

| Platform | Control channel |
|---|---|
| Windows | `\\.\pipe\hurricane-party` |
| macOS | `~/Library/Caches/hurricane-party.sock` (a Unix domain socket, when the player runs there, #187) |
| Linux | `$XDG_RUNTIME_DIR/hurricane-party.sock` (likewise) |

**Local only, by design** (D9, D11, D29). The pipe is not a network port, and the player opens no connection for any of this. A rig on another machine (a Raspberry Pi behind an LED strip, say) is fed by a program the person runs on the player's machine, which reads the viz stream and forwards it however it likes. That relay is theirs; the player stays offline.

Any local process may connect. There is no authentication: an attacker who can open the pipe already runs code as the person. A client can limit *itself* to part of the protocol at the handshake (`want`, below), which is what a bars-only client should do.

**Messages** are UTF-8 JSON, one object per line. A request has a `cmd` and an `id` of the client's choosing; the reply echoes the `id`:

```jsonc
→ {"id":6, "cmd":"status"}
← {"id":6, "ok":true, "result":{…}}
← {"id":7, "ok":false, "error":"\"search\" needs q"}
```

An **event** is a line with an `event` field and no `id`, sent when something changes. A client reading replies and events off the same stream tells them apart by `id`.

---

## Handshake

`hello` comes first. Anything else before it is refused, and no event is sent until it is answered.

```jsonc
→ {"id":0, "cmd":"hello", "client":"led-bridge", "protocol_version":1, "want":["viz","palette"]}
← {"id":0, "ok":true, "result":{
     "protocol_version": 1,
     "app_version": "1.0.0",
     "capabilities": ["transport","viz","layout","library","palette"],
     "granted": ["viz","palette"],
     "stable": true
   }}
```

- `protocol_version` must be `1`. Another number is refused rather than guessed at.
- `client` is a name for the player's log. Optional.
- `want` lists the capabilities this connection will use. **Optional; absent means all of them.** A word that is not a capability, or an empty list, is refused with the list of real ones.
- `capabilities` is everything this build does. Look for the word you need, not for a version number.
- `granted` is what this connection may use. A command outside it is refused with the capability it needs; an event outside it is never sent to it.
- `hello` may be sent again to change `want`.

---

## Capabilities

| Capability | Commands | Events |
|---|---|---|
| `transport` | `status` `play` `pause` `toggle` `stop` `next` `prev` `seek` `volume` | `now_playing_changed` `state_changed` |
| `viz` | `subscribe_viz` | *(the viz channel)* |
| `layout` | `layout` | `layout_changed` |
| `library` | `playlists` `search` `queue_playlist` `play` with a `media_id` | |
| `palette` | `palette` | `palette_changed` |

---

## Commands

### Transport

```jsonc
{"id":1, "cmd":"toggle"}
{"id":2, "cmd":"seek", "pos_s":42.5}
{"id":3, "cmd":"volume", "level":0.7}
{"id":4, "cmd":"status"}
```

`play` `pause` `toggle` `stop` `next` `prev` take nothing. `seek` takes `pos_s`, seconds. `volume` takes `level`, 0 to 1; a value outside is clamped. A transport command's reply says it was accepted (`{"accepted":"toggle"}`), not that it has happened: the change arrives as `state_changed`.

`status` answers with where the transport stands:

```jsonc
{"id":4, "ok":true, "result":{"state":"playing", "kind":"audio", "media_id":89, "title":"…", "duration_s":240, "pos_s":61.5, "volume":0.8, "shuffle":false, "repeat":"all"}}
```

- `state` is `playing`, `paused` or `stopped`.
- **One thing plays at a time** (D69, D70). Starting a video pauses the track, starting a track pauses the video, and nothing resumes by itself. `kind` says which, `audio` or `video`, and describes whichever last started. `play` `pause` `toggle` `stop` `seek` `volume` act on it; `stop` on a video is pause-and-rewind. Closing the video window hands the transport back to the track.
- `next` and `prev` step through the list that is playing, videos and tracks alike. `play` with nothing loaded starts that list, at the playlist window's selected row or the top. Only a track *ending* honours repeat one: `next` always moves on.
- `shuffle` and `repeat` (`off`, `one`, `all`) are the library's play order (D97).
- The Main window's own buttons go through the same router as these commands (D81), so a client and the person see the same thing.

### Viz

```jsonc
→ {"id":5, "cmd":"subscribe_viz", "bands":32, "rate_hz":30, "depth":"u8", "include":["spectrum","level","beat"]}
← {"id":5, "ok":true, "result":{"stream":"\\\\.\\pipe\\hurricane-party-viz-7f3a"}}
```

| Field | Values | Default |
|---|---|---|
| `bands` | 8 to 128 | 32 |
| `rate_hz` | 15, 30 or 60 | 30 |
| `depth` | `"u8"` (LED-friendly) or `"f32"` | `"u8"` |
| `include` | any of `"spectrum"`, `"level"`, `"beat"` | all three |

Every field is optional. A value out of range is refused by name, never clamped: a frame shaped differently from what a rig asked for is worse than an error it can read.

`stream` is a pipe of this subscriber's own, `\\.\pipe\hurricane-party-viz-` and four hex digits. It exists before the reply is sent, and the client has ten seconds to open it. **The subscription belongs to that pipe, not to the control connection:** close the control connection and the frames keep coming; close the viz pipe and the subscription ends. Several subscribers at different sizes and rates are fine. What the frames hold is under [Viz channel](#viz-channel).

### Layout

Where the windows are, for anything that wants to put something on them.

```jsonc
→ {"id":7, "cmd":"layout"}
← {"id":7, "ok":true, "result":{
     "windows":[
       {"id":"main",     "x":420,  "y":300, "w":550, "h":232, "group":true,  "shaded":false, "visible":true},
       {"id":"playlist", "x":420,  "y":532, "w":550, "h":232, "group":true,  "shaded":false, "visible":true},
       {"id":"library",  "x":-1200,"y":80,  "w":916, "h":659, "group":false, "shaded":false, "visible":false}
     ],
     "bonds":[{"a":"main", "b":"playlist", "edge":"bottom", "span":[420, 970]}]
   }}
```

- Coordinates are **physical pixels** on the virtual desktop, the player's own convention, so nothing has to guess a scale factor. They can be negative on a display left of or above the primary.
- `main`, `eq` and `playlist` are the classic windows, `group: true`. `library`, `video`, `prep` and `visuals` (#167) are the decorated windows, listed while they exist, always `group: false`, and never in `bonds`.
- `shaded`: collapsed to the windowshade strip; `h` is the strip's.
- `visible`: shown and not minimised, as the operating system reports it. A window that is not visible keeps its last rectangle, which is where it will come back.
- A bond's `edge` is the side of `a` that `b` sits against, always `right` or `bottom`; `span` is the shared stretch along it.

### Library

```jsonc
→ {"id":8,  "cmd":"playlists"}
← {"id":8,  "ok":true, "result":{"playlists":[{"id":12, "name":"Road Tripping", "count":22, "smart":false}]}}

→ {"id":9,  "cmd":"search", "q":"cure"}
← {"id":9,  "ok":true, "result":{"total":212, "tracks":[{"id":89, "title":"Pictures of You", "uploader":"The Cure", "kind":"audio", "duration_s":288.0}]}}

→ {"id":10, "cmd":"queue_playlist", "playlist_id":12}
← {"id":10, "ok":true, "result":{"playlist_id":12, "name":"Road Tripping", "count":22}}

→ {"id":11, "cmd":"play", "media_id":89}
← {"id":11, "ok":true, "result":{"media_id":89, "title":"Pictures of You"}}
```

- `playlists`: every playlist. `smart` marks one that fills itself from a rule. `count` is what can play now; a track on a drive that is not plugged in is not counted.
- `search`: every word of `q` must appear in the title or the artist, blind to case and accents, as the library's search box matches; since v1.5 a letter like ø, æ or ß also matches what a person types for it, so `eivor` finds "Eivør" (D172). At most 50 tracks, newest first, and `total` for how many matched. An empty or missing `q` is refused.
- `queue_playlist`: that list becomes what plays, and it starts, from the top or on a fresh shuffle.
- `play` with a `media_id` plays that track: in the list that is playing if it is there, otherwise in the whole library. `play` without one is the transport's.
- A playlist that does not exist, one with nothing that can play now, a track that is not in the library, and a track on a drive that is not plugged in are each refused with the reason.

### Palette

```jsonc
→ {"id":12, "cmd":"palette"}
← {"id":12, "ok":true, "result":{"viscolor":["#04160b", "#06280f", "…24 in all"]}}
```

The 24 colours the analyser draws with right now, darkest first, each `#rrggbb` in lower case: the theme's, or a skin's own when it brings one. For Purricane it is the base ramp, not the kaleidoscope's drifting hue. Empty only in the moment before the Main window has opened.

---

## Events

```jsonc
{"event":"now_playing_changed", "kind":"audio", "media_id":89, "title":"…", "uploader":"…", "duration_s":240}
{"event":"state_changed", "state":"paused"}
{"event":"layout_changed", "windows":[…], "bonds":[…]}
{"event":"palette_changed", "viscolor":["#04160b", …]}
```

| Event | Capability | Sent |
|---|---|---|
| `now_playing_changed` | `transport` | when a different track or video starts |
| `state_changed` | `transport` | playing, paused or stopped changed; not on every position tick |
| `layout_changed` | `layout` | a window moved, resized, shaded, shown or hidden, a bond was made or broken, a display came or went. At most every 50 ms during a drag, always once after the last change, never twice the same. Same shape as `layout`'s result |
| `palette_changed` | `palette` | the skin or the theme changed the ramp. Same shape as `palette`'s result |

---

## Viz channel

Each frame is a fixed 18-byte header and the spectrum. Little-endian.

```
offset  size  field
0       4     magic          the ASCII bytes "HPV1" (as a little-endian u32, 0x31565048)
4       8     timestamp_us   microseconds since the UNIX epoch, source clock, when the analyser was read
12      1     n_bands
13      1     depth          0 = u8, 1 = f32
14      1     flags          bit0 = beat detected
15      1     reserved       0
16      1     level_peak     0–255
17      1     level_rms      0–255
18      n     spectrum       n_bands × (1 or 4 bytes), 0..1 full scale
```

The length is in the header, so a client that loses its place resyncs on the magic.

- **`timestamp_us`** is wall-clock, taken the instant the analyser is read, so a client on the same machine subtracts it from its own clock and has the latency.
- **`spectrum`** is the log-spaced 50 Hz to 16 kHz bands the Main window draws, the loudest bin per band, with **no smoothing** (the on-screen bars decay; a rig can add smoothing but could not remove ours). `f32` carries the analyser's 8-bit resolution scaled to 0..1 today; finer resolution would be additive.
- **`level_peak`** and **`level_rms`**: the last FFT window's time-domain peak and RMS, 255 = full scale.
- **`beat`** (flags bit 0): an onset heuristic on 40 to 160 Hz against the last second's average, held 200 ms. Good enough to blink to; not a tempo tracker.
- A part left out by `include` is zeroed, not removed (`n_bands = 0` for the spectrum), so the header never changes shape.
- The player captures only while someone is subscribed, at the highest rate asked; a slower subscriber gets every second or fourth frame.

**Backpressure: frames are dropped, never queued.** Stale visualisation is worse than missing visualisation, and an LED wall that lags reads as broken. A subscriber that stops reading gets the newest frame when it resumes, not a backlog.

**Where the frame sits against the sound.** The analyser is at the output end of the audio graph, and on the development machine the output latency is about 50 ms. So a frame reaches a client about 50 ms *before* the speaker plays what it describes. A rig that wants its lights exactly on the beat delays frames by about that much.

---

## Errors

Every refusal is a reply with `"ok": false` and an `error` in words:

- `unknown command "set_eq"`
- `"search" needs q`: a field is missing
- `"subscribe_viz": bands must be 8..=128, not 4`: a field is out of range
- `protocol version 2 is not supported (this server speaks 1)`
- `malformed request: …`: the line is not a request
- `say hello first: …`
- `"next" needs "transport", and this connection asked for ["viz"] at hello`
- and the domain's own: `no playlist 99`, `"At Port" is on hp, which isn't plugged in`

A client should show the text; it is written for a person.

---

## Versioning

- **Protocol 1 is frozen.** It grows only by adding: a new command, a new optional field in a request or a reply, a new event, a new capability. The viz frame's layout does not change.
- **A client ignores what it does not know**: fields it did not expect, events it did not ask about.
- **A breaking change is protocol 2.** `hello` names the version, the player refuses one it does not speak, and the previous major is supported for one release cycle after a new one ships.
- **Look for capabilities, not versions.** A command's capability is in `hello`'s `capabilities` from the build that has it.

---

## Examples

- [`examples/viz_bars.py`](../examples/viz_bars.py): Python, standard library only. Hello asking for `viz` alone, subscribe, and bars in a terminal. Driving LEDs is the same loop with the `print` swapped for your strip's library.
- `tools/control-client.ps1`: every command from PowerShell (`status`, `layout`, `playlists`, `search "…"`, `queue_playlist 12`, `play 89`, `palette`), and `listen` to watch events.
- `tools/viz-client.ps1`: a viz subscriber that measures latency, cadence and the drop policy (`-Bands`, `-Rate`, `-Depth`, `-Seconds`, `-Show`, `-StallSeconds`).

---

## Design notes

Why it is shaped this way. Not part of the contract.

### Why this exists

Transport control alone was a convenience the windowshade mini-player already solves. **A visualisation stream so someone can drive an LED wall from the analyser** is a different thing: nothing else in the app provides it (D15). That made the API a public contract strangers write against, which is why it is versioned and documented and why protocol 1 is frozen (D8, D24).

### Two channels, binary for viz

32 spectrum bands as JSON floats at 60 Hz is about 240 KB/s of number formatting and parsing, for data that is natively 32 bytes; a binary frame is about 50. Latency and jitter are the product for an LED rig. And mixing framed binary with newline-delimited JSON on one stream is a parsing hazard for every client ever written, so the two channels are kept apart.

### The webview hop, measured

The audio graph lives in the webview (Web Audio's `AnalyserNode`, D5), not in Rust, so the spectrum path is:

```
<audio> → AnalyserNode → JS getByteFrequencyData()
        → Tauri IPC → Rust → per-subscriber pipe
```

Measured at v0.4b on the development machine (Windows 11, WebView2), ten-second runs of `tools\viz-client.ps1` with music playing, the client in a second process reading each frame's wall-clock timestamp against its own clock (D77). `HP_VIZ_TRACE=1` on the app prints the same path from Rust once a second.

| | 15 Hz, 128 bands, f32 | 30 Hz, 32 bands, u8 | 60 Hz, 19 bands, u8 |
|---|---|---|---|
| Analyser read → client process, p50 | 1.35 ms | 1.2 ms | 1.1 ms |
| p95 | 1.7 ms | 1.6 ms | 1.5 ms |
| max | 8 ms | 8 ms | 7.5 ms |
| Cadence, p50 / p95 / max | 67.4 / 68.9 / 69.4 ms | 32.7 / 36.6 / 37.7 ms | 16.2 / 20.2 / 21.6 ms |
| Frames delivered | 15.0 Hz | 30.0 Hz | 60.0 Hz |

The webview-to-Rust hop is about 1.0 ms median and under 2 ms at p95; the rest is the pipe. Two subscribers at once (30 and 60 Hz) each got their own rate from one capture loop. After a client stopped reading for two seconds, four stale frames arrived before live ones, not sixty: each subscriber's writer reads from a slot holding only the newest frame, its pipe's outbound buffer is two frames deep rather than the 64 KB default, and the source drops a tick rather than queueing one while the last frame's IPC is still out.

`AudioContext.outputLatency + baseLatency` is 50 ms on that machine, which is why a frame leads the speaker (above). The socket does not lag the sound; the analyser's own window (2048 samples, 43 ms at 48 kHz) is the bigger delay, and moving analysis into Rust with `symphonia` would not change it. The latency is not on the wire; if a client needs it, it is an additive field on `subscribe_viz`'s reply.

### Security

Any local process can connect. The alternative is a token dance that makes integration annoying, to protect against an attacker who already has code execution. `want` at the handshake is a limit a client puts on itself, so a bars-only client cannot also skip tracks or search the library through a bug; it is not security, and it was built for the freeze because narrowing what a connection may do after the fact would have been a breaking change (D151).

### How it arrived

| Milestone | What landed |
|---|---|
| v0.3 | The control channel: handshake, transport, events. Unstable, on purpose |
| v0.4b | The viz channel, with the analyser; latency measured |
| v1.0 | `layout` and `layout_changed` (#181, D148); the library commands (#182, D149); `palette` and `palette_changed`, placed at v0.5 and missed there (#183, D150); `want`, hello first, `"stable": true`, this reference and the Python example (#184, D151) |

This is the protocol's own history; the canonical milestone table is in `decisions.md` (D27).

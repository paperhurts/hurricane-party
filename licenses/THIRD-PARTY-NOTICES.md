# Third-party software in the download

The Windows zip carries three programs beside `hurricane-party.exe`. hurricane-party starts each as a separate program and links none of them into itself, so each keeps its own licence, whose text is in this folder.

| Program | Version | Licence | Text here |
|---|---|---|---|
| ffmpeg | N-126504-g1b8a2b690b (2026-09-11), yt-dlp's Windows x64 GPL build | GPL version 3 or later: built with `--enable-gpl --enable-version3` | `ffmpeg-GPL-3.0.txt` |
| yt-dlp | 2026.08.19, the official Windows executable | The Unlicense. The executable bundles Python and other components under their own licences | `yt-dlp-LICENSE.txt`, `yt-dlp-THIRD_PARTY_LICENSES.txt` |
| Deno | 2.6.4 | MIT | `deno-LICENSE.md` |

## Where the source is

**ffmpeg**, as shipped:

- FFmpeg at the commit this build was made from: <https://github.com/FFmpeg/FFmpeg/tree/1b8a2b690b>
- The scripts that built it, which name every library in it and the version of each: <https://github.com/yt-dlp/FFmpeg-Builds/tree/0309b22040edfc40b855f3a2d917c39c2d3975af>
- The release the binary came from: <https://github.com/yt-dlp/FFmpeg-Builds/releases/tag/autobuild-2026-09-11-17-43>
- `ffmpeg -version` prints the build's full configuration.

**yt-dlp**: <https://github.com/yt-dlp/yt-dlp/tree/2026.08.19>

**Deno**: <https://github.com/denoland/deno/tree/v2.6.4>

A person can point hurricane-party at an ffmpeg of their own instead (the **ffmpeg…** button in the library). That copy is theirs, under whatever licence it came with.

---

*For whoever bumps a pin in `tools/fetch-sidecars.ps1`: update the version and the links above, and replace the matching licence text, in the same pull request (D133).*

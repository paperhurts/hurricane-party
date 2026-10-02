# Third-party software in the download

The download carries three programs beside hurricane-party itself. hurricane-party starts each as a separate program and links none of them into itself, so each keeps its own licence, whose text is in this folder.

| Program | Version | Licence | Text here |
|---|---|---|---|
| ffmpeg | N-127083-g65a3870462 (2026-10-01), yt-dlp's GPL build: Windows x64, and Linux x64 | GPL version 3 or later: built with `--enable-gpl --enable-version3` | `ffmpeg-GPL-3.0.txt` |
| yt-dlp | 2026.08.19, the official standalone executable for Windows (`yt-dlp.exe`) and Linux (`yt-dlp_linux`) | The Unlicense. The executable bundles Python and other components under their own licences | `yt-dlp-LICENSE.txt`, `yt-dlp-THIRD_PARTY_LICENSES.txt` |
| Deno | 2.6.4 | MIT | `deno-LICENSE.md` |

## Where the source is

**ffmpeg**, as shipped:

- FFmpeg at the commit this build was made from: <https://github.com/FFmpeg/FFmpeg/tree/65a3870462>
- The scripts that built it, which name every library in it and the version of each: <https://github.com/yt-dlp/FFmpeg-Builds/tree/1c5094a1995b3a1cff6d8696d339bfb304384716>
- The release the binary came from: <https://github.com/yt-dlp/FFmpeg-Builds/releases/tag/autobuild-2026-10-01-19-27>, which yt-dlp removes after a few weeks; the same archives are kept at <https://github.com/paperhurts/hurricane-party/releases/tag/sidecars-2026-10> (D183)
- `ffmpeg -version` prints the build's full configuration.

**yt-dlp**: <https://github.com/yt-dlp/yt-dlp/tree/2026.08.19>

**Deno**: <https://github.com/denoland/deno/tree/v2.6.4>

A person can point hurricane-party at an ffmpeg of their own instead (the **ffmpeg…** button in the library). That copy is theirs, under whatever licence it came with.

## Data inside the program

| Data | Source | Terms |
|---|---|---|
| The ZIP code table the Cone radar centres on (`src-tauri/src/radar_zips.txt`): each ZIP code and its point, nothing else | U.S. Census Bureau, 2025 Gazetteer Files, ZIP Code Tabulation Areas national file: <https://www2.census.gov/geo/docs/maps-data/data/gazetteer/2025_Gazetteer/2025_Gaz_zcta_national.zip>, trimmed by `tools/zip-points.ps1` | A work of the U.S. government, in the public domain |

It is read from inside the program and never fetched: the app asks no one where a ZIP code is.

---

*For whoever bumps a pin in `tools/fetch-sidecars.ps1`: update the version and the links above, and replace the matching licence text, in the same pull request (D133).*

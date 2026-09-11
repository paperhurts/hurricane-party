# Third-party notices

Code and data in this repository that came from someone else, and the licence
it came under. Dependencies are not listed here: `package.json`,
`src-tauri/Cargo.toml` and the lockfiles name those, and their licences travel
with them.

## Webamp — the classic skin sprite offsets

`src/lib/wsz.ts` carries the sprite rectangles of the `.wsz` format. A classic
Winamp skin declares nothing about where its sprites sit inside each BMP; the
offsets are conventional, and the reference for them is Webamp's
`skinSprites.ts`. They were transcribed into this repo's own structure, along
with the character order of `TEXT.BMP`.

<https://github.com/captbaritone/webamp>

```
The MIT License (MIT)

Copyright (c) [2015] [Jordan Eldredge]

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

No skin art from Webamp or from any skin author is in this repository. The
skins this app ships are its own (`skins/eyewall/`, D90).

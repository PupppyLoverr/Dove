# Third-party notices

Dove includes work from the following projects. Their licenses are reproduced
or referenced below; everything else in this repository is Apache-2.0.

## Zeron (MIT)

Copyright (c) 2026 Wing. https://github.com/zeronsh/zeron

Dove vendors Zeron's `proto`, `harness`, `voice-media` and `markdown` crates under `vendor/`
(see `vendor/README.md`). Portions of Dove's user interface are adapted from Zeron: the glass and frost
surface helpers, rounded clipping, window controls, the new-thread wallpaper
effects (dither / halftone / scanlines), the theme tokens, the streaming
markdown "mend" pass (`apps/dove/src/markdown_mend.rs`, from `crates/markdown`),
and the transcript's collapsible tool-call groups: task-tree rail, shimmering
summary, chip headers, fold motion and the thought flattening that renders
streamed reasoning (`apps/dove/src/tool_calls.rs`, from `crates/ui/src/transcript.rs`),
and the file-type icon resolver (`apps/dove/src/file_icons.rs`, from
`crates/ui/src/file_icons.rs`).

> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in
> all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
> SOFTWARE.

## zui / GPUI (Apache-2.0)

GPUI by Zed Industries, via the zeronsh/zui fork, and gpui-base from
zeronsh/gpui-component (Apache-2.0, originally by Longbridge).

## Symbols file icons (MIT)

Copyright (c) 2020-22 Miguel Solorio. https://github.com/miguelsolorio/vscode-symbols

The file and folder icons under `apps/dove/assets/file-icons/` and the theme
manifest `apps/dove/src/file-icons.json` are copied, through Zeron, from
`vscode-symbols` at commit `296ef1b62287fb2315cb5651e552e09e8c8e1de8`. The full
license is kept beside the assets in `apps/dove/assets/file-icons/LICENSE.symbols`.

## Solar Icons (CC BY 4.0)

The interface icons in `apps/dove/assets/icons` are Zeron's icon set: mostly
"Solar Icons" (Linear) by 480 Design, licensed under CC BY 4.0
(https://creativecommons.org/licenses/by/4.0/), plus glyphs Zeron draws in the
same style (MIT, see above). A few Dove-only glyphs are Solar Icons, or are
drawn by Dove to match.

Agent brand marks (Claude, OpenAI, Cursor, Devin, Grok, OpenCode, Pi, Hermes,
Antigravity) identify the respective products and remain the property of their
owners.

## Geist and Geist Mono (SIL OFL 1.1)

Copyright Vercel. See `apps/dove/assets/fonts/Geist-OFL.txt`.

## Great Vibes (SIL OFL 1.1)

Copyright 2015 The Great Vibes Pro Project Authors
(https://github.com/googlefonts/great-vibes). Used only at build time to bake
the handwritten "Dove" wordmark in the first-launch intro
(`media/intro/src/build-assets.mjs`); the font file itself is not shipped in
the app. See `media/intro/src/fonts/GreatVibes-OFL.txt`.

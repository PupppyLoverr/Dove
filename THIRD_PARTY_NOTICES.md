# Third-party notices

Dove includes work from the following projects. Their licenses are reproduced
or referenced below; everything else in this repository is Apache-2.0.

## Zeron (MIT)

Copyright (c) 2026 Wing. https://github.com/zeronsh/zeron

Dove vendors Zeron's `proto`, `harness` and `voice-media` crates under `vendor/`
(see `vendor/README.md`). Portions of Dove's user interface are adapted from Zeron: the glass and frost
surface helpers, rounded clipping, window controls, the new-thread wallpaper
effects (dither / halftone / scanlines), the theme tokens, and the streaming
markdown "mend" pass (`apps/dove/src/markdown_mend.rs`, from `crates/markdown`).

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

## Solar Icons (CC BY 4.0)

Most interface icons in `apps/dove/assets/icons` are "Solar Icons" (Linear) by
480 Design, licensed under CC BY 4.0: https://creativecommons.org/licenses/by/4.0/

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

## Lucide icons

`apps/dove/assets/icons/*.svg` (except the harness brand marks, `dove-logo`,
and the window caption glyphs) come from [Lucide](https://lucide.dev)
(`lucide-static` 0.511.0), licensed ISC:

> Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2022 as part
> of Feather (MIT). All other copyright (c) for Lucide are held by Lucide
> Contributors 2022.
>
> Permission to use, copy, modify, and/or distribute this software for any
> purpose with or without fee is hereby granted, provided that the above
> copyright notice and this permission notice appear in all copies.
>
> THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH
> REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY
> AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT,
> INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM
> LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR
> OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR
> PERFORMANCE OF THIS SOFTWARE.

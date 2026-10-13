# Dove intro

The intro plays the first time Dove starts on a device. The window goes black, a silver shimmer runs around the rim, and then "Dove" is handwritten in the center before the app fades in (about 3.9 s).

![Dove intro](dove-intro.webp)

Shareable files: [`dove-intro.mp4`](dove-intro.mp4), [`dove-intro.webm`](dove-intro.webm), [`dove-intro.webp`](dove-intro.webp) (loops), [`poster.png`](poster.png).

## How it is built

One timeline drives both the app and the video:

| Source | What it is |
| --- | --- |
| `src/timeline.json` | Beats: border, shimmer, pen light, writing, fade |
| `src/centerlines.json` | Hand-traced pen paths for D, o, v, e (Great Vibes at 400 px) |
| `src/fonts/GreatVibes-Regular.ttf` | The script font (OFL), used at build time only |

- `build-assets.mjs` slices each glyph along its pen path into ordered "ink" polygons. Filling them in order looks like a pen writing, while the letterforms stay exactly the font's. It writes `apps/dove/src/intro/data.rs` for the app and `src/data.js` for the video.
- `scene.html` is the reference renderer. Open `scene.html?debug` for a scrubber. `apps/dove/src/intro/mod.rs` draws the same scene natively in GPUI, so keep the two in step.
- `capture.mjs` renders frames using the system's Edge or Chrome, and `encode.ps1` produces the videos. The script fails if any output is over 1 MB.

```bash
cd media/intro/src && npm install && npm run all
```

After changing the timeline or the pen paths, run `npm run assets` and rebuild the app.

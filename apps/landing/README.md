# Dove website

Static landing page: `index.html`, `styles.css`, `main.js` and `assets/`. No build step, no runtime dependencies, no third-party requests (fonts and logos are self-hosted).

## Preview

```sh
cd apps/landing
python3 -m http.server 4321    # http://localhost:4321
```

`?theme=dark` or `?theme=light` forces a theme (useful for screenshots); otherwise the page follows the system and remembers the toggle.

## Deploy (free)

**Cloudflare Pages:** create a Pages project from this repo, framework preset "None", build command empty, output directory `apps/landing`. Or from a terminal:

```sh
npx wrangler pages deploy apps/landing --project-name dove
```

**GitHub Pages:** Settings → Pages → Source "GitHub Actions", then use the stock "Static HTML" workflow with `path: apps/landing`.

## Screenshots

`assets/shots/*.jpg` are captures of the running Dove app (`cargo run -p dove`), saved as JPEG at 2× and roughly 1600–2400 px wide. Replace them when the UI changes; keep the file names.

## Credits

Inter 4.1 and JetBrains Mono 2.304 (SIL OFL 1.1), Instrument Serif (SIL OFL 1.1), K22 icons, agent logos are trademarks of their owners and are used to show compatibility.

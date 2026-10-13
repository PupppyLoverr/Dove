// Captures scene.html frame by frame with the system's Edge (or Chrome) via
// playwright-core, so no browser download is needed.
//
//   node capture.mjs                 -> ../frames/f0000.png ... at 30 fps
//   node capture.mjs --at 0.9 2.5    -> ../frames/at-0.90.png ... single frames

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { chromium } from 'playwright-core';

const here = path.dirname(fileURLToPath(import.meta.url));
const data = fs.readFileSync(path.join(here, 'data.js'), 'utf8');
const { timeline } = JSON.parse(data.slice(data.indexOf('{'), data.lastIndexOf('}') + 1));

const W = 1280, H = 800, FPS = 30;
const outDir = path.resolve(here, '../frames');
fs.mkdirSync(outDir, { recursive: true });

const args = process.argv.slice(2);
const at = args[0] === '--at' ? args.slice(1).map(Number) : null;

async function launch() {
  for (const channel of ['msedge', 'chrome']) {
    try { return await chromium.launch({ channel }); } catch {}
  }
  return chromium.launch();
}

const browser = await launch();
const page = await browser.newPage({ viewport: { width: W, height: H }, deviceScaleFactor: 1 });
await page.goto(pathToFileURL(path.join(here, 'scene.html')).href + `?w=${W}&h=${H}`);
await page.waitForFunction(() => typeof window.render === 'function');

const shoot = async (t, file) => {
  await page.evaluate((t) => window.render(t), t);
  await page.screenshot({ path: path.join(outDir, file), clip: { x: 0, y: 0, width: W, height: H } });
};

if (at) {
  for (const t of at) await shoot(t, `at-${t.toFixed(2)}.png`);
} else {
  for (const f of fs.readdirSync(outDir)) if (/^f\d+\.png$/.test(f)) fs.unlinkSync(path.join(outDir, f));
  const frames = Math.round(timeline.duration * FPS);
  for (let i = 0; i < frames; i++) {
    await shoot(i / FPS, `f${String(i).padStart(4, '0')}.png`);
    if (i % 30 === 0) process.stdout.write(`frame ${i}/${frames}\r`);
  }
  console.log(`captured ${frames} frames to ${path.relative(process.cwd(), outDir)}`);
}
await browser.close();

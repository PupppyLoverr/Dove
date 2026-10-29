// Builds the load-ready extension: `node build.mjs` writes dist/keplorayx-for-chrome/
// with only the files Chrome runs (no tests, no README), then checks that every
// path the manifest, pages and modules reference is in it. Load that folder with
// chrome://extensions › Developer mode › Load unpacked.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const out = path.join(here, 'dist', 'keplorayx-for-chrome');
const files = [
  'manifest.json',
  'background.js',
  'page.js',
  'panel.html',
  'panel.css',
  'panel.js',
  'review.html',
  'review.js',
  'lib/policy.js',
  'lib/review.js',
];

fs.rmSync(out, { recursive: true, force: true });
const manifest = JSON.parse(fs.readFileSync(path.join(here, 'manifest.json'), 'utf8'));
const icons = new Set([...Object.values(manifest.icons), ...Object.values(manifest.action.default_icon)]);
for (const f of [...files, ...icons]) {
  fs.mkdirSync(path.dirname(path.join(out, f)), { recursive: true });
  fs.copyFileSync(path.join(here, f), path.join(out, f));
}

// Every reference must resolve inside the output folder.
const refs = [manifest.background.service_worker, manifest.side_panel.default_path, ...icons];
for (const f of files.filter((f) => /\.(html|js)$/.test(f))) {
  const text = fs.readFileSync(path.join(out, f), 'utf8');
  for (const m of text.matchAll(/(?:from\s+|import\s+|src=|href=)['"](\.{0,2}\/?[\w./-]+\.(?:js|css))['"]/g)) {
    refs.push(path.posix.join(path.posix.dirname(f), m[1]));
  }
}
const missing = [...new Set(refs)].filter((r) => !fs.existsSync(path.join(out, r)));
if (missing.length) {
  console.error(`Missing from the build: ${missing.join(', ')}`);
  process.exit(1);
}
console.log(`${manifest.name} ${manifest.version} → ${out}`);

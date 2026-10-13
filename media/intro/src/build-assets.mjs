// Bakes the intro's shared data from timeline.json, centerlines.json and the
// Great Vibes font into:
//   - data.js                            (read by scene.html for the video)
//   - apps/dove/src/intro/data.rs        (compiled into the app)
//
// The handwritten wordmark is made by slicing each glyph along a hand-traced
// pen path: slice i is the part of the glyph swept by the pen between samples
// i and i+1 that no earlier slice has claimed. Filling slices in order looks
// like ink leaving a pen while keeping the font's exact letterforms, and needs
// no masking (GPUI has none).

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import opentype from 'opentype.js';
import pcRaw from 'polygon-clipping';

// polygon-clipping occasionally recurses forever on near-degenerate input.
// Snap to a grid and, on failure, retry with the operands nudged slightly.
const snap = (g, e = 0) => g.map((poly) => poly.map((r) => r.map(([x, y], i) => [
  Math.round((x + e * Math.sin(i * 12.9898)) * 1000) / 1000,
  Math.round((y + e * Math.cos(i * 78.233)) * 1000) / 1000,
])));
const asMulti = (g) => (g.length && typeof g[0][0][0] === 'number' ? [g] : g);
const safeOp = (op) => (...gs) => {
  for (const e of [0, 0.003, 0.02, 0.08]) {
    try { return pcRaw[op](...gs.map((g) => snap(asMulti(g), e))); } catch {}
  }
  console.warn(`  ${op} failed; skipped`);
  return op === 'union' ? asMulti(gs[0]) : op === 'difference' ? asMulti(gs[0]) : [];
};
const pc = { union: safeOp('union'), intersection: safeOp('intersection'), difference: safeOp('difference') };

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../../..');
const readJson = (f) => JSON.parse(fs.readFileSync(path.join(here, f), 'utf8').replace(/^\uFEFF/, ''));
const timeline = readJson('timeline.json');
const centerlines = readJson('centerlines.json');
const font = opentype.loadSync(path.join(here, 'fonts/GreatVibes-Regular.ttf'));

const WORD = 'Dove';
const FONT_SIZE = 400; // centerlines.json is traced at this size
const STEP = 8; // pen sample spacing, font px
// The swept band follows the local stroke: half-width = distance from the pen
// to the nearest glyph edge, padded. A fixed width would grab stubs of thick
// strokes wherever a hairline crosses them.
const BAND_SCALE = 1.45, BAND_PAD = 2.5, BAND_MIN = 4, BAND_MAX = 16;
const MIN_AREA = 0.4;

// ---------- geometry helpers ----------
const area = (r) => {
  let a = 0;
  for (let i = 0; i < r.length; i++) {
    const [x1, y1] = r[i], [x2, y2] = r[(i + 1) % r.length];
    a += x1 * y2 - x2 * y1;
  }
  return a / 2;
};
const centroid = (r) => {
  let x = 0, y = 0;
  for (const p of r) { x += p[0]; y += p[1]; }
  return [x / r.length, y / r.length];
};
const dist2 = (a, b) => (a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2;
const segDist = (p, a, b) => {
  const dx = b[0] - a[0], dy = b[1] - a[1], l2 = dx * dx + dy * dy;
  const t = l2 ? Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2)) : 0;
  return Math.sqrt(dist2(p, [a[0] + t * dx, a[1] + t * dy]));
};

function flatten(cmds) {
  const rings = [];
  let ring = null, cur = [0, 0];
  const push = (p) => { ring.push(p); cur = p; };
  for (const c of cmds) {
    if (c.type === 'M') { if (ring && ring.length > 2) rings.push(ring); ring = []; push([c.x, c.y]); }
    else if (c.type === 'L') push([c.x, c.y]);
    else if (c.type === 'Q') {
      const p0 = cur, n = 10;
      for (let i = 1; i <= n; i++) {
        const t = i / n, u = 1 - t;
        push([u * u * p0[0] + 2 * u * t * c.x1 + t * t * c.x, u * u * p0[1] + 2 * u * t * c.y1 + t * t * c.y]);
      }
    } else if (c.type === 'C') {
      const p0 = cur, n = 14;
      for (let i = 1; i <= n; i++) {
        const t = i / n, u = 1 - t;
        push([
          u * u * u * p0[0] + 3 * u * u * t * c.x1 + 3 * u * t * t * c.x2 + t * t * t * c.x,
          u * u * u * p0[1] + 3 * u * u * t * c.y1 + 3 * u * t * t * c.y2 + t * t * t * c.y,
        ]);
      }
    } else if (c.type === 'Z') { if (ring && ring.length > 2) rings.push(ring); ring = null; }
  }
  if (ring && ring.length > 2) rings.push(ring);
  return rings;
}

function glyphGeom(rings) {
  const outerSign = Math.sign(area(rings.reduce((a, r) => (Math.abs(area(r)) > Math.abs(area(a)) ? r : a))));
  const outers = rings.filter((r) => Math.sign(area(r)) === outerSign).map((r) => [r]);
  const holes = rings.filter((r) => Math.sign(area(r)) !== outerSign).map((r) => [r]);
  let g = pc.union(...outers);
  if (holes.length) g = pc.difference(g, ...holes);
  return g;
}

function catmullRom(pts, perSeg = 16) {
  const out = [];
  for (let i = 0; i < pts.length - 1; i++) {
    const p0 = pts[Math.max(0, i - 1)], p1 = pts[i], p2 = pts[i + 1], p3 = pts[Math.min(pts.length - 1, i + 2)];
    for (let j = 0; j < perSeg; j++) {
      const t = j / perSeg, t2 = t * t, t3 = t2 * t;
      out.push([0, 1].map((k) => 0.5 * ((2 * p1[k]) + (-p0[k] + p2[k]) * t + (2 * p0[k] - 5 * p1[k] + 4 * p2[k] - p3[k]) * t2 + (-p0[k] + 3 * p1[k] - 3 * p2[k] + p3[k]) * t3)));
    }
  }
  out.push(pts[pts.length - 1]);
  return out;
}

function resample(line, step) {
  const out = [line[0]];
  let acc = 0;
  for (let i = 1; i < line.length; i++) {
    let a = line[i - 1];
    const b = line[i];
    let seg = Math.sqrt(dist2(a, b));
    while (acc + seg >= step) {
      const t = (step - acc) / seg;
      a = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
      out.push(a);
      seg = Math.sqrt(dist2(a, b));
      acc = 0;
    }
    acc += seg;
  }
  if (dist2(out[out.length - 1], line[line.length - 1]) > 4) out.push(line[line.length - 1]);
  return out;
}

function hull(points) {
  const p = points.slice().sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const cross = (o, a, b) => (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
  const lower = [], upper = [];
  for (const q of p) { while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], q) <= 0) lower.pop(); lower.push(q); }
  for (const q of p.reverse()) { while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], q) <= 0) upper.pop(); upper.push(q); }
  return lower.slice(0, -1).concat(upper.slice(0, -1));
}

const polysToRings = (mp) => mp.flatMap((poly) => poly).map((r) => (dist2(r[0], r[r.length - 1]) < 1e-9 ? r.slice(0, -1) : r)).filter((r) => r.length >= 3);
const mpArea = (mp) => mp.reduce((s, poly) => s + poly.reduce((t, r, k) => t + (k ? -1 : 1) * Math.abs(area(r)), 0), 0);

// ---------- glyphs ----------
const glyphs = [];
font.forEachGlyph(WORD, 0, 0, FONT_SIZE, { kerning: true }, (g, x, y) => {
  glyphs.push(glyphGeom(flatten(g.getPath(x, y, FONT_SIZE).commands)));
});
if (glyphs.length !== centerlines.length) throw new Error('one centerline per letter expected');

// ---------- letter timing ----------
const [w0, w1] = timeline.write.span;
const lift = timeline.write.lift;
const samplesPer = centerlines.map((cl) => resample(catmullRom(cl), STEP));
const lengths = samplesPer.map((s) => (s.length - 1) * STEP);
const weights = lengths.map((l) => Math.pow(l, 0.6));
const strokeTime = w1 - w0 - lift * (glyphs.length - 1);
const wsum = weights.reduce((a, b) => a + b, 0);

let tCursor = w0;
const letters = glyphs.map((glyph, li) => {
  const samples = samplesPer[li];
  const dur = (strokeTime * weights[li]) / wsum;
  const t0 = tCursor, t1 = t0 + dur;
  tCursor = t1 + lift;

  // Pen time per sample: slower where the path turns, eased in and out.
  const segCost = [];
  for (let i = 0; i < samples.length - 1; i++) {
    const a = samples[Math.max(0, i - 1)], b = samples[i], c = samples[i + 1];
    const ang1 = Math.atan2(b[1] - a[1], b[0] - a[0]), ang2 = Math.atan2(c[1] - b[1], c[0] - b[0]);
    let turn = Math.abs(ang2 - ang1);
    if (turn > Math.PI) turn = 2 * Math.PI - turn;
    segCost.push(1 + 1.6 * Math.min(turn, 0.6));
  }
  const total = segCost.reduce((s, c) => s + c, 0);
  const sampleT = [t0];
  let acc = 0;
  for (const c of segCost) {
    acc += c;
    const p = acc / total;
    const x = 0.6 * p + 0.4 * (Math.acos(1 - 2 * p) / Math.PI);
    sampleT.push(t0 + dur * x);
  }

  // Slices.
  const normals = samples.map((_, i) => {
    const a = samples[Math.max(0, i - 1)], b = samples[Math.min(samples.length - 1, i + 1)];
    const dx = b[0] - a[0], dy = b[1] - a[1], l = Math.hypot(dx, dy) || 1;
    return [-dy / l, dx / l];
  });
  const edges = polysToRings(glyph).flatMap((r) => r.map((p, i) => [p, r[(i + 1) % r.length]]));
  const rawHalf = samples.map((p) => {
    let best = Infinity;
    for (const [a, b] of edges) best = Math.min(best, segDist(p, a, b));
    return best;
  });
  // Where strokes cross, the ink widens and the nearest edge is far away; a
  // running minimum keeps the band at the stroke's own width through the cross.
  const halfWidth = rawHalf.map((_, i) => {
    let m = Infinity;
    for (let k = Math.max(0, i - 4); k <= Math.min(rawHalf.length - 1, i + 4); k++) m = Math.min(m, rawHalf[k]);
    return Math.min(BAND_MAX, Math.max(BAND_MIN, m * BAND_SCALE + BAND_PAD));
  });
  const slices = [];
  let claimed = null;
  for (let i = 0; i < samples.length - 1; i++) {
    const quad = hull([0, 1].flatMap((k) => {
      const p = samples[i + k], n = normals[i + k], hw = halfWidth[i + k];
      return [[p[0] + n[0] * hw, p[1] + n[1] * hw], [p[0] - n[0] * hw, p[1] - n[1] * hw]];
    }));
    let piece = pc.intersection(glyph, [[quad]]);
    if (claimed && piece.length) piece = pc.difference(piece, claimed);
    piece = piece.filter((poly) => Math.abs(area(poly[0])) > MIN_AREA);
    slices.push({ t0: sampleT[i], t1: sampleT[i + 1], geom: piece });
    if (piece.length) claimed = claimed ? pc.union(claimed, piece) : piece;
  }

  // A wider second sweep picks up what the tight band missed (edges where the
  // trace runs off-center, flared terminals), still in pen order.
  let rest = claimed ? pc.difference(glyph, claimed) : glyph;
  for (let i = 0; i < samples.length - 1 && rest.length; i++) {
    const quad = hull([0, 1].flatMap((k) => {
      const p = samples[i + k], n = normals[i + k], hw = halfWidth[i + k] * 2.2 + 4;
      return [[p[0] + n[0] * hw, p[1] + n[1] * hw], [p[0] - n[0] * hw, p[1] - n[1] * hw]];
    }));
    const extra = pc.intersection(rest, [[quad]]).filter((poly) => Math.abs(area(poly[0])) > MIN_AREA);
    if (!extra.length) continue;
    slices[i].geom = slices[i].geom.length ? pc.union(slices[i].geom, extra) : extra;
    rest = pc.difference(rest, extra);
  }
  // Anything still left joins the nearest slice.
  for (const poly of rest) {
    if (Math.abs(area(poly[0])) < MIN_AREA) continue;
    const c = centroid(poly[0]);
    let best = 0, bd = Infinity;
    for (let i = 0; i < samples.length - 1; i++) {
      const d = dist2(c, samples[i]);
      if (d < bd) { bd = d; best = i; }
    }
    slices[best].geom = pc.union(slices[best].geom.length ? slices[best].geom : [], [poly]);
  }
  const covered = mpArea(slices.flatMap((s) => s.geom));
  console.log(`${WORD[li]}: ${samples.length} samples, ${slices.length} slices, ${(dur * 1000).toFixed(0)}ms, coverage ${(100 * covered / mpArea(glyph)).toFixed(2)}%`);

  return { glyph, slices, pen: samples.map((p, i) => [sampleT[i], p[0], p[1]]), t0, t1 };
});

// ---------- normalize: ink box centered on (0,0), width 1 ----------
let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
for (const l of letters) for (const r of polysToRings(l.glyph)) for (const [x, y] of r) {
  minX = Math.min(minX, x); maxX = Math.max(maxX, x); minY = Math.min(minY, y); maxY = Math.max(maxY, y);
}
const cx = (minX + maxX) / 2, cy = (minY + maxY) / 2, bw = maxX - minX;
const norm = ([x, y]) => [+((x - cx) / bw).toFixed(5), +((y - cy) / bw).toFixed(5)];
const r4 = (v) => +v.toFixed(4);

const out = letters.map((l) => ({
  t0: r4(l.t0),
  t1: r4(l.t1),
  glyph: polysToRings(l.glyph).map((r) => r.map(norm)),
  slices: l.slices.filter((s) => s.geom.length).map((s) => ({ t0: r4(s.t0), t1: r4(s.t1), rings: polysToRings(s.geom).map((r) => r.map(norm)) })),
  pen: l.pen.map(([t, x, y]) => [r4(t), ...norm([x, y])]),
}));
const wordmark = { aspect: r4((maxY - minY) / bw), letters: out };

fs.writeFileSync(path.join(here, 'data.js'), `// @generated by build-assets.mjs\nwindow.INTRO = ${JSON.stringify({ timeline, wordmark })};\n`);

// ---------- Rust ----------
const f = (v) => {
  const s = (+v).toString();
  return s.includes('.') || s.includes('e') ? s : s + '.0';
};
const pair = (a) => `[${f(a[0])}, ${f(a[1])}]`;
const T = timeline;

const points = [], rings = [], slices = [], lettersRs = [], pen = [];
const pushRings = (rs) => {
  const start = rings.length;
  for (const r of rs) { rings.push([points.length, r.length]); points.push(...r); }
  return [start, rs.length];
};
for (const l of out) {
  const glyph = pushRings(l.glyph);
  const sStart = slices.length;
  for (const s of l.slices) slices.push({ t0: s.t0, t1: s.t1, rings: pushRings(s.rings) });
  const pStart = pen.length;
  pen.push(...l.pen);
  lettersRs.push(`Letter { t0: ${f(l.t0)}, t1: ${f(l.t1)}, glyph: (${glyph[0]}, ${glyph[1]}), slices: (${sStart}, ${l.slices.length}), pen: (${pStart}, ${l.pen.length}) }`);
}

const rs = `// @generated by media/intro/src/build-assets.mjs from timeline.json and
// centerlines.json. Do not edit by hand; run \`npm run assets\` there instead.

#![allow(dead_code, clippy::excessive_precision, clippy::unreadable_literal)]

pub struct Shimmer {
    pub t0: f32,
    pub t1: f32,
    pub alpha: f32,
}

/// Ranges index into [\`RINGS\`], [\`SLICES\`] and [\`PEN\`] as (start, len).
pub struct Letter {
    pub t0: f32,
    pub t1: f32,
    pub glyph: (u32, u32),
    pub slices: (u32, u32),
    pub pen: (u32, u32),
}

pub struct Slice {
    pub t0: f32,
    pub t1: f32,
    pub rings: (u32, u32),
}

pub const DURATION: f32 = ${f(T.duration)};
pub const FADE_OUT: (f32, f32) = (${f(T.fade_out[0])}, ${f(T.fade_out[1])});
pub const BORDER_IN: (f32, f32) = (${f(T.border_in[0])}, ${f(T.border_in[1])});
pub const BORDER_ALPHA: f32 = ${f(T.border_alpha)};
pub const SHIMMERS: &[Shimmer] = &[${T.shimmers.map((s) => `Shimmer { t0: ${f(s.span[0])}, t1: ${f(s.span[1])}, alpha: ${f(s.alpha)} }`).join(', ')}];

pub const PEN_APPEAR: (f32, f32) = (${f(T.pen.appear[0])}, ${f(T.pen.appear[1])});
pub const WRITE_SPAN: (f32, f32) = (${f(T.write.span[0])}, ${f(T.write.span[1])});
pub const WRITE_WIDTH_FRAC: f32 = ${f(T.write.width_frac)};
pub const WRITE_MAX_HEIGHT_FRAC: f32 = ${f(T.write.max_height_frac)};

/// Height / width of the wordmark's ink box. Coordinates are centered on
/// (0, 0) with the word 1.0 wide.
pub const WORD_ASPECT: f32 = ${f(wordmark.aspect)};
pub const LETTERS: &[Letter] = &[
${lettersRs.map((l) => `    ${l},`).join('\n')}
];
pub const SLICES: &[Slice] = &[
${slices.map((s) => `    Slice { t0: ${f(s.t0)}, t1: ${f(s.t1)}, rings: (${s.rings[0]}, ${s.rings[1]}) },`).join('\n')}
];
pub const PEN: &[[f32; 3]] = &[${pen.map((p) => `[${p.map(f).join(', ')}]`).join(', ')}];
pub const RINGS: &[(u32, u32)] = &[${rings.map((r) => `(${r[0]}, ${r[1]})`).join(', ')}];
pub const POINTS: &[[f32; 2]] = &[${points.map(pair).join(', ')}];
`;
const rsPath = path.join(repo, 'apps/dove/src/intro/data.rs');
fs.mkdirSync(path.dirname(rsPath), { recursive: true });
fs.writeFileSync(rsPath, rs);
console.log(`wrote ${path.relative(repo, rsPath)} (${(rs.length / 1024).toFixed(0)} KB, ${points.length} points, ${slices.length} slices)`);

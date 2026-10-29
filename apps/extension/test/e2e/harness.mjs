// Test plumbing: a stand-in for Keplorayx's loopback bridge (the same wire
// protocol as apps/dove/src/browser/control.rs), a tiny DevTools client, and
// browser discovery. Clearly a test double: it lives only under test/.
import http from 'node:http';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { spawn } from 'node:child_process';

// ---------- the stand-in Keplorayx bridge ----------
export function startBridge() {
  const token = crypto.randomBytes(32).toString('hex');
  let sock = null;
  let next = 1;
  const pending = new Map();
  const events = [];
  let onHello = null;
  const hello = new Promise((r) => { onHello = r; });
  const server = http.createServer((_, res) => { res.writeHead(404); res.end(); });
  server.on('upgrade', (req, socket) => {
    const origin = req.headers.origin || '';
    if (!origin.startsWith('chrome-extension://') || req.url !== '/extension') {
      events.push({ rejected: origin });
      socket.end('HTTP/1.1 403 Forbidden\r\n\r\n');
      return;
    }
    const accept = crypto.createHash('sha1').update(req.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    let buf = Buffer.alloc(0);
    let paired = false;
    socket.on('error', () => {});
    socket.on('data', (chunk) => {
      buf = Buffer.concat([buf, chunk]);
      for (;;) {
        const f = readFrame(buf);
        if (!f) break;
        buf = buf.subarray(f.used);
        if (f.op === 8) { socket.end(); return; }
        if (f.op === 9) { socket.write(frame(10, f.payload)); continue; }
        if (f.op !== 1) continue;
        let msg; try { msg = JSON.parse(f.payload.toString()); } catch (_) { continue; }
        if (msg.type === 'hello' && !paired) {
          if (msg.token !== token) { socket.write(frame(1, JSON.stringify({ type: 'rejected', reason: 'bad token' }))); socket.end(); return; }
          paired = true;
          sock = socket;
          socket.write(frame(1, JSON.stringify({ type: 'welcome', app: 'Keplorayx (test bridge)' })));
          onHello(msg);
        } else if (msg.type === 'result' && paired) {
          const p = pending.get(msg.id);
          if (p) { pending.delete(msg.id); p(msg); }
        }
      }
    });
    socket.on('close', () => { if (sock === socket) sock = null; });
  });
  const api = {
    token, events, hello,
    get connected() { return !!sock; },
    /** Send a raw string (to test validation). */
    raw(text) { sock.write(frame(1, text)); },
    call(tool, args = {}, timeoutMs = 60_000) {
      const id = next++;
      return new Promise((resolve, reject) => {
        const t = setTimeout(() => { pending.delete(id); reject(new Error(`${tool} timed out`)); }, timeoutMs);
        pending.set(id, (m) => { clearTimeout(t); resolve(m); });
        sock.write(frame(1, JSON.stringify({ type: 'call', id, tool, args })));
      });
    },
    close: () => new Promise((r) => { sock?.destroy(); server.close(() => r()); }),
  };
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => { api.port = server.address().port; resolve(api); }));
}

function readFrame(buf) {
  if (buf.length < 2) return null;
  const op = buf[0] & 0x0f;
  const masked = (buf[1] & 0x80) !== 0;
  let len = buf[1] & 0x7f;
  let off = 2;
  if (len === 126) { if (buf.length < 4) return null; len = buf.readUInt16BE(2); off = 4; }
  else if (len === 127) { if (buf.length < 10) return null; len = Number(buf.readBigUInt64BE(2)); off = 10; }
  const maskAt = off;
  if (masked) off += 4;
  if (buf.length < off + len) return null;
  const payload = Buffer.from(buf.subarray(off, off + len));
  if (masked) for (let i = 0; i < len; i++) payload[i] ^= buf[maskAt + (i % 4)];
  return { op, payload, used: off + len };
}

function frame(op, data) {
  const payload = Buffer.isBuffer(data) ? data : Buffer.from(data);
  const n = payload.length;
  const head = n < 126 ? Buffer.from([0x80 | op, n]) : n < 65536 ? Buffer.from([0x80 | op, 126, n >> 8, n & 255]) : (() => { const h = Buffer.alloc(10); h[0] = 0x80 | op; h[1] = 127; h.writeBigUInt64BE(BigInt(n), 2); return h; })();
  return Buffer.concat([head, payload]);
}

// ---------- DevTools ----------
export async function cdpConnect(wsUrl) {
  const ws = new WebSocket(wsUrl);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  let id = 1;
  const waiting = new Map();
  const listeners = [];
  ws.onmessage = (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id && waiting.has(m.id)) { const { resolve, reject } = waiting.get(m.id); waiting.delete(m.id); m.error ? reject(new Error(m.error.message)) : resolve(m.result); }
    else listeners.forEach((l) => l(m));
  };
  return {
    send(method, params = {}, sessionId) {
      const mid = id++;
      ws.send(JSON.stringify({ id: mid, method, params, ...(sessionId ? { sessionId } : {}) }));
      return new Promise((resolve, reject) => waiting.set(mid, { resolve, reject }));
    },
    on(fn) { listeners.push(fn); },
    close() { ws.close(); },
  };
}

/** Evaluate in a target as if the user did it (a real user gesture). */
export async function evalIn(cdp, targetId, expression) {
  const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });
  try {
    const r = await cdp.send('Runtime.evaluate', { expression, awaitPromise: true, userGesture: true, returnByValue: true }, sessionId);
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  } finally {
    await cdp.send('Target.detachFromTarget', { sessionId }).catch(() => {});
  }
}

export async function waitFor(fn, timeoutMs = 10_000, every = 100) {
  const end = Date.now() + timeoutMs;
  for (;;) {
    const v = await fn();
    if (v) return v;
    if (Date.now() > end) throw new Error('timed out waiting');
    await new Promise((r) => setTimeout(r, every));
  }
}

// ---------- browsers ----------
export function findBrowsers() {
  const env = process.env.DOVE_E2E_BROWSERS?.split(path.delimiter).filter(Boolean);
  if (env?.length) return env.map((p) => ({ name: path.basename(p), path: p }));
  const pf = process.env['ProgramFiles'] || 'C:\\Program Files';
  const pf86 = process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)';
  const local = process.env.LOCALAPPDATA || '';
  const candidates = [
    ['Chrome', path.join(pf, 'Google/Chrome/Application/chrome.exe')],
    ['Chrome', path.join(local, 'Google/Chrome/Application/chrome.exe')],
    ['Edge', path.join(pf86, 'Microsoft/Edge/Application/msedge.exe')],
    ['Edge', path.join(pf, 'Microsoft/Edge/Application/msedge.exe')],
    ['Brave', path.join(pf, 'BraveSoftware/Brave-Browser/Application/brave.exe')],
    ['Chrome', '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'],
    ['Edge', '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge'],
    ['Chromium', '/usr/bin/chromium'],
    ['Chrome', '/usr/bin/google-chrome'],
  ];
  const seen = new Set();
  return candidates.filter(([n, p]) => fs.existsSync(p) && !seen.has(n) && seen.add(n)).map(([name, p]) => ({ name, path: p }));
}

/**
 * Launch `browser` headless with a copy of the extension. The copy's only
 * change: host access to the fixture origin is pre-granted, because a
 * headless browser cannot show the permission prompt a real user answers.
 */
export async function launch(browser, extensionDir, fixtureOrigin) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'dove-e2e-'));
  const ext = path.join(tmp, 'ext');
  fs.cpSync(extensionDir, ext, { recursive: true, filter: (p) => !p.includes(`${path.sep}test`) });
  const manifest = JSON.parse(fs.readFileSync(path.join(ext, 'manifest.json'), 'utf8'));
  manifest.host_permissions = [`${fixtureOrigin}/*`];
  fs.writeFileSync(path.join(ext, 'manifest.json'), JSON.stringify(manifest, null, 2));
  const profile = path.join(tmp, 'profile');
  const proc = spawn(browser.path, [
    '--headless=new', `--user-data-dir=${profile}`, '--remote-debugging-port=0', '--no-first-run', '--no-default-browser-check',
    '--disable-features=DisableLoadExtensionCommandLineSwitch', `--disable-extensions-except=${ext}`, `--load-extension=${ext}`, 'about:blank',
  ], { stdio: 'ignore' });
  const portFile = path.join(profile, 'DevToolsActivePort');
  const port = await waitFor(() => fs.existsSync(portFile) && fs.readFileSync(portFile, 'utf8').split('\n')[0], 20_000);
  const version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  const cdp = await cdpConnect(version.webSocketDebuggerUrl);
  await cdp.send('Target.setDiscoverTargets', { discover: true });
  const sw = await waitFor(async () => {
    const { targetInfos } = await cdp.send('Target.getTargets');
    return targetInfos.find((t) => t.type === 'service_worker' && t.url.endsWith('/background.js') && t.url.startsWith('chrome-extension://'));
  }, 20_000);
  const extId = new URL(sw.url).host;
  return {
    cdp, extId, version: version.Browser, tmp, proc,
    async targets() { return (await cdp.send('Target.getTargets')).targetInfos; },
    async close() {
      await Promise.race([cdp.send('Browser.close').catch(() => {}), new Promise((r) => setTimeout(r, 2000))]);
      cdp.close();
      await new Promise((r) => setTimeout(r, 500));
      try { proc.kill(); } catch (_) {}
      try { fs.rmSync(tmp, { recursive: true, force: true }); } catch (_) {}
    },
  };
}

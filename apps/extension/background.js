// Dove for Chrome — service worker. Holds one WebSocket to the Dove app on
// 127.0.0.1 and executes chrome_* tool calls in the user's real tabs.
import { pageSnapshot, pageLocate, pageFocusClear, pageSelect, pageRead, pageScroll, pageWaitFor, pageIndicator, pageAskAllow } from './page.js';

const LOG_MAX = 200;
let ws = null;
let state = 'unpaired'; // unpaired | connecting | connected | offline | rejected
let controlled = null; // tab id Dove last acted on
const attached = new Set();

// ---------- storage ----------
const local = () => chrome.storage.local.get({ pairing: null, allowed: {}, paused: false });
async function setStatus(next, detail = '') {
  state = next;
  await chrome.storage.session.set({ status: { state, detail, at: Date.now() } });
  chrome.action.setBadgeText({ text: '' });
  chrome.action.setTitle({ title: `Dove — ${next}${detail ? ': ' + detail : ''}` });
}
async function log(entry) {
  const { log = [] } = await chrome.storage.session.get('log');
  log.unshift({ at: Date.now(), ...entry });
  await chrome.storage.session.set({ log: log.slice(0, LOG_MAX) });
}

// ---------- connection ----------
async function connect() {
  const { pairing } = await local();
  if (!pairing) return setStatus('unpaired');
  if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) return;
  await setStatus('connecting');
  let sock;
  try { sock = new WebSocket(`ws://127.0.0.1:${pairing.port}/extension`); } catch (e) { return setStatus('offline', String(e)); }
  ws = sock;
  sock.onopen = () => sock.send(JSON.stringify({ type: 'hello', token: pairing.token, agent: navigator.userAgent, version: chrome.runtime.getManifest().version }));
  sock.onmessage = async (ev) => {
    let msg; try { msg = JSON.parse(ev.data); } catch (_) { return; }
    if (msg.type === 'welcome') { await setStatus('connected', msg.app || 'Dove'); return; }
    if (msg.type === 'rejected') { await setStatus('rejected', msg.reason || 'pairing code was not accepted'); return; }
    if (msg.type === 'call') {
      const reply = await run(msg.tool, msg.args || {}).then(
        (r) => ({ type: 'result', id: msg.id, ok: true, ...r }),
        (e) => ({ type: 'result', id: msg.id, ok: false, error: e?.message || String(e) }),
      );
      await log({ tool: msg.tool, args: summarize(msg.args), ok: reply.ok, error: reply.error });
      if (sock.readyState === WebSocket.OPEN) sock.send(JSON.stringify(reply));
    }
  };
  sock.onclose = async () => { if (ws === sock) ws = null; if (state !== 'rejected') await setStatus('offline', 'Dove is not running'); };
  sock.onerror = () => {};
}
// Keep the worker alive while connected (Chrome 116+ extends lifetime on WS traffic).
setInterval(() => { if (ws?.readyState === WebSocket.OPEN) ws.send('{"type":"ping"}'); else connect(); }, 20000);
chrome.alarms.create('dove-reconnect', { periodInMinutes: 0.5 });
chrome.alarms.onAlarm.addListener(() => connect());
chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(() => {
  chrome.sidePanel.setPanelBehavior({ openPanelOnActionClick: true }).catch(() => {});
  connect();
});
connect();

chrome.runtime.onMessage.addListener((msg, sender, respond) => {
  (async () => {
    if (msg.type === 'pair') {
      const code = String(msg.code || '').trim();
      const m = code.match(/^dove-(\d{2,5})-([A-Za-z0-9]{16,})$/);
      if (!m) return respond({ error: 'That is not a Dove pairing code. Copy it from Dove → Settings → Integrations.' });
      await chrome.storage.local.set({ pairing: { port: Number(m[1]), token: m[2] } });
      ws?.close(); ws = null; await connect();
      return respond({ ok: true });
    }
    if (msg.type === 'unpair') { await chrome.storage.local.set({ pairing: null }); ws?.close(); ws = null; await setStatus('unpaired'); return respond({ ok: true }); }
    if (msg.type === 'stop') { await chrome.storage.local.set({ paused: true }); await release(); await log({ tool: 'stop', ok: true, args: 'stopped by you' }); return respond({ ok: true }); }
    if (msg.type === 'resume') { await chrome.storage.local.set({ paused: false }); return respond({ ok: true }); }
    if (msg.type === 'reconnect') { ws?.close(); ws = null; await connect(); return respond({ ok: true }); }
    if (msg.type === 'clear-log') { await chrome.storage.session.set({ log: [] }); return respond({ ok: true }); }
    respond({ error: 'unknown message' });
  })();
  return true;
});

chrome.debugger.onDetach.addListener(({ tabId }) => attached.delete(tabId));
chrome.tabs.onRemoved.addListener((id) => { attached.delete(id); if (controlled === id) controlled = null; });
chrome.tabs.onUpdated.addListener((id, info) => {
  if (id !== controlled || info.status !== 'complete') return;
  // Re-show the indicator after a reload, but only where the user already allowed Dove.
  Promise.all([chrome.tabs.get(id), local()])
    .then(([tab, { allowed, paused }]) => {
      if (!paused && allowed[originOf(tab.url || '')]) return indicator(id, true);
    })
    .catch(() => {});
});

// ---------- helpers ----------
const summarize = (a) => { try { const s = JSON.stringify(a || {}); return s.length > 160 ? s.slice(0, 160) + '…' : s; } catch (_) { return ''; } };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const originOf = (url) => { try { const u = new URL(url); return /^https?:$/.test(u.protocol) ? u.origin : null; } catch (_) { return null; } };

async function tabFor(args) {
  if (args.tab != null) return chrome.tabs.get(Number(args.tab));
  if (controlled != null) { try { return await chrome.tabs.get(controlled); } catch (_) { controlled = null; } }
  const [t] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  if (!t) throw new Error('No browser tab is open.');
  return t;
}
async function exec(tabId, func, args = []) {
  const [res] = await chrome.scripting.executeScript({ target: { tabId }, func, args, world: 'ISOLATED' });
  const v = res?.result;
  if (v && v.error) throw new Error(v.error);
  return v;
}
async function indicator(tabId, on) {
  try { await chrome.scripting.executeScript({ target: { tabId }, func: pageIndicator, args: [on, 'Dove is controlling this tab'] }); } catch (_) {}
}
async function release() {
  for (const id of [...attached]) { try { await chrome.debugger.detach({ tabId: id }); } catch (_) {} }
  attached.clear();
  if (controlled != null) await indicator(controlled, false);
  controlled = null;
}
async function cdp(tabId, method, params = {}) {
  if (!attached.has(tabId)) { await chrome.debugger.attach({ tabId }, '1.3'); attached.add(tabId); }
  return chrome.debugger.sendCommand({ tabId }, method, params);
}
async function waitLoad(tabId, timeout = 15000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const t = await chrome.tabs.get(tabId);
    if (t.status === 'complete') return t;
    await sleep(120);
  }
  return chrome.tabs.get(tabId);
}

/** Every page-touching tool goes through here: user pause, per-site allow, indicator. */
async function claim(tab) {
  const { allowed, paused } = await local();
  if (paused) throw new Error('The user paused Dove in this browser (Dove side panel → Resume).');
  const origin = originOf(tab.url || '');
  if (!origin) throw new Error(`Dove can only act on http(s) pages; this tab is ${tab.url || 'blank'}.`);
  if (!allowed[origin]) {
    await chrome.tabs.update(tab.id, { active: true });
    const ok = await exec(tab.id, pageAskAllow, [origin]).catch(() => false);
    if (!ok) throw new Error(`The user has not allowed Dove on ${origin}. Ask them to allow it from the Dove side panel.`);
    const cur = await local();
    await chrome.storage.local.set({ allowed: { ...cur.allowed, [origin]: true } });
    await log({ tool: 'allow', ok: true, args: origin });
  }
  if (controlled !== tab.id && controlled != null) await indicator(controlled, false);
  controlled = tab.id;
  await indicator(tab.id, true);
  return tab;
}

const KEYS = {
  Enter: [13, '\r'], Tab: [9, '\t'], Escape: [27, ''], Backspace: [8, ''], Delete: [46, ''], Space: [32, ' '], ' ': [32, ' '],
  ArrowUp: [38, ''], ArrowDown: [40, ''], ArrowLeft: [37, ''], ArrowRight: [39, ''], Home: [36, ''], End: [35, ''], PageUp: [33, ''], PageDown: [34, ''],
};
const MOD = { Alt: 1, Control: 2, Ctrl: 2, Meta: 4, Cmd: 4, Command: 4, Shift: 8 };
async function press(tabId, combo) {
  const parts = String(combo).split('+').map((p) => p.trim()).filter(Boolean);
  let key = parts.pop() || 'Enter';
  const modifiers = parts.reduce((m, p) => m | (MOD[p] || 0), 0);
  const known = KEYS[key];
  const code = known ? known[0] : key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0;
  const text = known ? known[1] : key.length === 1 && !(modifiers & 6) ? key : '';
  if (key === 'Space') key = ' ';
  await cdp(tabId, 'Input.dispatchKeyEvent', { type: text ? 'keyDown' : 'rawKeyDown', key, windowsVirtualKeyCode: code, modifiers, text, unmodifiedText: text });
  await cdp(tabId, 'Input.dispatchKeyEvent', { type: 'keyUp', key, windowsVirtualKeyCode: code, modifiers });
}
async function mouse(tabId, x, y, click) {
  await cdp(tabId, 'Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
  if (!click) return;
  await cdp(tabId, 'Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: click });
  await cdp(tabId, 'Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: click });
}
const need = (args, k) => { if (args[k] == null || args[k] === '') throw new Error(`Missing "${k}".`); return args[k]; };
const tabLine = (t) => `[${t.id}]${t.active ? ' (active)' : ''}${t.id === controlled ? ' (Dove)' : ''} ${t.title || ''} — ${t.url || ''}`;

// ---------- tools ----------
async function run(tool, args) {
  switch (tool) {
    case 'tabs': {
      const tabs = await chrome.tabs.query({});
      return { text: tabs.map(tabLine).join('\n') || 'No tabs.' };
    }
    case 'tab_open': {
      const { paused } = await local();
      if (paused) throw new Error('The user paused Dove in this browser.');
      const t = await chrome.tabs.create({ url: args.url || 'about:blank', active: args.background !== true });
      controlled = t.id;
      const done = args.url ? await waitLoad(t.id) : t;
      if (originOf(done.url || '')) await claim(done).catch(() => {});
      return { text: `Opened ${tabLine(done)}` };
    }
    case 'tab_close': {
      const t = await tabFor(args);
      await chrome.tabs.remove(t.id);
      return { text: `Closed tab ${t.id}.` };
    }
    case 'tab_activate': {
      const t = await chrome.tabs.update(Number(need(args, 'tab')), { active: true });
      await chrome.windows.update(t.windowId, { focused: true });
      controlled = t.id;
      return { text: `Active: ${tabLine(t)}` };
    }
    case 'navigate': {
      const t = await tabFor(args);
      const { paused } = await local();
      if (paused) throw new Error('The user paused Dove in this browser.');
      let url = String(need(args, 'url'));
      if (!/^[a-z]+:/i.test(url)) url = 'https://' + url;
      if (!/^https?:/i.test(url)) throw new Error('Only http(s) URLs.');
      await chrome.tabs.update(t.id, { url });
      controlled = t.id;
      await sleep(150);
      const done = await waitLoad(t.id, Number(args.timeout_ms) || 20000);
      await claim(done);
      return { text: `${done.status === 'complete' ? 'Loaded' : 'Still loading'}: ${done.title} — ${done.url}` };
    }
    case 'snapshot': {
      const t = await claim(await tabFor(args));
      const s = await exec(t.id, pageSnapshot, [args.ref || null, args.full === true]);
      return { text: `Tab ${t.id}: ${s.title}\n${s.url}\n\n${s.outline || '(empty page)'}` };
    }
    case 'click': case 'hover': {
      const t = await claim(await tabFor(args));
      const at = await exec(t.id, pageLocate, [String(need(args, 'ref'))]);
      await mouse(t.id, at.x, at.y, tool === 'click' ? (args.double ? 2 : 1) : 0);
      if (tool === 'click') { await sleep(250); await waitLoad(t.id, 8000); }
      return { text: `${tool === 'click' ? 'Clicked' : 'Hovering'} ${args.ref} (${at.tag}).` };
    }
    case 'type': {
      const t = await claim(await tabFor(args));
      const ref = String(need(args, 'ref'));
      await exec(t.id, pageLocate, [ref]);
      const ok = await exec(t.id, pageFocusClear, [ref, args.clear !== false]);
      if (!ok) throw new Error(`Could not focus ${ref}.`);
      await cdp(t.id, 'Input.insertText', { text: String(args.text ?? '') });
      if (args.submit) { await press(t.id, 'Enter'); await sleep(250); await waitLoad(t.id, 8000); }
      return { text: `Typed ${String(args.text ?? '').length} characters into ${ref}${args.submit ? ' and pressed Enter' : ''}.` };
    }
    case 'press': {
      const t = await claim(await tabFor(args));
      if (args.ref) await exec(t.id, pageFocusClear, [String(args.ref), false]);
      await press(t.id, need(args, 'key'));
      await sleep(150);
      return { text: `Pressed ${args.key}.` };
    }
    case 'scroll': {
      const t = await claim(await tabFor(args));
      if (args.ref) { await exec(t.id, pageLocate, [String(args.ref)]); return { text: `Scrolled ${args.ref} into view.` }; }
      const amount = Number(args.amount) || 600;
      const dir = String(args.direction || 'down');
      const p = await exec(t.id, pageScroll, [dir === 'left' ? -amount : dir === 'right' ? amount : 0, dir === 'up' ? -amount : dir === 'down' ? amount : 0]);
      return { text: `Scroll position y=${p.y} of ${p.maxY}.` };
    }
    case 'select': {
      const t = await claim(await tabFor(args));
      const r = await exec(t.id, pageSelect, [String(need(args, 'ref')), String(need(args, 'value'))]);
      return { text: `Selected "${r.ok}".` };
    }
    case 'screenshot': {
      const t = await claim(await tabFor(args));
      if (!t.active) { await chrome.tabs.update(t.id, { active: true }); await sleep(200); }
      const data = await chrome.tabs.captureVisibleTab(t.windowId, { format: 'png' });
      return { text: `Screenshot of tab ${t.id}: ${t.title}`, image: { mime: 'image/png', data: data.replace(/^data:image\/png;base64,/, '') } };
    }
    case 'read': {
      const t = await claim(await tabFor(args));
      const r = await exec(t.id, pageRead, [args.full === true]);
      return { text: `${r.title}\n${r.url}\n\n${r.text}` };
    }
    case 'wait': {
      const t = await tabFor(args);
      const ms = Math.min(Number(args.timeout_ms) || 10000, 60000);
      if (!args.text && !args.selector && args.ms) { await sleep(Math.min(Number(args.ms), 30000)); return { text: `Waited ${args.ms} ms.` }; }
      await waitLoad(t.id, ms);
      if (args.text || args.selector) { await claim(t); await exec(t.id, pageWaitFor, [args.text || null, args.selector || null, ms]); }
      return { text: args.text ? `Found "${args.text}".` : args.selector ? `Found ${args.selector}.` : 'Page loaded.' };
    }
    case 'release': {
      await release();
      return { text: 'Released the browser.' };
    }
    default:
      throw new Error(`Unknown chrome tool: ${tool}`);
  }
}

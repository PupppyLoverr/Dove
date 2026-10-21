// Dove for Chrome — service worker. Holds one WebSocket to the Dove app on
// 127.0.0.1 and executes chrome_* tool calls in the user's real tabs.
//
// Trust boundaries:
// - Dove: authenticated by the pairing token; its messages are validated
//   (known types, known tools, known arguments, size limits) before use.
// - Web pages: everything read from them is returned to agents as untrusted
//   data, and nothing a page does can approve anything. Site access and
//   consequential actions are decided in an extension window (review.html),
//   which a page cannot draw over or click.
// - Our own pages (side panel, review window) are the only senders allowed
//   privileged messages; injected scripts may only ask to stop.
import {
  pageSnapshot, pageLocate, pageDescribe, pageContext, pageFind, pageExtract,
  pageFocusClear, pageSelect, pageRead, pageScroll, pageWaitFor, pageIndicator,
} from './page.js';
import {
  validateInbound, senderMayRequest, parsePairingCode, originOf, normalizeUrl,
  consequential, sensitiveField, summarize, clampText, untrusted,
} from './lib/policy.js';
import { Reviews } from './lib/review.js';

const LOG_MAX = 200;
const OWN_ORIGIN = chrome.runtime.getURL('');
let ws = null;
let state = 'unpaired'; // unpaired | connecting | connected | offline | rejected
let controlled = null; // tab id Dove last acted on
let epoch = 0; // bumped by Stop: calls from an older epoch give up
const attached = new Set();
const reviews = new Reviews();
let reviewWindow = null;

// ---------- storage ----------
const local = () => chrome.storage.local.get({ pairing: null, allowed: {}, paused: false });
async function setStatus(next, detail = '') {
  state = next;
  await chrome.storage.session.set({ status: { state, detail, at: Date.now() } });
  const waiting = reviews.list().length;
  chrome.action.setBadgeText({ text: waiting ? String(waiting) : '' });
  chrome.action.setTitle({ title: `Dove — ${next}${detail ? ': ' + detail : ''}` });
}
async function log(entry) {
  const { log = [] } = await chrome.storage.session.get('log');
  log.unshift({ at: Date.now(), ...entry });
  await chrome.storage.session.set({ log: log.slice(0, LOG_MAX) });
}
async function publishReviews() {
  const list = reviews.list();
  await chrome.storage.session.set({ reviews: list });
  chrome.action.setBadgeText({ text: list.length ? String(list.length) : '' });
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
    const problem = validateInbound(ev.data);
    if (problem) {
      // A malformed call still gets an answer, so Dove does not hang on it.
      try { const m = JSON.parse(ev.data); if (Number.isSafeInteger(m?.id)) sock.send(JSON.stringify({ type: 'result', id: m.id, ok: false, error: `Rejected by Dove for Chrome: ${problem}` })); } catch (_) {}
      return;
    }
    const msg = JSON.parse(ev.data);
    if (msg.type === 'welcome') { await setStatus('connected', msg.app || 'Dove'); return; }
    if (msg.type === 'rejected') { await setStatus('rejected', msg.reason || 'pairing code was not accepted'); return; }
    if (msg.type !== 'call') return;
    const started = epoch;
    const at = Date.now();
    const reply = await run(msg.tool, msg.args || {}, started).then(
      (r) => ({ type: 'result', id: msg.id, ok: true, ...r, text: clampText(r.text) }),
      (e) => ({ type: 'result', id: msg.id, ok: false, error: e?.message || String(e) }),
    );
    const redact = !!reply.redacted;
    delete reply.redacted;
    await log({ tool: msg.tool, args: summarize(msg.args, redact), ok: reply.ok, error: reply.error, ms: Date.now() - at, who: msg.args?.context || null });
    if (sock.readyState === WebSocket.OPEN) sock.send(JSON.stringify(reply));
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

async function stopAll(why) {
  epoch++;
  reviews.denyAll(why);
  await publishReviews();
  await release();
}

chrome.runtime.onMessage.addListener((msg, sender, respond) => {
  (async () => {
    const type = msg?.type;
    if (!senderMayRequest(sender, chrome.runtime.id, OWN_ORIGIN, type)) return respond({ error: 'not allowed' });
    if (type === 'pair') {
      const p = parsePairingCode(msg.code);
      if (!p) return respond({ error: 'That is not a Dove pairing code. Copy it from Dove → Settings → Integrations.' });
      await chrome.storage.local.set({ pairing: p });
      ws?.close(); ws = null; await connect();
      return respond({ ok: true });
    }
    if (type === 'unpair') { await chrome.storage.local.set({ pairing: null }); ws?.close(); ws = null; await setStatus('unpaired'); return respond({ ok: true }); }
    if (type === 'stop') { await chrome.storage.local.set({ paused: true }); await stopAll('stopped by you'); await log({ tool: 'stop', ok: true, args: 'stopped by you' }); return respond({ ok: true }); }
    if (type === 'resume') { await chrome.storage.local.set({ paused: false }); return respond({ ok: true }); }
    if (type === 'reconnect') { ws?.close(); ws = null; await connect(); return respond({ ok: true }); }
    if (type === 'clear-log') { await chrome.storage.session.set({ log: [] }); return respond({ ok: true }); }
    if (type === 'reviews') return respond({ reviews: reviews.list() });
    if (type === 'review-decision') {
      const ok = reviews.decide(String(msg.id), msg.allow === true, msg.allow === true ? 'allowed by you' : 'denied by you');
      await publishReviews();
      return respond({ ok });
    }
    respond({ error: 'unknown message' });
  })();
  return true;
});

chrome.debugger.onDetach.addListener(({ tabId }) => attached.delete(tabId));
chrome.tabs.onRemoved.addListener((id) => { attached.delete(id); if (controlled === id) controlled = null; });
chrome.windows.onRemoved.addListener((id) => { if (id === reviewWindow) { reviewWindow = null; reviews.denyAll('review window closed'); publishReviews(); } });
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
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
/** Sleep that gives up the moment the user presses Stop. */
async function nap(ms, started) {
  const end = Date.now() + ms;
  while (Date.now() < end) { checkEpoch(started); await sleep(Math.min(100, end - Date.now())); }
  checkEpoch(started);
}

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
async function waitLoad(tabId, timeout = 15000, started = epoch) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    checkEpoch(started);
    const t = await chrome.tabs.get(tabId);
    if (t.status === 'complete') return t;
    await sleep(120);
  }
  return chrome.tabs.get(tabId);
}
function checkEpoch(started) {
  if (started !== epoch) throw new Error('Stopped by the user in the browser. Ask them before trying again.');
}

/** Ask the user in an extension window; resolves true when allowed. */
async function review(details) {
  const { id, decided } = reviews.request(details);
  await publishReviews();
  try {
    if (reviewWindow != null) {
      await chrome.windows.update(reviewWindow, { focused: true, drawAttention: true });
    } else {
      const w = await chrome.windows.create({ url: chrome.runtime.getURL(`review.html#${id}`), type: 'popup', width: 380, height: 300, focused: true });
      reviewWindow = w.id;
    }
  } catch (_) { /* The side panel lists reviews too. */ }
  const { allow, why } = await decided;
  await publishReviews();
  if (!reviews.list().length && reviewWindow != null) { chrome.windows.remove(reviewWindow).catch(() => {}); reviewWindow = null; }
  await log({ tool: 'review', ok: allow, args: `${details.what} — ${why}` });
  return allow;
}

/** Every page-touching tool goes through here: user pause, per-site allow, indicator. */
async function claim(tab, started) {
  checkEpoch(started);
  const { allowed, paused } = await local();
  if (paused) throw new Error('The user paused Dove in this browser (Dove side panel → Resume).');
  const origin = originOf(tab.url || '');
  if (!origin) throw new Error(`Dove can only act on http(s) pages; this tab is ${tab.url || 'blank'}.`);
  const granted = () => chrome.permissions.contains({ origins: [`${origin}/*`] });
  if (!allowed[origin] || !(await granted())) {
    const ok = await review({ kind: 'site', origin, what: `Let Dove read and act on ${new URL(origin).host}` });
    checkEpoch(started);
    if (!ok || !(await granted())) throw new Error(`The user has not allowed Dove on ${origin}. Ask them to allow it from the Dove side panel.`);
    const cur = await local();
    await chrome.storage.local.set({ allowed: { ...cur.allowed, [origin]: true } });
    await log({ tool: 'allow', ok: true, args: origin });
  }
  if (controlled !== tab.id && controlled != null) await indicator(controlled, false);
  controlled = tab.id;
  await indicator(tab.id, true);
  return tab;
}

/** Stop before a consequential action until the user says yes. */
async function guard(tab, action, el, started) {
  const reason = consequential(action, el);
  if (!reason) return;
  const target = el.text || el.label || el.value || el.name || el.tag;
  const ok = await review({ kind: 'action', origin: originOf(tab.url || ''), what: `${action === 'click' ? 'Click' : 'Submit'} “${String(target).slice(0, 60)}” on ${new URL(tab.url).host}`, reason });
  checkEpoch(started);
  if (!ok) throw new Error(`The user declined: ${reason}. Do not retry this action; tell the user what you wanted to do.`);
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
const pageText = (tab, text) => untrusted(originOf(tab.url || '') || 'this page', text);

// ---------- tools ----------
async function run(tool, args, started) {
  switch (tool) {
    case 'tabs': {
      const tabs = await chrome.tabs.query({});
      return { text: tabs.map(tabLine).join('\n') || 'No tabs.' };
    }
    case 'tab_open': {
      const { paused } = await local();
      if (paused) throw new Error('The user paused Dove in this browser.');
      const url = args.url ? normalizeUrl(args.url) : 'about:blank';
      const t = await chrome.tabs.create({ url, active: args.background !== true });
      controlled = t.id;
      const done = args.url ? await waitLoad(t.id) : t;
      if (originOf(done.url || '')) await claim(done, started).catch(() => {});
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
      checkEpoch(started);
      const { paused } = await local();
      if (paused) throw new Error('The user paused Dove in this browser.');
      const url = normalizeUrl(need(args, 'url'));
      await chrome.tabs.update(t.id, { url });
      controlled = t.id;
      await sleep(150);
      const done = await waitLoad(t.id, Math.min(Number(args.timeout_ms) || 20000, 60000), started);
      await claim(done, started);
      if (done.status !== 'complete') return { text: `Still loading after ${Math.round((Number(args.timeout_ms) || 20000) / 1000)} s: ${done.title} — ${done.url}. Take a snapshot or wait, rather than navigating again.` };
      return { text: `Loaded: ${done.title} — ${done.url}` };
    }
    case 'context': {
      const t = await claim(await tabFor(args), started);
      const c = await exec(t.id, pageContext);
      return { text: pageText(t, JSON.stringify(c, null, 1)) };
    }
    case 'snapshot': {
      const t = await claim(await tabFor(args), started);
      const s = await exec(t.id, pageSnapshot, [args.ref || null, args.full === true]);
      return { text: `Tab ${t.id}: ${s.title}\n${s.url}\n\n${pageText(t, s.outline || '(empty page)')}` };
    }
    case 'find': {
      const t = await claim(await tabFor(args), started);
      const r = await exec(t.id, pageFind, [args.text || '', args.role || null, Math.min(Number(args.limit) || 20, 100)]);
      return { text: pageText(t, r.text) };
    }
    case 'extract': {
      const t = await claim(await tabFor(args), started);
      const r = await exec(t.id, pageExtract, [String(need(args, 'kind')), args.selector || null, args.limit || 50]);
      return { text: pageText(t, JSON.stringify(r)) };
    }
    case 'click': case 'hover': {
      const t = await claim(await tabFor(args), started);
      const ref = String(need(args, 'ref'));
      if (tool === 'click') await guard(t, 'click', await exec(t.id, pageDescribe, [ref, false]), started);
      const at = await exec(t.id, pageLocate, [ref]);
      checkEpoch(started);
      await mouse(t.id, at.x, at.y, tool === 'click' ? (args.double ? 2 : 1) : 0);
      if (tool === 'click') { await sleep(250); await waitLoad(t.id, 8000); }
      return { text: `${tool === 'click' ? 'Clicked' : 'Hovering'} ${ref} (${at.tag}). Take a new snapshot to see the result; refs from before may be stale.` };
    }
    case 'type': {
      const t = await claim(await tabFor(args), started);
      const ref = String(need(args, 'ref'));
      const el = await exec(t.id, pageDescribe, [ref, false]);
      const sensitive = sensitiveField(el);
      if (sensitive) throw new Error(`Dove does not type into ${sensitive}. Ask the user to fill it in themselves, then continue.`);
      if (args.submit) await guard(t, 'type', { ...el, submit: true }, started);
      await exec(t.id, pageLocate, [ref]);
      const ok = await exec(t.id, pageFocusClear, [ref, args.clear !== false]);
      if (!ok) throw new Error(`Could not focus ${ref}.`);
      checkEpoch(started);
      await cdp(t.id, 'Input.insertText', { text: String(args.text ?? '') });
      if (args.submit) { await press(t.id, 'Enter'); await sleep(250); await waitLoad(t.id, 8000); }
      return { text: `Typed ${String(args.text ?? '').length} characters into ${ref}${args.submit ? ' and pressed Enter' : ''}.` };
    }
    case 'press': {
      const t = await claim(await tabFor(args), started);
      const key = String(need(args, 'key'));
      if (args.ref) await exec(t.id, pageFocusClear, [String(args.ref), false]);
      if (/(^|\+)Enter$/.test(key)) {
        const el = await exec(t.id, pageDescribe, [args.ref ? String(args.ref) : null, true]);
        await guard(t, 'press', el ? { ...el, enter: true } : null, started);
      }
      checkEpoch(started);
      await press(t.id, key);
      await sleep(150);
      return { text: `Pressed ${key}.` };
    }
    case 'scroll': {
      const t = await claim(await tabFor(args), started);
      if (args.ref) { await exec(t.id, pageLocate, [String(args.ref)]); return { text: `Scrolled ${args.ref} into view.` }; }
      const amount = Number(args.amount) || 600;
      const dir = String(args.direction || 'down');
      const p = await exec(t.id, pageScroll, [dir === 'left' ? -amount : dir === 'right' ? amount : 0, dir === 'up' ? -amount : dir === 'down' ? amount : 0]);
      return { text: `Scroll position y=${p.y} of ${p.maxY}.` };
    }
    case 'select': {
      const t = await claim(await tabFor(args), started);
      const r = await exec(t.id, pageSelect, [String(need(args, 'ref')), String(need(args, 'value'))]);
      return { text: `Selected "${r.ok}".` };
    }
    case 'upload': {
      const t = await claim(await tabFor(args), started);
      const ref = String(need(args, 'ref'));
      const paths = Array.isArray(args.paths) ? args.paths.map(String) : [];
      if (!paths.length || paths.length > 10 || paths.some((p) => !/^([a-zA-Z]:[\\/]|\/)/.test(p))) {
        throw new Error('upload needs "paths": 1-10 absolute file paths.');
      }
      const el = await exec(t.id, pageDescribe, [ref, false]);
      if (!el || el.tag !== 'input' || el.type !== 'file') throw new Error(`${ref} is not a file input.`);
      const names = paths.map((p) => p.split(/[\\/]/).pop()).join(', ');
      const ok = await review({ kind: 'action', origin: originOf(t.url), what: `Upload ${names} to ${new URL(t.url).host}`, reason: 'sends files from this computer to the site' });
      checkEpoch(started);
      if (!ok) throw new Error('The user declined the upload.');
      const { root } = await cdp(t.id, 'DOM.getDocument', { depth: -1, pierce: true });
      const { nodeId } = await cdp(t.id, 'DOM.querySelector', { nodeId: root.nodeId, selector: `[data-dove-ref="${ref}"]` });
      if (!nodeId) throw new Error(`STALE_REF: ${ref} is not on the page any more. Take a new chrome_snapshot.`);
      await cdp(t.id, 'DOM.setFileInputFiles', { files: paths, nodeId });
      return { text: `Attached ${names} to ${ref}. Nothing is sent until the form is submitted.` };
    }
    case 'screenshot': {
      const t = await claim(await tabFor(args), started);
      const { data } = await cdp(t.id, 'Page.captureScreenshot', { format: 'png' });
      return { text: `Screenshot of tab ${t.id}: ${t.title}`, image: { mime: 'image/png', data } };
    }
    case 'read': {
      const t = await claim(await tabFor(args), started);
      const r = await exec(t.id, pageRead, [args.full === true]);
      return { text: `${r.title}\n${r.url}\n\n${pageText(t, r.text)}` };
    }
    case 'wait': {
      const t = await tabFor(args);
      const ms = Math.min(Number(args.timeout_ms) || 10000, 60000);
      if (!args.text && !args.selector && args.ms) { await nap(Math.min(Number(args.ms), 30000), started); return { text: `Waited ${args.ms} ms.` }; }
      await waitLoad(t.id, ms, started);
      if (args.text || args.selector) { await claim(t, started); await exec(t.id, pageWaitFor, [args.text || null, args.selector || null, ms]); }
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

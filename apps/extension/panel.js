import { card } from './review.js';

const $ = (id) => document.getElementById(id);
const send = (msg) => chrome.runtime.sendMessage(msg);
const LABEL = { connected: 'Connected to Dove', connecting: 'Connecting…', offline: 'Dove isn\u2019t running', unpaired: 'Not paired', rejected: 'Pairing code not accepted' };
const DETAIL = { offline: 'Open the Dove app; this reconnects on its own.', rejected: 'Copy a fresh code from Dove → Settings → Plugins.' };

const host = (u) => { try { return new URL(u).host.replace(/^www\./, ''); } catch (_) { return u || ''; } };
const quote = (s, n = 48) => `\u201c${String(s).length > n ? String(s).slice(0, n - 1) + '\u2026' : s}\u201d`;
const target = (a) => a.ref ? (/^e\d+$/.test(a.ref) ? `element ${a.ref}` : quote(a.ref, 32)) : 'the page';

/** One human line per action, instead of the raw tool call. */
function describe(e) {
  let a = {};
  try { a = typeof e.args === 'string' ? JSON.parse(e.args) : (e.args || {}); } catch (_) { a = { value: e.args }; }
  switch (e.tool) {
    case 'allow': return ['Allowed on this site', host(e.args)];
    case 'tabs': return ['Looked at your tabs', ''];
    case 'tab_open': return ['Opened a tab', a.url ? host(a.url) : ''];
    case 'tab_close': return ['Closed a tab', ''];
    case 'tab_activate': return ['Switched tabs', ''];
    case 'navigate': return ['Went to a page', a.url ? host(a.url) : ''];
    case 'snapshot': return ['Read the page layout', ''];
    case 'read': return ['Read the page text', ''];
    case 'screenshot': return ['Took a screenshot', ''];
    case 'click': return [a.double ? 'Double-clicked' : 'Clicked', target(a)];
    case 'hover': return ['Hovered', target(a)];
    case 'type': return ['Typed', `${quote(a.text ?? '')}${a.submit ? ' and pressed Enter' : ''}`];
    case 'press': return ['Pressed', a.key || ''];
    case 'scroll': return ['Scrolled', a.ref ? `to ${target(a)}` : (a.direction || 'down')];
    case 'select': return ['Chose an option', a.value ? quote(a.value) : ''];
    case 'wait': return ['Waited', a.text ? `for ${quote(a.text)}` : a.selector ? `for ${a.selector}` : 'for the page to load'];
    case 'release': return ['Handed the browser back', ''];
    case 'context': return ['Looked at the page', ''];
    case 'find': return ['Searched the page', a.text ? quote(a.text) : (a.role || '')];
    case 'extract': return ['Pulled data from the page', a.kind || ''];
    case 'upload': return ['Attached files', (a.paths || []).join(', ')];
    case 'review': return [e.ok ? 'You allowed' : 'Not allowed', String(e.args || '')];
    case 'stop': return ['Stopped by you', ''];
    default: return [String(e.tool || '').replace(/_/g, ' '), ''];
  }
}

function ago(at) {
  const s = Math.round((Date.now() - at) / 1000);
  if (s < 10) return 'now';
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  return new Date(at).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}
let origin = null;
let tabId = null;

async function currentTab() {
  const [t] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  tabId = t?.id ?? null;
  try { const u = new URL(t?.url || ''); origin = /^https?:$/.test(u.protocol) ? u.origin : null; } catch (_) { origin = null; }
}

async function render() {
  await currentTab();
  const { status = { state: 'unpaired' }, log = [], reviews = [] } = await chrome.storage.session.get(['status', 'log', 'reviews']);
  $('reviews').replaceChildren(...reviews.map((r) => card(r, (id, allow) => send({ type: 'review-decision', id, allow }).then(render))));
  const { pairing, allowed = {}, paused } = await chrome.storage.local.get({ pairing: null, allowed: {}, paused: false });
  const st = pairing ? status.state : 'unpaired';
  $('status').classList.toggle('on', st === 'connected');
  $('status').classList.toggle('busy', st === 'connecting');
  $('status-text').textContent = LABEL[st] || st;
  $('status-detail').textContent = DETAIL[st] || '';
  $('status').title = status.detail || '';
  $('reconnect').hidden = !pairing || st === 'connected';
  $('pair').hidden = !!pairing && st !== 'rejected';
  $('unpair').hidden = !pairing;
  const ok = !!(origin && allowed[origin]);
  $('host').textContent = origin ? host(origin) : 'This page';
  $('site-hint').textContent = !origin ? 'Dove works on regular web pages only'
    : ok ? 'Agents can read and act here' : 'Agents ask before acting here';
  $('allow').disabled = !origin;
  $('allow').checked = ok;
  const recent = log[0];
  const active = !paused && recent && Date.now() - recent.at < 60000 && recent.tool !== 'release' && recent.tool !== 'stop';
  $('control').hidden = !active || !ok;
  $('paused').hidden = !paused;
  $('empty').hidden = log.length > 0;
  $('clear').hidden = log.length === 0;
  $('log').replaceChildren(...log.slice(0, 80).map((e) => {
    const [what, detail] = describe(e);
    const li = document.createElement('li');
    if (!e.ok) li.className = 'fail';
    const n = document.createElement('span'); n.className = 'n'; n.textContent = what;
    const t = document.createElement('span'); t.className = 't'; t.textContent = ago(e.at);
    t.title = new Date(e.at).toLocaleString();
    const a = document.createElement('span'); a.className = 'a'; a.textContent = e.ok ? detail : (e.error || detail);
    li.append(n, t, a);
    if (e.who) { const w = document.createElement('span'); w.className = 'w'; w.textContent = String(e.who).slice(0, 60); li.append(w); }
    return li;
  }));
}

$('pair-btn').onclick = async () => {
  const r = await send({ type: 'pair', code: $('code').value });
  $('pair-err').textContent = r?.error || '';
  if (r?.ok) $('code').value = '';
  render();
};
$('code').addEventListener('keydown', (e) => { if (e.key === 'Enter') $('pair-btn').click(); });
$('allow').onchange = async (e) => {
  if (!origin) return;
  const { allowed = {} } = await chrome.storage.local.get({ allowed: {} });
  if (e.target.checked) {
    const granted = await chrome.permissions.request({ origins: [`${origin}/*`] }).catch(() => false);
    if (!granted) { e.target.checked = false; return render(); }
    allowed[origin] = true;
  } else {
    delete allowed[origin];
    await chrome.permissions.remove({ origins: [`${origin}/*`] }).catch(() => {});
  }
  await chrome.storage.local.set({ allowed });
  if (!e.target.checked && tabId != null) await send({ type: 'stop' }).then(() => send({ type: 'resume' }));
  render();
};
$('stop').onclick = () => send({ type: 'stop' }).then(render);
$('resume').onclick = () => send({ type: 'resume' }).then(render);
$('clear').onclick = () => send({ type: 'clear-log' }).then(render);
$('reconnect').onclick = () => send({ type: 'reconnect' }).then(render);
$('unpair').onclick = () => send({ type: 'unpair' }).then(render);
chrome.storage.onChanged.addListener(render);
chrome.tabs.onActivated.addListener(render);
chrome.tabs.onUpdated.addListener((_, info) => { if (info.url || info.status === 'complete') render(); });
render();
setInterval(render, 5000);

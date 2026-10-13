const $ = (id) => document.getElementById(id);
const send = (msg) => chrome.runtime.sendMessage(msg);
const LABEL = { connected: 'Connected', connecting: 'Connecting…', offline: 'Dove not running', unpaired: 'Not paired', rejected: 'Pairing rejected' };
let origin = null;
let tabId = null;

async function currentTab() {
  const [t] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  tabId = t?.id ?? null;
  try { const u = new URL(t?.url || ''); origin = /^https?:$/.test(u.protocol) ? u.origin : null; } catch (_) { origin = null; }
}

async function render() {
  await currentTab();
  const { status = { state: 'unpaired' }, log = [] } = await chrome.storage.session.get(['status', 'log']);
  const { pairing, allowed = {}, paused } = await chrome.storage.local.get({ pairing: null, allowed: {}, paused: false });
  const st = pairing ? status.state : 'unpaired';
  $('status').classList.toggle('on', st === 'connected');
  $('status-text').textContent = LABEL[st] || st;
  $('status').title = status.detail || '';
  $('pair').hidden = !!pairing && st !== 'rejected';
  $('unpair').hidden = !pairing;
  $('origin').textContent = origin || 'Not a web page';
  $('allow').disabled = !origin;
  $('allow').checked = !!(origin && allowed[origin]);
  const recent = log[0];
  const active = !paused && recent && Date.now() - recent.at < 60000 && recent.tool !== 'release' && recent.tool !== 'stop';
  $('control').hidden = !active || !origin || !allowed[origin];
  $('paused').hidden = !paused;
  $('empty').hidden = log.length > 0;
  $('log').replaceChildren(...log.slice(0, 80).map((e) => {
    const li = document.createElement('li');
    if (!e.ok) li.className = 'fail';
    const t = document.createElement('span'); t.className = 't'; t.textContent = new Date(e.at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
    const n = document.createElement('span'); n.className = 'n'; n.textContent = e.tool.replace(/_/g, ' ');
    const a = document.createElement('span'); a.className = 'a'; a.textContent = e.error || e.args || '';
    li.append(t, n, a);
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
  if (e.target.checked) allowed[origin] = true; else delete allowed[origin];
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

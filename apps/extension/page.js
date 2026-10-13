// Functions injected into pages with chrome.scripting.executeScript. Each one
// must be self-contained: Chrome serializes the function source, not its scope.

/** Outline of the visible page; actionable elements carry [ref=eN]. */
export function pageSnapshot(scopeRef, full) {
  const ATTR = 'data-dove-ref';
  let next = Number(document.documentElement.getAttribute('data-dove-next') || '1');
  const root = scopeRef ? document.querySelector(`[${ATTR}="${scopeRef}"]`) : document.body;
  if (!root) return { error: `No element with ref ${scopeRef}. Take a new chrome_snapshot.` };
  const lines = [];
  const limit = full ? 600 : 160;
  const visible = (el) => {
    const s = getComputedStyle(el);
    if (s.display === 'none' || s.visibility === 'hidden' || Number(s.opacity) === 0) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };
  const role = (el) => {
    const explicit = el.getAttribute('role');
    if (explicit) return explicit;
    const tag = el.tagName.toLowerCase();
    if (tag === 'a' && el.hasAttribute('href')) return 'link';
    if (tag === 'button' || tag === 'summary') return 'button';
    if (tag === 'select') return 'combobox';
    if (tag === 'textarea') return 'textbox';
    if (tag === 'input') {
      const t = (el.getAttribute('type') || 'text').toLowerCase();
      if (['button', 'submit', 'reset', 'image'].includes(t)) return 'button';
      if (t === 'checkbox') return 'checkbox';
      if (t === 'radio') return 'radio';
      if (t === 'range') return 'slider';
      if (t === 'hidden') return null;
      return t === 'search' ? 'searchbox' : 'textbox';
    }
    if (/^h[1-6]$/.test(tag)) return 'heading';
    if (tag === 'img') return 'img';
    if (el.isContentEditable && el.getAttribute('contenteditable') !== 'inherit') return 'textbox';
    if (el.hasAttribute('onclick') || el.tabIndex >= 0 && tag !== 'div' && tag !== 'span') return 'button';
    return null;
  };
  const name = (el) => {
    const label = el.getAttribute('aria-label') || el.getAttribute('alt') || el.getAttribute('title');
    if (label) return label.trim();
    if (el.labels && el.labels.length) return [...el.labels].map((l) => l.innerText.trim()).join(' ');
    if (el.placeholder) return el.placeholder;
    return (el.innerText || el.value || '').replace(/\s+/g, ' ').trim().slice(0, 100);
  };
  const actionable = new Set(['link', 'button', 'textbox', 'searchbox', 'combobox', 'checkbox', 'radio', 'slider', 'menuitem', 'tab', 'option', 'switch']);
  const walk = (el, depth) => {
    if (lines.length >= limit || !(el instanceof Element)) return;
    if (['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'SVG'].includes(el.tagName.toUpperCase())) return;
    if (el.id === 'dove-control-indicator') return;
    if (!visible(el)) return;
    const r = role(el);
    if (r) {
      let ref = el.getAttribute(ATTR);
      if (actionable.has(r) || r === 'heading' || r === 'img') {
        if (!ref) { ref = `e${next++}`; el.setAttribute(ATTR, ref); }
      }
      let line = `${'  '.repeat(Math.min(depth, 8))}- ${r === 'heading' ? 'heading ' + el.tagName[1] : r} "${name(el)}"`;
      if ('value' in el && el.value && r !== 'button') line += ` value="${String(el.value).slice(0, 80)}"`;
      if (el.checked) line += ' [checked]';
      if (el.disabled) line += ' [disabled]';
      if (r === 'link') line += ` href=${el.getAttribute('href').slice(0, 120)}`;
      if (r === 'combobox' && el.options) line += ` options=[${[...el.options].slice(0, 20).map((o) => o.text.trim()).join(' | ')}]`;
      if (ref) line += ` [ref=${ref}]`;
      lines.push(line);
      if (r !== 'heading' && actionable.has(r) && r !== 'combobox') { /* children are its label */ }
      else for (const c of el.children) walk(c, depth + 1);
      return;
    }
    // Loose text directly inside this element.
    let text = '';
    for (const n of el.childNodes) if (n.nodeType === 3) text += n.textContent;
    text = text.replace(/\s+/g, ' ').trim();
    if (text.length > 1) lines.push(`${'  '.repeat(Math.min(depth, 8))}- text "${text.slice(0, full ? 400 : 140)}"`);
    for (const c of el.children) walk(c, depth + (text ? 1 : 0));
    if (el.shadowRoot) for (const c of el.shadowRoot.children) walk(c, depth);
  };
  walk(root, 0);
  document.documentElement.setAttribute('data-dove-next', String(next));
  const more = lines.length >= limit ? `\n… (truncated at ${limit} lines; pass full or a ref to narrow)` : '';
  return { title: document.title, url: location.href, outline: lines.join('\n') + more };
}

/** Scroll an element into view and return its viewport center. */
export function pageLocate(target) {
  let el = null;
  if (/^e\d+$/.test(target)) el = document.querySelector(`[data-dove-ref="${target}"]`);
  if (!el) { try { el = document.querySelector(target); } catch (_) { el = null; } }
  if (!el) {
    const want = target.trim().toLowerCase();
    const all = document.querySelectorAll('a,button,input,textarea,select,[role],label,summary,[contenteditable]');
    el = [...all].find((e) => (e.innerText || e.value || e.getAttribute('aria-label') || '').trim().toLowerCase() === want)
      || [...all].find((e) => (e.innerText || e.getAttribute('aria-label') || '').toLowerCase().includes(want));
  }
  if (!el) return { error: `Nothing matches "${target}". Take a new chrome_snapshot and use its refs.` };
  el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
  const r = el.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2, tag: el.tagName.toLowerCase() };
}

export function pageFocusClear(target, clear) {
  const el = document.querySelector(`[data-dove-ref="${target}"]`) || (() => { try { return document.querySelector(target); } catch (_) { return null; } })();
  if (!el) return false;
  el.focus();
  if (clear) {
    if ('value' in el) { el.value = ''; el.dispatchEvent(new Event('input', { bubbles: true })); }
    else if (el.isContentEditable) el.textContent = '';
  }
  return true;
}

export function pageSelect(target, value) {
  const el = document.querySelector(`[data-dove-ref="${target}"]`) || (() => { try { return document.querySelector(target); } catch (_) { return null; } })();
  if (!el || el.tagName !== 'SELECT') return { error: `${target} is not a <select>.` };
  const want = String(value).toLowerCase();
  const opt = [...el.options].find((o) => o.value.toLowerCase() === want || o.text.trim().toLowerCase() === want);
  if (!opt) return { error: `No option "${value}". Options: ${[...el.options].map((o) => o.text.trim()).join(', ')}` };
  el.value = opt.value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return { ok: opt.text.trim() };
}

export function pageRead(full) {
  const text = (document.body?.innerText || '').replace(/\n{3,}/g, '\n\n').trim();
  const max = full ? 200000 : 40000;
  return { title: document.title, url: location.href, text: text.length > max ? text.slice(0, max) + '\n… (truncated)' : text };
}

export function pageScroll(dx, dy) {
  window.scrollBy({ left: dx, top: dy, behavior: 'instant' });
  return { x: Math.round(scrollX), y: Math.round(scrollY), maxY: Math.round(document.documentElement.scrollHeight - innerHeight) };
}

export async function pageWaitFor(text, selector, timeoutMs) {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) {
    if (document.readyState === 'complete') {
      if (selector) { try { if (document.querySelector(selector)) return { ok: true }; } catch (e) { return { error: String(e) }; } }
      else if (text) { if ((document.body?.innerText || '').includes(text)) return { ok: true }; }
      else return { ok: true };
    }
    await new Promise((r) => setTimeout(r, 150));
  }
  return { error: `Timed out after ${timeoutMs} ms waiting for ${selector ? 'selector ' + selector : text ? '"' + text + '"' : 'the page to load'}.` };
}

/** The "Dove is controlling this tab" indicator: a hairline frame and a pill. */
export function pageIndicator(on, label) {
  const ID = 'dove-control-indicator';
  let host = document.getElementById(ID);
  if (!on) { host?.remove(); return true; }
  if (!host) {
    host = document.createElement('div');
    host.id = ID;
    host.style.cssText = 'all:initial;position:fixed;inset:0;pointer-events:none;z-index:2147483647;';
    const root = host.attachShadow({ mode: 'closed' });
    const dark = matchMedia('(prefers-color-scheme: dark)').matches;
    const fg = dark ? '#f5f5f5' : '#111';
    const bg = dark ? 'rgba(24,24,24,.86)' : 'rgba(255,255,255,.86)';
    root.innerHTML = `<style>
      .frame{position:fixed;inset:0;border:1.5px solid ${fg};opacity:.28;border-radius:10px;animation:f 2.4s ease-in-out infinite}
      @keyframes f{50%{opacity:.12}}
      .pill{position:fixed;left:50%;bottom:18px;transform:translateX(-50%);display:flex;gap:10px;align-items:center;
        font:500 12px/1 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;color:${fg};background:${bg};
        backdrop-filter:blur(18px) saturate(1.4);-webkit-backdrop-filter:blur(18px) saturate(1.4);
        border:1px solid ${dark ? 'rgba(255,255,255,.12)' : 'rgba(0,0,0,.10)'};border-radius:999px;padding:8px 8px 8px 14px;
        box-shadow:0 6px 24px rgba(0,0,0,.12);pointer-events:auto}
      .dot{width:6px;height:6px;border-radius:50%;background:${fg};animation:f 1.2s ease-in-out infinite}
      button{all:unset;cursor:pointer;padding:5px 10px;border-radius:999px;border:1px solid ${dark ? 'rgba(255,255,255,.18)' : 'rgba(0,0,0,.14)'}}
      button:hover{background:${dark ? 'rgba(255,255,255,.08)' : 'rgba(0,0,0,.05)'}}
    </style><div class="frame"></div><div class="pill"><span class="dot"></span><span class="t"></span><button>Stop</button></div>`;
    root.querySelector('button').addEventListener('click', () => chrome.runtime.sendMessage({ type: 'stop' }));
    host._root = root;
    document.documentElement.appendChild(host);
  }
  const t = host._root?.querySelector('.t');
  if (t) t.textContent = label || 'Dove is controlling this tab';
  return true;
}

/** Ask the user, inside the page, whether Dove may act on this site. */
export function pageAskAllow(origin) {
  return new Promise((resolve) => {
    const host = document.createElement('div');
    host.style.cssText = 'all:initial;position:fixed;inset:0;z-index:2147483647;';
    const root = host.attachShadow({ mode: 'closed' });
    const dark = matchMedia('(prefers-color-scheme: dark)').matches;
    const fg = dark ? '#f5f5f5' : '#111', bg = dark ? 'rgba(24,24,24,.9)' : 'rgba(255,255,255,.92)';
    root.innerHTML = `<style>
      .card{position:fixed;right:20px;top:20px;width:320px;font:13px/1.45 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;color:${fg};
        background:${bg};backdrop-filter:blur(20px);-webkit-backdrop-filter:blur(20px);border-radius:14px;padding:16px;
        border:1px solid ${dark ? 'rgba(255,255,255,.12)' : 'rgba(0,0,0,.1)'};box-shadow:0 12px 40px rgba(0,0,0,.18)}
      h1{all:unset;display:block;font-weight:600;font-size:13px;margin-bottom:4px}
      p{margin:0 0 14px;opacity:.7} .row{display:flex;gap:8px;justify-content:flex-end}
      button{all:unset;cursor:pointer;padding:6px 12px;border-radius:8px;border:1px solid ${dark ? 'rgba(255,255,255,.18)' : 'rgba(0,0,0,.14)'}}
      .primary{background:${fg};color:${dark ? '#111' : '#fff'};border-color:transparent}
    </style><div class="card"><h1>Allow Dove on this site?</h1><p></p>
      <div class="row"><button class="deny">Not now</button><button class="primary allow">Allow</button></div></div>`;
    root.querySelector('p').textContent = `An agent in Dove wants to read and act on ${origin} using your session here.`;
    const done = (v) => { host.remove(); resolve(v); };
    root.querySelector('.allow').addEventListener('click', () => done(true));
    root.querySelector('.deny').addEventListener('click', () => done(false));
    setTimeout(() => done(false), 120000);
    document.documentElement.appendChild(host);
  });
}

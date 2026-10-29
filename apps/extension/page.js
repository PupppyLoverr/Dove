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
    // nodeType, not instanceof: frames have their own Element class.
    if (lines.length >= limit || !el || el.nodeType !== 1) return;
    if (['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'SVG'].includes(el.tagName.toUpperCase())) return;
    if (el.id === 'dove-control-indicator') return;
    if (!visible(el)) return;
    // Frames are containers, whatever their tabindex says.
    const r = el.tagName === 'IFRAME' ? null : role(el);
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
    if (el.tagName === 'IFRAME') {
      let doc = null;
      try { doc = el.contentDocument; } catch (_) { doc = null; }
      const pad = '  '.repeat(Math.min(depth, 8));
      if (doc?.body) { lines.push(`${pad}- frame "${el.title || el.name || el.src || ''}"`); for (const c of doc.body.children) walk(c, depth + 1); }
      else lines.push(`${pad}- frame (cross-origin, not readable) src=${(el.src || '').slice(0, 120)}`);
    }
  };
  walk(root, 0);
  document.documentElement.setAttribute('data-dove-next', String(next));
  const more = lines.length >= limit ? `\n… (truncated at ${limit} lines; pass full or a ref to narrow)` : '';
  return { title: document.title, url: location.href, outline: lines.join('\n') + more };
}

/** Scroll an element into view and return its viewport center. */
export function pageLocate(target) {
  // Search the document, open shadow roots and same-origin frames; add each
  // frame's offset so the click lands in the right place.
  const deep = (root, sel) => {
    let hit = null;
    try { hit = root.querySelector(sel); } catch (_) { return null; }
    if (hit) return { el: hit, dx: 0, dy: 0 };
    for (const host of root.querySelectorAll('*')) {
      if (host.shadowRoot) { const r = deep(host.shadowRoot, sel); if (r) return r; }
      if (host.tagName === 'IFRAME') {
        let doc = null; try { doc = host.contentDocument; } catch (_) { doc = null; }
        if (doc) { const r = deep(doc, sel); if (r) { const b = host.getBoundingClientRect(); return { el: r.el, dx: r.dx + b.left, dy: r.dy + b.top }; } }
      }
    }
    return null;
  };
  let found = null;
  if (/^e\d+$/.test(target)) {
    found = deep(document, `[data-dove-ref="${target}"]`);
    if (!found) return { error: `STALE_REF: ${target} is not on the page any more (it navigated or re-rendered). Take a new chrome_snapshot and use its refs.` };
  }
  if (!found) found = deep(document, target);
  if (!found) {
    const want = target.trim().toLowerCase();
    const all = document.querySelectorAll('a,button,input,textarea,select,[role],label,summary,[contenteditable]');
    const el = [...all].find((e) => (e.innerText || e.value || e.getAttribute('aria-label') || '').trim().toLowerCase() === want)
      || [...all].find((e) => (e.innerText || e.getAttribute('aria-label') || '').toLowerCase().includes(want));
    if (el) found = { el, dx: 0, dy: 0 };
  }
  if (!found) return { error: `Nothing matches "${target}". Take a new chrome_snapshot and use its refs.` };
  const { el, dx, dy } = found;
  el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
  const r = el.getBoundingClientRect();
  if (r.width === 0 && r.height === 0) return { error: `${target} is hidden (zero size). Take a new chrome_snapshot.` };
  return { x: dx + r.left + r.width / 2, y: dy + r.top + r.height / 2, tag: el.tagName.toLowerCase() };
}

/** What an element is, for safety checks: tag, type, visible text, form context. */
export function pageDescribe(target, enter) {
  const byRef = (root) => {
    let hit = null;
    try { hit = root.querySelector(`[data-dove-ref="${target}"]`) || root.querySelector(target); } catch (_) { hit = null; }
    if (hit) return hit;
    for (const host of root.querySelectorAll('*')) {
      if (host.shadowRoot) { const r = byRef(host.shadowRoot); if (r) return r; }
      if (host.tagName === 'IFRAME') {
        let d = null; try { d = host.contentDocument; } catch (_) { d = null; }
        if (d) { const r = byRef(d); if (r) return r; }
      }
    }
    return null;
  };
  const el = target ? byRef(document) : document.activeElement;
  if (!el) return null;
  const label = el.getAttribute('aria-label') || (el.labels && el.labels.length ? [...el.labels].map((l) => l.innerText.trim()).join(' ') : '');
  return {
    tag: el.tagName.toLowerCase(),
    type: (el.getAttribute('type') || '').toLowerCase(),
    text: (el.innerText || '').replace(/\s+/g, ' ').trim().slice(0, 120),
    value: el.tagName === 'INPUT' && ['submit', 'button'].includes((el.type || '').toLowerCase()) ? el.value : '',
    label,
    title: el.getAttribute('title') || '',
    name: el.getAttribute('name') || '',
    id: el.id || '',
    placeholder: el.getAttribute('placeholder') || '',
    autocomplete: el.getAttribute('autocomplete') || '',
    inForm: !!el.closest('form'),
    enter: !!enter,
    origin: location.origin,
  };
}

/** A compact picture of the page: what it is, its landmarks, forms and counts. */
export function pageContext() {
  const text = (e) => (e?.innerText || e?.textContent || '').replace(/\s+/g, ' ').trim();
  const headings = [...document.querySelectorAll('h1,h2,h3')].slice(0, 12).map((h) => `${h.tagName.toLowerCase()} ${text(h).slice(0, 90)}`);
  const forms = [...document.forms].slice(0, 6).map((f, i) => {
    const fields = [...f.elements].filter((e) => e.name || e.id).slice(0, 15)
      .map((e) => `${e.tagName.toLowerCase()}${e.type ? ':' + e.type : ''} ${e.name || e.id}${e.required ? '*' : ''}`);
    const label = f.getAttribute('aria-label');
    return `form ${i + 1}${label ? ` "${label}"` : ''}${f.action ? ' -> ' + f.action.slice(0, 100) : ''}: ${fields.join(', ')}`;
  });
  const dialogs = [...document.querySelectorAll('dialog[open],[role=dialog],[aria-modal=true]')]
    .filter((d) => d.getBoundingClientRect().height > 0).map((d) => text(d).slice(0, 120));
  return {
    title: document.title,
    url: location.href,
    lang: document.documentElement.lang || '',
    selection: String(getSelection() || '').slice(0, 500),
    headings,
    forms,
    dialogs,
    counts: {
      links: document.links.length,
      buttons: document.querySelectorAll('button,[role=button],input[type=submit]').length,
      inputs: document.querySelectorAll('input,textarea,select').length,
      iframes: document.querySelectorAll('iframe').length,
      tables: document.querySelectorAll('table').length,
    },
    scroll: { y: Math.round(scrollY), max: Math.round(document.documentElement.scrollHeight - innerHeight) },
  };
}

/** Elements whose name or text matches `query` (optionally of one role), with refs. */
export function pageFind(query, role, limit) {
  const ATTR = 'data-dove-ref';
  let next = Number(document.documentElement.getAttribute('data-dove-next') || '1');
  const want = String(query || '').trim().toLowerCase();
  const tags = { a: 'link', button: 'button', select: 'combobox', textarea: 'textbox', summary: 'button' };
  const roleOf = (el) => el.getAttribute('role') || tags[el.tagName.toLowerCase()]
    || (el.tagName === 'INPUT' ? (['button', 'submit', 'reset'].includes(el.type) ? 'button' : el.type === 'checkbox' ? 'checkbox' : el.type === 'radio' ? 'radio' : 'textbox') : null);
  const name = (el) => (el.getAttribute('aria-label') || (el.labels && el.labels[0]?.innerText) || el.placeholder || el.innerText || el.value || el.title || '').replace(/\s+/g, ' ').trim();
  const out = [];
  for (const el of document.querySelectorAll('a,button,input,textarea,select,summary,[role],[contenteditable=true],[tabindex]')) {
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    const n = name(el);
    const ro = roleOf(el) || 'element';
    if (role && ro !== role) continue;
    if (want && !n.toLowerCase().includes(want)) continue;
    let ref = el.getAttribute(ATTR);
    if (!ref) { ref = `e${next++}`; el.setAttribute(ATTR, ref); }
    out.push(`- ${ro} "${n.slice(0, 80)}" [ref=${ref}]`);
    if (out.length >= (limit || 20)) break;
  }
  document.documentElement.setAttribute('data-dove-next', String(next));
  return { text: out.length ? out.join('\n') : `Nothing visible matches "${query || role}".` };
}

/** Structured data: tables as rows, links, headings, form fields, or one selector's text. */
export function pageExtract(kind, selector, limit) {
  const max = Math.min(Number(limit) || 50, 500);
  const clean = (s) => String(s || '').replace(/\s+/g, ' ').trim();
  switch (kind) {
    case 'tables':
      return [...document.querySelectorAll('table')].slice(0, 10).map((t) => {
        const rows = [...t.rows].map((r) => [...r.cells].map((c) => clean(c.innerText).slice(0, 200)));
        const head = t.tHead ? rows.shift() : null;
        return { caption: clean(t.caption?.innerText), head, rows: rows.slice(0, max) };
      });
    case 'links':
      return [...document.links].map((a) => ({ text: clean(a.innerText).slice(0, 120), href: a.href })).filter((l) => l.text || l.href).slice(0, max);
    case 'headings':
      return [...document.querySelectorAll('h1,h2,h3,h4')].map((h) => ({ level: Number(h.tagName[1]), text: clean(h.innerText).slice(0, 200) })).slice(0, max);
    case 'forms':
      return [...document.forms].map((f) => ({
        action: f.action,
        method: (f.method || 'get').toLowerCase(),
        fields: [...f.elements].filter((e) => e.name || e.id).map((e) => ({
          name: e.name || e.id,
          tag: e.tagName.toLowerCase(),
          type: e.type || '',
          required: !!e.required,
          label: clean(e.labels?.[0]?.innerText),
          value: e.type === 'password' ? (e.value ? '•••' : '') : String(e.value || '').slice(0, 200),
        })),
      }));
    case 'selector': {
      if (!selector) return { error: 'extract kind "selector" needs a selector.' };
      let els;
      try { els = [...document.querySelectorAll(selector)]; } catch (e) { return { error: `Bad selector: ${e.message}` }; }
      return els.slice(0, max).map((e) => clean(e.innerText || e.textContent).slice(0, 1000));
    }
    default:
      return { error: `Unknown extract kind "${kind}". Use tables, links, headings, forms or selector.` };
  }
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

/** The "Keplorayx is controlling this tab" indicator: a hairline frame and a pill. */
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
  if (t) t.textContent = label || 'Keplorayx is controlling this tab';
  return true;
}

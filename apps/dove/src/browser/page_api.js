(() => {
  if (window.__dove && window.__dove.v === 4) return;
  const refs = new Map();
  const ids = new WeakMap();
  let next = 1;
  const logs = [];
  for (const level of ['log', 'info', 'warn', 'error']) {
    const orig = console[level];
    console[level] = function (...args) {
      try {
        logs.push({ level, text: args.map(a => typeof a === 'string' ? a : (() => { try { return JSON.stringify(a); } catch (e) { return String(a); } })()).join(' ').slice(0, 500) });
        if (logs.length > 200) logs.shift();
      } catch (e) {}
      return orig.apply(this, args);
    };
  }
  window.addEventListener('error', e => { logs.push({ level: 'error', text: String(e.message || e) }); if (logs.length > 200) logs.shift(); });

  const INTERACTIVE = 'a[href],button,input,textarea,select,summary,[contenteditable=""],[contenteditable="true"],[role=button],[role=link],[role=checkbox],[role=radio],[role=switch],[role=tab],[role=menuitem],[role=option],[role=combobox],[role=textbox],[role=slider],[role=searchbox],[tabindex]:not([tabindex="-1"]),[onclick]';
  const LANDMARK = { NAV: 'navigation', MAIN: 'main', HEADER: 'banner', FOOTER: 'contentinfo', ASIDE: 'complementary', FORM: 'form', DIALOG: 'dialog', TABLE: 'table', UL: 'list', OL: 'list' };
  const SKIP = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'SVG', 'HEAD', 'META', 'LINK']);

  function refOf(el) {
    let id = ids.get(el);
    if (!id) {
      id = 'e' + next++;
      ids.set(el, id);
      refs.set(id, new WeakRef(el));
    }
    return id;
  }
  function visible(el) {
    if (!(el instanceof Element)) return false;
    if (el.getAttribute('aria-hidden') === 'true') return false;
    const st = getComputedStyle(el);
    if (st.display === 'none' || st.visibility === 'hidden' || Number(st.opacity) === 0) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0 || st.display === 'contents';
  }
  function clean(s, n) {
    s = String(s || '').replace(/\s+/g, ' ').trim();
    return s.length > n ? s.slice(0, n - 1) + '…' : s;
  }
  function role(el) {
    const r = el.getAttribute('role');
    if (r) return r;
    const tag = el.tagName;
    if (tag === 'A') return 'link';
    if (tag === 'BUTTON' || tag === 'SUMMARY') return 'button';
    if (tag === 'SELECT') return 'combobox';
    if (tag === 'TEXTAREA') return 'textbox';
    if (tag === 'IMG') return 'img';
    if (/^H[1-6]$/.test(tag)) return 'heading';
    if (tag === 'INPUT') {
      const t = (el.type || 'text').toLowerCase();
      if (t === 'checkbox' || t === 'radio') return t;
      if (t === 'submit' || t === 'button' || t === 'reset' || t === 'image') return 'button';
      if (t === 'range') return 'slider';
      if (t === 'search') return 'searchbox';
      return 'textbox';
    }
    if (el.isContentEditable) return 'textbox';
    return 'generic';
  }
  function name(el) {
    const aria = el.getAttribute('aria-label');
    if (aria) return clean(aria, 80);
    const by = el.getAttribute('aria-labelledby');
    if (by) {
      const t = by.split(/\s+/).map(i => document.getElementById(i)).filter(Boolean).map(n => n.innerText).join(' ');
      if (t.trim()) return clean(t, 80);
    }
    if (el.id) {
      const l = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
      if (l && l.innerText.trim()) return clean(l.innerText, 80);
    }
    const wrap = el.closest('label');
    if (wrap && wrap !== el && wrap.innerText.trim()) return clean(wrap.innerText, 80);
    if (el.tagName === 'IMG') return clean(el.alt || el.title, 80);
    if (el.tagName === 'INPUT' && ['submit', 'button', 'reset'].includes((el.type || '').toLowerCase())) return clean(el.value, 80);
    const text = el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' ? '' : el.innerText;
    return clean(text || el.getAttribute('placeholder') || el.title || el.name || '', 80);
  }
  function describe(el) {
    const r = role(el);
    let line = r + (name(el) ? ' "' + name(el).replace(/"/g, "'") + '"' : '');
    const bits = ['ref=' + refOf(el)];
    if (r === 'heading') bits.unshift(el.tagName.toLowerCase());
    if (el.disabled) bits.push('disabled');
    if (el.checked) bits.push('checked');
    if (el.getAttribute('aria-expanded')) bits.push('expanded=' + el.getAttribute('aria-expanded'));
    if (el.getAttribute('aria-selected') === 'true') bits.push('selected');
    if (document.activeElement === el) bits.push('focused');
    line += ' [' + bits.join(', ') + ']';
    if (el.tagName === 'A' && el.getAttribute('href')) line += ' -> ' + clean(el.getAttribute('href'), 100);
    if ('value' in el && el.tagName !== 'BUTTON' && r !== 'button' && el.type !== 'checkbox' && el.type !== 'radio') {
      const v = el.type === 'password' ? (el.value ? '••••' : '') : el.value;
      line += ': "' + clean(v, 120) + '"';
      if (el.placeholder && !el.value) line += ' (placeholder "' + clean(el.placeholder, 60) + '")';
    }
    if (el.tagName === 'SELECT') {
      line += ' options=[' + [...el.options].slice(0, 20).map(o => clean(o.text, 30)).join(' | ') + ']';
    }
    return line;
  }

  function snapshot(opts) {
    opts = opts || {};
    const max = opts.max || 30000;
    const out = [];
    let size = 0;
    let truncated = false;
    const root = opts.ref ? resolve(opts.ref) : document.body;
    function push(depth, text) {
      if (truncated) return;
      const line = '  '.repeat(depth) + '- ' + text;
      size += line.length + 1;
      if (size > max) { truncated = true; return; }
      out.push(line);
    }
    function walk(el, depth) {
      if (truncated || !el || SKIP.has(el.tagName)) return;
      if (!visible(el)) return;
      if (el.matches(INTERACTIVE) || /^H[1-6]$/.test(el.tagName) || (el.tagName === 'IMG' && el.alt)) {
        push(depth, describe(el));
        if (el.tagName === 'A' || el.tagName === 'BUTTON' || el.tagName === 'SELECT' || el.tagName === 'TEXTAREA' || /^H[1-6]$/.test(el.tagName)) return;
      }
      let d = depth;
      const lm = LANDMARK[el.tagName] || (['navigation', 'main', 'dialog', 'form', 'list', 'table', 'tablist', 'menu', 'listbox', 'alert'].includes(el.getAttribute('role')) ? el.getAttribute('role') : null);
      if (lm && el !== document.body) {
        const label = el.getAttribute('aria-label');
        push(depth, lm + (label ? ' "' + clean(label, 60) + '"' : ''));
        d = depth + 1;
      }
      for (const child of el.childNodes) {
        if (child.nodeType === 3) {
          const t = clean(child.textContent, opts.full ? 400 : 160);
          if (t.length > 1 && !(child.parentElement && child.parentElement.matches(INTERACTIVE))) push(d, 'text: ' + t);
        } else if (child.nodeType === 1) {
          walk(child, d);
          if (child.shadowRoot) for (const c of child.shadowRoot.children) walk(c, d + 1);
        }
      }
    }
    walk(root, 0);
    const head = 'Page: ' + clean(document.title, 120) + ' — ' + location.href + '\nScroll: ' + Math.round(scrollY) + ' / ' + Math.max(0, document.documentElement.scrollHeight - innerHeight) + ' px, viewport ' + innerWidth + '×' + innerHeight;
    return head + '\n' + out.join('\n') + (truncated ? '\n… (truncated: scroll or pass ref to snapshot part of the page)' : '');
  }

  function resolve(target) {
    if (!target) return null;
    target = String(target).trim();
    const m = /^\[?(?:ref=)?(e\d+)\]?$/.exec(target);
    if (m) {
      const w = refs.get(m[1]);
      const el = w && w.deref();
      if (el && el.isConnected) return el;
      throw new Error('ref ' + m[1] + ' is stale; take a new browser_snapshot');
    }
    try { const el = document.querySelector(target); if (el) return el; } catch (e) {}
    const want = target.toLowerCase();
    let best = null;
    let loose = null;
    for (const el of document.querySelectorAll(INTERACTIVE + ',label,h1,h2,h3,h4,li,td,span,div,p')) {
      if (!visible(el)) continue;
      const keys = [name(el), el.getAttribute('placeholder'), el.getAttribute('aria-label'), el.title, el.id, el.getAttribute('name'), el.tagName === 'INPUT' ? el.value : '']
        .filter(Boolean).map(k => String(k).trim().toLowerCase());
      if (!keys.length) continue;
      const interactive = el.matches(INTERACTIVE);
      if (keys.includes(want)) {
        if (interactive) return el;
        // A label: act on its control.
        if (el.tagName === 'LABEL' && el.control) return el.control;
        if (!best) best = el;
      } else if (!loose && keys.some(k => k.includes(want)) && interactive) {
        loose = el;
      }
    }
    if (best || loose) return best || loose;
    throw new Error('No element matches "' + target + '"');
  }

  function point(target) {
    const el = resolve(target);
    el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    const r = el.getBoundingClientRect();
    const x = Math.round(r.left + r.width / 2), y = Math.round(r.top + Math.min(r.height / 2, 20));
    const top = document.elementFromPoint(x, y);
    const covered = top && top !== el && !el.contains(top) && !top.contains(el) ? describe(top) : null;
    return { x, y, label: describe(el), covered };
  }

  let cursorRoot = null;
  function cursor(x, y, kind) {
    if (!cursorRoot || !cursorRoot.isConnected) {
      const host = document.createElement('dove-agent-cursor');
      host.style.cssText = 'position:fixed;left:0;top:0;width:0;height:0;z-index:2147483647;pointer-events:none';
      const sh = host.attachShadow({ mode: 'open' });
      sh.innerHTML = '<style>' +
        '.c{position:fixed;left:0;top:0;transition:transform .32s cubic-bezier(.2,.8,.2,1);will-change:transform;filter:drop-shadow(0 2px 4px rgba(0,0,0,.35))}' +
        '.t{position:absolute;left:18px;top:16px;font:600 11px system-ui,sans-serif;color:#fff;background:#7c5cff;border-radius:6px;padding:2px 6px;white-space:nowrap}' +
        '.r{position:fixed;width:28px;height:28px;margin:-14px 0 0 -14px;border-radius:50%;border:2px solid #7c5cff;opacity:0;pointer-events:none}' +
        '.r.on{animation:p .45s ease-out}' +
        '@keyframes p{from{opacity:.9;transform:scale(.3)}to{opacity:0;transform:scale(1.6)}}' +
        '</style><div class="c"><svg width="22" height="22" viewBox="0 0 24 24"><path d="M4.5 3.75 19.5 10.125 13.125 12.375 10.35 19.125Z" fill="#7c5cff" stroke="#fff" stroke-width="1.75" stroke-linejoin="round"/></svg><div class="t">Agent</div></div><div class="r"></div>';
      (document.documentElement || document.body).appendChild(host);
      cursorRoot = sh;
    }
    const c = cursorRoot.querySelector('.c');
    c.style.transform = 'translate(' + (x - 4) + 'px,' + (y - 2) + 'px)';
    if (kind === 'click') {
      const r = cursorRoot.querySelector('.r');
      r.style.left = x + 'px';
      r.style.top = y + 'px';
      r.classList.remove('on');
      void r.offsetWidth;
      r.classList.add('on');
    }
    return true;
  }

  function synthClick(target, opts) {
    const el = resolve(target);
    const p = point(target);
    const o = { bubbles: true, cancelable: true, composed: true, clientX: p.x, clientY: p.y, button: 0, detail: opts && opts.double ? 2 : 1 };
    el.dispatchEvent(new PointerEvent('pointerdown', o));
    el.dispatchEvent(new MouseEvent('mousedown', o));
    if (el.focus) el.focus();
    el.dispatchEvent(new PointerEvent('pointerup', o));
    el.dispatchEvent(new MouseEvent('mouseup', o));
    el.click();
    if (opts && opts.double) el.dispatchEvent(new MouseEvent('dblclick', o));
    return true;
  }

  function setValue(target, text, clear) {
    const el = target ? resolve(target) : document.activeElement;
    if (!el) throw new Error('Nothing is focused to type into');
    el.focus();
    if (el.isContentEditable) {
      if (clear) el.textContent = '';
      document.execCommand('insertText', false, text);
      return true;
    }
    const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : el.tagName === 'SELECT' ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
    setter.call(el, (clear ? '' : el.value) + text);
    el.dispatchEvent(new InputEvent('input', { bubbles: true, data: text, inputType: 'insertText' }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return true;
  }

  function focusFor(target, clear) {
    const el = resolve(target);
    el.scrollIntoView({ block: 'center', behavior: 'instant' });
    el.focus();
    if (clear) {
      if (el.select) el.select();
      else if (el.isContentEditable) {
        const r = document.createRange();
        r.selectNodeContents(el);
        const s = getSelection();
        s.removeAllRanges();
        s.addRange(r);
      }
    }
    return true;
  }

  function press(key) {
    const el = document.activeElement || document.body;
    const o = { key, code: key.length === 1 ? 'Key' + key.toUpperCase() : key, bubbles: true, cancelable: true };
    const down = el.dispatchEvent(new KeyboardEvent('keydown', o));
    el.dispatchEvent(new KeyboardEvent('keyup', o));
    if (down && key === 'Enter' && el.form) el.form.requestSubmit ? el.form.requestSubmit() : el.form.submit();
    return true;
  }

  function select(target, values) {
    const el = resolve(target);
    if (el.tagName !== 'SELECT') throw new Error('Not a <select>: ' + describe(el));
    const want = values.map(v => String(v).toLowerCase());
    let hit = 0;
    for (const o of el.options) {
      const on = want.includes(o.value.toLowerCase()) || want.includes(o.text.trim().toLowerCase());
      if (on) hit++;
      o.selected = on;
    }
    if (!hit) throw new Error('No option matches ' + JSON.stringify(values));
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return describe(el);
  }

  function text(max) {
    const t = (document.body ? document.body.innerText : '').replace(/\n{3,}/g, '\n\n').trim();
    return 'Page: ' + document.title + ' — ' + location.href + '\n\n' + (t.length > max ? t.slice(0, max) + '\n… (truncated)' : t);
  }

  function find(q) {
    q = String(q).toLowerCase();
    return document.body && document.body.innerText.toLowerCase().includes(q);
  }

  window.__dove = { v: 4, snapshot, resolve, point, cursor, synthClick, setValue, focusFor, press, select, text, find, describe, logs: () => logs.splice(0) , state: () => ({ url: location.href, title: document.title, ready: document.readyState }) };
})();

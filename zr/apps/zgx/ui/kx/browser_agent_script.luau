--!strict
-- What runs in a page while agents work in it (ui/kx/browser_agent runs
-- it there): `window.__kxAgent`.
--
--   sync(list)   the agents on the page now, each `{ key, n }`: a cursor
--                each ("Agent n", in its own colour), the page's blue
--                shimmer while there's any, gone with the last
--   op(q)        one step `{ id, op, key, n, args }`: read, click, type,
--                key, scroll, wait. Its cursor moves there first. Its
--                answer is kept as JSON text until
--   take(id)     takes it (null while the step is still going; "__lost"
--                for a step begun on the page before this one)
--
-- The shimmer is a canvas over the page that takes no presses: columns of
-- blue glyphs falling on a grid, a band of light crossing them, and a lit
-- edge. A pill at the top says agents are using the page and has Stop:
-- pressed, every step is refused for a minute.
--
-- Built with createElement alone (no innerHTML: pages with Trusted Types
-- refuse it), under one root marked `data-kx-agent`, which reading skips.

return [=[
(function () {
  if (window.__kxAgent) { return; }
  var A = window.__kxAgent = { results: {}, begun: {}, cursors: {}, refs: [], stoppedUntil: 0 };
  var SVG = 'http://www.w3.org/2000/svg';
  var COLORS = ['#4da3ff', '#ff9a4d', '#5fd38d', '#c58bff', '#ffd24d', '#ff6b9a', '#4de0e0', '#b9c2d6'];
  var GLYPHS = '01ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜ0110';
  var CELL = 18;
  var root = null, layer = null, canvas = null, ctx = null, edge = null, pill = null, pillText = null;
  var cols = [], raf = 0, last = 0, t0 = performance.now(), lit = false, lightPage = true;

  // Whether the page is a light one: the glyphs are a deeper blue on it,
  // a pale blue on a dark page, so the shimmer reads on both.
  function isLight() {
    try {
      var c = getComputedStyle(document.body || document.documentElement).backgroundColor;
      var m = c.match(/[\d.]+/g);
      if (!m || m.length < 3 || (m.length > 3 && Number(m[3]) === 0)) { return true; }
      return 0.299 * m[0] + 0.587 * m[1] + 0.114 * m[2] > 140;
    } catch (x) { return true; }
  }

  function el(tag, css) {
    var e = document.createElement(tag);
    if (css) { e.style.cssText = css; }
    return e;
  }

  function resize() {
    if (!canvas) { return; }
    var w = Math.max(1, window.innerWidth), h = Math.max(1, window.innerHeight);
    canvas.width = w; canvas.height = h;
    ctx = canvas.getContext('2d');
    lightPage = isLight();
    cols = [];
    for (var x = 0; x < w; x += CELL) {
      cols.push({ y: Math.random() * h, v: 50 + Math.random() * 110, len: 5 + Math.floor(Math.random() * 13), seed: Math.floor(Math.random() * 9973) });
    }
  }

  function mount() {
    if (root && root.isConnected) { return; }
    root = el('div', 'all:initial;display:block;position:fixed;left:0;top:0;right:0;bottom:0;z-index:2147483646;pointer-events:none;overflow:hidden;');
    root.setAttribute('data-kx-agent', '');
    root.setAttribute('aria-hidden', 'true');
    canvas = el('canvas', 'position:absolute;left:0;top:0;width:100%;height:100%;opacity:0;transition:opacity .4s ease;');
    edge = el('div', 'position:absolute;left:0;top:0;right:0;bottom:0;opacity:0;transition:opacity .4s ease;box-shadow:inset 0 0 0 2px rgba(77,163,255,.6),inset 0 0 46px rgba(40,120,255,.3);');
    pill = el('div', 'position:absolute;top:10px;left:50%;transform:translateX(-50%);display:flex;align-items:center;gap:8px;padding:5px 6px 5px 12px;border-radius:999px;background:rgba(8,16,32,.82);border:1px solid rgba(120,180,255,.45);box-shadow:0 6px 24px rgba(0,20,60,.45);color:#dcebff;font:500 12px/16px system-ui,-apple-system,"Segoe UI",sans-serif;opacity:0;transition:opacity .4s ease;pointer-events:none;white-space:nowrap;');
    var dot = el('div', 'width:7px;height:7px;border-radius:50%;background:#4da3ff;box-shadow:0 0 8px #4da3ff;');
    pillText = el('span', '');
    var stop = el('button', 'all:unset;cursor:pointer;padding:2px 10px;border-radius:999px;background:rgba(120,180,255,.18);color:#fff;font:600 12px/16px system-ui,-apple-system,"Segoe UI",sans-serif;');
    stop.textContent = 'Stop';
    stop.addEventListener('click', function (e) {
      e.preventDefault(); e.stopPropagation();
      A.stoppedUntil = Date.now() + 60000;
      A.sync([]);
    }, true);
    pill.appendChild(dot); pill.appendChild(pillText); pill.appendChild(stop);
    // (In a shadow tree of its own: the page's styles stop at its edge.)
    var inside = root.attachShadow ? root.attachShadow({ mode: 'closed' }) : root;
    layer = el('div', 'position:absolute;left:0;top:0;right:0;bottom:0;overflow:hidden;pointer-events:none;');
    inside.appendChild(layer);
    layer.appendChild(canvas); layer.appendChild(edge); layer.appendChild(pill);
    (document.documentElement || document.body).appendChild(root);
    resize();
  }

  function glyph(col, row, tick) {
    var slow = Math.floor(tick / (5 + (row % 7)));
    return GLYPHS.charAt((cols[col].seed + row * 131 + slow * 17) % GLYPHS.length);
  }

  function frame(now) {
    raf = requestAnimationFrame(frame);
    if (now - last < 33 || !ctx) { return; }
    var dt = Math.min(0.1, (now - last) / 1000);
    last = now;
    var w = canvas.width, h = canvas.height;
    ctx.clearRect(0, 0, w, h);
    ctx.font = '13px Consolas,"Cascadia Mono",Menlo,ui-monospace,monospace';
    ctx.textBaseline = 'top';
    var tick = Math.floor(now / 80);
    // (The band of light: it crosses the page corner to corner, again and again.)
    var span = w + h + 520;
    var band = (((now - t0) / 2800) % 1) * span - 260;
    for (var c = 0; c < cols.length; c++) {
      var col = cols[c];
      col.y += col.v * dt;
      if (col.y - col.len * CELL > h) {
        col.y = -Math.random() * h * 0.6;
        col.v = 50 + Math.random() * 110;
        col.len = 5 + Math.floor(Math.random() * 13);
      }
      var x = c * CELL + 4;
      var head = Math.floor(col.y / CELL);
      for (var i = 0; i < col.len; i++) {
        var row = head - i;
        var y = row * CELL;
        if (y < -CELL || y > h) { continue; }
        var near = 1 - Math.min(1, Math.abs((x + y) - band) / 300);
        var a = (1 - i / col.len) * (0.2 + 0.6 * near);
        if (i === 0) { a = Math.min(0.98, a + 0.3); }
        if (lightPage) { a = Math.min(0.98, a * 1.35); }
        ctx.fillStyle = (i === 0 ? (lightPage ? 'rgba(8,62,190,' : 'rgba(200,228,255,') : (lightPage ? 'rgba(24,104,236,' : 'rgba(64,148,255,')) + a.toFixed(3) + ')';
        ctx.fillText(glyph(c, row, tick), x, y);
      }
    }
    ctx.fillStyle = 'rgba(30,90,200,0.05)';
    ctx.fillRect(0, 0, w, h);
    var g = ctx.createLinearGradient(band / 2 - 150, band / 2 - 150, band / 2 + 150, band / 2 + 150);
    g.addColorStop(0, 'rgba(70,150,255,0)');
    g.addColorStop(0.5, 'rgba(120,190,255,0.2)');
    g.addColorStop(1, 'rgba(70,150,255,0)');
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, w, h);
  }

  function light(on) {
    if (on === lit && (!on || (root && root.isConnected))) { return; }
    lit = on;
    if (on) {
      mount();
      canvas.style.opacity = '1'; edge.style.opacity = '1'; pill.style.opacity = '1';
      pill.style.pointerEvents = 'auto';
      if (!raf) { last = 0; raf = requestAnimationFrame(frame); }
    } else if (root) {
      canvas.style.opacity = '0'; edge.style.opacity = '0'; pill.style.opacity = '0';
      pill.style.pointerEvents = 'none';
      setTimeout(function () {
        if (!lit && raf) { cancelAnimationFrame(raf); raf = 0; if (ctx) { ctx.clearRect(0, 0, canvas.width, canvas.height); } }
      }, 450);
    }
  }

  function place(c) {
    c.node.style.transform = 'translate3d(' + Math.round(c.x) + 'px,' + Math.round(c.y) + 'px,0)';
  }

  function cursor(key, n) {
    var c = A.cursors[key];
    if (c && c.node.isConnected) { c.at = Date.now(); return c; }
    mount();
    var color = COLORS[(n - 1) % COLORS.length];
    var node = el('div', 'position:absolute;left:0;top:0;opacity:0;transition:transform .46s cubic-bezier(.22,.8,.24,1),opacity .3s ease;will-change:transform;');
    var svg = document.createElementNS(SVG, 'svg');
    svg.setAttribute('width', '22'); svg.setAttribute('height', '22'); svg.setAttribute('viewBox', '0 0 22 22');
    svg.style.cssText = 'display:block;filter:drop-shadow(0 2px 4px rgba(0,0,0,.45));';
    var path = document.createElementNS(SVG, 'path');
    path.setAttribute('d', 'M3 2 L3 18 L7.6 14 L10.6 20.4 L13.3 19.2 L10.4 12.9 L16.6 12.9 Z');
    path.setAttribute('fill', color); path.setAttribute('stroke', '#ffffff'); path.setAttribute('stroke-width', '1.3'); path.setAttribute('stroke-linejoin', 'round');
    svg.appendChild(path);
    var label = el('div', 'position:absolute;left:15px;top:19px;padding:2px 8px;border-radius:10px;background:' + color + ';color:#06121f;font:600 11px/15px system-ui,-apple-system,"Segoe UI",sans-serif;white-space:nowrap;box-shadow:0 2px 8px rgba(0,0,0,.35);');
    label.textContent = 'Agent ' + n;
    node.appendChild(svg); node.appendChild(label);
    layer.appendChild(node);
    c = { node: node, n: n, color: color, at: Date.now(),
      x: (c && c.x) || Math.max(40, window.innerWidth - 120), y: (c && c.y) || Math.min(window.innerHeight - 60, 90 + n * 44) };
    A.cursors[key] = c;
    place(c);
    requestAnimationFrame(function () { node.style.opacity = '1'; });
    return c;
  }

  function moveTo(c, x, y) {
    c.x = Math.max(2, Math.min(window.innerWidth - 8, x));
    c.y = Math.max(2, Math.min(window.innerHeight - 8, y));
    c.at = Date.now();
    place(c);
  }

  function ripple(c) {
    if (!layer) { return; }
    var r = el('div', 'position:absolute;left:' + (c.x - 14) + 'px;top:' + (c.y - 14) + 'px;width:28px;height:28px;box-sizing:border-box;border-radius:50%;border:2px solid ' + c.color + ';background:' + c.color + '33;opacity:.95;transform:scale(.3);transition:transform .5s ease-out,opacity .5s ease-out;');
    layer.appendChild(r);
    requestAnimationFrame(function () { r.style.transform = 'scale(1.6)'; r.style.opacity = '0'; });
    setTimeout(function () { r.remove(); }, 560);
  }

  // The agents on the page now. Others' cursors leave; with none, the
  // shimmer goes. Says "stopped" while the person's Stop holds.
  A.sync = function (list) {
    if (Date.now() < A.stoppedUntil) { list = []; }
    var keep = {};
    for (var i = 0; i < list.length; i++) { keep[list[i].key] = true; cursor(list[i].key, list[i].n); }
    Object.keys(A.cursors).forEach(function (key) {
      if (!keep[key]) {
        var c = A.cursors[key];
        delete A.cursors[key];
        c.node.style.opacity = '0';
        setTimeout(function () { c.node.remove(); }, 320);
      }
    });
    var count = Object.keys(A.cursors).length;
    if (pillText) { pillText.textContent = count === 1 ? 'An agent is using this page' : count + ' agents are using this page'; }
    light(count > 0);
    return Date.now() < A.stoppedUntil ? 'stopped' : 'ok';
  };

  // ── Reading ──────────────────────────────────────────────────────────
  var CONTROLS = 'a[href],button,input:not([type=hidden]),textarea,select,summary,[role=button],[role=link],[role=tab],[role=menuitem],[role=checkbox],[role=radio],[role=option],[role=switch],[role=combobox],[role=textbox],[role=searchbox],[contenteditable=""],[contenteditable=true],[onclick],[tabindex]:not([tabindex="-1"])';

  function box(e) {
    var r = e.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) { return null; }
    var s = getComputedStyle(e);
    if (s.visibility === 'hidden' || s.display === 'none') { return null; }
    return r;
  }
  function inView(r) {
    return r.bottom > 0 && r.top < window.innerHeight && r.right > 0 && r.left < window.innerWidth;
  }
  function squeeze(t, most) {
    return String(t || '').replace(/\s+/g, ' ').trim().slice(0, most);
  }
  function isField(e) {
    var t = e.tagName;
    return t === 'INPUT' || t === 'TEXTAREA' || t === 'SELECT';
  }
  function nameOf(e) {
    var t = e.getAttribute('aria-label') || e.getAttribute('alt') || e.getAttribute('title') || '';
    if (!t && isField(e)) {
      if (e.id) {
        try { var l = document.querySelector('label[for="' + CSS.escape(e.id) + '"]'); if (l) { t = l.innerText; } } catch (x) {}
      }
      if (!t) { var wrap = e.closest('label'); if (wrap) { t = wrap.innerText; } }
      // (A tick box or a choice with no label of its own: the words beside
      // it say what it's for, never its value, which is "on" for them all.)
      var type = (e.getAttribute('type') || '').toLowerCase();
      if (!t && (type === 'checkbox' || type === 'radio')) {
        var next = e.nextElementSibling;
        if (next) { t = next.innerText || next.textContent || ''; }
        if (!squeeze(t, 90) && e.parentElement) { t = e.parentElement.innerText || ''; }
        return squeeze(t, 90) || squeeze(e.getAttribute('name') || type, 90);
      }
      if (!t) { t = e.getAttribute('placeholder') || e.getAttribute('name') || ''; }
    }
    if (!t) { t = e.innerText || e.value || ''; }
    return squeeze(t, 90);
  }
  function kindOf(e) {
    var role = e.getAttribute('role');
    if (role) { return role; }
    var t = e.tagName;
    if (t === 'A') { return 'link'; }
    if (t === 'BUTTON' || t === 'SUMMARY') { return 'button'; }
    if (t === 'INPUT') { return (e.getAttribute('type') || 'text').toLowerCase(); }
    if (t === 'TEXTAREA') { return 'textarea'; }
    if (t === 'SELECT') { return 'select'; }
    if (e.isContentEditable) { return 'editor'; }
    return 'control';
  }
  function describe(e) {
    return squeeze(kindOf(e) + ' "' + nameOf(e) + '"', 120);
  }
  function scrollOf() {
    var d = document.documentElement;
    return { y: Math.round(window.scrollY), height: Math.round(Math.max(d.scrollHeight, document.body ? document.body.scrollHeight : 0)), view: window.innerHeight };
  }

  function read(all) {
    A.refs = [];
    var controls = [], most = all ? 320 : 160, cut = false;
    var nodes = document.querySelectorAll(CONTROLS);
    for (var i = 0; i < nodes.length; i++) {
      var e = nodes[i];
      if (e.disabled || e.closest('[data-kx-agent]')) { continue; }
      var r = box(e);
      if (!r || (!all && !inView(r))) { continue; }
      if (controls.length >= most) { cut = true; break; }
      A.refs.push(e);
      var item = { ref: A.refs.length, kind: kindOf(e), name: nameOf(e) };
      if (isField(e)) {
        var type = (e.getAttribute('type') || '').toLowerCase();
        if (type === 'checkbox' || type === 'radio') { item.checked = !!e.checked; }
        else { item.value = type === 'password' ? (e.value ? '(hidden)' : '') : squeeze(e.value, 120); }
      }
      if (e.tagName === 'A') { item.href = String(e.href || '').slice(0, 160); }
      controls.push(item);
    }
    var parts = [], len = 0, room = all ? 24000 : 6000;
    if (document.body) {
      var walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
      var n;
      while ((n = walker.nextNode())) {
        var s = squeeze(n.nodeValue, 2000);
        if (s.length < 2) { continue; }
        var p = n.parentElement;
        if (!p) { continue; }
        var tag = p.tagName;
        if (tag === 'SCRIPT' || tag === 'STYLE' || tag === 'NOSCRIPT' || tag === 'TEMPLATE' || p.closest('[data-kx-agent]')) { continue; }
        var pr = box(p);
        if (!pr || (!all && !inView(pr))) { continue; }
        parts.push(s);
        len += s.length;
        if (len > room) { cut = true; break; }
      }
    }
    return { url: location.href, title: document.title, scroll: scrollOf(), text: parts.join('\n'), controls: controls, cut: cut || undefined };
  }

  // ── Acting ───────────────────────────────────────────────────────────
  function byRef(ref) {
    var e = A.refs[Number(ref) - 1];
    if (!e || !e.isConnected) { throw new Error('That ref is gone (the page changed): call browser_read again.'); }
    return e;
  }
  function byText(text) {
    var want = String(text).toLowerCase(), nodes = document.querySelectorAll(CONTROLS), best = null, bestRank = 99;
    for (var i = 0; i < nodes.length; i++) {
      var e = nodes[i];
      if (e.disabled || e.closest('[data-kx-agent]')) { continue; }
      var name = nameOf(e).toLowerCase();
      if (name.indexOf(want) < 0) { continue; }
      var r = box(e);
      if (!r) { continue; }
      var said = String(e.innerText || e.value || '').toLowerCase().indexOf(want) >= 0;
      var rank = (name === want ? 0 : 4) + (said ? 0 : 2) + (inView(r) ? 0 : 1);
      if (rank < bestRank) { best = e; bestRank = rank; }
    }
    if (!best) { throw new Error('Nothing to click says "' + text + '": call browser_read for what is there.'); }
    return best;
  }
  function centre(e) {
    var r = e.getBoundingClientRect();
    if (!inView(r) || r.top < 0 || r.bottom > window.innerHeight) {
      e.scrollIntoView({ block: 'center', inline: 'center' });
      r = e.getBoundingClientRect();
    }
    return { x: Math.max(1, Math.min(window.innerWidth - 2, r.left + r.width / 2)), y: Math.max(1, Math.min(window.innerHeight - 2, r.top + r.height / 2)) };
  }
  function fire(e, type, at) {
    var init = { bubbles: true, cancelable: true, composed: true, view: window, clientX: at.x, clientY: at.y, button: 0, buttons: type.indexOf('down') > 0 ? 1 : 0, pointerId: 1, pointerType: 'mouse', isPrimary: true };
    var ev = (type.indexOf('pointer') === 0 && window.PointerEvent) ? new PointerEvent(type, init) : new MouseEvent(type, init);
    return e.dispatchEvent(ev);
  }
  function setValue(e, value) {
    var proto = e.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : (e.tagName === 'SELECT' ? HTMLSelectElement.prototype : HTMLInputElement.prototype);
    var d = Object.getOwnPropertyDescriptor(proto, 'value');
    if (d && d.set) { d.set.call(e, value); } else { e.value = value; }
  }
  var KEYCODES = { Enter: 13, Escape: 27, Tab: 9, Backspace: 8, Delete: 46, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40, PageUp: 33, PageDown: 34, Home: 36, End: 35, ' ': 32 };
  function press(target, key) {
    var code = KEYCODES[key] || (key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0);
    var init = { key: key, code: key.length === 1 ? 'Key' + key.toUpperCase() : key, keyCode: code, which: code, bubbles: true, cancelable: true, composed: true, view: window };
    var went = target.dispatchEvent(new KeyboardEvent('keydown', init));
    if (key.length === 1 || key === 'Enter') { target.dispatchEvent(new KeyboardEvent('keypress', init)); }
    target.dispatchEvent(new KeyboardEvent('keyup', init));
    return went;
  }
  function submit(e) {
    var form = e.form || (e.closest && e.closest('form'));
    if (form) {
      if (form.requestSubmit) { form.requestSubmit(); } else { form.submit(); }
      return true;
    }
    return false;
  }
  function scroller() {
    var d = document.scrollingElement || document.documentElement;
    if (d.scrollHeight > d.clientHeight + 4) { return window; }
    var e = document.elementFromPoint(window.innerWidth / 2, window.innerHeight / 2);
    while (e && e !== document.body && e !== document.documentElement) {
      var s = getComputedStyle(e);
      if ((s.overflowY === 'auto' || s.overflowY === 'scroll') && e.scrollHeight > e.clientHeight + 4) { return e; }
      e = e.parentElement;
    }
    return window;
  }

  A.op = function (q) {
    var done = function (v) { A.results[q.id] = JSON.stringify(v); };
    A.begun[q.id] = true;
    try {
      if (Date.now() < A.stoppedUntil) {
        done({ error: 'The person pressed Stop on the page: agents are kept off it for a minute. Ask them before trying again.' });
        return;
      }
      var a = q.args || {};
      var c = cursor(q.key, q.n);
      A.sync(Object.keys(A.cursors).map(function (key) { return { key: key, n: A.cursors[key].n }; }));
      if (q.op === 'read') {
        moveTo(c, c.x + (Math.random() * 30 - 15), c.y + (Math.random() * 30 - 15));
        done(read(a.all === true));
        return;
      }
      if (q.op === 'click') {
        var e = a.ref != null ? byRef(a.ref) : (a.text ? byText(a.text) : null);
        var at;
        if (e) { at = centre(e); }
        else if (typeof a.x === 'number' && typeof a.y === 'number') {
          at = { x: a.x, y: a.y };
          e = document.elementFromPoint(a.x, a.y);
          if (!e) { throw new Error('Nothing is at that point of the view (' + window.innerWidth + ' by ' + window.innerHeight + ').'); }
        } else { throw new Error('Say what to click: a ref, text, or x and y.'); }
        moveTo(c, at.x, at.y);
        setTimeout(function () {
          try {
            ripple(c);
            fire(e, 'pointerover', at); fire(e, 'mouseover', at); fire(e, 'pointermove', at); fire(e, 'mousemove', at);
            fire(e, 'pointerdown', at); fire(e, 'mousedown', at);
            if (e.focus) { try { e.focus({ preventScroll: true }); } catch (x) {} }
            fire(e, 'pointerup', at); fire(e, 'mouseup', at);
            e.click();
            var said = describe(e);
            setTimeout(function () { done({ clicked: said, url: location.href, title: document.title }); }, 140);
          } catch (x) { done({ error: String((x && x.message) || x) }); }
        }, 500);
        return;
      }
      if (q.op === 'type') {
        var f = a.ref != null ? byRef(a.ref) : document.activeElement;
        if (!f || f === document.body || f === document.documentElement) { throw new Error('No field has the keyboard: give the field\'s ref.'); }
        var where = centre(f);
        moveTo(c, where.x, where.y);
        setTimeout(function () {
          try {
            ripple(c);
            if (f.focus) { f.focus({ preventScroll: true }); }
            var text = String(a.text == null ? '' : a.text);
            if (f.tagName === 'SELECT') {
              var want = text.toLowerCase(), hit = null;
              for (var i = 0; i < f.options.length; i++) {
                var o = f.options[i];
                if (o.value.toLowerCase() === want || squeeze(o.text, 200).toLowerCase() === want) { hit = o; break; }
                if (!hit && squeeze(o.text, 200).toLowerCase().indexOf(want) >= 0) { hit = o; }
              }
              if (!hit) { throw new Error('That list has no choice "' + text + '".'); }
              setValue(f, hit.value);
              f.dispatchEvent(new Event('input', { bubbles: true }));
              f.dispatchEvent(new Event('change', { bubbles: true }));
            } else if (isField(f)) {
              setValue(f, a.append === true ? String(f.value || '') + text : text);
              f.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: text }));
              f.dispatchEvent(new Event('change', { bubbles: true }));
            } else if (f.isContentEditable) {
              if (a.append !== true) { document.execCommand('selectAll', false, null); }
              document.execCommand('insertText', false, text);
            } else { throw new Error('That isn\'t something to type into: ' + describe(f) + '.'); }
            var sent = false;
            if (a.submit === true) {
              if (press(f, 'Enter')) { sent = submit(f); } else { sent = true; }
            }
            var said = describe(f);
            setTimeout(function () { done({ typed: said, submitted: sent || undefined, url: location.href }); }, 140);
          } catch (x) { done({ error: String((x && x.message) || x) }); }
        }, 420);
        return;
      }
      if (q.op === 'key') {
        var key = String(a.key || '');
        if (key === 'Space') { key = ' '; }
        if (!key) { throw new Error('Say which key.'); }
        var t = document.activeElement && document.activeElement !== document.documentElement ? document.activeElement : document.body;
        var went = press(t, key);
        var did = 'pressed';
        if (went) {
          var onPage = t === document.body;
          if (key === 'Enter') {
            if (t.tagName === 'A' || t.tagName === 'BUTTON' || t.getAttribute('role') === 'button') { t.click(); did = 'clicked ' + describe(t); }
            else if (isField(t) && t.tagName !== 'TEXTAREA' && submit(t)) { did = 'submitted'; }
          } else if (key === 'Backspace' && isField(t) && t.tagName !== 'SELECT') {
            setValue(t, String(t.value || '').slice(0, -1));
            t.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'deleteContentBackward' }));
          } else if (key.length === 1 && isField(t) && t.tagName !== 'SELECT') {
            setValue(t, String(t.value || '') + key);
            t.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: key }));
          } else if (onPage || !isField(t)) {
            var s = scroller(), page = window.innerHeight * 0.85;
            var by = { ArrowDown: 80, ArrowUp: -80, PageDown: page, PageUp: -page, ' ': page }[key];
            if (by) { s.scrollBy({ top: by, behavior: 'smooth' }); did = 'scrolled'; }
            else if (key === 'Home') { s.scrollTo({ top: 0, behavior: 'smooth' }); did = 'scrolled'; }
            else if (key === 'End') { s.scrollTo({ top: 1e9, behavior: 'smooth' }); did = 'scrolled'; }
          }
        }
        setTimeout(function () { done({ key: key, did: did, url: location.href }); }, 160);
        return;
      }
      if (q.op === 'scroll') {
        moveTo(c, window.innerWidth / 2 + (c.n - 1) * 26, window.innerHeight / 2);
        var sc = scroller();
        if (a.ref != null) { byRef(a.ref).scrollIntoView({ block: 'center', behavior: 'smooth' }); }
        else if (a.to === 'top') { sc.scrollTo({ top: 0, behavior: 'smooth' }); }
        else if (a.to === 'bottom') { sc.scrollTo({ top: 1e9, behavior: 'smooth' }); }
        else { sc.scrollBy({ top: typeof a.dy === 'number' ? a.dy : window.innerHeight * 0.85, behavior: 'smooth' }); }
        setTimeout(function () { done({ scroll: scrollOf() }); }, 520);
        return;
      }
      if (q.op === 'wait') {
        var ms = Math.max(0, Math.min(10000, typeof a.ms === 'number' ? a.ms : (a.text ? 8000 : 1000)));
        if (!a.text) { setTimeout(function () { done({ waited: ms }); }, ms); return; }
        var wantText = String(a.text).toLowerCase(), until = Date.now() + ms;
        var look = function () {
          c.at = Date.now();
          var has = document.body && document.body.innerText.toLowerCase().indexOf(wantText) >= 0;
          if (has || Date.now() >= until) { done({ found: !!has, url: location.href }); return; }
          setTimeout(look, 200);
        };
        look();
        return;
      }
      throw new Error('The page has no step "' + q.op + '".');
    } catch (x) {
      done({ error: String((x && x.message) || x) });
    }
  };

  // A step's answer, taken: null while it's still going; "__lost" for a
  // step this page never began (it was begun on the page before this one).
  A.take = function (id) {
    if (!A.begun[id]) { return '__lost'; }
    var v = A.results[id];
    if (v === undefined) { return null; }
    delete A.results[id];
    delete A.begun[id];
    return v;
  };

  window.addEventListener('resize', resize);
})();
]=]

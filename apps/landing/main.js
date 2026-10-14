/* Dove landing: no dependencies. Every effect degrades to static content
   without JS, WebGL or motion. */
(function () {
  "use strict";
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  var $ = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };

  /* ---------------------------------------------------- split letters */
  $$(".split").forEach(function (el) {
    var text = el.textContent; el.textContent = ""; el.setAttribute("aria-hidden", "true");
    var i = 0;
    text.split("").forEach(function (c) {
      var s = document.createElement("span");
      if (c === " ") { s.className = "sp"; } else { s.className = "ch"; s.textContent = c; s.style.setProperty("--i", i++); }
      el.appendChild(s);
    });
  });
  requestAnimationFrame(function () { $$(".hero-title .split").forEach(function (e) { e.classList.add("is-in"); }); });

  /* ------------------------------------------------------------ reveal */
  var io = new IntersectionObserver(function (entries) {
    entries.forEach(function (e) {
      if (e.isIntersecting) { e.target.classList.add("is-in"); io.unobserve(e.target); }
    });
  }, { rootMargin: "0px 0px -10% 0px", threshold: 0.12 });
  $$(".reveal, .viz-bars, .cta-title .split").forEach(function (el) { io.observe(el); });

  /* ------------------------------------------------- download for my OS */
  var os = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent) ? "macOS" : /Linux/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent) ? "Linux" : "Windows";
  var dl = $("#dl-primary span"); if (dl) dl.textContent = "Download for " + os;

  /* ------------------------------------------------------- nav + progress */
  var nav = $(".nav"), bar = $("#progress");
  function onScroll() {
    var y = window.scrollY, h = document.documentElement.scrollHeight - innerHeight;
    nav.classList.toggle("is-scrolled", y > 30);
    if (bar) bar.style.transform = "scaleX(" + (h > 0 ? y / h : 0) + ")";
    scrub(); tour(); stage(y);
  }

  /* ------------------------------------------------ scroll-scrubbed text */
  var scrubEl = $("#scrub"), words = [];
  if (scrubEl) {
    var t = scrubEl.textContent.trim().split(/\s+/); scrubEl.textContent = "";
    t.forEach(function (w, i) {
      var s = document.createElement("span"); s.className = "w"; s.textContent = w;
      scrubEl.appendChild(s); if (i < t.length - 1) scrubEl.appendChild(document.createTextNode(" "));
      words.push(s);
    });
  }
  function scrub() {
    if (!scrubEl) return;
    var r = scrubEl.getBoundingClientRect();
    var p = Math.min(1, Math.max(0, (innerHeight * 0.85 - r.top) / (r.height + innerHeight * 0.35)));
    var n = Math.round(p * words.length);
    for (var i = 0; i < words.length; i++) words[i].classList.toggle("on", i < n || reduce);
  }

  /* -------------------------------------------------------- pinned tour */
  var steps = $$(".step"), shots = $$(".shot"), rail = $("#tour-rail"), current = -1;
  function tour() {
    if (!steps.length) return;
    var mid = innerHeight * 0.5, best = 0, bestD = Infinity;
    steps.forEach(function (s, i) {
      var r = s.getBoundingClientRect(), d = Math.abs(r.top + r.height / 2 - mid);
      if (d < bestD) { bestD = d; best = i; }
    });
    if (best !== current) {
      current = best;
      steps.forEach(function (s, i) { s.classList.toggle("is-active", i === best); });
      var k = +steps[best].dataset.shot;
      shots.forEach(function (s, i) { s.classList.toggle("is-active", i === k); });
      if (rail) rail.style.transform = "scaleY(" + (best + 1) / steps.length + ")";
    }
  }

  /* ------------------------------------------------ hero window: 3D tilt */
  var win = $("#window"), stageEl = $("#stage"), baseRx = 12, scrollRx = 12;
  function stage(y) {
    if (!win || reduce) return;
    scrollRx = Math.max(0, baseRx - y / 30);
    win.style.setProperty("--rx", scrollRx + "deg");
  }
  if (win && !reduce) {
    stageEl.addEventListener("pointermove", function (e) {
      var r = win.getBoundingClientRect();
      var x = (e.clientX - r.left) / r.width - 0.5, y = (e.clientY - r.top) / r.height - 0.5;
      win.style.setProperty("--ry", (x * 8).toFixed(2) + "deg");
      win.style.setProperty("--rx", (scrollRx - y * 6).toFixed(2) + "deg");
      win.style.setProperty("--gx", (x + 0.5) * 100 + "%");
      win.style.setProperty("--gy", (y + 0.5) * 100 + "%");
    });
    stageEl.addEventListener("pointerleave", function () { win.style.setProperty("--ry", "0deg"); win.style.setProperty("--rx", scrollRx + "deg"); });
  }

  /* -------------------------------------------- magnetic buttons + glow */
  if (!reduce && matchMedia("(pointer: fine)").matches) {
    $$(".magnetic").forEach(function (b) {
      b.addEventListener("pointermove", function (e) {
        var r = b.getBoundingClientRect(), x = e.clientX - r.left, y = e.clientY - r.top;
        b.style.transform = "translate(" + ((x - r.width / 2) * 0.18).toFixed(1) + "px," + ((y - r.height / 2) * 0.25).toFixed(1) + "px)";
        b.style.setProperty("--mx", x + "px"); b.style.setProperty("--my", y + "px");
      });
      b.addEventListener("pointerleave", function () { b.style.transform = ""; });
    });
    $$(".spot").forEach(function (c) {
      c.addEventListener("pointermove", function (e) {
        var r = c.getBoundingClientRect();
        c.style.setProperty("--sx", e.clientX - r.left + "px"); c.style.setProperty("--sy", e.clientY - r.top + "px");
      });
    });
    var glow = $("#cta-glow"), cta = $(".cta");
    if (glow && cta) cta.addEventListener("pointermove", function (e) {
      var r = cta.getBoundingClientRect();
      glow.style.transform = "translate(calc(-50% + " + ((e.clientX - r.left - r.width / 2) * 0.25) + "px), calc(-50% + " + ((e.clientY - r.top - r.height / 2) * 0.25) + "px))";
    });
  }

  /* ----------------------------------------------------------- counters */
  var cio = new IntersectionObserver(function (entries) {
    entries.forEach(function (e) {
      if (!e.isIntersecting) return; cio.unobserve(e.target);
      var el = e.target, to = +el.dataset.count, pre = el.dataset.prefix || "", suf = el.dataset.suffix || "";
      var pre2 = pre.replace("&lt;", "<");
      if (reduce || to === 0) { el.textContent = pre2 + to + suf; return; }
      var t0 = performance.now(), dur = 1400;
      (function step(now) {
        var p = Math.min(1, (now - t0) / dur), v = Math.round(to * (1 - Math.pow(1 - p, 4)));
        el.textContent = pre2 + v + suf; if (p < 1) requestAnimationFrame(step);
      })(t0);
    });
  }, { threshold: 0.6 });
  $$("[data-count]").forEach(function (el) { cio.observe(el); });

  /* ------------------------------------------------------------ billing */
  $$(".billing button").forEach(function (b) {
    b.addEventListener("click", function () {
      $$(".billing button").forEach(function (x) { x.classList.toggle("is-on", x === b); });
      var year = b.dataset.period === "year";
      $$(".price b[data-month]").forEach(function (p) { p.textContent = year ? p.dataset.year : p.dataset.month; });
    });
  });

  /* --------------------------------------------------- WebGL silk field */
  function silk() {
    var c = $("#silk"); if (!c) return;
    var gl = c.getContext("webgl", { antialias: false, premultipliedAlpha: false, alpha: true });
    if (!gl) { c.style.background = "radial-gradient(80% 60% at 50% 0%, rgba(138,180,255,.25), transparent 70%)"; return; }
    var vs = "attribute vec2 p;void main(){gl_Position=vec4(p,0.,1.);}";
    var fs = [
      "precision highp float;uniform vec2 r;uniform float t;uniform vec2 m;",
      "float h(vec2 p){return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453);}",
      "float n(vec2 p){vec2 i=floor(p),f=fract(p);f=f*f*(3.-2.*f);",
      "return mix(mix(h(i),h(i+vec2(1,0)),f.x),mix(h(i+vec2(0,1)),h(i+vec2(1,1)),f.x),f.y);}",
      "float fbm(vec2 p){float v=0.,a=.5;mat2 R=mat2(.8,.6,-.6,.8);for(int i=0;i<5;i++){v+=a*n(p);p=R*p*2.02;a*=.5;}return v;}",
      "void main(){vec2 uv=(gl_FragCoord.xy-.5*r)/r.y;vec2 q=uv*1.6;float T=t*.06;",
      "vec2 mm=(m-.5)*vec2(r.x/r.y,1.);q+=mm*.35*exp(-2.*length(uv-mm));",
      "vec2 w=vec2(fbm(q+vec2(0.,T)),fbm(q+vec2(5.2,1.3)-T));",
      "vec2 w2=vec2(fbm(q+3.*w+vec2(1.7,9.2)+T*1.3),fbm(q+3.*w+vec2(8.3,2.8)-T));",
      "float f=fbm(q+2.6*w2);",
      "float silk=smoothstep(.35,.95,f)*1.1;float sheen=pow(smoothstep(.55,.9,fbm(q*1.5+w2*4.+T)),3.);",
      "vec3 base=vec3(.027,.027,.04);vec3 blue=vec3(.42,.58,1.);vec3 vio=vec3(.66,.55,1.);",
      "vec3 col=base+mix(blue,vio,w.x)*silk*.62+vec3(.9,.92,1.)*sheen*.42;",
      "float v=smoothstep(1.7,.1,length(uv*vec2(.7,1.1)-vec2(0.,.25)));col*=v;",
      "gl_FragColor=vec4(col,1.);}"
    ].join("");
    function sh(type, src) { var s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); return s; }
    var pr = gl.createProgram(); gl.attachShader(pr, sh(gl.VERTEX_SHADER, vs)); gl.attachShader(pr, sh(gl.FRAGMENT_SHADER, fs)); gl.linkProgram(pr);
    if (!gl.getProgramParameter(pr, gl.LINK_STATUS)) return;
    gl.useProgram(pr);
    var b = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, b);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
    var loc = gl.getAttribLocation(pr, "p"); gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    var ur = gl.getUniformLocation(pr, "r"), ut = gl.getUniformLocation(pr, "t"), um = gl.getUniformLocation(pr, "m");
    var mouse = [0.5, 0.6], target = [0.5, 0.6], visible = true, dpr = Math.min(devicePixelRatio || 1, 1.5);
    function size() { c.width = Math.floor(c.clientWidth * dpr * 0.75); c.height = Math.floor(c.clientHeight * dpr * 0.75); gl.viewport(0, 0, c.width, c.height); }
    size(); addEventListener("resize", size);
    addEventListener("pointermove", function (e) { var r = c.getBoundingClientRect(); target = [(e.clientX - r.left) / r.width, 1 - (e.clientY - r.top) / r.height]; });
    new IntersectionObserver(function (e) { visible = e[0].isIntersecting; }).observe(c);
    var t0 = performance.now();
    (function frame(now) {
      if (visible) {
        mouse[0] += (target[0] - mouse[0]) * 0.04; mouse[1] += (target[1] - mouse[1]) * 0.04;
        gl.uniform2f(ur, c.width, c.height); gl.uniform1f(ut, reduce ? 12 : (now - t0) / 1000); gl.uniform2f(um, mouse[0], mouse[1]);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      }
      if (!reduce) requestAnimationFrame(frame);
    })(t0);
  }

  /* ------------------------------------------------------ voice orb (2D) */
  function orb() {
    var c = $("#orb"); if (!c) return; var x = c.getContext("2d"); if (!x) return;
    var dpr = Math.min(devicePixelRatio || 1, 2), visible = false;
    function size() { c.width = c.clientWidth * dpr; c.height = c.clientHeight * dpr; }
    size(); addEventListener("resize", size);
    new IntersectionObserver(function (e) { visible = e[0].isIntersecting; }).observe(c);
    (function frame(now) {
      if (visible || reduce) {
        var w = c.width, h = c.height, cx = w / 2, cy = h * 0.42, R = Math.min(w, h) * 0.26, t = now / 1000;
        x.clearRect(0, 0, w, h);
        var g = x.createRadialGradient(cx, cy, R * 0.2, cx, cy, R * 2.1);
        g.addColorStop(0, "rgba(138,180,255,0.35)"); g.addColorStop(0.5, "rgba(181,156,255,0.10)"); g.addColorStop(1, "rgba(0,0,0,0)");
        x.fillStyle = g; x.fillRect(0, 0, w, h);
        var speak = (Math.sin(t * 0.7) + 1) / 2;
        for (var ring = 0; ring < 4; ring++) {
          x.beginPath();
          for (var a = 0; a <= Math.PI * 2 + 0.01; a += Math.PI / 90) {
            var k = 1 + (0.05 + 0.07 * speak) * Math.sin(a * (3 + ring) + t * (1.4 + ring * 0.3)) + 0.03 * Math.sin(a * 9 - t * 2.2 + ring);
            var rr = R * (1 + ring * 0.12) * k, px = cx + Math.cos(a) * rr, py = cy + Math.sin(a) * rr;
            a === 0 ? x.moveTo(px, py) : x.lineTo(px, py);
          }
          x.strokeStyle = ring === 0 ? "rgba(244,244,246,0.85)" : "rgba(160,185,255," + (0.4 - ring * 0.08) + ")";
          x.lineWidth = (ring === 0 ? 2 : 1.2) * dpr; x.stroke();
        }
        var core = x.createRadialGradient(cx, cy, 0, cx, cy, R * 0.95);
        core.addColorStop(0, "rgba(255,255,255,0.22)"); core.addColorStop(1, "rgba(138,180,255,0)");
        x.fillStyle = core; x.beginPath(); x.arc(cx, cy, R * 0.95, 0, Math.PI * 2); x.fill();
      }
      if (!reduce) requestAnimationFrame(frame);
    })(performance.now());
  }

  silk(); orb();
  addEventListener("scroll", onScroll, { passive: true });
  onScroll();
})();

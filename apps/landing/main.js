(function () {
  "use strict";
  var root = document.documentElement;
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  var fine = window.matchMedia("(hover: hover) and (pointer: fine)").matches;
  var vh = window.innerHeight;
  var clamp = function (v, a, b) { return v < a ? a : v > b ? b : v; };
  var isDark = function () { return root.dataset.theme === "dark"; };

  // Theme: a circular reveal from the toggle where View Transitions exist.
  var toggle = document.getElementById("theme-toggle");
  if (toggle) {
    toggle.addEventListener("click", function () {
      var next = isDark() ? "light" : "dark";
      var apply = function () {
        root.dataset.theme = next;
        try { localStorage.setItem("dove-theme", next); } catch (e) {}
      };
      if (!document.startViewTransition || reduce) { apply(); return; }
      var r = toggle.getBoundingClientRect();
      var x = r.left + r.width / 2, y = r.top + r.height / 2;
      var end = Math.hypot(Math.max(x, innerWidth - x), Math.max(y, innerHeight - y));
      document.startViewTransition(apply).ready.then(function () {
        root.animate({ clipPath: ["circle(0px at " + x + "px " + y + "px)", "circle(" + end + "px at " + x + "px " + y + "px)"] },
          { duration: 700, easing: "cubic-bezier(.16,1,.3,1)", pseudoElement: "::view-transition-new(root)" });
      });
    });
  }

  // Reveal on enter.
  var io = "IntersectionObserver" in window ? new IntersectionObserver(function (entries) {
    entries.forEach(function (e) {
      if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); }
    });
  }, { rootMargin: "0px 0px -8% 0px", threshold: 0.08 }) : null;
  document.querySelectorAll(".reveal").forEach(function (el, i) {
    if (!io) { el.classList.add("in"); return; }
    el.style.transitionDelay = Math.min((i % 4) * 70, 210) + "ms";
    io.observe(el);
  });

  // Split the statement into words for the scroll scrub.
  var scrub = document.querySelector("[data-scrub]");
  var words = [];
  if (scrub) {
    scrub.innerHTML = scrub.textContent.trim().split(/\s+/).map(function (w) { return '<span class="sw">' + w + "</span>"; }).join(" ");
    words = scrub.querySelectorAll(".sw");
  }

  var nav = document.querySelector(".nav");
  var heroWin = document.getElementById("hero-window");
  var giant = document.querySelector(".giant");
  var ticking = false;
  function onScroll() {
    ticking = false;
    var y = window.scrollY;
    nav.classList.toggle("scrolled", y > 24);
    if (heroWin && !reduce) heroWin.style.setProperty("--p", clamp(y / (vh * 0.65), 0, 1).toFixed(3));
    if (words.length) {
      var r = scrub.getBoundingClientRect();
      var p = reduce ? 1 : clamp((vh * 0.82 - r.top) / (r.height + vh * 0.3), 0, 1);
      var lit = Math.round(p * words.length);
      for (var i = 0; i < words.length; i++) words[i].classList.toggle("on", i < lit);
    }
    if (giant) {
      var g = giant.getBoundingClientRect();
      giant.style.setProperty("--g", clamp((vh - g.top) / g.height, 0, 1).toFixed(3));
    }
  }
  window.addEventListener("scroll", function () { if (!ticking) { ticking = true; requestAnimationFrame(onScroll); } }, { passive: true });
  window.addEventListener("resize", function () { vh = window.innerHeight; onScroll(); });
  onScroll();

  // Pointer parallax on the hero window and phone.
  if (fine && !reduce) {
    var tilt = function (el, amt) {
      if (!el) return;
      el.parentElement.addEventListener("pointermove", function (e) {
        var r = el.getBoundingClientRect();
        var dx = (e.clientX - r.left) / r.width - 0.5, dy = (e.clientY - r.top) / r.height - 0.5;
        el.style.setProperty("--ry", (dx * amt).toFixed(2) + "deg");
        el.style.setProperty("--rx", (-dy * amt).toFixed(2) + "deg");
      });
      el.parentElement.addEventListener("pointerleave", function () {
        el.style.setProperty("--ry", "0deg"); el.style.setProperty("--rx", "0deg");
      });
    };
    tilt(heroWin, 4);
    tilt(document.getElementById("phone"), 14);

    document.querySelectorAll(".magnet").forEach(function (b) {
      b.addEventListener("pointermove", function (e) {
        var r = b.getBoundingClientRect();
        b.style.transform = "translate(" + ((e.clientX - r.left - r.width / 2) * 0.22).toFixed(1) + "px," + ((e.clientY - r.top - r.height / 2) * 0.35).toFixed(1) + "px)";
      });
      b.addEventListener("pointerleave", function () { b.style.transform = ""; });
    });

    document.querySelectorAll(".spot").forEach(function (c) {
      c.addEventListener("pointermove", function (e) {
        var r = c.getBoundingClientRect();
        c.style.setProperty("--mx", (e.clientX - r.left) + "px");
        c.style.setProperty("--my", (e.clientY - r.top) + "px");
        c.style.setProperty("--lit", "1");
      });
      c.addEventListener("pointerleave", function () { c.style.setProperty("--lit", "0"); });
    });
  }

  // Sticky story: the step in the middle of the viewport drives the stage.
  var steps = document.querySelectorAll(".step");
  var shots = document.querySelectorAll(".stage-frame img");
  var dots = document.querySelectorAll(".stage-dots i");
  if (io && steps.length) {
    var sio = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (!e.isIntersecting) return;
        var n = +e.target.dataset.step;
        steps.forEach(function (s, i) { s.classList.toggle("on", i === n); });
        shots.forEach(function (s, i) { s.classList.toggle("on", i === n); });
        dots.forEach(function (s, i) { s.classList.toggle("on", i === n); });
      });
    }, { rootMargin: "-45% 0px -45% 0px" });
    steps.forEach(function (s) { sio.observe(s); });
  }

  // Count up the stats once.
  var counters = document.querySelectorAll("[data-count]");
  if (io && !reduce) {
    var cio = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (!e.isIntersecting) return;
        cio.unobserve(e.target);
        var el = e.target, to = +el.dataset.count, t0 = performance.now();
        if (!to) return;
        (function step(t) {
          var k = clamp((t - t0) / 1400, 0, 1);
          el.textContent = Math.round(to * (1 - Math.pow(1 - k, 4)));
          if (k < 1) requestAnimationFrame(step);
        })(t0);
      });
    }, { threshold: 0.6 });
    counters.forEach(function (c) { c.textContent = c.dataset.count === "0" ? "0" : "0"; cio.observe(c); });
  }

  // Chat ⇄ terminal card flips on its own while visible.
  var flip = document.querySelector(".flip");
  if (flip && io && !reduce) {
    var flipTimer = null;
    new IntersectionObserver(function (entries) {
      var vis = entries[0].isIntersecting;
      if (vis && !flipTimer) flipTimer = setInterval(function () { flip.classList.toggle("term"); }, 2600);
      if (!vis && flipTimer) { clearInterval(flipTimer); flipTimer = null; }
    }).observe(flip);
  }

  // Name the visitor's platform on the hero button.
  var ua = navigator.userAgent || "";
  var os = /Windows/i.test(ua) ? "windows" : /Mac OS X|Macintosh/i.test(ua) && !/iPhone|iPad/i.test(ua) ? "apple" : /Linux/i.test(ua) && !/Android/i.test(ua) ? "linux" : null;
  var names = { apple: "macOS", windows: "Windows", linux: "Linux" };
  if (os) {
    var btn = document.getElementById("hero-download");
    btn.querySelector("[data-os-label]").textContent = "Download for " + names[os];
    btn.querySelector(".i-os").classList.add("i-" + os);
  }

  document.querySelectorAll(".seg").forEach(function (seg) {
    seg.addEventListener("click", function () {
      var period = seg.dataset.period;
      document.querySelectorAll(".seg").forEach(function (s) {
        var on = s === seg;
        s.classList.toggle("on", on);
        s.setAttribute("aria-pressed", on ? "true" : "false");
      });
      document.querySelectorAll(".price b[data-" + period + "]").forEach(function (b) {
        if (b.textContent === b.dataset[period]) return;
        b.textContent = b.dataset[period];
        b.classList.remove("roll"); void b.offsetWidth; b.classList.add("roll");
      });
    });
  });

  document.querySelectorAll("[data-copy]").forEach(function (b) {
    b.addEventListener("click", function () {
      var text = document.querySelector(b.dataset.copy).textContent;
      var done = function () { b.textContent = "Copied"; setTimeout(function () { b.textContent = "Copy"; }, 1600); };
      if (navigator.clipboard) navigator.clipboard.writeText(text).then(done, function () {});
    });
  });

  // Run a render loop only while its element is on screen and the tab is visible.
  function whenVisible(el, start, stop) {
    var on = false, vis = false;
    var sync = function () {
      var want = vis && !document.hidden;
      if (want && !on) { on = true; start(); } else if (!want && on) { on = false; stop(); }
    };
    if (io) new IntersectionObserver(function (e) { vis = e[0].isIntersecting; sync(); }).observe(el);
    else { vis = true; sync(); }
    document.addEventListener("visibilitychange", sync);
  }

  // Hero field: a domain-warped monochrome noise shader that bends around the cursor.
  (function field() {
    var canvas = document.getElementById("field");
    if (!canvas) return;
    var gl = canvas.getContext("webgl", { antialias: false, alpha: false, premultipliedAlpha: false, powerPreference: "low-power" });
    if (!gl) return;
    var vs = "attribute vec2 a;void main(){gl_Position=vec4(a,0.,1.);}";
    var fs = [
      "precision highp float;",
      "uniform vec2 r;uniform float t;uniform vec2 m;uniform float dk;uniform float k;",
      "float h(vec2 p){p=fract(p*vec2(123.34,456.21));p+=dot(p,p+45.32);return fract(p.x*p.y);}",
      "float n(vec2 p){vec2 i=floor(p),f=fract(p);vec2 u=f*f*(3.-2.*f);",
      " return mix(mix(h(i),h(i+vec2(1.,0.)),u.x),mix(h(i+vec2(0.,1.)),h(i+vec2(1.,1.)),u.x),u.y);}",
      "float fbm(vec2 p){float v=0.,s=.5;mat2 R=mat2(1.6,1.2,-1.2,1.6);for(int i=0;i<5;i++){v+=s*n(p);p=R*p;s*=.5;}return v;}",
      "void main(){",
      " vec2 p=(gl_FragCoord.xy-.5*r)/r.y;",
      " vec2 mm=(m-.5)*vec2(r.x/r.y,1.);",
      " vec2 dv=p-mm;float d=dot(dv,dv);",
      " p-=dv*.22*exp(-d*4.);",
      " float T=t*.04;",
      " vec2 q=vec2(fbm(p*1.25+vec2(0.,T)),fbm(p*1.25+vec2(5.2,1.3-T)));",
      " vec2 w=vec2(fbm(p*1.25+2.6*q+vec2(1.7,9.2)+T*1.4),fbm(p*1.25+2.6*q+vec2(8.3,2.8)-T));",
      " float f=fbm(p*1.25+2.4*w);",
      " float silk=pow(abs(sin(f*11.+w.x*4.)),7.);",
      " float body=smoothstep(.3,.95,f);",
      " float v=body*.5+silk*body*.75+.2*smoothstep(.55,.95,length(w));",
      " float edge=smoothstep(.15,1.2,length(p*vec2(.62,1.)-vec2(0.,.08)));",
      " v*=mix(.35,1.,edge);",
      " v+=(h(gl_FragCoord.xy+fract(t*7.)*91.)-.5)*.05;",
      " vec3 bg=mix(vec3(.98),vec3(.039),dk);vec3 ink=mix(vec3(.0),vec3(1.),dk);",
      " gl_FragColor=vec4(mix(bg,ink,clamp(v*k,0.,1.)),1.);",
      "}"
    ].join("\n");
    var sh = function (type, src) { var s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); return gl.getShaderParameter(s, gl.COMPILE_STATUS) ? s : null; };
    var v = sh(gl.VERTEX_SHADER, vs), f = sh(gl.FRAGMENT_SHADER, fs);
    if (!v || !f) return;
    var pr = gl.createProgram(); gl.attachShader(pr, v); gl.attachShader(pr, f); gl.linkProgram(pr);
    if (!gl.getProgramParameter(pr, gl.LINK_STATUS)) return;
    gl.useProgram(pr);
    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    var loc = gl.getAttribLocation(pr, "a");
    gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    var U = {}; ["r", "t", "m", "dk", "k"].forEach(function (u) { U[u] = gl.getUniformLocation(pr, u); });

    var mouse = [0.5, 0.62], target = [0.5, 0.62], raf = 0, t0 = performance.now(), shown = false;
    function size() {
      var scale = Math.min(window.devicePixelRatio || 1, 2) * (innerWidth < 700 ? 0.45 : 0.6);
      var w = Math.round(canvas.clientWidth * scale), h = Math.round(canvas.clientHeight * scale);
      if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; gl.viewport(0, 0, w, h); }
    }
    function frame(now) {
      size();
      mouse[0] += (target[0] - mouse[0]) * 0.05; mouse[1] += (target[1] - mouse[1]) * 0.05;
      var dk = isDark() ? 1 : 0;
      gl.uniform2f(U.r, canvas.width, canvas.height);
      gl.uniform1f(U.t, reduce ? 12 : (now - t0) / 1000 + 12);
      gl.uniform2f(U.m, mouse[0], mouse[1]);
      gl.uniform1f(U.dk, dk);
      gl.uniform1f(U.k, dk ? 0.42 : 0.2);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      if (!shown) { shown = true; canvas.classList.add("ready"); }
      raf = reduce ? 0 : requestAnimationFrame(frame);
    }
    window.addEventListener("pointermove", function (e) {
      var r = canvas.getBoundingClientRect();
      target[0] = (e.clientX - r.left) / r.width; target[1] = 1 - (e.clientY - r.top) / r.height;
    }, { passive: true });
    new MutationObserver(function () { if (!raf) frame(performance.now()); }).observe(root, { attributes: true, attributeFilter: ["data-theme"] });
    whenVisible(canvas, function () { raf = requestAnimationFrame(frame); }, function () { cancelAnimationFrame(raf); raf = 0; });
  })();

  // Download: the Dove mark as a field of particles that scatter from the cursor.
  (function flock() {
    var canvas = document.getElementById("flock");
    if (!canvas) return;
    var ctx = canvas.getContext("2d");
    var pts = [], W = 0, H = 0, dpr = 1, raf = 0, mx = -1e4, my = -1e4, ink = "#fff", t = 0;
    var img = new Image();
    img.src = "assets/dove-logo.svg";
    function build() {
      dpr = Math.min(window.devicePixelRatio || 1, 2);
      W = canvas.clientWidth; H = canvas.clientHeight;
      canvas.width = W * dpr; canvas.height = H * dpr;
      var wide = W > 760;
      var S = Math.min(wide ? W * 0.42 : W * 0.7, H * 0.85, 520);
      var cx = wide ? W * 0.27 : W / 2, cy = H / 2;
      var off = document.createElement("canvas"), N = 160;
      off.width = off.height = N;
      var o = off.getContext("2d");
      o.drawImage(img, 0, 0, N, N);
      var data = o.getImageData(0, 0, N, N).data, old = pts;
      pts = [];
      for (var y = 0; y < N; y += 2) for (var x = 0; x < N; x += 2) {
        if (data[(y * N + x) * 4 + 3] > 90) {
          var hx = cx + (x / N - 0.5) * S, hy = cy + (y / N - 0.5) * S;
          var prev = old[pts.length];
          pts.push({ hx: hx, hy: hy, x: prev ? prev.x : Math.random() * W, y: prev ? prev.y : Math.random() * H, vx: 0, vy: 0, s: Math.random() * 6.28 });
        }
      }
      if (reduce) pts.forEach(function (p) { p.x = p.hx; p.y = p.hy; });
    }
    function draw() {
      t += 0.016;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, W, H);
      ctx.fillStyle = ink;
      for (var i = 0; i < pts.length; i++) {
        var p = pts[i];
        if (!reduce) {
          var dx = p.x - mx, dy = p.y - my, d2 = dx * dx + dy * dy;
          if (d2 < 12000) { var d = Math.sqrt(d2) || 1, f = (1 - d / 110) * 3.2; p.vx += dx / d * f; p.vy += dy / d * f; }
          p.vx += (p.hx + Math.sin(t + p.s) * 1.2 - p.x) * 0.03; p.vy += (p.hy + Math.cos(t * 0.8 + p.s) * 1.2 - p.y) * 0.03;
          p.vx *= 0.86; p.vy *= 0.86; p.x += p.vx; p.y += p.vy;
        }
        var sz = 1.3 + (Math.sin(p.s + t * 2) + 1) * 0.45;
        ctx.globalAlpha = 0.55 + (Math.sin(p.s * 3 + t) + 1) * 0.22;
        ctx.fillRect(p.x, p.y, sz, sz);
      }
      ctx.globalAlpha = 1;
      raf = reduce ? 0 : requestAnimationFrame(draw);
    }
    function colour() { ink = getComputedStyle(root).getPropertyValue("--ink").trim() || "#fff"; }
    canvas.parentElement.addEventListener("pointermove", function (e) { var r = canvas.getBoundingClientRect(); mx = e.clientX - r.left; my = e.clientY - r.top; });
    canvas.parentElement.addEventListener("pointerleave", function () { mx = my = -1e4; });
    new MutationObserver(function () { colour(); if (!raf) draw(); }).observe(root, { attributes: true, attributeFilter: ["data-theme"] });
    window.addEventListener("resize", function () { if (img.complete) build(); });
    img.onload = function () {
      colour(); build();
      whenVisible(canvas, function () { raf = requestAnimationFrame(draw); }, function () { cancelAnimationFrame(raf); raf = 0; });
    };
  })();
})();

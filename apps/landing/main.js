(function () {
  "use strict";
  var root = document.documentElement;

  var toggle = document.getElementById("theme-toggle");
  if (toggle) {
    toggle.addEventListener("click", function () {
      var next = root.dataset.theme === "dark" ? "light" : "dark";
      root.dataset.theme = next;
      try { localStorage.setItem("dove-theme", next); } catch (e) {}
    });
  }

  var nav = document.querySelector(".nav");
  var onScroll = function () { nav.classList.toggle("scrolled", window.scrollY > 8); };
  window.addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  var items = document.querySelectorAll(".reveal");
  if ("IntersectionObserver" in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); }
      });
    }, { rootMargin: "0px 0px -6% 0px", threshold: 0.08 });
    items.forEach(function (el, i) {
      el.style.transitionDelay = Math.min((i % 6) * 50, 250) + "ms";
      io.observe(el);
    });
  } else {
    items.forEach(function (el) { el.classList.add("in"); });
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
        b.textContent = b.dataset[period];
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
})();

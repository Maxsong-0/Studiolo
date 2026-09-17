/*
  Droppy Code marketing site, page script
  ========================================

    1. The binary field: a canvas of zeros and ones behind the hero and
       the outro, scrubbed by scroll like Droppy's hero and outro art.
    2. The highlight rail controller, ported from the home page's
       showcase carousel (drag, arrows, keyboard, aria-current).
    3. The theme picker: the real app captured in every theme, one big window at a time.
     4. (Was: film playback. The page is stills only now, so initFilms is a no-op.)
*/

(function () {
  "use strict";

  var reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

  /* ---------------------------------------------------------------- */
  /* 1. Binary field                                                    */
  /* ---------------------------------------------------------------- */

  function initBinaryField(host) {
    var canvas = host.querySelector("canvas");
    if (!canvas) return;
    var ctx = canvas.getContext("2d", { alpha: true });
    if (!ctx) return;

    /* Scroll-scrubbed, like the frame sequences behind getdroppy.app's
       hero and outro: nothing moves while the page is still, and the
       digits stream down as the visitor scrolls. "pagetop" spends the
       first --scroll-span pixels of the page (the hero); "viewport" runs
       while the host crosses the viewport (the outro). */
    var mode = host.getAttribute("data-progress") || "pagetop";
    var span = parseFloat(host.getAttribute("data-scroll-span")) || 480;
    var density = parseFloat(host.getAttribute("data-density")) || 1;

    var CELL = 22;
    var FONT = "500 13px ui-monospace, 'SF Mono', Menlo, monospace";
    var TRAIL = 9;
    var cols = 0, rows = 0, dpr = 1;
    var bits = [];      // one char per cell, seeded once
    var heads = [];     // per column: { start, travel }
    var frame = 0;
    var lastProgress = -1;

    function resize() {
      var rect = host.getBoundingClientRect();
      dpr = Math.min(window.devicePixelRatio || 1, 2);
      canvas.width = Math.max(1, Math.round(rect.width * dpr));
      canvas.height = Math.max(1, Math.round(rect.height * dpr));
      canvas.style.width = rect.width + "px";
      canvas.style.height = rect.height + "px";
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      cols = Math.ceil(rect.width / CELL) + 1;
      rows = Math.ceil(rect.height / CELL) + 1;
      bits = new Array(cols * rows);
      for (var i = 0; i < bits.length; i++) bits[i] = Math.random() < 0.5 ? "0" : "1";
      heads = [];
      for (var c = 0; c < cols; c++) {
        var active = Math.random() < 0.55 * density;
        heads.push({
          start: active ? -Math.random() * rows * 0.6 : Infinity,
          travel: rows * (1.1 + Math.random() * 0.9)
        });
      }
      lastProgress = -1;
      requestDraw();
    }

    function progress() {
      if (reducedMotion.matches) return 0;
      if (mode === "viewport") {
        var rect = host.getBoundingClientRect();
        var vh = window.innerHeight || 1;
        return Math.max(0, Math.min(1, (vh - rect.top) / (vh + rect.height)));
      }
      return Math.max(0, Math.min(1, window.scrollY / span));
    }

    function draw(p) {
      ctx.clearRect(0, 0, canvas.width / dpr, canvas.height / dpr);
      ctx.font = FONT;
      ctx.textBaseline = "top";
      for (var c = 0; c < cols; c++) {
        var head = heads[c];
        var y = head.start === Infinity ? -1e9 : head.start + p * head.travel;
        var yi = Math.floor(y);
        for (var r = 0; r < rows; r++) {
          var i = r * cols + c;
          var base = 0.045 + 0.035 * (((c * 7 + r * 3) % 11) / 10);
          var d = yi - r;
          var g = d >= 0 && d < TRAIL ? 1 - d / TRAIL : 0;
          var alpha = Math.min(0.6, base + g * 0.5);
          var bit = g > 0 ? (((r * 7 + c * 13 + yi) & 1) ? "1" : "0") : bits[i];
          ctx.fillStyle = g > 0
            ? "rgba(140, 196, 255, " + alpha.toFixed(3) + ")"
            : "rgba(120, 165, 230, " + alpha.toFixed(3) + ")";
          ctx.fillText(bit, c * CELL + 4, r * CELL + 3);
        }
      }
    }

    function tick() {
      frame = 0;
      var p = progress();
      if (Math.abs(p - lastProgress) < 0.002) return;
      lastProgress = p;
      draw(p);
    }
    function requestDraw() {
      if (!frame) frame = window.requestAnimationFrame(tick);
    }

    resize();
    var resizeTimer = 0;
    var lastSize = "";
    function scheduleResize() {
      window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(function () {
        var rect = host.getBoundingClientRect();
        var size = Math.round(rect.width) + "x" + Math.round(rect.height);
        if (size === lastSize) return;
        lastSize = size;
        resize();
      }, 80);
    }
    if ("ResizeObserver" in window) {
      new ResizeObserver(scheduleResize).observe(host);
    } else {
      window.addEventListener("resize", scheduleResize, { passive: true });
      window.addEventListener("load", scheduleResize);
    }

    var active = true;
    function onScroll() { if (active) requestDraw(); }
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (entries) {
        active = entries[0].isIntersecting;
        if (active) requestDraw();
      }, { rootMargin: "20% 0px" }).observe(host);
    }
    window.addEventListener("scroll", onScroll, { passive: true });
    if (document.fonts && document.fonts.ready) {
      document.fonts.ready.then(function () { lastProgress = -1; requestDraw(); }, function () {});
    }
  }

  /* ---------------------------------------------------------------- */
  /* 2. Highlight rail                                                  */
  /* ---------------------------------------------------------------- */

  function initHighlightRail(root) {
    var rail = root.querySelector("[data-highlight-rail]");
    var cards = Array.prototype.slice.call(root.querySelectorAll("[data-highlight-slide]"));
    var prev = root.querySelector("[data-highlight-prev]");
    var next = root.querySelector("[data-highlight-next]");
    if (!rail || !cards.length || !prev || !next) return;

    var activeIndex = 0;
    var scrollFrame = 0;
    var dragPointer = null;
    var dragStartX = 0;
    var dragStartScroll = 0;

    function pageGutter() {
      return parseFloat(window.getComputedStyle(rail).paddingLeft) || 0;
    }
    function targetLeft(index) {
      return Math.max(0, cards[index].offsetLeft - pageGutter());
    }
    function nearestIndex() {
      var closest = 0;
      var distance = Infinity;
      cards.forEach(function (card, index) {
        var d = Math.abs(rail.scrollLeft - targetLeft(index));
        if (d < distance) { closest = index; distance = d; }
      });
      return closest;
    }
    function setActive(index) {
      activeIndex = Math.max(0, Math.min(cards.length - 1, index));
      cards.forEach(function (card, i) {
        var isActive = i === activeIndex;
        if (isActive) card.setAttribute("aria-current", "true");
        else card.removeAttribute("aria-current");
      });
      prev.disabled = rail.scrollLeft <= 2;
      next.disabled = rail.scrollLeft >= rail.scrollWidth - rail.clientWidth - 2;
    }
    function syncFromScroll() {
      scrollFrame = 0;
      setActive(nearestIndex());
    }
    function goTo(index, behavior) {
      var destination = Math.max(0, Math.min(cards.length - 1, index));
      rail.scrollTo({
        left: targetLeft(destination),
        behavior: behavior || (reducedMotion.matches ? "auto" : "smooth")
      });
    }

    rail.addEventListener("scroll", function () {
      if (scrollFrame) cancelAnimationFrame(scrollFrame);
      scrollFrame = requestAnimationFrame(syncFromScroll);
    }, { passive: true });
    prev.addEventListener("click", function () { goTo(activeIndex - 1); });
    next.addEventListener("click", function () { goTo(activeIndex + 1); });
    rail.addEventListener("keydown", function (event) {
      if (event.key === "Home") { event.preventDefault(); goTo(0); return; }
      if (event.key === "End") { event.preventDefault(); goTo(cards.length - 1); return; }
      if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
      event.preventDefault();
      goTo(activeIndex + (event.key === "ArrowRight" ? 1 : -1));
    });
    rail.addEventListener("pointerdown", function (event) {
      if (event.pointerType !== "mouse" || event.button !== 0) return;
      if (event.target.closest("button, a, input, select, textarea")) return;
      dragPointer = event.pointerId;
      dragStartX = event.clientX;
      dragStartScroll = rail.scrollLeft;
      rail.classList.add("is-dragging");
      rail.setPointerCapture(event.pointerId);
      event.preventDefault();
    });
    rail.addEventListener("pointermove", function (event) {
      if (event.pointerId !== dragPointer) return;
      rail.scrollLeft = dragStartScroll - (event.clientX - dragStartX);
    });
    function finishDrag(event) {
      if (event.pointerId !== dragPointer) return;
      dragPointer = null;
      rail.classList.remove("is-dragging");
      goTo(nearestIndex());
    }
    rail.addEventListener("pointerup", finishDrag);
    rail.addEventListener("pointercancel", finishDrag);
    window.addEventListener("resize", function () { goTo(activeIndex, "auto"); }, { passive: true });
    setActive(0);
    /* The arrows' enabled state reads the rail's overflow, which is only
       known once the stylesheet and fonts have laid the cards out. Deferred
       scripts can run before that, so re-read it when the rail's box
       changes and once the page has fully loaded. */
    function resync() { setActive(nearestIndex()); }
    window.addEventListener("load", resync);
    if ("ResizeObserver" in window) new ResizeObserver(resync).observe(rail);
  }

  /* ---------------------------------------------------------------- */
  /* 3. Theme wall                                                      */
  /* ---------------------------------------------------------------- */

  // Mirrors AppTheme.swift: [id, name, detail]. Each id is a capture of the real app in
  // that theme, cut from the same window, under site-assets/droppy-code/themes (the
  // desktop capture run's set, one still per theme; the tour prefix has no per-theme stills).
  var THEMES = [
    ["system", "System", "Follows your Mac"],
    ["light", "Light", "Light · System accent"],
    ["dark", "Dark", "Dark · System accent"],
    ["catppuccinMocha", "Catppuccin Mocha", "Dark · Mauve"],
    ["catppuccinLatte", "Catppuccin Latte", "Light · Mauve"],
    ["dracula", "Dracula", "Dark · Purple"],
    ["tokyoNight", "Tokyo Night", "Dark · Blue"],
    ["nord", "Nord", "Dark · Frost"],
    ["gruvbox", "Gruvbox", "Dark · Orange"],
    ["gruvboxLight", "Gruvbox Light", "Light · Rust"],
    ["oneDark", "One Dark", "Dark · Blue"],
    ["everforest", "Everforest", "Dark · Green"],
    ["kanagawa", "Kanagawa", "Dark · Wave blue"],
    ["rosePine", "Rosé Pine", "Dark · Rose"],
    ["solarizedDark", "Solarized Dark", "Dark · Blue"],
    ["solarizedLight", "Solarized Light", "Light · Blue"],
    ["githubDark", "GitHub Dark", "Dark · Blue"],
    ["githubLight", "GitHub Light", "Light · Blue"],
    ["ayu", "Ayu", "Dark · Amber"],
    ["nightOwl", "Night Owl", "Dark · Blue"],
    ["monokai", "Monokai", "Dark · Pink"],
    ["claude", "Claude", "Dark · Terracotta"],
    ["claudeLight", "Claude Light", "Light · Terracotta"],
    ["codex", "Codex", "Dark · Magenta"],
    ["cursor", "Cursor", "Dark · Frost"],
    ["matrix", "Matrix", "Dark · Green"]
  ];

  var THEME_BASE = "https://droppy-releases.jordylegrand.workers.dev/site-assets/droppy-code/tour/v5/themes/";

  /* One big window, the real app in the chosen theme, and the 26 names under it. Cycles on
     its own until the visitor picks one, and only fetches a theme when it is about to show. */
  function buildThemePicker(host) {
    var stage = document.createElement("div");
    stage.className = "themes__stage";
    var names = document.createElement("div");
    names.className = "themes__names";
    names.setAttribute("role", "tablist");
    var caption = document.createElement("p");
    caption.className = "themes__caption";
    var images = {};
    var chips = {};
    var current = null;
    var timer = 0;

    function image(id) {
      if (images[id]) return images[id];
      var img = document.createElement("img");
      img.src = THEME_BASE + id + ".webp";
      img.width = 2080;
      img.height = 1300;
      img.decoding = "async";
      img.alt = "Droppy Code in the " + id + " theme";
      stage.appendChild(img);
      images[id] = img;
      return img;
    }

    function show(id) {
      if (current === id) return;
      var theme = THEMES.filter(function (t) { return t[0] === id; })[0];
      if (!theme) return;
      var next = image(id);
      var reveal = function () {
        Object.keys(images).forEach(function (key) { images[key].classList.toggle("is-current", key === id); });
        Object.keys(chips).forEach(function (key) {
          chips[key].classList.toggle("is-current", key === id);
          chips[key].setAttribute("aria-selected", key === id ? "true" : "false");
        });
        caption.textContent = theme[1] + " · " + theme[2];
      };
      current = id;
      if (next.complete && next.naturalWidth) reveal();
      else next.addEventListener("load", reveal, { once: true });
    }

    THEMES.forEach(function (t) {
      var chip = document.createElement("button");
      chip.type = "button";
      chip.className = "themes__name";
      chip.setAttribute("role", "tab");
      chip.textContent = t[1];
      chip.addEventListener("click", function () {
        window.clearInterval(timer);
        timer = 0;
        show(t[0]);
      });
      chips[t[0]] = chip;
      names.appendChild(chip);
    });
    host.appendChild(stage);
    host.appendChild(names);
    host.appendChild(caption);

    var order = THEMES.map(function (t) { return t[0]; });
    show("dark");
    // The next few, warmed so the cycle never shows an empty stage.
    ["claude", "catppuccinMocha", "codex"].forEach(image);

    function cycle() {
      if (reducedMotion.matches) return;
      var index = order.indexOf(current);
      var id = order[(index + 1) % order.length];
      image(order[(index + 2) % order.length]);
      show(id);
    }
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (entries) {
        if (entries[0].isIntersecting) {
          if (!timer && !reducedMotion.matches) timer = window.setInterval(cycle, 2600);
        } else if (timer) {
          window.clearInterval(timer);
          timer = 0;
        }
      }, { threshold: 0.4 }).observe(host);
    }
  }

  /* ---------------------------------------------------------------- */
  /* 4. (Films removed: the page is stills only; nothing to play.)     */
  /* ---------------------------------------------------------------- */

  function initFilms() {}

  function onReady(callback) {
    if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", callback);
    else callback();
  }

  onReady(function () {
    Array.prototype.forEach.call(document.querySelectorAll("[data-binary-field]"), initBinaryField);
    Array.prototype.forEach.call(document.querySelectorAll("[data-highlight-carousel]"), initHighlightRail);
    Array.prototype.forEach.call(document.querySelectorAll("[data-theme-picker]"), buildThemePicker);
    initFilms();
  });
})();

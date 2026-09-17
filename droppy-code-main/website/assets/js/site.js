/*
  Droppy Code marketing site, shared site script
  ===============================================

  Plain vanilla JS, no build step, ported from getdroppy.app's site.js
  and trimmed to what this page uses:

    1. Staggered reveal-on-load and reveal-on-scroll for `.reveal`.
    2. FAQ accordion toggle for `.faq__item` / `.faq__question`.
    3. The phone notch menu, the skip link, the active nav link and the
       footer year.
    4. Footer wordmark parallax.
    5. A guard against copying: selection, copy, drag and the context menu are off.
    6. The button morph hover: the two content layers every button's
       shared hover needs, built at runtime (see "Morph hover" in site.css).
*/

(function () {
  "use strict";

  /* ---------------------------------------------------------------- */
  /* 1. Reveal-on-load / reveal-on-scroll                              */
  /* ---------------------------------------------------------------- */

  function initReveal() {
    var elements = Array.prototype.slice.call(document.querySelectorAll(".reveal"));
    if (!elements.length) return;

    var loadElements = [];
    var scrollElements = [];
    elements.forEach(function (el) {
      if (el.hasAttribute("data-reveal-on-load")) loadElements.push(el);
      else scrollElements.push(el);
    });

    function settleReveal(el) {
      window.setTimeout(function () { el.classList.add("is-settled"); }, 1100);
    }
    function show(el) {
      el.classList.add("is-visible");
      settleReveal(el);
    }

    // Double rAF so the browser paints the initial (opacity: 0) state
    // before the class flips, guaranteeing the transition actually runs.
    requestAnimationFrame(function () {
      requestAnimationFrame(function () {
        loadElements.forEach(show);
      });
    });

    if (!scrollElements.length) return;

    if (!("IntersectionObserver" in window)) {
      scrollElements.forEach(show);
      return;
    }

    var REVEAL_RATIO = 0.12;
    var observer = new IntersectionObserver(
      function (entries) {
        entries.forEach(function (entry) {
          if (!entry.isIntersecting) return;
          var rootHeight = entry.rootBounds ? entry.rootBounds.height : window.innerHeight;
          var reachesRatio = entry.boundingClientRect.height * REVEAL_RATIO <= rootHeight;
          var ready = reachesRatio
            ? entry.intersectionRatio >= REVEAL_RATIO
            : entry.intersectionRect.height >= rootHeight * REVEAL_RATIO;
          if (!ready) return;
          show(entry.target);
          observer.unobserve(entry.target);
        });
      },
      {
        root: null,
        rootMargin: "0px 0px -8% 0px",
        threshold: [0, 0.005, 0.01, 0.02, 0.04, 0.08, REVEAL_RATIO],
      }
    );
    scrollElements.forEach(function (el) { observer.observe(el); });

    document.addEventListener("transitionend", function (event) {
      var t = event.target;
      if (t && t.classList && t.classList.contains("reveal") && t.classList.contains("is-visible")) {
        t.classList.add("is-settled");
      }
    });
  }

  /* ---------------------------------------------------------------- */
  /* 2. FAQ accordion                                                   */
  /* ---------------------------------------------------------------- */

  function initFaq() {
    var questions = Array.prototype.slice.call(document.querySelectorAll(".faq__question"));
    questions.forEach(function (question) {
      question.addEventListener("click", function () {
        var item = question.closest(".faq__item");
        if (!item) return;
        var isOpen = item.classList.contains("is-open");
        item.classList.toggle("is-open", !isOpen);
        question.setAttribute("aria-expanded", String(!isOpen));
      });
    });
  }

  /* ---------------------------------------------------------------- */
  /* 3. Small shared helpers                                           */
  /* ---------------------------------------------------------------- */

  function markActiveNavLink() {
    var hash = window.location.hash;
    if (!hash) return;
    Array.prototype.forEach.call(document.querySelectorAll(".nav__link"), function (link) {
      if (link.getAttribute("href") === hash) link.classList.add("is-active");
    });
  }

  function setCurrentYear() {
    var year = String(new Date().getFullYear());
    Array.prototype.forEach.call(document.querySelectorAll("[data-current-year]"), function (el) {
      el.textContent = year;
    });
  }

  function initSkipLink() {
    var main = document.querySelector("main");
    if (!main) return;
    if (!main.id) main.id = "main";
    var link = document.createElement("a");
    link.className = "skip-link";
    link.href = "#" + main.id;
    link.textContent = "Skip to content";
    document.body.insertBefore(link, document.body.firstChild);
    if (!main.hasAttribute("tabindex")) main.setAttribute("tabindex", "-1");
  }

  function initNotchMenu() {
    var pill = document.querySelector(".nav__pill");
    var btn = document.querySelector(".nav__menu");
    if (!pill || !btn) return;
    var links = pill.querySelector(".nav__links");
    if (!links) return;
    if (!links.id) links.id = "site-navigation-links";
    btn.setAttribute("aria-controls", links.id);

    function isPhoneDock() { return window.matchMedia("(max-width: 720px)").matches; }
    function syncInteractivity(open) {
      if (isPhoneDock() && !open) links.setAttribute("inert", "");
      else links.removeAttribute("inert");
    }
    function close() {
      pill.classList.remove("is-open");
      btn.setAttribute("aria-expanded", "false");
      syncInteractivity(false);
    }
    syncInteractivity(false);
    btn.addEventListener("click", function (e) {
      e.stopPropagation();
      var open = pill.classList.toggle("is-open");
      btn.setAttribute("aria-expanded", open ? "true" : "false");
      syncInteractivity(open);
    });
    pill.addEventListener("click", function (e) {
      if (e.target && e.target.closest && e.target.closest(".nav__link")) close();
    });
    document.addEventListener("click", function (e) {
      if (!pill.contains(e.target)) close();
    });
    document.addEventListener("keydown", function (e) {
      if (e.key === "Escape" && pill.classList.contains("is-open")) {
        close();
        btn.focus();
      }
    });
    window.addEventListener("resize", close);
  }

  /* ---------------------------------------------------------------- */
  /* 4. Footer wordmark: subtle upward drift as it scrolls into view.  */
  /* ---------------------------------------------------------------- */

  function initWordmarkParallax() {
    var span = document.querySelector(".footer__wordmark span");
    if (!span) return;
    var wrap = span.parentElement;
    var frame = 0;
    var active = false;
    function update() {
      frame = 0;
      var rect = wrap.getBoundingClientRect();
      var vh = window.innerHeight || 1;
      var progress = Math.min(1, Math.max(0, (vh - rect.top) / (rect.height + vh * 0.25)));
      var drift = (1 - progress) * 24;
      span.style.transform = "translateY(calc(0.06em + " + drift.toFixed(1) + "px))";
    }
    function request() { if (!frame) frame = window.requestAnimationFrame(update); }
    function start() {
      if (active) return;
      active = true;
      window.addEventListener("scroll", request, { passive: true });
      window.addEventListener("resize", request, { passive: true });
      request();
    }
    function stop() {
      if (!active) return;
      active = false;
      window.removeEventListener("scroll", request);
      window.removeEventListener("resize", request);
      if (frame) window.cancelAnimationFrame(frame);
      frame = 0;
    }
    if (!("IntersectionObserver" in window)) { start(); return; }
    new IntersectionObserver(function (entries) {
      if (entries[0].isIntersecting) start();
      else stop();
    }, { rootMargin: "100% 0px", threshold: 0 }).observe(wrap);
  }

  /* ---------------------------------------------------------------- */
  /* 5. Nothing on the page is for taking                              */
  /* ---------------------------------------------------------------- */

  function initGuard() {
    var swallow = function (event) { event.preventDefault(); };
    ["contextmenu", "copy", "cut", "dragstart", "selectstart"].forEach(function (name) {
      document.addEventListener(name, swallow);
    });
    // Save-as and view-source shortcuts.
    document.addEventListener("keydown", function (event) {
      if (!(event.metaKey || event.ctrlKey)) return;
      var key = (event.key || "").toLowerCase();
      if (key === "s" || key === "u" || (key === "c" && !event.shiftKey)) event.preventDefault();
    });
  }

  function onReady(callback) {
    if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", callback);
    else callback();
  }

  onReady(function () {
    initGuard();
    initSkipLink();
    initReveal();
    initFaq();
    initNotchMenu();
    markActiveNavLink();
    setCurrentYear();
    initWordmarkParallax();
  });
})();

/* ------------------------------------------------------------------- */
/* Button morph hover: builds the two content layers the shared hover    */
/* needs (see "Morph hover" in assets/css/site.css). Ported from         */
/* getdroppy.app's site.js.                                              */
/* ------------------------------------------------------------------- */
/*
  Every button hovers the same way: its content dissolves and reforms
  through a thick real blur, from the full content to a reduced form.

    <a class="btn--applepill">                       becomes
      <span class="btn-morph__face btn-morph__face--rest"> …original children… </span>
      <span class="btn-morph__face btn-morph__face--hover" aria-hidden="true"> …mark… </span>

  The mark comes from, in priority order: data-morph-glyph="<name>" (a
  glyph from the dictionary; "none" opts out to a plain label dissolve),
  else a leading <svg>/<img> icon cloned into the hover face (the Apple
  mark on the download pill is the canonical case), else the label matched
  against LABEL_GLYPHS, else the label itself dissolves and reforms.

  The rest face is only ever faded and blurred, never removed or hidden,
  and the accessible name is pinned with aria-label, so the name is the
  same at rest and on hover.
*/
(function () {
  "use strict";

  var SELECTOR = [".btn", ".btn--applepill", ".btn--darkpill", ".footer__cta"].join(",");

  var GLYPH_HEAD =
    '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" ' +
    'stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">';
  var GLYPHS = {
    "arrow-down": '<path d="M8 2.75v10"/><path d="M3.75 8.75 8 13l4.25-4.25"/>',
    "arrow-right": '<path d="M2.75 8h10"/><path d="M8.75 3.75 13 8l-4.25 4.25"/>',
    "arrow-up-right": '<path d="M6.5 9.5 13 3"/><path d="M5.75 3H13v7.25"/>',
    "arrow-left": '<path d="M13.25 8h-10"/><path d="M7.25 3.75 3 8l4.25 4.25"/>',
    check: '<path d="m3 8.7 3.3 3.3L13 4.4"/>',
    house: '<path d="m2.7 7.7 5.3-5 5.3 5"/><path d="M4.3 6.5v6.6h7.4V6.5"/>',
    envelope:
      '<rect x="2.2" y="3.6" width="11.6" height="8.8" rx="1.6"/>' +
      '<path d="m2.8 4.6 5.2 4.2 5.2-4.2"/>',
    question:
      '<path d="M5.9 5.5a2.2 2.2 0 1 1 3.1 2c-.7.4-1 .9-1 1.7"/>' +
      '<path d="M8 12.3h.01"/>'
  };

  var LABEL_GLYPHS = [
    [/^back to home\b|^home\b/, "house"],
    [/^back\b/, "arrow-left"],
    [/^continue\b/, "arrow-right"],
    [/^what is\b|\?$/, "question"],
    [/^(download|get droppy)\b/, "arrow-down"],
    [/^(view source|open source|source|gitlab|github|meet)\b/, "arrow-up-right"],
    [/^(see what it does|see how it works|show more)\b/, "arrow-down"]
  ];

  function glyphNode(name) {
    var body = GLYPHS[name];
    if (!body) return null;
    var tmp = document.createElement("span");
    tmp.innerHTML = GLYPH_HEAD + body + "</svg>";
    return tmp.firstChild;
  }

  function findMark(btn, label) {
    var named = btn.getAttribute("data-morph-glyph");
    if (named === "none") return null;
    if (named) {
      var g = glyphNode(named);
      if (g) return { node: g, move: true };
    }
    var kids = btn.children;
    for (var i = 0; i < kids.length; i++) {
      var tag = kids[i].tagName;
      if (tag === "SVG" || tag === "svg" || tag === "IMG") {
        return { node: kids[i], move: false };
      }
    }
    var text = (label || "").toLowerCase();
    for (var j = 0; j < LABEL_GLYPHS.length; j++) {
      if (LABEL_GLYPHS[j][0].test(text)) {
        var node = glyphNode(LABEL_GLYPHS[j][1]);
        if (node) return { node: node, move: true };
      }
    }
    return null;
  }

  function prepare(btn) {
    if (!btn || btn.getAttribute("data-morph") === "off") return;
    if (btn.querySelector(":scope > .btn-morph__face--rest")) return;
    if (!btn.firstChild) return;

    var label = (btn.textContent || "").trim();
    var mark = findMark(btn, label);
    if (!mark && !label) return;

    if (label && !btn.getAttribute("aria-label") && !btn.getAttribute("aria-labelledby")) {
      btn.setAttribute("aria-label", label);
    }

    var rest = document.createElement("span");
    rest.className = "btn-morph__face btn-morph__face--rest";
    while (btn.firstChild) rest.appendChild(btn.firstChild);
    btn.appendChild(rest);

    if (mark) {
      var hover = document.createElement("span");
      hover.className = "btn-morph__face btn-morph__face--hover";
      hover.setAttribute("aria-hidden", "true");
      var node = mark.move ? mark.node : mark.node.cloneNode(true);
      node.removeAttribute("id");
      if (node.classList) node.classList.add("btn-morph__mark");
      node.setAttribute("aria-hidden", "true");
      hover.appendChild(node);
      btn.appendChild(hover);
      btn.classList.add("btn-morph--icon");
    } else {
      btn.classList.add("btn-morph--label");
    }
    btn.classList.add("btn-morph");
  }

  /* Tap morph: touch keeps the morph as a one-shot round trip, added in a
     click listener so it can never gate the click itself. */
  var TAP_CLASS = "is-tapmorph";
  function tapMorph(btn) {
    if (!btn || btn.classList.contains(TAP_CLASS)) return;
    btn.classList.add(TAP_CLASS);
    var clear = function () { btn.classList.remove(TAP_CLASS); };
    btn.addEventListener("animationend", clear, { once: true });
    window.setTimeout(clear, 700);
  }
  function onClick(event) {
    if (window.matchMedia && window.matchMedia("(hover: hover) and (pointer: fine)").matches) return;
    var target = event.target;
    if (!target || !target.closest) return;
    tapMorph(target.closest(".btn-morph"));
  }

  function start() {
    var nodes = document.querySelectorAll(SELECTOR);
    for (var i = 0; i < nodes.length; i++) prepare(nodes[i]);
    document.addEventListener("click", onClick, true);
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();
})();

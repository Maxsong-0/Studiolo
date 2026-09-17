(function () {
  "use strict";

  function initWordscroll(section) {
    if (!section || section.__droppyWordscroll) return;
    section.__droppyWordscroll = true;

    var stage = section.querySelector(".wordscroll__stage");
    var demo = section.querySelector(".wordscroll__demo");
    // The slot is usually the video itself (the homepage shelf demo). It can
    // also be a card that WRAPS a video, as on the creators page, where the
    // video sits inside a post card that links out, and there can be more than
    // one such card in the slot. Either way the section owns play/pause for
    // every media element, so an off-screen stage is never decoding frames.
    var videos = [];
    Array.prototype.forEach.call(
      section.querySelectorAll(".wordscroll__demo-video"),
      function (slot) {
        var found = slot.tagName === "VIDEO" ? slot : slot.querySelector("video");
        if (found) videos.push(found);
      }
    );
    var splitLayout = section.hasAttribute("data-wordscroll-split");
    // Drifting glyph marks behind a split-layout stage. The legacy selector is
    // the social license page's own class, kept so that page keeps working
    // untouched; new pages opt in with [data-wordscroll-marks] instead, so this
    // shared file stops naming one page's classes.
    var infinityMarks = splitLayout
      ? Array.prototype.slice.call(section.querySelectorAll(".social-benchmark__infinities span, [data-wordscroll-marks] span"))
      : [];
    var reduceMotion = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (!stage) return;

    if (videos.length && location.hostname !== "localhost" && location.hostname !== "127.0.0.1") {
      videos.forEach(function (video) {
        var remote = video.getAttribute("data-remote");
        if (remote) video.src = remote;
      });
    }

    var paragraphs = Array.prototype.slice.call(section.querySelectorAll("[data-wordsplit]"));
    var lits = [];
    var wraps = [];
    // The drifting glow follows the word "your" by default. Sections can
    // name extra words to light the same way with data-wordscroll-glow (a
    // space-separated list, matched case-insensitively on its first
    // occurrence) so a headline that earns two glows does not need another
    // paragraph to smuggle a second "your" into the section.
    var glow = Array.prototype.slice.call(
      (section.getAttribute("data-wordscroll-glow") || "your").split(/\s+/)
    ).map(function (word) { return word.replace(/[^a-z]/gi, "").toLowerCase(); });
    var hlEntries = [];
    paragraphs.forEach(function (paragraph) {
      var value = paragraph.textContent;
      var words = value.split(/\s+/).filter(Boolean);
      paragraph.textContent = "";
      words.forEach(function (word, index) {
        var wrap = document.createElement("span");
        wrap.className = "w";
        var dim = document.createElement("span");
        dim.className = "w-dim";
        dim.textContent = word;
        var lit = document.createElement("span");
        lit.className = "w-lit";
        lit.setAttribute("aria-hidden", "true");
        lit.textContent = word;
        wrap.appendChild(dim);
        wrap.appendChild(lit);
        var normalized = word.replace(/[^a-z]/gi, "").toLowerCase();
        if (glow.indexOf(normalized) !== -1 && !hlEntries.some(function (entry) { return entry.word === normalized; })) {
          hlEntries.push({
            word: normalized,
            index: lits.length,
            span: null,
            lastHighlight: -1,
          });
        }
        var entry = null;
        for (var e = hlEntries.length - 1; e >= 0; e -= 1) {
          if (hlEntries[e].index === lits.length && !hlEntries[e].span) {
            entry = hlEntries[e];
            break;
          }
        }
        if (entry) {
          entry.span = document.createElement("span");
          entry.span.className = "w-glo";
          entry.span.setAttribute("aria-hidden", "true");
          entry.span.textContent = word;
          wrap.appendChild(entry.span);
        }
        paragraph.appendChild(wrap);
        lits.push(lit);
        wraps.push(wrap);
        if (index < words.length - 1) paragraph.appendChild(document.createTextNode(" "));
      });
    });

    section.classList.add("ws-ready");

    if (reduceMotion) {
      section.classList.add("ws-reduced-motion");
      if (demo) demo.inert = false;
      videos.forEach(function (video) { video.setAttribute("controls", ""); });
      return;
    }

    if (demo) demo.inert = true;
    function clamp(value, min, max) { return Math.max(min, Math.min(max, value)); }

    var active = false;
    var ticking = false;
    var lastValues = new Array(lits.length).fill(-1);
    var variables = { enter: -1, glow: -1, copy: -1, demo: -1, bloom: -1, media: -1, "field-shift": -1 };
    function setVariable(name, value) {
      if (Math.abs(value - variables[name]) > 0.001) {
        stage.style.setProperty("--ws-" + name, value);
        variables[name] = value;
      }
    }

    var preloadHinted = false;
    var videoPlaying = false;
    function playDemo() {
      if (!videos.length || videoPlaying) return;
      videoPlaying = true;
      videos.forEach(function (video) {
        var promise = video.play();
        if (promise && promise.catch) promise.catch(function () { videoPlaying = false; });
      });
    }
    function pauseDemo() {
      if (!videos.length || !videoPlaying) return;
      videoPlaying = false;
      videos.forEach(function (video) { video.pause(); });
    }

    function update() {
      ticking = false;
      if (!active) return;

      var rect = section.getBoundingClientRect();
      var viewportHeight = window.innerHeight;
      var total = rect.height - viewportHeight;
      if (total <= 0) return;
      var progress = clamp(-rect.top / total, 0, 1);

      setVariable("enter", clamp(progress / 0.10, 0, 1));

      var startProgress = splitLayout ? 0.08 : 0.10;
      var endProgress = splitLayout ? 0.66 : 0.40;
      var count = lits.length;
      var span = endProgress - startProgress;
      var wordSpan = count > 1 ? (span / count) * 2.4 : span;
      for (var index = 0; index < count; index++) {
        var start = count > 1 ? startProgress + (span - wordSpan) * (index / (count - 1)) : startProgress;
        var value = clamp((progress - start) / wordSpan, 0, 1);
        if (Math.abs(value - lastValues[index]) > 0.001) {
          lits[index].style.opacity = value;
          var rest = (1 - value) * (1 - value);
          var translate = 0.12 * rest - 0.04 * Math.sin(Math.PI * value);
          wraps[index].style.transform = value >= 1 ? "" : "translateY(" + translate.toFixed(4) + "em)";
          lastValues[index] = value;
        }
      }

      var sweep = clamp((progress - startProgress) / (endProgress - startProgress), 0, 1);
      var copyValue = splitLayout ? 1 : 1 - clamp((progress - 0.47) / 0.08, 0, 1);
      var demoValue = splitLayout ? 1 : clamp((progress - 0.52) / 0.08, 0, 1);
      setVariable("glow", sweep);
      setVariable("copy", copyValue);
      setVariable("demo", demoValue);
      setVariable("bloom", splitLayout ? clamp(1 - Math.abs(progress - 0.48) / 0.42, 0, 1) : clamp(1 - Math.abs(progress - 0.53) / 0.11, 0, 1));
      if (splitLayout) {
        setVariable("media", progress);
        setVariable("field-shift", (progress - 0.5) * 9);
        infinityMarks.forEach(function (mark, index) {
          var group = Math.floor(index / 6);
          var start = [0, 0.18, 0.36, 0.56][group];
          var peak = start + 0.16;
          var end = Math.min(1, start + 0.4);
          var rise = clamp((progress - start) / (peak - start), 0, 1);
          var fall = 1 - clamp((progress - peak) / (end - peak), 0, 1);
          var phase = Math.min(rise, fall);
          var opacity = phase * (0.5 + (index % 4) * 0.08);
          var direction = index % 2 ? -1 : 1;
          var driftX = direction * (progress - peak) * 5;
          var driftY = -direction * (progress - peak) * 7;
          mark.style.opacity = opacity.toFixed(3);
          mark.style.transform = "translate3d(" + driftX.toFixed(2) + "px," + driftY.toFixed(2) + "px,0) scale(" + (0.92 + phase * 0.08).toFixed(3) + ")";
        });
      }

      var demoActive = splitLayout ? true : demoValue >= 0.99;
      stage.classList.toggle("ws-demo-active", demoActive);
      if (demo) demo.inert = !demoActive;

      if (hlEntries.length) {
        hlEntries.forEach(function (entry) {
          if (!entry.span) return;
          var highlightLit = count > 1
            ? startProgress + (span - wordSpan) * (entry.index / (count - 1)) + wordSpan
            : startProgress;
          var highlight = clamp(1 - Math.abs(progress - (highlightLit + 0.02)) / 0.05, 0, 1);
          if (Math.abs(highlight - entry.lastHighlight) > 0.001) {
            entry.span.style.opacity = highlight;
            entry.lastHighlight = highlight;
          }
        });
      }

      if (videos.length && !preloadHinted && progress > (splitLayout ? 0.02 : 0.25)) {
        preloadHinted = true;
        videos.forEach(function (video) { video.preload = "auto"; });
      }
      if (progress > (splitLayout ? 0.1 : 0.5)) playDemo(); else pauseDemo();
    }

    function requestUpdate() {
      if (!active || ticking) return;
      ticking = true;
      requestAnimationFrame(update);
    }

    if ("IntersectionObserver" in window) {
      var observer = new IntersectionObserver(function (entries) {
        entries.forEach(function (entry) {
          active = entry.isIntersecting;
          if (active) requestUpdate();
          else pauseDemo();
        });
      }, { rootMargin: "20% 0px" });
      observer.observe(section);
    } else {
      active = true;
    }

    document.addEventListener("visibilitychange", function () {
      if (document.hidden) pauseDemo();
      else requestUpdate();
    });
    window.addEventListener("scroll", requestUpdate, { passive: true });
    window.addEventListener("resize", requestUpdate, { passive: true });
    active = true;
    requestUpdate();
  }

  Array.prototype.slice.call(document.querySelectorAll("[data-wordscroll]")).forEach(initWordscroll);
})();

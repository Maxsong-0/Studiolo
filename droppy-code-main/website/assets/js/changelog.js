(function () {
  "use strict";

  var root = document.getElementById("changelog-root");
  if (!root) {
    return;
  }

  /* Long releases collapse each group past this many bullets behind a
     "Show more" toggle, so hundred-item releases stay compact. */
  var VISIBLE_ITEMS_PER_GROUP = 5;

  var GROUP_LABELS = {
    "new features": "New features",
    "bug fixes": "Bug fixes",
    "refinements": "Refinements",
    "updates": "Updates"
  };

  // Everything rendered from the fetched JSON goes through
  // `textContent` (never innerHTML), so there is no HTML-injection
  // surface for release note strings, no matter what they contain.

  function setState(kind, buildInner) {
    root.replaceChildren();
    root.setAttribute("aria-busy", kind === "loading" ? "true" : "false");
    var stateEl = document.createElement("div");
    stateEl.className = "changelog-state";
    stateEl.setAttribute("data-changelog-state", kind);
    buildInner(stateEl);
    root.appendChild(stateEl);
  }

  function showLoading() {
    setState("loading", function (el) {
      var spinner = document.createElement("div");
      spinner.className = "changelog-spinner";
      spinner.setAttribute("aria-hidden", "true");

      var text = document.createElement("p");
      text.className = "changelog-state__text";
      text.textContent = "Loading release notes...";

      el.appendChild(spinner);
      el.appendChild(text);
    });
  }

  function showError() {
    setState("error", function (el) {
      var title = document.createElement("p");
      title.className = "changelog-state__title";
      title.textContent = "Couldn't load the changelog";

      var text = document.createElement("p");
      text.className = "changelog-state__text";
      text.textContent = "Check your connection and try again.";

      var retry = document.createElement("button");
      retry.type = "button";
      retry.className = "btn btn--sm";
      retry.textContent = "Try again";
      retry.addEventListener("click", loadChangelog);

      el.appendChild(title);
      el.appendChild(text);
      el.appendChild(retry);
    });
  }

  function showEmpty() {
    setState("empty", function (el) {
      var title = document.createElement("p");
      title.className = "changelog-state__title";
      title.textContent = "No releases yet";

      var text = document.createElement("p");
      text.className = "changelog-state__text";
      text.textContent = "Check back soon, new releases will show up here first.";

      el.appendChild(title);
      el.appendChild(text);
    });
  }

  // Formats "2026-07-06" as "July 6, 2026" without shifting a day
  // across timezones (constructs the Date from local components
  // instead of parsing the ISO string directly). Falls back to the
  // raw string for anything that doesn't match.
  function formatDate(raw) {
    if (typeof raw !== "string") {
      return "";
    }
    var match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(raw.trim());
    if (!match) {
      return raw;
    }
    var year = Number(match[1]);
    var month = Number(match[2]);
    var day = Number(match[3]);
    var date = new Date(year, month - 1, day);
    if (isNaN(date.getTime())) {
      return raw;
    }
    try {
      return date.toLocaleDateString("en-US", {
        month: "long",
        day: "numeric",
        year: "numeric"
      });
    } catch (err) {
      return raw;
    }
  }

  // The name a human would use for a build: "1.3.0" -> "Droppy Code 1.3.0".
  function displayTitle(version) {
    var raw = String(version || "").trim();
    return raw ? "Droppy Code " + raw : "New release";
  }

  // The channel glyph, built as a real <svg> node because everything on
  // this page is assembled with createElement and never innerHTML. A
  // droplet, for the build everyone can just download: finished,
  // stable, ready. A teardrop on x=8, top at y=1.9, base spanning
  // 3.4..12.6 -- a single solid silhouette on the same 16-unit grid, so
  // at 11px on a coloured pill it stays legible.
  function dropletGlyph() {
    var ns = "http://www.w3.org/2000/svg";
    var svg = document.createElementNS(ns, "svg");
    svg.setAttribute("class", "changelog-entry__channel-glyph");
    svg.setAttribute("viewBox", "0 0 16 16");
    svg.setAttribute("aria-hidden", "true");
    svg.setAttribute("focusable", "false");
    var path = document.createElementNS(ns, "path");
    path.setAttribute("d", "M8 1.9C8 1.9 3.4 7.7 3.4 10.6a4.6 4.6 0 0 0 9.2 0C12.6 7.7 8 1.9 8 1.9Z");
    path.setAttribute("fill", "currentColor");
    svg.appendChild(path);
    return svg;
  }

  // Every entry on this page is a stable release, so every entry wears
  // the one blue pill: the droplet glyph and the Stable label.
  function channelBadge() {
    var badge = document.createElement("span");
    badge.className = "changelog-entry__channel changelog-entry__channel--stable";
    badge.appendChild(dropletGlyph());
    badge.appendChild(document.createTextNode("Stable"));
    return badge;
  }

  function groupLabel(title) {
    var key = String(title || "").trim().toLowerCase();
    return GROUP_LABELS[key] || (String(title || "").trim() || "Updates");
  }

  function buildGroup(section) {
    if (!section || typeof section !== "object") {
      return null;
    }
    var items = (Array.isArray(section.items) ? section.items : []).filter(function (item) {
      return item !== null && item !== undefined && item !== "";
    });
    if (!items.length) {
      return null;
    }

    var block = document.createElement("div");
    block.className = "changelog-group";

    var heading = document.createElement("h3");
    heading.className = "changelog-group__label";
    var labelText = groupLabel(section.title);
    var iconFile = ({
      "new features": "new-features",
      "bug fixes": "bug-fixes",
      "refinements": "refinements"
    })[String(labelText).toLowerCase()];
    if (iconFile) {
      var icon = document.createElement("img");
      icon.className = "changelog-group__icon";
      icon.src = "assets/images/changelog-icons/" + iconFile + ".png";
      icon.alt = "";
      icon.width = 15;
      icon.height = 15;
      heading.appendChild(icon);
    }
    heading.appendChild(document.createTextNode(labelText));
    block.appendChild(heading);

    var list = document.createElement("ul");
    list.className = "changelog-item-list";

    function appendItem(item) {
      var li = document.createElement("li");
      li.className = "changelog-item";
      li.textContent = String(item);
      list.appendChild(li);
    }

    var visible = items.slice(0, VISIBLE_ITEMS_PER_GROUP);
    var hidden = items.slice(VISIBLE_ITEMS_PER_GROUP);
    visible.forEach(appendItem);
    block.appendChild(list);

    if (hidden.length) {
      var more = document.createElement("button");
      more.type = "button";
      more.className = "changelog-more";
      more.textContent = "Show " + hidden.length + " more";
      more.addEventListener("click", function () {
        hidden.forEach(appendItem);
        more.remove();
      });
      block.appendChild(more);
    }

    return block;
  }

  function buildEntry(release, index) {
    var entry = document.createElement("article");
    entry.className = "changelog-entry reveal is-stable";
    entry.style.setProperty("--reveal-delay", Math.min(index * 70, 350) + "ms");

    /* Left column: date + display title, pinned while notes scroll. */
    var aside = document.createElement("div");
    aside.className = "changelog-entry__aside";
    var pin = document.createElement("div");
    pin.className = "changelog-entry__pin";

    var version = typeof release.version === "string" && release.version.trim() ? release.version.trim() : "";

    var formattedDate = formatDate(release.date);
    if (formattedDate) {
      var dateRow = document.createElement("p");
      dateRow.className = "changelog-entry__date";
      dateRow.appendChild(document.createTextNode(formattedDate));
      dateRow.appendChild(channelBadge());
      pin.appendChild(dateRow);
    }

    var title = document.createElement("h2");
    title.className = "changelog-entry__title";
    title.textContent = displayTitle(version);
    pin.appendChild(title);

    if (version) {
      var versionLine = document.createElement("p");
      versionLine.className = "changelog-entry__version";
      versionLine.textContent = version;
      pin.appendChild(versionLine);
    }

    var islot = document.createElement("div");
    islot.className = "changelog-entry__islot";
    islot.setAttribute("aria-hidden", "true");
    pin.appendChild(islot);

    aside.appendChild(pin);
    entry.appendChild(aside);

    /* Right column: short summary, then compact grouped notes. */
    var body = document.createElement("div");
    body.className = "changelog-entry__body";

    var summary = typeof release.summary === "string" ? release.summary.trim() : "";
    if (summary) {
      var summaryEl = document.createElement("p");
      summaryEl.className = "changelog-entry__summary";
      summaryEl.textContent = summary;
      body.appendChild(summaryEl);
    }

    var sections = Array.isArray(release.sections) ? release.sections : [];
    sections.forEach(function (section) {
      var block = buildGroup(section);
      if (block) {
        body.appendChild(block);
      }
    });

    entry.appendChild(body);
    return entry;
  }

  // Sorts newest first by version, numerically per segment, without
  // mutating the array returned by the fetch.
  function compareVersions(a, b) {
    var pa = String(a || "").split(".").map(Number);
    var pb = String(b || "").split(".").map(Number);
    var len = Math.max(pa.length, pb.length);
    for (var i = 0; i < len; i++) {
      var x = i < pa.length && !isNaN(pa[i]) ? pa[i] : 0;
      var y = i < pb.length && !isNaN(pb[i]) ? pb[i] : 0;
      if (x !== y) {
        return x - y;
      }
    }
    return 0;
  }

  function sortedReleases(releases) {
    return releases.slice().sort(function (a, b) {
      return compareVersions(b && b.version, a && a.version);
    });
  }

  function wireRevealOnScroll(elements) {
    if (!elements.length) {
      return;
    }
    if (!("IntersectionObserver" in window)) {
      elements.forEach(function (el) {
        el.classList.add("is-visible");
      });
      return;
    }
    // threshold: 0 fires as soon as the card starts entering the
    // viewport (not once some fraction of its total area is
    // visible), so very long release cards reveal immediately
    // instead of staying invisible until scrolled far into them.
    var observer = new IntersectionObserver(
      function (entries) {
        entries.forEach(function (entry) {
          if (entry.isIntersecting) {
            entry.target.classList.add("is-visible");
            observer.unobserve(entry.target);
          }
        });
      },
      { root: null, rootMargin: "0px 0px -5% 0px", threshold: 0 }
    );
    elements.forEach(function (el) {
      observer.observe(el);
    });
  }

  function renderReleases(data) {
    var releases = data && Array.isArray(data.releases) ? data.releases : [];
    if (!releases.length) {
      showEmpty();
      return;
    }

    var ordered = sortedReleases(releases);
    var fragment = document.createDocumentFragment();
    var cards = [];

    ordered.forEach(function (release, index) {
      if (!release || typeof release !== "object") {
        return;
      }
      var card = buildEntry(release, index);
      cards.push(card);
      fragment.appendChild(card);
    });

    if (!cards.length) {
      showEmpty();
      return;
    }

    root.replaceChildren();
    root.setAttribute("aria-busy", "false");
    root.appendChild(fragment);
    wireRevealOnScroll(cards);
    initEntryIcon(root);
  }

  /* One floating icon shared by every release. It lives inside the active
     entry's reserved slot, so it scrolls natively with the page (sticky
     included, zero chase lag on any device); only the hand-off between
     entries animates, as a decaying glide. With a single face there is no
     cross-dissolve to run: the icon just glides down the page. */
  function initEntryIcon(root) {
    var slots = Array.prototype.slice.call(root.querySelectorAll(".changelog-entry__islot"));
    if (!slots.length || document.querySelector(".clog-float-icon")) {
      return;
    }
    var entries = slots.map(function (slot) {
      var entry = slot.closest ? slot.closest(".changelog-entry") : null;
      return entry || slot;
    });
    var cv = document.createElement("div");
    cv.className = "clog-float-icon";
    cv.style.width = "84px";
    cv.style.height = "84px";
    cv.setAttribute("aria-hidden", "true");
    var face = document.createElement("img");
    face.className = "clog-float-icon__face is-front";
    face.src = "assets/brand/droppy-code-icon-256.webp";
    face.alt = "";
    face.decoding = "async";
    cv.appendChild(face);
    var reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

    function activeSlot() {
      var vh = window.innerHeight || 1;
      var line = vh * 0.42;
      var best = slots[0];
      var bestDist = Infinity;
      for (var i = 0; i < entries.length; i++) {
        var r = entries[i].getBoundingClientRect();
        if (r.top <= line && r.bottom > line) {
          return slots[i];
        }
        var d = r.top > line ? r.top - line : line - r.bottom;
        if (d < bestDist) {
          bestDist = d;
          best = slots[i];
        }
      }
      return best;
    }

    var current = activeSlot();
    current.appendChild(cv);
    requestAnimationFrame(function () {
      cv.classList.add("is-on");
    });

    var gliding = false;
    function handoff() {
      if (gliding) return;
      var next = activeSlot();
      if (next === current) return;
      var from = cv.getBoundingClientRect();
      current = next;
      next.appendChild(cv);
      if (reduceMotion) return;
      var to = cv.getBoundingClientRect();
      var dx = from.left - to.left;
      var dy = from.top - to.top;
      if (!dx && !dy) return;
      gliding = true;
      cv.style.transition = "none";
      cv.style.transform = "translate3d(" + dx + "px," + dy + "px,0)";
      requestAnimationFrame(function () {
        cv.style.transition = "transform 640ms cubic-bezier(0.22, 0.61, 0.36, 1)";
        cv.style.transform = "translate3d(0,0,0)";
      });
      window.setTimeout(function () {
        cv.style.transition = "none";
        gliding = false;
        handoff();
      }, 680);
    }

    var handoffFrame = 0;
    function requestHandoff() {
      if (handoffFrame) return;
      handoffFrame = requestAnimationFrame(function () {
        handoffFrame = 0;
        handoff();
      });
    }
    window.addEventListener("scroll", requestHandoff, { passive: true });
    window.addEventListener("resize", requestHandoff, { passive: true });
    requestHandoff();
  }

  function loadChangelog() {
    showLoading();

    fetch("changelog.json", { cache: "no-cache" })
      .then(function (response) {
        if (!response.ok) {
          throw new Error("Changelog request failed with status " + response.status);
        }
        return response.json();
      })
      .then(function (data) {
        renderReleases(data);
      })
      .catch(function (err) {
        console.error("[changelog] failed to load changelog.json:", err);
        showError();
      });
  }

  loadChangelog();
})();

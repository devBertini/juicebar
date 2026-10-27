/* Copyright (c) 2026 Claudio Bertini (devBertini)
   SPDX-License-Identifier: GPL-3.0-or-later */
/* JuiceBar harness: builds the desktop scene around the option's slots.
   Include it synchronously at the START of <body> (after glyphs.js and sample-state.js);
   the slot elements exist right after this script runs, so option scripts can fill them.
   Static markup: put <template data-slot="panel">...</template> anywhere in the body; its
   content is moved into the slot when the document is parsed. */
(function () {
  var html = document.documentElement;
  var theme = new URLSearchParams(location.search).get("theme") === "dark" ? "dark" : "light";
  html.setAttribute("data-theme", theme);
  html.style.colorScheme = theme;

  var body = document.body;
  var isMac = body.classList.contains("scene-mac");
  var isWin = body.classList.contains("scene-win");
  if (!isMac && !isWin) throw new Error("body needs class scene-mac or scene-win");

  function svg(inner, filled) {
    return '<svg class="hx-svg' + (filled ? "" : " s") + '" viewBox="0 0 16 16" aria-hidden="true">' + inner + "</svg>";
  }
  var wifi = svg('<path d="M1.5 6a9 9 0 0 1 13 0M3.7 8.4a5.8 5.8 0 0 1 8.6 0M6 10.8a2.6 2.6 0 0 1 4 0"/><circle cx="8" cy="13" r=".6" fill="currentColor"/>');
  var sample = window.JUICEBAR_SAMPLE;
  var clock = sample ? sample.clock : { mac: "seg. 26 de out.  14:32", winTime: "14:32", winDate: "26/10/2026" };
  var A = document.createElement("div"); // scene chrome, inserted before the option's own markup

  if (isMac) {
    A.innerHTML =
      '<div class="hx-wall"></div>' +
      '<div class="hx-menubar">' +
        '<div class="hx-mb-cell">' + svg('<circle cx="8" cy="8" r="5.5"/>') + "</div>" +
        '<div class="hx-mb-cell">' + svg('<rect x="2.5" y="2.5" width="11" height="11" rx="3"/>') + "</div>" +
        '<div class="hx-mb-cell">' + svg('<path d="M3 5h10M3 8h10M3 11h6"/>') + "</div>" +
        '<div id="bar-item"></div>' +
        '<div class="hx-mb-cell">' +
          '<svg class="hx-svg" style="width:25px" viewBox="0 0 25 16" aria-hidden="true"><rect x="1" y="3.5" width="20" height="9" rx="2.6" fill="none" stroke="currentColor" opacity=".55"/><rect x="2.6" y="5.1" width="12.5" height="5.8" rx="1.3" fill="currentColor"/><path d="M22.5 6.3v3.4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity=".55"/></svg>' +
        "</div>" +
        '<div class="hx-mb-cell">' + wifi + "</div>" +
        '<div class="hx-mb-cell">' + svg('<rect x="1.5" y="3.5" width="13" height="3.6" rx="1.8"/><circle cx="5" cy="5.3" r="1" fill="currentColor"/><rect x="1.5" y="8.9" width="13" height="3.6" rx="1.8"/><circle cx="11" cy="10.7" r="1" fill="currentColor"/>') + "</div>" +
        '<div class="hx-mb-clock">' + clock.mac + "</div>" +
      "</div>" +
      '<div id="widget"></div><div id="notification"></div><div id="panel"></div>' +
      '<div class="hx-caption">Mac · ' + theme + " · fonte substituta do SF Pro</div>";
  } else {
    var app = function (on) { return '<div class="hx-tb-app' + (on ? " on" : "") + '"><i></i></div>'; };
    A.innerHTML =
      '<div class="hx-wall"></div>' +
      '<div class="hx-taskbar">' +
        '<div class="hx-tb-center">' +
          '<div class="hx-tb-app"><svg class="hx-svg" style="width:20px;height:20px" viewBox="0 0 16 16" aria-hidden="true"><path fill="#3b9bff" d="M1 1h6.5v6.5H1zM8.5 1H15v6.5H8.5zM1 8.5h6.5V15H1zM8.5 8.5H15V15H8.5z"/></svg></div>' +
          app(false) + app(true) + app(true) + app(false) +
        "</div>" +
        '<div class="hx-tb-right">' +
          '<div class="hx-tb-chev">' + svg('<path d="M4 10l4-4 4 4"/>') + "</div>" +
          '<div id="tray"></div>' +
          '<div class="hx-tb-sys">' + wifi +
            svg('<path d="M2 6h2.5L8 3v10L4.5 10H2z"/><path d="M10.5 5.8a3.2 3.2 0 0 1 0 4.4M12.2 4a5.6 5.6 0 0 1 0 8"/>') +
            '<svg class="hx-svg" style="width:18px" viewBox="0 0 18 16" aria-hidden="true"><rect x="1" y="4" width="14" height="8" rx="2" fill="none" stroke="currentColor" stroke-width="1.4"/><rect x="2.8" y="5.8" width="8" height="4.4" rx=".8" fill="currentColor"/><path d="M16.6 6.8v2.4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>' +
          "</div>" +
          '<div class="hx-tb-clock"><span>' + clock.winTime + "</span><span>" + clock.winDate + "</span></div>" +
          '<div class="hx-tb-bell">' + svg('<path d="M4 11.5h8l-1-1.6V7a3 3 0 0 0-6 0v2.9z"/><path d="M6.8 13.5a1.4 1.4 0 0 0 2.4 0"/>') + "</div>" +
        "</div>" +
        '<div class="hx-caption">Windows · ' + theme + "</div>" +
      "</div>" +
      '<div id="toast"></div><div id="flyout"></div>';
  }
  while (A.firstChild) body.insertBefore(A.firstChild, body.firstChild);

  function slot(id) { return document.getElementById(id); }

  // Anchor the open panel / flyout to its bar item / tray: centered on the anchor, kept 8-12 px inside the scene.
  function reposition() {
    var W = isMac ? 1200 : 1600;
    var anchor = isMac ? slot("bar-item") : slot("tray");
    var pop = isMac ? slot("panel") : slot("flyout");
    if (!anchor || !pop) return;
    var r = anchor.getBoundingClientRect();
    var cx = isMac ? r.left + r.width / 2 : r.right - 16; // Windows: the last tray cell
    var margin = isMac ? 8 : 12;
    var left = Math.min(Math.max(cx - pop.offsetWidth / 2, margin), W - margin - pop.offsetWidth);
    pop.style.left = Math.round(left) + "px";
  }

  function flushTemplates() {
    var ts = document.querySelectorAll("template[data-slot]");
    for (var i = 0; i < ts.length; i++) {
      var target = slot(ts[i].getAttribute("data-slot"));
      if (target) target.appendChild(ts[i].content.cloneNode(true));
    }
    reposition();
  }
  document.addEventListener("DOMContentLoaded", flushTemplates);
  window.addEventListener("load", function () {
    reposition();
    if (window.ResizeObserver) {
      var ro = new ResizeObserver(reposition);
      [isMac ? "bar-item" : "tray", isMac ? "panel" : "flyout"].forEach(function (id) { if (slot(id)) ro.observe(slot(id)); });
    }
    html.setAttribute("data-ready", "1");
  });

  window.JuiceBarFrame = { theme: theme, platform: isMac ? "mac" : "win", slot: slot, reposition: reposition };
})();

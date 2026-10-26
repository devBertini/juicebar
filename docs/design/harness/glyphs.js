/* JuiceBar harness: inline SVG sprite (our own drawings, 24x24, line style).
   Use:  <svg class="g"><use href="#g-mouse"/></svg>   or   JuiceBarGlyph("g-mouse", 18)
   Style them from CSS on svg.g (fill, stroke, stroke-width); the defaults sit below. */
(function () {
  var teeth = [0, 45, 90, 135, 180, 225, 270, 315].map(function (a) {
    return '<path d="M12 2.5v2" transform="rotate(' + a + ' 12 12)"/>';
  }).join("");
  var ids = {
    "g-mouse": '<path d="M12 3c-3.3 0-5.5 2.2-5.5 5.5v7c0 3.3 2.2 5.5 5.5 5.5s5.5-2.2 5.5-5.5v-7C17.5 5.2 15.3 3 12 3z"/><path d="M6.5 11h11"/><path d="M12 3v2.5"/><rect x="10.75" y="5.5" width="2.5" height="4.5" rx="1.25"/>',
    "g-keyboard": '<rect x="2.5" y="5.5" width="19" height="13" rx="2.5"/><path d="M6 9h.01M9 9h.01M12 9h.01M15 9h.01M18 9h.01M6.5 12h.01M9.5 12h.01M12.5 12h.01M15.5 12h.01M17.5 12h.01M8 15.5h8"/>',
    "g-headset": '<path d="M4.5 14v-2a7.5 7.5 0 0 1 15 0v2"/><rect x="3" y="13" width="4" height="6.5" rx="1.75"/><rect x="17" y="13" width="4" height="6.5" rx="1.75"/><path d="M5 19.5c0 1.6 1.4 2 3 2h3.5"/>',
    "g-bolt": '<path d="M13 2.5L5.5 13.5H11l-1 8 7.5-11H12l1-8z"/>',
    "g-bluetooth": '<path d="M7 7.5l10 9-5 4.5V3l5 4.5-10 9"/>',
    "g-wireless": '<circle cx="12" cy="18" r="1"/><path d="M8.5 14.5a5 5 0 0 1 7 0"/><path d="M5.5 11.5a9 9 0 0 1 13 0"/><path d="M2.5 8.5a13 13 0 0 1 19 0"/>',
    "g-warning": '<path d="M12 3.5l9.5 16.5h-19z"/><path d="M12 10v4.5"/><path d="M12 17.5h.01"/>',
    "g-gear": '<circle cx="12" cy="12" r="6.5"/><circle cx="12" cy="12" r="2.5"/>' + teeth,
    "g-leaf": '<path d="M19.5 4.5c-9 0-14 4-14 10.5 0 1.4.3 2.7 1 3.8 1.2.8 2.6 1.2 4 1.2 6 0 9-5 9-15.5z"/><path d="M5.5 20.5C8 15 11 12 15 9.5"/>',
    "g-gamepad": '<path d="M7.5 7h9c2.8 0 4.6 2.3 5 5.5l.4 3.5c.2 1.9-1.1 3.5-2.8 3.5-1.1 0-2-.6-2.5-1.6L15.8 16H8.2l-.8 1.9c-.5 1-1.4 1.6-2.5 1.6-1.7 0-3-1.6-2.8-3.5l.4-3.5C2.9 9.3 4.7 7 7.5 7z"/><path d="M7.5 10.5v3M6 12h3"/><path d="M15.5 11h.01M17.5 13h.01"/>',
    "g-earbuds": '<path d="M7 4.5a3 3 0 0 1 3 3v2.5a3 3 0 0 1-2 2.8V19a1.5 1.5 0 0 1-3 0v-9.5a5 5 0 0 1 2-5z"/><path d="M17 4.5a3 3 0 0 0-3 3v2.5a3 3 0 0 0 2 2.8V19a1.5 1.5 0 0 0 3 0v-9.5a5 5 0 0 0-2-5z"/>'
  };
  var symbols = Object.keys(ids).map(function (id) {
    return '<symbol id="' + id + '" viewBox="0 0 24 24">' + ids[id] + "</symbol>";
  }).join("");
  var html = '<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0" style="position:absolute" aria-hidden="true"><defs>' + symbols + "</defs></svg>" +
    "<style>svg.g{width:1em;height:1em;display:block;flex:none;fill:none;stroke:currentColor;stroke-width:1.75;stroke-linecap:round;stroke-linejoin:round}</style>";
  function inject() { document.body.insertAdjacentHTML("afterbegin", html); }
  if (document.body) inject(); else document.addEventListener("DOMContentLoaded", inject);
  window.JuiceBarGlyph = function (id, size) {
    var s = size ? ' style="width:' + size + "px;height:" + size + 'px"' : "";
    return '<svg class="g"' + s + ' aria-hidden="true"><use href="#' + id + '"/></svg>';
  };
  window.JuiceBarGlyphIds = Object.keys(ids);
})();

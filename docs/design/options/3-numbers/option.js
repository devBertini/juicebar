/* Copyright (c) 2026 Claudio Bertini (devBertini)
   SPDX-License-Identifier: GPL-3.0-or-later */
(function () {
  var S = window.JUICEBAR_SAMPLE, F = JuiceBarFrame, mac = F.platform === "mac";
  var devs = S.devices.filter(function (d) { return !d.platforms || d.platforms.indexOf(F.platform) >= 0; });
  var byId = {}; S.devices.forEach(function (d) { byId[d.id] = d; });
  function g(id) { return '<svg class="g"><use href="#' + id + '"/></svg>'; }
  function cls(d) { return d.state === "off" ? "dim" : d.level <= S.lowThreshold ? "low" : d.state === "charging" ? "chg" : ""; }
  function numHtml(d) { return d.level + "<sup>%</sup>"; }
  function fc(d) {
    if (d.state === "off") return "desligado · <b>" + d.lastSeen + "</b>";
    if (d.state === "charging") return "cheio às <b>" + d.etaClock + "</b>";
    if (d.id === "xbox") return "acaba qua <b>" + d.etaClock + "</b>";
    return "acaba hoje <b>" + d.etaClock + "</b>";
  }
  function chart(d) {
    var W = 340, H = 46, x0 = -168, x1 = 6;
    function X(h) { return (h <= 0 ? (h - x0) / (0 - x0) * 292 : 292 + h / 3 * 44).toFixed(1); }
    function Y(v) { return (H - 3 - v / 100 * (H - 8)).toFixed(1); }
    var p = d.history.map(function (q, i) { return (i ? "L" : "M") + X(q[0]) + " " + Y(q[1]); }).join(" ");
    var fcast = "M" + X(0) + " " + Y(d.level) + " L" + X(3) + " " + Y(0);
    var lo = Y(S.lowThreshold);
    return '<svg class="chart" viewBox="0 0 ' + W + " " + H + '" preserveAspectRatio="none">' +
      '<line x1="0" x2="' + W + '" y1="' + lo + '" y2="' + lo + '" stroke="var(--hair)" stroke-width="1" stroke-dasharray="1 3"/>' +
      '<path d="' + p + '" fill="none" stroke="var(--ink)" stroke-width="1" vector-effect="non-scaling-stroke"/>' +
      '<path d="' + fcast + '" fill="none" stroke="var(--red)" stroke-width="1" stroke-dasharray="3 3" vector-effect="non-scaling-stroke"/>' +
      '<circle cx="' + X(0) + '" cy="' + Y(d.level) + '" r="2.2" fill="var(--red)"/></svg>';
  }
  function block(d) {
    var c = cls(d), hero = d.id === "g703";
    var conn = d.on
      ? '<span class="cn">' + g(d.connectionGlyph) + d.connection + (d.state === "charging" ? '<span class="chg">' + g("g-bolt") + "</span>" : "") + "</span>"
      : '<span class="cn">sem conexão</span>';
    var h = '<div class="dev' + (hero ? " hero" : "") + '"><div class="top">' + g(d.glyph) + '<span class="nm">' + d.name + "</span>" + conn + "</div>" +
      '<div class="row"><div class="num ' + c + '">' + numHtml(d) + '</div><div class="fc">' + fc(d) + "</div></div>";
    if (hero) {
      h += chart(d) + '<div class="facts"><span>Hoje <b>' + d.usageToday + ' de uso</b></span><span>Saúde <b>' + d.health + "%</b></span></div>" +
        '<div class="warn">' + d.warning + "</div>";
    }
    return h + "</div>";
  }
  function sheet() {
    var hk = mac ? S.hotkey.mac : S.hotkey.win;
    return '<div class="nb nb-surface sheet ' + (mac ? "" : "win") + '">' + devs.map(block).join("") +
      '<div class="foot"><span class="lf">' + g("g-leaf") + 'Economia de RGB</span><span class="tg"></span><span class="sp gear">' + g("g-gear") + '</span><span class="hk">' + hk + "</span></div></div>";
  }
  function noteHtml(n, kind, big, bigCls, when) {
    return '<div class="nb nb-surface note ' + kind + '"><div class="big num ' + bigCls + '">' + big + '</div><div class="tx"><div class="app">' + g("g-bolt") + "JuiceBar" + (when ? '<span class="when">' + when + "</span>" : "") +
      '</div><div class="ti">' + n.title + '</div><div class="bd">' + n.body + "</div></div></div>";
  }

  if (mac) {
    F.slot("bar-item").innerHTML = '<div class="nb bar">' + S.pinned.map(function (id) {
      var d = byId[id];
      return '<span class="it ' + cls(d) + '">' + g(d.glyph) + "<span>" + (d.state === "off" ? "–" : d.level) + "</span>" +
        (d.state === "charging" ? '<svg class="g bolt"><use href="#g-bolt"/></svg>' : "") + "</span>";
    }).join("") + "</div>";
    F.slot("panel").innerHTML = sheet();
    var cols = S.pinned.map(function (id) {
      var d = byId[id], s = d.state === "off" ? "desligado" : d.state === "charging" ? "cheio às " + d.etaClock : "acaba às " + d.etaClock;
      return '<div class="col"><div class="l">' + g(d.glyph) + d.name + '</div><div class="num ' + cls(d) + '">' + numHtml(d) + '</div><div class="s">' + s + "</div></div>";
    }).join("");
    var oth = devs.filter(function (d) { return S.pinned.indexOf(d.id) < 0; }).map(function (d) { return "<span>" + d.name + "<b>" + d.level + "%</b></span>"; }).join("");
    F.slot("widget").innerHTML = '<div class="nb nb-surface widget"><div class="cols">' + cols + '</div><div class="others">' + oth + "</div></div>";
    F.slot("notification").innerHTML = noteHtml(S.notifications.warning, "mac", "23<sup>%</sup>", "low", "agora");
  } else {
    F.slot("tray").innerHTML = "";
    S.pinned.forEach(function (id) {
      var d = byId[id]; if (!d.on) return;
      var el = document.createElement("div"); el.className = "tico " + cls(d);
      el.innerHTML = '<span class="d">' + d.level + '</span><span class="u"><i style="width:' + d.level + '%"></i></span>';
      F.slot("tray").appendChild(el);
    });
    F.slot("flyout").innerHTML = sheet();
    F.slot("toast").innerHTML = noteHtml(S.notifications.full, "wn", "100<sup>%</sup>", "chg", "");
  }
  F.reposition();
})();

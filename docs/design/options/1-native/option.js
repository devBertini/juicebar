/* Copyright (c) 2026 Claudio Bertini (devBertini)
   SPDX-License-Identifier: GPL-3.0-or-later */
// JuiceBar option 1 Native: one script for both scenes, branches on JuiceBarFrame.platform.
(function () {
  var S = window.JUICEBAR_SAMPLE, F = JuiceBarFrame, G = window.JuiceBarGlyph;
  var mac = F.platform === "mac";
  var devs = S.devices.filter(function (d) { return !d.platforms || d.platforms.indexOf(F.platform) >= 0; });
  var low = function (d) { return d.state !== "off" && d.level <= S.lowThreshold; };
  var tone = function (d) { return d.state === "off" ? "off" : low(d) ? "low" : d.state === "charging" ? "chg" : "on"; };

  function sub(d) {
    if (d.state === "off") return "desligado · " + d.lastSeen;
    return d.forecast;
  }
  function conn(d) { return d.connection || ""; }

  function row(d, open) {
    var t = tone(d);
    return '<div class="row t-' + t + (open ? " open" : "") + '">' +
      '<div class="ico">' + G(d.glyph, 17) + '</div>' +
      '<div class="mid">' +
        '<div class="l1"><span class="nm">' + d.name + '</span><span class="cn">' + conn(d) + '</span></div>' +
        '<div class="cap"><i style="width:' + d.level + '%"></i></div>' +
        '<div class="l3">' + (d.state === "charging" ? G("g-bolt", 11) : "") + '<span>' + sub(d) + '</span></div>' +
      '</div>' +
      '<div class="pc">' + d.level + '<small>%</small></div>' +
    '</div>';
  }

  function chart(d) {
    var W = 316, H = 96, padL = 26, padR = 6, padT = 6, padB = 16;
    var x0 = -168, x1 = 6;
    var iw = W - padL - padR, ih = H - padT - padB;
    var X = function (h) { return padL + (h - x0) / (x1 - x0) * iw; };
    var Y = function (p) { return padT + (1 - p / 100) * ih; };
    var pts = d.history.map(function (p) { return X(p[0]).toFixed(1) + "," + Y(p[1]).toFixed(1); });
    var line = "M" + pts.join("L");
    var area = line + "L" + X(0).toFixed(1) + "," + Y(0) + "L" + X(-168).toFixed(1) + "," + Y(0) + "Z";
    var grid = [0, 50, 100].map(function (p) {
      return '<line class="gl" x1="' + padL + '" x2="' + (W - padR) + '" y1="' + Y(p) + '" y2="' + Y(p) + '"/><text class="ax" x="' + (padL - 5) + '" y="' + (Y(p) + 3) + '" text-anchor="end">' + p + '</text>';
    }).join("");
    var days = ["seg", "ter", "qua", "qui", "sex", "sáb", "dom"].map(function (n, i) {
      return '<text class="ax" x="' + X(-168 + 12 + i * 24).toFixed(1) + '" y="' + (H - 3) + '" text-anchor="middle">' + n + '</text>';
    }).join("");
    var th = '<line class="th" x1="' + padL + '" x2="' + (W - padR) + '" y1="' + Y(S.lowThreshold) + '" y2="' + Y(S.lowThreshold) + '"/>';
    return '<svg class="chart" viewBox="0 0 ' + W + ' ' + H + '" width="' + W + '" height="' + H + '">' +
      '<defs><linearGradient id="ag" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="var(--accent)" stop-opacity=".34"/><stop offset="1" stop-color="var(--accent)" stop-opacity="0"/></linearGradient></defs>' +
      grid + th + '<path d="' + area + '" fill="url(#ag)"/><path class="ln" d="' + line + '"/>' +
      '<line class="fc" x1="' + X(0).toFixed(1) + '" y1="' + Y(d.level).toFixed(1) + '" x2="' + X(3.1).toFixed(1) + '" y2="' + Y(0) + '"/>' +
      '<circle class="nowdot" cx="' + X(0).toFixed(1) + '" cy="' + Y(d.level).toFixed(1) + '" r="3"/>' + days + '</svg>';
  }

  function detail(d) {
    return '<div class="detail">' +
      '<div class="warn">' + G("g-warning", 14) + '<span>' + d.warning + '</span></div>' +
      '<div class="chartbox"><div class="chtitle"><span>Últimos 7 dias</span><span class="fcl"><b></b>previsão</span></div>' + chart(d) + '</div>' +
      '<div class="stats"><div><span>Hoje</span><b>' + d.usageToday + ' de uso</b></div><div><span>Saúde da bateria</span><b>' + (d.health != null ? d.health + "%" : "aprendendo") + '</b></div></div>' +
    '</div>';
  }

  function sheet() {
    var on = devs.filter(function (d) { return d.state !== "off"; });
    var off = devs.filter(function (d) { return d.state === "off"; });
    var h = '<div class="sheet"><div class="head"><span class="ttl">' + S.appName + '</span><span class="gear">' + G("g-gear", 17) + '</span></div><div class="list">';
    on.forEach(function (d) {
      h += d.warning ? '<div class="card">' + row(d, true) + detail(d) + '</div>' : row(d);
    });
    if (off.length) {
      h += '<div class="sep"><span>Desligados</span></div>';
      off.forEach(function (d) { h += row(d); });
    }
    var sw = '<span class="sw' + (S.rgbEconomy ? " on" : "") + '"><i></i></span>';
    h += '</div><div class="foot"><div class="rgb"><span class="fi">' + G("g-leaf", 15) + '</span><span>Economia de RGB</span>' + sw + '</div>' +
      '<kbd>' + (mac ? S.hotkey.mac : S.hotkey.win) + '</kbd></div></div>';
    return h;
  }

  function mini(d) {
    return '<div class="mrow t-' + tone(d) + '"><span class="mi">' + G(d.glyph, 14) + '</span><span class="mn">' + d.name + '</span>' +
      '<span class="cap"><i style="width:' + d.level + '%"></i></span><span class="mp">' + d.level + '%</span></div>';
  }

  function byId(id) { return devs.filter(function (d) { return d.id === id; })[0]; }

  if (mac) {
    var pinned = S.pinned.map(byId);
    F.slot("bar-item").innerHTML = '<div class="bi">' + pinned.map(function (d) {
      var t = tone(d);
      return '<span class="bc t-' + t + '">' + G(d.glyph, 14) + (t === "chg" ? '<em>' + G("g-bolt", 7) + '</em>' : "") + '<i class="lv"><b style="width:' + d.level + '%"></b></i></span>';
    }).join("") + '</div>';
    F.slot("panel").innerHTML = sheet();
    var n = devs.filter(function (d) { return d.state !== "off"; }).length;
    F.slot("widget").innerHTML = '<div class="wd"><div class="wh"><span>' + S.appName + '</span><span class="wc">' + n + ' ligados</span></div>' +
      devs.map(mini).join("") + '</div>';
    var nt = S.notifications.warning;
    F.slot("notification").innerHTML = '<div class="nt"><div class="app">' + G("g-bolt", 22) + '</div><div class="tx"><div class="r1"><b>' + nt.title + '</b><span>agora</span></div><div class="r2">' + nt.body + '</div></div></div>';
  } else {
    var bars = S.pinned.map(byId).map(function (d, i) {
      var x = 2.5 + i * 4.5, hh = Math.max(1.5, 12 * d.level / 100);
      var cls = low(d) ? "lowb" : d.state === "off" ? "offb" : "";
      return '<rect class="trk" x="' + x + '" y="2" width="3" height="12" rx="1.5"/><rect class="fil ' + cls + '" x="' + x + '" y="' + (14 - hh) + '" width="3" height="' + hh + '" rx="1.5"/>';
    }).join("");
    F.slot("tray").innerHTML = '<div class="tri"><svg width="16" height="16" viewBox="0 0 16 16">' + bars + '</svg></div>';
    F.slot("flyout").innerHTML = sheet();
    var nf = S.notifications.full;
    F.slot("toast").innerHTML = '<div class="ts"><div class="tapp"><span class="tic">' + G("g-bolt", 10) + '</span><span>' + S.appName + '</span><span class="tdots">···</span></div>' +
      '<div class="tbody"><div class="tbi">' + G("g-keyboard", 24) + '</div><div><div class="t1">' + nf.title + '</div><div class="t2">' + nf.body + '</div></div></div></div>';
  }
  F.reposition();
})();

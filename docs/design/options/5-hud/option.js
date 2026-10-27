/* Copyright (c) 2026 Claudio Bertini (devBertini)
   SPDX-License-Identifier: GPL-3.0-or-later */
(function () {
  var S = window.JUICEBAR_SAMPLE, F = JuiceBarFrame, win = F.platform === "win";
  var devs = S.devices.filter(function (d) { return !d.platforms || d.platforms.indexOf(F.platform) >= 0; });
  var low = function (d) { return d.level <= S.lowThreshold; };
  function g(id) { return '<svg class="g"><use href="#' + id + '"/></svg>'; }
  function seg(d, n) {
    var lit = Math.ceil(d.level / 100 * n), h = "";
    for (var i = 0; i < n; i++) h += "<i" + (i < lit ? ' class="on"' : "") + "></i>";
    var c = d.state === "off" ? "off" : d.state === "charging" ? "chg" : low(d) ? "low" : "";
    return '<div class="seg ' + c + '">' + h + "</div>";
  }
  function pad(n) { return (n < 10 ? "0" : "") + n; }
  function stat(d) {
    if (d.state === "off") return "<span>DESLIGADO</span><span>ÚLTIMO <b>" + d.lastSeen.replace(" às ", " ") + "</b></span>";
    if (d.state === "charging") return "<span>CHEIO EM <b>01:10</b></span>";
    var h = parseInt(d.forecast.replace(/\D+/g, ""), 10);
    return "<span>AUTONOMIA <b>" + pad(h) + "H</b></span>";
  }
  function row(d) {
    var cls = "dev " + (d.state === "off" ? "off" : "live") + (low(d) ? " low" : "") + (d.state === "charging" ? " chg" : "");
    var conn = d.connection ? '<span class="cn">' + g(d.connectionGlyph) + d.connection + "</span>" : "";
    return '<div class="' + cls + '"><div class="tile">' + g(d.glyph) + '</div><div><div class="l1"><span class="nm">' + d.name + "</span>" + conn +
      '<span class="pc mono">' + (d.state === "charging" ? g("g-bolt") : "") + d.level + "<small>%</small></span></div>" + seg(d, 10) +
      '<div class="l3 mono">' + stat(d) + "</div></div></div>";
  }
  function chart(d) {
    var W = 332, H = 68, x0 = -168, x1 = 6;
    var X = function (h) { return (h - x0) / (x1 - x0) * W; }, Y = function (p) { return H - p / 100 * (H - 4) - 2; };
    var pts = d.history.map(function (p) { return X(p[0]).toFixed(1) + "," + Y(p[1]).toFixed(1); });
    var line = "M" + pts.join(" L"), area = line + " L" + X(0) + "," + H + " L0," + H + " Z";
    var fx = X(3.15), fy = Y(0), nx = X(0), ny = Y(d.level);
    return '<svg viewBox="0 0 ' + W + " " + H + '" preserveAspectRatio="none"><defs><linearGradient id="ar" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="var(--acc)" stop-opacity=".30"/><stop offset="1" stop-color="var(--acc)" stop-opacity="0"/></linearGradient></defs>' +
      '<line x1="0" x2="' + W + '" y1="' + Y(20) + '" y2="' + Y(20) + '" stroke="var(--acc2)" stroke-opacity=".5" stroke-dasharray="2 3" stroke-width="1"/>' +
      '<line x1="0" x2="' + W + '" y1="' + Y(100) + '" y2="' + Y(100) + '" stroke="var(--off)" stroke-width="1"/>' +
      '<path d="' + area + '" fill="url(#ar)"/><path d="' + line + '" fill="none" stroke="var(--acc)" stroke-width="1.5" stroke-linejoin="round" style="filter:drop-shadow(0 0 3px var(--acc))" vector-effect="non-scaling-stroke"/>' +
      '<path d="M' + nx + "," + ny + " L" + fx + "," + fy + '" stroke="var(--acc2)" stroke-width="1.6" stroke-dasharray="4 3" fill="none"/>' +
      '<circle cx="' + nx + '" cy="' + ny + '" r="3" fill="var(--acc2)"/><circle cx="' + fx + '" cy="' + fy + '" r="2" fill="none" stroke="var(--acc2)" stroke-width="1.2"/></svg>' +
      '<div class="ax mono"><span>-7 DIAS</span><span>AGORA</span><span>' + d.etaClock + "</span></div>";
  }
  function detail(d) {
    return '<div class="detail">' + chart(d) + '<div class="read mono"><div><span>USO HOJE</span><b>' + d.usageToday.replace(" min", "m").replace(" h ", "h ") + "</b></div><div><span>SAÚDE</span><b>" + d.health +
      "%</b></div><div><span>AUTONOMIA</span><b>03H</b></div></div></div>";
  }
  function card() {
    var h = '<div class="hud ' + F.platform + '"><div class="card"><div class="top"><span class="logo"><i style="height:6px"></i><i style="height:10px"></i><i style="height:14px"></i></span><b>JuiceBar</b><span class="sp"></span><span class="tm mono">' + S.now.time + "</span></div>" +
      '<div class="alert">' + g("g-warning") + "<span><b>G703 23%</b> " + S.notifications.warning.body + "</span></div>";
    devs.forEach(function (d) { h += row(d); if (d.id === "g703") h += detail(d); });
    h += '<div class="foot"><div class="sw">' + g("g-leaf") + 'Economia de RGB<span class="trk"></span></div><span class="sp"></span><span class="key mono">' +
      (win ? S.hotkey.win : S.hotkey.mac) + '</span><span class="gear">' + g("g-gear") + "</span></div></div></div>";
    return h;
  }
  function nicon() { return '<div class="nicon"><i class="a"></i><i class="b"></i><i class="c"></i></div>'; }
  var ps = S.pinned.map(function (id) { return devs.filter(function (d) { return d.id === id; })[0]; });

  if (!win) {
    F.slot("bar-item").innerHTML = '<div class="hud bi">' + ps.map(function (d) {
      var c = d.state === "off" ? "off" : low(d) ? "low" : "";
      return '<div class="m ' + c + '">' + g(d.glyph) + seg(d, 5) + (d.state === "charging" ? '<svg class="g bolt"><use href="#g-bolt"/></svg>' : "") + "</div>";
    }).join("") + "</div>";
    F.slot("panel").innerHTML = card();
    var tiles = devs.map(function (d) {
      var c = "wt" + (d.state === "off" ? " off" : "") + (low(d) ? " low" : "") + (d.state === "charging" ? " chg" : "");
      return '<div class="' + c + '"><div class="r">' + g(d.glyph) + (d.state === "charging" ? '<svg class="g bl"><use href="#g-bolt"/></svg>' : "") + '<b class="mono">' + d.level + "%</b></div>" + seg(d, 10) + "</div>";
    }).join("");
    tiles += '<div class="wt sum mono"><span>G703 ACABA</span><b>~' + devs[0].etaClock + "</b></div>";
    F.slot("widget").innerHTML = '<div class="hud wg"><div class="hd"><b>JuiceBar</b><span class="mono">5 DISPOSITIVOS</span></div><div class="grid">' + tiles + "</div></div>";
    F.slot("notification").innerHTML = '<div class="hud nt">' + nicon() + '<div><div class="t1"><span style="margin:0">JuiceBar</span><span>agora</span></div><div class="t2">' + S.notifications.warning.title + '</div><div class="t3">' + S.notifications.warning.body + "</div></div></div>";
  } else {
    var svg = '<svg width="16" height="16" viewBox="0 0 16 16" shape-rendering="crispEdges">';
    ps.forEach(function (d, r) {
      var lit = Math.ceil(d.level / 100 * 5), y = 2 + r * 4.5;
      for (var i = 0; i < 5; i++) {
        var on = i < lit, x = 1 + i * 3;
        if (d.state === "off") svg += '<rect x="' + (x + 0.5) + '" y="' + (y + 0.5) + '" width="2" height="2" fill="none" stroke="' + (on ? "var(--mut)" : "var(--off)") + '" stroke-width="1"/>';
        else svg += '<rect x="' + x + '" y="' + y + '" width="2" height="3" fill="' + (on ? (low(d) ? "var(--acc2)" : "var(--acc)") : "var(--mut)") + '" fill-opacity="' + (on ? 1 : 0.35) + '"/>';
      }
    });
    F.slot("tray").innerHTML = '<div class="hud">' + svg + "</svg></div>";
    F.slot("flyout").innerHTML = card();
    F.slot("toast").innerHTML = '<div class="hud tt"><div class="ap"><svg class="g" style="width:14px;height:14px;color:var(--acc)"><use href="#g-bolt"/></svg>JuiceBar<span class="x">...</span></div><div class="bd">' + nicon() +
      '<div><div class="t2">' + S.notifications.full.title + '</div><div class="t3">' + S.notifications.full.body + '</div></div></div><div class="bt"><span class="p">Entendi</span><span>Dispensar</span></div></div>';
  }
  F.reposition();
})();

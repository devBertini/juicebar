// JuiceBar option 2: Rings
(function () {
  var S = window.JUICEBAR_SAMPLE, F = JuiceBarFrame, plat = F.platform;
  var devs = S.devices.filter(function (d) { return !d.platforms || d.platforms.indexOf(plat) >= 0; });
  var byId = {}; devs.forEach(function (d) { byId[d.id] = d; });
  var SHORT = { xbox: "Xbox", airpods: "AirPods" };
  function short(d) { return SHORT[d.id] || d.name; }

  function col(l) { return l <= S.lowThreshold ? "var(--red)" : l <= 40 ? "var(--amber)" : "var(--green)"; }

  // One ring: arc = level, glyph inside. o: stroke (viewBox units), sw (glyph stroke), gs (glyph size), level, charging
  function ring(d, px, o) {
    o = o || {};
    var lv = o.level != null ? o.level : d.level;
    var ch = o.charging != null ? o.charging : d.state === "charging";
    var off = d.state === "off";
    var sw = o.stroke || 8, gs = o.gs || 44, gx = 50 - gs / 2;
    var c = col(lv), k = "ring" + (off ? " off" : "") + (ch ? " charging" : "");
    var h = '<svg class="' + k + '" width="' + px + '" height="' + px + '" viewBox="0 0 100 100" aria-hidden="true">';
    if (off) {
      h += '<circle class="offc" cx="50" cy="50" r="44" pathLength="96" stroke-width="' + sw + '" stroke-dasharray="' + (o.dash || "4 4") + '" stroke-linecap="butt"/>';
    } else {
      h += '<circle class="trk" cx="50" cy="50" r="44" stroke-width="' + sw + '"/>';
      if (ch && lv < 100) {
        h += '<circle class="lead" cx="50" cy="50" r="44" pathLength="100" stroke-width="' + sw + '" stroke="' + c + '" stroke-opacity=".28" stroke-dasharray="' + (100 - lv) + ' 100" stroke-dashoffset="-' + lv + '" transform="rotate(-90 50 50)"/>';
      }
      h += '<circle class="arc" cx="50" cy="50" r="44" pathLength="100" stroke-width="' + sw + '" stroke="' + c + '" style="color:' + c + '" stroke-linecap="round" stroke-dasharray="' + Math.max(lv, 0.1) + ' 100" transform="rotate(-90 50 50)"/>';
    }
    h += '<g class="gl" stroke-width="' + (o.gsw || 1.75) + '"><use href="#' + d.glyph + '" x="' + gx + '" y="' + (50 - gs / 2) + '" width="' + gs + '" height="' + gs + '"/></g>';
    if (ch) {
      var br = o.badge || 19;
      h += '<g style="color:' + c + '"><circle class="bdg" cx="' + (100 - br - 1) + '" cy="' + (100 - br - 1) + '" r="' + br + '"/></g>' +
        '<g class="bolt" stroke-width="1.4"><use href="#g-bolt" x="' + (100 - br * 2 + (br * 2 - br * 1.25) / 2 - 1) + '" y="' + (100 - br * 2 + (br * 2 - br * 1.25) / 2 - 1) + '" width="' + br * 1.25 + '" height="' + br * 1.25 + '"/></g>';
    }
    return h + "</svg>";
  }

  // ---- bar item / tray ----
  var pinned = S.pinned.map(function (id) { return byId[id]; }).filter(Boolean);
  if (plat === "mac") {
    F.slot("bar-item").innerHTML = '<div class="bi jb">' + pinned.map(function (d) {
      return ring(d, 16, { stroke: 11, gs: 46, gsw: 2.5, badge: 24, dash: "5 4.6" });
    }).join("") + "</div>";
  } else {
    var t = F.slot("tray");
    pinned.filter(function (d) { return d.on; }).forEach(function (d) {
      var cell = document.createElement("div");
      cell.className = "tray-cell jb";
      cell.innerHTML = ring(d, 18, { stroke: 11, gs: 46, gsw: 2.5, badge: 24 });
      t.appendChild(cell);
    });
  }

  // ---- shared panel pieces ----
  var CH = { w: 346, h: 100, l: 24, r: 4, t: 6, b: 15, x0: -168, x1: 22 };
  var FC = { g703: 3, xbox: 18, airpods: 4 }; // hours until empty
  function chart() {
    var on = devs.filter(function (d) { return d.on; });
    var iw = CH.w - CH.l - CH.r, ih = CH.h - CH.t - CH.b;
    function X(h) { return (CH.l + (h - CH.x0) / (CH.x1 - CH.x0) * iw).toFixed(1); }
    function Y(v) { return (CH.t + (1 - v / 100) * ih).toFixed(1); }
    var s = '<svg width="' + CH.w + '" height="' + CH.h + '" viewBox="0 0 ' + CH.w + " " + CH.h + '">';
    [0, 50, 100].forEach(function (v) {
      s += '<line x1="' + CH.l + '" x2="' + (CH.w - CH.r) + '" y1="' + Y(v) + '" y2="' + Y(v) + '" stroke="var(--line)" stroke-width="1"/>' +
        '<text x="' + (CH.l - 5) + '" y="' + (+Y(v) + 3) + '" text-anchor="end">' + v + '</text>';
    });
    var days = ["ter", "qua", "qui", "sex", "sáb", "dom", "seg"];
    days.forEach(function (n, i) {
      var h = -158.5 + i * 24;
      s += '<line x1="' + X(h - 12) + '" x2="' + X(h - 12) + '" y1="' + CH.t + '" y2="' + (CH.t + ih) + '" stroke="var(--line)" stroke-width="1" opacity=".6"/>' +
        '<text x="' + X(h) + '" y="' + (CH.h - 3) + '" text-anchor="middle">' + n + "</text>";
    });
    s += '<line x1="' + X(0) + '" x2="' + X(0) + '" y1="' + CH.t + '" y2="' + (CH.t + ih) + '" stroke="var(--muted)" stroke-width="1" stroke-dasharray="2 3"/>';
    on.forEach(function (d) {
      var c = "var(--c-" + d.id + ")";
      var p = d.history.map(function (a, i) { return (i ? "L" : "M") + X(a[0]) + " " + Y(a[1]); }).join("");
      s += '<path d="' + p + '" fill="none" stroke="' + c + '" stroke-width="1.6" stroke-linejoin="round" stroke-linecap="round"/>';
      var fx, fy;
      if (d.state === "charging") { fx = 1.17; fy = 100; } else { fx = FC[d.id]; fy = 0; }
      if (fx != null) s += '<path d="M' + X(0) + " " + Y(d.level) + "L" + X(fx) + " " + Y(fy) + '" fill="none" stroke="' + c + '" stroke-width="1.6" stroke-dasharray="3 3" stroke-linecap="round"/>';
      s += '<circle cx="' + X(0) + '" cy="' + Y(d.level) + '" r="2.8" fill="' + c + '" stroke="var(--solid)" stroke-width="1.2"/>';
    });
    s += "</svg>";
    var lg = '<div class="legend">' + on.map(function (d) { return '<span><i style="background:var(--c-' + d.id + ')"></i>' + short(d) + "</span>"; }).join("") +
      '<span><span class="dsh"></span>previsão</span></div>';
    return '<div class="sec"><h4>Últimos 7 dias</h4><div class="chartwrap">' + s + lg + "</div></div>";
  }
  function usageChips() {
    return '<div class="chips"><span class="lbl">Uso hoje</span>' + devs.map(function (d) {
      return '<span class="chip"><i style="background:var(--c-' + d.id + ')"></i>' + short(d) + " <b>" + d.usageToday.replace(" min", "") + "</b></span>";
    }).join("") + "</div>";
  }
  function healthChips() {
    var learn = devs.filter(function (d) { return d.health == null; }).map(short).join(", ");
    return '<div class="chips"><span class="lbl">Saúde</span>' + devs.filter(function (d) { return d.health != null; }).map(function (d) {
      return '<span class="chip">' + short(d) + " <b>" + d.health + "%</b></span>";
    }).join("") + (learn ? '<span class="chip">' + learn + " <b>aprendendo</b></span>" : "") + "</div>";
  }
  function callout() {
    var g = byId.g703, w = S.notifications.warning;
    return '<div class="callout">' + JuiceBarGlyph("g-warning") + "<div><b>" + w.title + "</b><span>" + w.body + "</span></div></div>";
  }
  function foot() {
    return '<div class="foot"><div class="tog"><span class="sw"></span>' + JuiceBarGlyph("g-leaf") + "<span>Economia de RGB</span></div><span class=\"sp\"></span>" +
      "<kbd>" + S.hotkey[plat] + '</kbd><span class="gearb">' + JuiceBarGlyph("g-gear") + "</span></div>";
  }
  function sub(d) { return d.state === "charging" ? d.forecast : d.eta; }

  if (plat === "mac") {
    F.slot("panel").innerHTML = '<div class="panel jb"><div class="grid">' + devs.map(function (d) {
      var off = d.state === "off";
      return '<div class="cell' + (off ? " off" : "") + '">' + ring(d, 60, { stroke: 8, gs: 42 }) +
        '<div class="pct">' + (off ? d.lastSeen : d.level + "%") + '</div><div class="nm">' + d.name + "</div>" +
        '<div class="sub">' + (off ? "desligado" : sub(d)) + "</div></div>";
    }).join("") + "</div>" + chart() + usageChips() + healthChips() + callout() + foot() + "</div>";

    F.slot("widget").innerHTML = '<div class="widget jb">' + devs.map(function (d) {
      var off = d.state === "off";
      return '<div class="w' + (off ? " off" : "") + '">' + ring(d, 52, { stroke: 9, gs: 44, gsw: 2, badge: 21 }) +
        '<div class="pct">' + d.level + "%</div><div class=\"nm\">" + short(d) + "</div></div>";
    }).join("") + "</div>";

    var n = S.notifications.warning;
    F.slot("notification").innerHTML = '<div class="note jb">' + ring(byId.g703, 38, { stroke: 10, gs: 44, gsw: 2 }) +
      '<div class="tx"><div class="top"><span class="ti">' + n.title + '</span><span class="tm">agora</span></div><div class="bd">' + n.body + '</div><div class="app">JuiceBar</div></div></div>';
  } else {
    F.slot("flyout").innerHTML = '<div class="panel win jb"><div class="cards">' + devs.map(function (d) {
      var off = d.state === "off";
      return '<div class="card' + (off ? " off" : "") + '">' + ring(d, 50, { stroke: 8, gs: 42 }) +
        '<div class="t"><span class="nm">' + d.name + '</span><span class="pct">' + (off ? d.lastSeen : d.level + "%") + '</span><span class="sub">' + (off ? "desligado" : sub(d).replace(" min", "")) + "</span></div></div>";
    }).join("") + "</div>" + chart() + usageChips() + healthChips() + callout() + foot() + "</div>";

    var f = S.notifications.full, g = byId.g915x;
    F.slot("toast").innerHTML = '<div class="toast jb"><div class="hd">' + ring(g, 14, { stroke: 12, gs: 46, gsw: 2.6, level: 100, charging: false }) +
      '<span>JuiceBar</span><span class="sp"></span><span>agora</span></div><div class="row">' +
      ring(g, 52, { stroke: 8, gs: 42, level: 100, charging: true, badge: 22 }) +
      '<div><div class="ti">' + f.title + '</div><div class="bd">' + f.body + '</div></div></div>' +
      '<div class="btns"><span class="btn">Dispensar</span><span class="btn">Abrir JuiceBar</span></div></div>';
  }
  F.reposition();
})();

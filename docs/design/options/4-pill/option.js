(function () {
  var S = window.JUICEBAR_SAMPLE, F = JuiceBarFrame, mac = F.platform === "mac";
  document.body.classList.add(mac ? "mac" : "win");
  var devs = S.devices.filter(function (d) { return !d.platforms || d.platforms.indexOf(F.platform) >= 0; });
  var H = 72, NOW = 14.5; // axis hours; clock hour at "agora"
  function pct(h) { return Math.min(100, Math.max(0, h / H * 100)); }
  function hrs(d) {
    if (d.state !== "discharging") return null;
    var m = /^acaba (hoje|\w{3}) ~(\d+):(\d+)$/.exec(d.eta);
    if (!m) return 3;
    var day = m[1] === "hoje" ? 0 : 1 + ["ter", "qua", "qui"].indexOf(m[1]);
    return day * 24 + (+m[2]) + (+m[3]) / 60 - NOW;
  }
  function G(id) { return '<svg class="g"><use href="#' + id + '"/></svg>'; }
  function lane(d, sel) {
    var low = d.state === "discharging" && d.level <= S.lowThreshold + 5;
    var cls = "lane" + (sel ? " sel" : "") + (low ? " low" : "") + (d.state === "charging" ? " chg" : "") + (d.state === "off" ? " off" : "");
    var bar = "", tx = "", p;
    if (d.state === "off") {
      bar = '<div class="bar" style="width:100%"></div>';
      tx = '<div class="tx" style="left:0">desligado · ' + d.lastSeen + "</div>";
    } else if (d.state === "charging") {
      bar = '<div class="bar"></div>';
      tx = '<div class="tx" style="left:40px">' + G("g-bolt") + "cheio às " + d.etaClock + "</div>";
    } else {
      p = pct(hrs(d));
      bar = '<div class="bar" style="width:' + p + '%"></div>' + (low ? '<div class="mk" style="left:' + p + '%">' + G("g-warning") + "</div>" : "");
      var t = d.forecast.replace(" de uso", "") + " · " + (low ? "até " + d.etaClock : d.eta.replace("acaba ", "").replace("hoje ", "").replace("~", ""));
      if (p > 30) tx = '<div class="tx" style="left:calc(' + p + '% + 8px)">' + d.forecast.replace(" de uso", "") + " · qua" + "</div>";
      else tx = '<div class="tx" style="left:calc(' + p + '% + ' + (low ? 14 : 8) + 'px)">' + t + "</div>";
    }
    return '<div class="' + cls + '"><div class="lb">' + G(d.glyph) + '<div class="t"><b>' + d.name + "</b><em>" + d.level + '%</em></div></div><div class="tr">' + bar + tx + "</div></div>";
  }
  function runway(sel, compact) {
    var layer = "", bands = [[0, 19 - NOW], [24 + 9 - NOW, 24 + 19 - NOW], [48 + 9 - NOW, 48 + 19 - NOW]];
    bands.forEach(function (b) { layer += '<div class="band" style="left:' + pct(b[0]) + "%;width:" + (pct(b[1]) - pct(b[0])) + '%"></div>'; });
    [24 - NOW, 48 - NOW, 72 - NOW].forEach(function (h) { layer += '<div class="tick" style="left:' + pct(h) + '%"></div>'; });
    layer += '<div class="nowline"></div>';
    var ax = '<i class="s now" style="left:0">agora</i><i style="left:' + pct(24 - NOW) + '%">ter</i><i style="left:' + pct(48 - NOW) + '%">qua</i><i style="left:' + pct(72 - NOW) + '%">qui</i>';
    ax += '<i class="e" style="left:100%">+3 dias</i>';
    return '<div class="rw"><div class="axis">' + ax + '</div><div class="lanes"><div class="layer">' + layer + "</div>" +
      devs.map(function (d) { return lane(d, !compact && d.id === sel); }).join("") + "</div></div>";
  }
  function spark(d) {
    var w = 352, h = 58;
    var pts = d.history.map(function (p) { return [(p[0] + 168) / 168 * w, h - 4 - p[1] / 100 * (h - 10)]; });
    var line = pts.map(function (p, i) { return (i ? "L" : "M") + p[0].toFixed(1) + " " + p[1].toFixed(1); }).join(" ");
    var ty = h - 4 - S.lowThreshold / 100 * (h - 10);
    return '<svg class="spark" viewBox="0 0 ' + w + " " + h + '" preserveAspectRatio="none"><defs><linearGradient id="sg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="var(--ink)" stop-opacity=".35"/><stop offset="1" stop-color="var(--ink)" stop-opacity="0"/></linearGradient></defs>' +
      '<line x1="0" x2="' + w + '" y1="' + ty + '" y2="' + ty + '" stroke="var(--amber)" stroke-width="1" stroke-dasharray="3 3" opacity=".85"/>' +
      '<path d="' + line + " L" + w + " " + h + " L0 " + h + 'Z" fill="url(#sg)"/><path d="' + line + '" fill="none" stroke="var(--ink)" stroke-width="1.6" stroke-linejoin="round" vector-effect="non-scaling-stroke"/></svg>' +
      '<div class="sx"><span>7 dias atrás</span><span>agora</span></div>';
  }
  function panel() {
    var g = S.devices[0];
    return '<div class="jb surface panel"><div class="hd"><b>JuiceBar</b><span>' + devs.length + " dispositivos, " + devs.filter(function (d) { return d.on; }).length + " ligados</span></div>" +
      runway("g703") +
      '<div class="warn">' + G("g-warning") + "<div><b>G703 com 23%.</b> " + g.warning + "</div></div>" +
      '<div class="dt"><div class="dh"><b>' + G(g.glyph) + g.name + "</b><span>nível nos últimos 7 dias</span></div>" + spark(g) +
      '<div class="stats"><div class="stat"><small>Hoje</small><b>' + g.usageToday + ' de uso</b></div><div class="stat"><small>Saúde</small><b>' + g.health + "%</b></div></div></div>" +
      '<div class="ft"><div class="tg">' + G("g-leaf") + 'Economia de RGB<span class="sw" style="margin-left:auto"></span></div><span class="kb">' + (mac ? S.hotkey.mac : S.hotkey.win) + '</span><span class="gear">' + G("g-gear") + "</span></div></div>";
  }
  var batt = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="7" width="17" height="10" rx="2.5"/><path d="M22 10.5v3"/><rect x="4.5" y="9.5" width="4" height="5" rx="1" fill="currentColor" stroke="none"/></svg>';
  function note(n) {
    var ico = '<div class="appico">' + batt + "</div>";
    if (mac) return '<div class="jb surface note">' + ico + '<div class="nt"><div class="nrow"><b>JuiceBar</b><span>agora</span></div><h4>' + n.title + "</h4><p>" + n.body + "</p></div></div>";
    return '<div class="jb surface note"><div class="whd"><div class="appico">' + batt + "</div><span>JuiceBar</span><span>agora</span></div><div><h4>" + n.title + "</h4><p>" + n.body + '</p></div><div class="btns"><span>Ok</span><span>Dispensar</span></div></div>';
  }
  if (mac) {
    var dots = devs.slice(1).map(function (d) { return "<i" + (d.on ? "" : ' class="o"') + "></i>"; }).join("");
    F.slot("bar-item").innerHTML = '<div class="jb barwrap"><div class="pill">' + G("g-mouse") + '~3 h</div><div class="dots">' + dots + "</div></div>";
    F.slot("panel").innerHTML = panel();
    F.slot("widget").innerHTML = '<div class="jb surface widget"><div class="wh"><b>Autonomia</b><span>quando cada um acaba</span></div>' + runway("", true) + "</div>";
    F.slot("notification").innerHTML = note(S.notifications.warning);
  } else {
    F.slot("tray").innerHTML = '<svg class="trayico" viewBox="0 0 16 16" style="color:var(--hx-fg)"><rect x=".75" y="2.75" width="12.5" height="8" rx="2" fill="none" stroke="currentColor" stroke-width="1.3"/><rect x="14" y="5.25" width="1.4" height="3" rx=".6" fill="currentColor"/><rect x="2.2" y="4.2" width="3" height="5.1" rx=".8" fill="#ffb52e"/><circle cx="11.5" cy="11.5" r="4.2" fill="#ffb52e"/><path d="M11.5 9.3c-1 0-1.6.7-1.6 1.6v1.2c0 1 .7 1.7 1.6 1.7s1.6-.7 1.6-1.7v-1.2c0-.9-.6-1.6-1.6-1.6zM10 11.4h3" fill="none" stroke="#2b1900" stroke-width=".9" stroke-linecap="round"/></svg>';
    F.slot("flyout").innerHTML = panel();
    F.slot("toast").innerHTML = note(S.notifications.full);
  }
  F.reposition();
})();

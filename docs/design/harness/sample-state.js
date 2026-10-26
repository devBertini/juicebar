/* JuiceBar harness: the illustrative data every option draws (pt-BR).
   Sample "now" is segunda-feira 26/10/2026, 14:32. History hours are offsets from now
   (negative = past), levels in percent, one point per hour plus the anchors. */
(function () {
  var NOW_HOD = 14.5; // clock hour of day at offset 0

  // anchors: [hourOffset, level]. Falling level = discharge (fast in work hours 9-19, slow at night);
  // rising level = charging (linear).
  function build(anchors) {
    var hs = {};
    anchors.forEach(function (a) { hs[a[0]] = true; });
    for (var h = Math.ceil(anchors[0][0]); h <= anchors[anchors.length - 1][0]; h++) hs[h] = true;
    var list = Object.keys(hs).map(Number).sort(function (a, b) { return a - b; });
    function weight(h) { var hod = (((NOW_HOD + h) % 24) + 24) % 24; return hod >= 9 && hod < 19 ? 1 : 0.15; }
    function level(h) {
      for (var i = 0; i < anchors.length - 1; i++) {
        var a = anchors[i], b = anchors[i + 1];
        if (h >= a[0] && h <= b[0]) {
          if (b[1] >= a[1]) return a[1] + (b[1] - a[1]) * (h - a[0]) / (b[0] - a[0]);
          var total = 0, part = 0, step = 0.25;
          for (var t = a[0]; t < b[0]; t += step) { var w = weight(t); total += w; if (t < h) part += w; }
          return a[1] - (a[1] - b[1]) * part / total;
        }
      }
      return anchors[anchors.length - 1][1];
    }
    return list.map(function (h) { return [h, Math.round(level(h) * 10) / 10]; });
  }

  window.JUICEBAR_SAMPLE = {
    appName: "JuiceBar",
    now: { weekday: "segunda-feira", date: "26/10/2026", time: "14:32" },
    clock: { mac: "seg. 26 de out.  14:32", winTime: "14:32", winDate: "26/10/2026" },
    historyHours: 168, // the history covers the last 7 days
    lowThreshold: 20,  // at or below this level a device counts as low
    rgbEconomy: false,
    hotkey: { win: "Ctrl+Alt+B", mac: "⌃⌥B" },
    devices: [
      {
        id: "g703", name: "G703", kind: "mouse", glyph: "g-mouse",
        on: true, connection: "LIGHTSPEED", connectionGlyph: "g-wireless",
        level: 23, state: "discharging",
        forecast: "~3 h de uso", eta: "acaba hoje ~17:40", etaClock: "17:40",
        warning: "Não chega ao fim do dia: carregue antes das 18h",
        usageToday: "4 h 10 min", health: 94,
        history: build([[-168, 70], [-120, 14], [-117, 100], [-52, 18], [-49, 100], [0, 23]])
      },
      {
        id: "g915x", name: "G915 X", kind: "keyboard", glyph: "g-keyboard",
        on: true, connection: "LIGHTSPEED", connectionGlyph: "g-wireless",
        level: 46, state: "charging",
        forecast: "cheio em ~1 h 10 min", eta: "cheio em ~1 h 10 min", etaClock: "15:42",
        warning: null,
        usageToday: "6 h 05 min", health: 98,
        history: build([[-168, 81], [-95, 22], [-92, 100], [-26, 33], [-1, 5], [0, 46]])
      },
      {
        id: "g733", name: "G733", kind: "headset", glyph: "g-headset",
        on: false, connection: null, connectionGlyph: null,
        level: 71, state: "off", lastSeen: "71% às 14:02",
        forecast: null, eta: null, etaClock: null,
        warning: null,
        usageToday: "1 h 30 min", health: 89,
        history: build([[-168, 60], [-120, 8], [-117, 100], [-50, 20], [-47, 100], [-0.5, 71]])
      },
      {
        id: "xbox", name: "Controle Xbox", kind: "gamepad", glyph: "g-gamepad",
        on: true, connection: "Bluetooth", connectionGlyph: "g-bluetooth",
        level: 64, state: "discharging",
        forecast: "~18 h de uso", eta: "acaba qua ~20:00", etaClock: "20:00",
        warning: null,
        usageToday: "1 h 15 min", health: null,
        history: build([[-168, 30], [-140, 100], [-30, 75], [0, 64]])
      },
      {
        id: "airpods", name: "AirPods Pro", kind: "earbuds", glyph: "g-earbuds",
        on: true, connection: "Bluetooth", connectionGlyph: "g-bluetooth", platforms: ["mac"],
        level: 80, state: "discharging",
        forecast: "~4 h de uso", eta: "acaba hoje ~18:30", etaClock: "18:30",
        warning: null,
        usageToday: "50 min", health: null,
        history: build([[-48, 100], [-46, 85], [-45, 100], [-1, 100], [0, 80]])
      }
    ],
    /* Devices come from providers: Logitech (HID++), any Bluetooth device that reports battery to the system,
       game controllers, and other brands' protocols. A device without "platforms" appears on both scenes; the
       Mac scene shows five devices and the Windows scene four, so every option must hold three to eight devices.
       "pinned" lists what the bar item shows by default (the user chooses later); health null = still learning. */
    pinned: ["g703", "g915x", "g733"],
    notifications: {
      warning: { title: "G703 com 23%", body: "Não chega ao fim do dia: carregue antes das 18h", text: "Não chega ao fim do dia: carregue antes das 18h" },
      full: { title: "G915 X carregado", body: "Pode tirar do cabo", text: "G915 X carregado: pode tirar do cabo" }
    }
  };
})();

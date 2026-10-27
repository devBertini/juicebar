/* Copyright (c) 2026 Claudio Bertini (devBertini)
   SPDX-License-Identifier: GPL-3.0-or-later */
// Template: fills every slot of the current scene with its name.
(function () {
  var ids = JuiceBarFrame.platform === "mac"
    ? ["bar-item", "panel", "widget", "notification"]
    : ["tray", "flyout", "toast"];
  ids.forEach(function (id) {
    JuiceBarFrame.slot(id).innerHTML = '<div class="slot-fill">#' + id + "</div>";
  });
  JuiceBarFrame.reposition();
})();

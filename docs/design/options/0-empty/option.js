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

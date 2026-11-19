pragma ComponentBehavior: Bound

import QtQuick
import qs.Commons
import qs.Commons as Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "io.github.hornos.gmc-300e"

  readonly property var monitorService: bar && bar.shell && bar.shell.serviceFor
    ? bar.shell.serviceFor(root.moduleName) : null

  readonly property bool haveReading: !!monitorService && monitorService.lastValue !== ""
  readonly property string value: haveReading && !monitorService.stale ? String(monitorService.lastValue) : ""
  readonly property string unit: haveReading ? String(monitorService.lastUnit) : ""
  readonly property string level: haveReading ? String(monitorService.lastLevel) : ""
  readonly property string levelLabel: haveReading ? String(monitorService.lastLevelLabel) : ""

  readonly property color green: "#7bd88f"
  readonly property color yellow: "#f5c451"
  readonly property color red: "#ff6b6b"
  readonly property color neutral: bar ? bar.barForeground : Commons.Color.foreground

  readonly property color levelColor: {
    if (monitorService && monitorService.lastError !== "") return root.red
    if (root.level === "green") return root.green
    if (root.level === "yellow") return root.yellow
    if (root.level === "red") return root.red
    return root.neutral
  }

  readonly property string horizontalText: root.value !== ""
    ? root.value + " " + root.unit : "—"
  readonly property string verticalText: root.value !== "" ? root.value : "—"

  readonly property string tooltipText: {
    if (!root.monitorService) return "GMC-300E: starting monitor…"
    if (root.monitorService.lastError !== "")
      return "GMC-300E: " + String(root.monitorService.lastError)
    if (!root.haveReading) return "GMC-300E: waiting for first reading"

    var when = root.monitorService.lastTime !== ""
      ? String(root.monitorService.lastTime)
      : Qt.formatTime(root.monitorService.lastUpdated, "hh:mm:ss")
    var cpm = root.monitorService.lastDeviceCpm !== ""
      ? " · " + String(root.monitorService.lastDeviceCpm) + " CPM" : ""
    var state = root.monitorService.stale ? " · stale" : ""
    return root.levelLabel + cpm + " · " + when + state
  }

  function bindService() {
    if (!root.monitorService || typeof root.monitorService.configure !== "function") return
    root.monitorService.configure(root.settings)
  }

  onMonitorServiceChanged: root.bindService()
  onSettingsChanged: root.bindService()
  Component.onCompleted: root.bindService()

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.vertical ? root.verticalText : root.horizontalText
    tooltipText: root.tooltipText
    foreground: root.levelColor
    dimmed: !!root.monitorService && root.monitorService.stale
    keepSpace: true
    interactive: true
    pressable: false
    fontSize: Style.font.body
  }
}

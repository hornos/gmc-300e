import QtQuick
import Quickshell.Io
import "Model.js" as Model

// Process-wide service: one Rust controller, one serial port and one heartbeat
// session shared by every bar instance. The bar never launches gmc300e itself.
Item {
  id: root

  property var settings: ({})
  property string omarchyPath: ""
  property var shell: null
  property var manifest: null

  property string binaryPath: String(setting("binaryPath", "gmc300e") || "gmc300e")
  property string devicePath: String(setting("devicePath", "") || "")
  property string safeMaxUsv: String(setting("safeMaxUsv", "1") || "1")
  property string dangerMinUsv: String(setting("dangerMinUsv", "10") || "10")

  property bool wanted: false
  property bool monitoring: false
  property bool stale: false
  property string lastError: ""
  property int samples: 0
  property string lastValue: ""
  property string lastUnit: ""
  property string lastLevel: ""
  property string lastLevelLabel: ""
  property string lastDeviceCpm: ""
  property string lastTime: ""
  property date lastUpdated: new Date(0)
  property double lastUpdateMs: 0

  property string settingsSignature: ""
  property bool _restarting: false
  property double startedMs: 0
  property int restartDelayMs: 1000

  function setting(name, fallback) {
    var value = settings ? settings[name] : undefined
    return value === undefined || value === null ? fallback : value
  }

  function cleanError(value) {
    var text = String(value === undefined || value === null ? "" : value)
    text = text.replace(/[\r\n\t]+/g, " ").trim()
    return text.length > 300 ? text.slice(0, 300) : text
  }

  function configure(nextSettings) {
    var next = nextSettings || {}
    var signature = JSON.stringify(next)
    if (signature === root.settingsSignature) return
    root.settingsSignature = signature
    root.settings = next
    root.restart()
  }

  function startNow() {
    if (!root.wanted || liveProcess.running) return
    root.lastError = ""
    root.stale = false
    root.startedMs = Date.now()
    liveProcess.running = true
  }

  function stop() {
    root.wanted = false
    root._restarting = false
    restartTimer.stop()
    liveProcess.running = false
    root.monitoring = false
    root.stale = false
  }

  function restart() {
    root.wanted = true
    restartTimer.stop()
    root._restarting = liveProcess.running
    if (liveProcess.running) {
      liveProcess.running = false
    } else {
      Qt.callLater(root.startNow)
    }
  }

  function noteError(value) {
    var text = cleanError(value)
    if (text !== "") root.lastError = text
  }

  // Keep the value exactly as the controller printed it. Level colouring and
  // the µSv/h cut-offs live in the Rust binary, including env overrides.
  function consumeLine(line) {
    var reading = Model.parseCsvLine(line)
    if (!reading) {
      var text = cleanError(line)
      if (text !== "" && text.indexOf("epoch,") !== 0) root.noteError(text)
      return
    }

    if (root.startedMs > 0 && Date.now() - root.startedMs > 10000)
      root.restartDelayMs = 1000

    root.samples += 1
    root.lastValue = reading.displayValue
    root.lastUnit = reading.displayUnit
    root.lastLevel = reading.level
    root.lastLevelLabel = reading.levelLabel
    root.lastDeviceCpm = reading.deviceCpm
    root.lastTime = reading.time
    root.lastUpdated = new Date()
    root.lastUpdateMs = Date.now()
    root.lastError = ""
    root.stale = false
  }

  Process {
    id: liveProcess
    command: [root.binaryPath, "--csv", "live"]
    environment: ({
      GMC_PORT: root.devicePath,
      GMC_SAFE_MAX_USV: root.safeMaxUsv,
      GMC_DANGER_MIN_USV: root.dangerMinUsv
    })
    stdout: SplitParser {
      onRead: function(line) { root.consumeLine(line) }
    }
    stderr: SplitParser {
      onRead: function(line) { root.noteError(line) }
    }
    onRunningChanged: {
      if (liveProcess.running) {
        root.monitoring = true
        root.stale = false
        root.lastError = ""
      } else {
        root.monitoring = false
      }
    }
    onExited: function(exitCode) {
      if (!root.wanted) return
      if (root._restarting) {
        root._restarting = false
        Qt.callLater(root.startNow)
        return
      }
      root.noteError("gmc300e exited with code " + exitCode)
      root.restartDelayMs = Math.min(root.restartDelayMs * 2, 30000)
      restartTimer.interval = root.restartDelayMs
      restartTimer.restart()
    }
  }

  Timer {
    id: restartTimer
    interval: root.restartDelayMs
    repeat: false
    onTriggered: root.startNow()
  }

  // A silent controller is worse than an obviously stale label: dim the bar
  // item rather than leaving a green number from ten seconds ago on screen.
  Timer {
    interval: 2000
    repeat: true
    running: root.monitoring || root.lastUpdateMs > 0
    onTriggered: {
      var age = root.lastUpdateMs > 0 ? Date.now() - root.lastUpdateMs : 0
      root.stale = !root.monitoring || root.lastUpdateMs === 0 || age > 10000
    }
  }

  IpcHandler {
    target: "io.github.hornos.gmc-300e"
    function start(): void { root.wanted = true; root.startNow() }
    function stop(): void { root.stop() }
    function restart(): void { root.restart() }
    function status(): string {
      return JSON.stringify({
        monitoring: root.monitoring,
        stale: root.stale,
        value: root.lastValue,
        unit: root.lastUnit,
        level: root.lastLevel,
        samples: root.samples,
        error: cleanError(root.lastError)
      })
    }
  }

  Component.onDestruction: root.stop()
}

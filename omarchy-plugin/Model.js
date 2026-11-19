// Parser for the Rust controller's `--csv live` stream.
//
// The same file is imported by QML and by the Node test harness. The guarded
// export at the bottom is ignored by QML, where `module` is undefined.

function cleanText(value, limit) {
  var text = value === undefined || value === null ? "" : String(value)
  text = text.replace(/[\r\n\t]+/g, " ").trim()
  return text.length > limit ? text.slice(0, limit) : text
}

function numericText(value) {
  var text = cleanText(value, 32)
  return /^-?\d+(?:\.\d+)?$/.test(text) ? text : ""
}

function parseCsvLine(line) {
  var text = cleanText(line, 512)
  if (text === "") return null
  if (text.indexOf("epoch,") === 0) return null

  var parts = text.split(",")
  if (parts.length < 11) return null

  var displayValue = numericText(parts[7])
  if (displayValue === "") return null

  var displayPrecise = numericText(parts[8])
  if (displayPrecise === "") return null

  var level = cleanText(parts[9], 16).toLowerCase()
  if (level !== "green" && level !== "yellow" && level !== "red") return null

  return {
    epoch: numericText(parts[0]),
    time: cleanText(parts[1], 16),
    cps: numericText(parts[2]),
    cumulative: numericText(parts[3]),
    avgCpm: numericText(parts[4]),
    deviceCpm: numericText(parts[5]),
    displayUnit: cleanText(parts[6], 8),
    displayValue: displayValue,
    displayPrecise: displayPrecise,
    level: level,
    levelLabel: cleanText(parts[10], 64)
  }
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    cleanText: cleanText,
    numericText: numericText,
    parseCsvLine: parseCsvLine
  }
}

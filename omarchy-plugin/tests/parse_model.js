const assert = require("node:assert/strict");
const { parseCsvLine } = require("../Model.js");

const green = "1795111204,19:00:04,0,0,0,16,mR/h,0.01,0.010,green,NORMAL[<1µSv/h]";
const yellow = "1795111205,19:00:05,3,9,12,42,mR/h,1.20,1.204,yellow,CAUTION[>=1µSv/h]";
const red = "1795111206,19:00:06,9,99,180,900,mR/h,12.50,12.503,red,EVACUATE[>=10µSv/h]";

const reading = parseCsvLine(green + "\r");
assert.ok(reading, "a valid live row parses");
assert.equal(reading.displayValue, "0.01");
assert.equal(reading.displayUnit, "mR/h");
assert.equal(reading.level, "green");
assert.equal(reading.levelLabel, "NORMAL[<1µSv/h]");
assert.equal(reading.deviceCpm, "16");
assert.equal(reading.time, "19:00:04");

assert.equal(parseCsvLine("epoch,time,cps,cumulative,avg_cpm,device_cpm,display_unit,display_value,display_precise,level,level_label"), null);
assert.equal(parseCsvLine(""), null);
assert.equal(parseCsvLine("   \r\n"), null);
assert.equal(parseCsvLine("1795111204,19:00:04,0,0,0,16,mR/h"), null);
assert.equal(parseCsvLine("1795111204,19:00:04,0,0,0,16,mR/h,nope,0.010,green,NORMAL"), null);
assert.equal(parseCsvLine("1795111204,19:00:04,0,0,0,16,mR/h,0.01,0.010,purple,NORMAL"), null);

assert.equal(parseCsvLine(yellow).level, "yellow");
assert.equal(parseCsvLine(yellow).displayValue, "1.20");
assert.equal(parseCsvLine(red).level, "red");
assert.equal(parseCsvLine(red).displayValue, "12.50");
assert.equal(parseCsvLine(red).levelLabel, "EVACUATE[>=10µSv/h]");

console.log("CSV live parser checks passed.");

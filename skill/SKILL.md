---
name: GMC-300E Geiger Counter
description: >
  Detect, control and troubleshoot a GQ GMC-300E / GMC-300Re Geiger-Muller
  counter connected over USB (QinHeng CH340 serial, 57600 8N1). Use when asked
  to check the counter or USB connection, read CPM/CPS or radiation levels,
  start live tracking or CSV logging, check battery or device clock, power the
  unit on/off, reboot it, set its clock, or diagnose 0 CPM / no counts /
  permission denied / device not found. Bundles a tested pure-bash controller
  plus the protocol details learned on this machine. Triggers: gmc-300e,
  gmc300e, geiger counter, geiger, radiation detector, radiation monitor,
  cpm, cps, ch340, usb serial geiger, 0 cpm, no counts, geiger live tracking,
  gq-rfc1201, gq-rfc1801.
---

# GMC-300E Geiger Counter

Control and diagnose a GQ GMC-300E (reports itself as `GMC-300Re 4.70`) through
its CH340 USB-serial bridge. The bundled script is pure bash + coreutils
(`stty`, `od`, `timeout`) — no pyserial, no helper daemon.

Always start with the controller's own `--help` if you need the full command
list. Read `references/protocol.md` before using `raw` or writing config, and
`references/troubleshooting.md` when counts look wrong.

## This machine's setup

| Item | Value |
| --- | --- |
| USB bridge | QinHeng `1a86:7523` (`USB Serial`), driver `ch341` bound |
| Port | `/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0` → `/dev/ttyUSB0` |
| Serial settings | **57600** baud, 8 data bits, no parity, 1 stop bit |
| Device node | `root:uucp` — user `sysop` is in `uucp` in `/etc/group`, but a fresh login session may not have it active |
| Firmware | `GMC-300Re 4.70`; serial `f7f4c55904d10c` (may change) |
| Battery | internal 3.7 V Li-ion, reads ~3.8–3.9 V via `GETVOLT` |
| Expected background | spec ≈ 0.2 pulses/s ≈ 12 CPM; a healthy unit reads roughly 10–25 CPM |
| Controller | `scripts/gmc300e.sh` in this skill (also kept at `~/gmc-300e/gmc300e.sh`) |

Let `S=~/.config/opencode/skills/gmc-300e/scripts` and `C="$S/gmc300e.sh"`.

## Ground rules

- **Read commands are safe** (`version`, `status`, `cpm`, `cps`, `volt`, `datetime`,
  `config`, `displayunit`, `displayvalue`, `history`, `live`). Use them freely.
- **Destructive commands require explicit user intent AND `--yes`**: `wcfg`,
  `ecfg`, `factoryreset`. Never add `--yes` just to make a command work. `off`,
  `reboot` and `setdatetime` are reversible but still change device state —
  mention them in the report.
- **Never invent a measurement.** If the port is inaccessible or the device
  returns no bytes, report that; never guess a CPM value. If CPM is 0 for more
  than a minute, treat it as abnormal and investigate (see troubleshooting).
- **Permission handling is automatic.** The script re-execs itself through
  `newgrp uucp` when the session lacks the group. Do not reach for sudo first;
  only suggest `sudo usermod -aG uucp "$USER"` + re-login if re-exec fails.
- **Binary parameters must really be binary.** `SETDATETIME`, `SETDATE*`,
  `SETTIME*`, `WCFG` and `SPIR` need raw bytes, not ASCII hex text. The script
  already does this correctly via `tx_bytes()` — do not reimplement them with
  `printf '%02x'`, which sends hex *text* and corrupts the device clock.
- **Stop the heartbeat.** `live` enables GMC heartbeat and always sends
  `HEARTBEAT0` on normal exit and on Ctrl-C. If a `live` run is killed
  abnormally (SIGKILL), send `<HEARTBEAT0>>` with `raw` before other reads.
- **After `off`, the unit stops answering.** Send `on` (or press a physical key)
  before retrying reads. A failed `version` right after `off` is expected.
- **`key N` really presses a button.** S2/S3 can open an on-screen menu where
  serial commands are ignored; press S1 with `key 0` to go back.

## Workflows

### 1. Detect / verify the USB connection

```sh
lsusb | grep -i 1a86
ls -l /dev/serial/by-id/usb-1a86_USB_Serial-if00-port0
"$C" version
"$C" status
```

A healthy result is a `GMC-300Re ...` version string plus a `status` block with
power ON. `status` also shows battery, CPM/CPS, the LCD unit with its converted
value, clock and serial number. If the port is missing, check the cable/driver;
if it is present but reads fail, see `references/troubleshooting.md`.

### 2. Read a measurement

```sh
"$C" cpm            # counts per minute, integer
"$C" cps            # counts per second (top bit of the reply is masked)
"$C" displayunit    # CPM, µSv/h or mR/h as configured on the LCD
"$C" displayvalue   # instrument CPM -> LCD value + precise value + level
"$C" status         # all of the above in one summary
"$C" --csv status   # one CSV row with header
```

Take two readings a few seconds apart before calling a value stable. Report the
raw CPM/CPS plus the LCD unit/value. Use `displayvalue` when comparing with the
physical screen: it converts the instrument's own CPM with the device's three
calibration points (piecewise linear) and divides by 10 for mR/h.

### 3. Live tracking / logging

```sh
"$C" live               # until Ctrl-C
"$C" live 60            # exactly 60 seconds
"$C" --csv live 3600 > /tmp/opencode/gmc-hour.csv
```

`live` reads the device's LCD display unit first (config byte 3: 0 = CPM,
1 = µSv/h, 2 = mR/h) and converts the rate using the device's own calibration
points, so the live view shows the same unit as the instrument. The converted
value comes from the instrument's own CPM, polled every second, because the LCD
uses that rolling CPM — not the heartbeat-derived running average. Output is
one row per second: timestamp, CPS, cumulative counts, heartbeat CPM, device
CPM, the LCD-style value (2 decimals) and a more precise 3-decimal value. The
extra `precise` column exists because at background levels ~15 CPM is one
0.01 mR/h step, so the LCD-style value can sit unchanged for many seconds.
Use `"$C" displayunit` to read just the unit, or `displayvalue` for a one-shot
comparison with the screen. For long runs prefer `--csv` to a file (`device_cpm`,
`display_unit`, `display_value`, `display_precise` and `level` columns are
included). Confirm afterwards that the heartbeat is off by running `"$C" cpm` —
it should return promptly with a single value.

Rows are colour-coded by dose rate when stdout is a terminal. The `LEVEL`
column uses radiation-safety wording with that level's cut-off in brackets:
**green `NORMAL[<1µSv/h]`**, **yellow `CAUTION[>=1µSv/h]`** (do not linger),
**red `EVACUATE[>=10µSv/h]`** (leave now). mR/h equivalents are 0.1 and 1 mR/h.
Override the cut-offs with `GMC_SAFE_MAX_USV` / `GMC_DANGER_MIN_USV` and the
labels update automatically. Force or disable colour with `--color` /
`--no-color`, or set `NO_COLOR`. `--csv` is never coloured and carries both
`level` (green/yellow/red) and `level_label` columns. These are conservative
logging/alert thresholds, not radiation-safety advice.

### 4. Power, reboot, keys, settings

```sh
"$C" on
"$C" off
"$C" reboot
"$C" key 1            # S2; use 'key 0' (S1) to leave a menu
"$C" speaker off
"$C" speaker on
"$C" alarm off
"$C" alarm on
```

Only turn the speaker/alarm back to the state the user expects; ask if unsure.

### 5. Clock

```sh
date '+%Y-%m-%d %H:%M:%S'
"$C" datetime          # device clock
"$C" synctime          # preferred: sync to host local time and verify read-back
"$C" setdatetime       # same sync without the read-back check
"$C" setdatetime 26 11 19 10 30 00   # explicit YY MM DD hh mm ss
```

`synctime` writes the host local time as raw bytes, reads the clock back and
reports both times plus the difference; warn if the difference exceeds 3 s.
The counter's RTC was several hours off once; the correct byte format is raw
binary `YY MM DD hh mm ss`, which the script handles. Setting the clock changes
stored history timestamps — tell the user.

### 6. Config and history (read-only unless asked)

```sh
"$C" -v config         # 256-byte dump plus decoded fields
"$C" history 0 64      # flash history as hex; 0x55 0xAA prefix is normal
"$C" raw '<GETCPM>>'   # arbitrary command, hex + ASCII reply
```

Config byte 0 is power state on the 300 series: `0x00` = ON, `0xFF` = OFF.
Config byte 3 is the LCD radiation unit: `0` = CPM, `1` = µSv/h, `2` = mR/h
(confirmed on the physical display of this unit). Calibration points are at
offsets 8, 14 and 20 (big-endian CPM plus little-endian float32 µSv/h);
conversion is piecewise linear, with mR/h = µSv/h ÷ 10. History records begin
with `55 aa`. An all-`FF` dump means empty/not logged.

### 7. Zero-CPM investigation

Follow `references/troubleshooting.md`. In short: confirm power ON and battery
voltage, waken with `on` or `key 1`, check heartbeat packets, try `reboot`,
allow a couple of minutes for tube recovery, and only then raise the possibility
of a hardware (HV/tube) fault. Never declare the counter broken because of one
zero reading.

## Reporting

Include: the port and firmware string, power/battery state, the measured
CPM/CPS with sampling window (e.g. "14 counts in 60 s ≈ 14 CPM"), whether the
LCD agrees if the user can see it, and any device state you changed (clock,
speaker, alarm, power). Keep the verdict modest: background radiation varies.

## Safety

- Never write config bytes without a clear request and `--yes`; a wrong byte can
  change baud rate or calibration. Back up with `config` before any `wcfg`.
- Do not send `<FACTORYRESET>>` or `<ECFG>>` unless the user explicitly asks for
  a factory reset / erase and understands the loss of settings.
- Do not point the script at arbitrary serial ports: only the GMC's CH340 port.
- The counter is an instrument, not a safety device — say so if the user wants
  to rely on it for radiation safety decisions.

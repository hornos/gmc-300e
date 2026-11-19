# GQ GMC-300E protocol notes (verified on this unit)

Sources: GQ-RFC1201 v1.40 (300 series), GQ-RFC1801 v1.00 (shared commands),
plus direct probing of `GMC-300Re 4.70` on 2026-11-19.

## Transport

- CH340 USB-serial (`1a86:7523`), driver `ch341`, port via
  `/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0`.
- **57600 8N1**, no flow control. A GETCPM "nudge" first helps a just-plugged
  unit answer; the bundled script does this on port open.
- Host-initiated only. A command is ASCII `<NAME...>>`. Replies are binary and
  exactly the documented length; read with a short timeout (~0.6–2 s).
- Multi-byte integers are **big-endian**. `GETCPS` replies have bit 15 set as a
  marker; mask with `0x3fff`.

## Command reference

| Command | Reply | Notes |
| --- | --- | --- |
| `<GETVER>>` | 14 ASCII bytes | e.g. `GMC-300Re 4.70` |
| `<GETCPM>>` | 2 bytes | unsigned big-endian counts/min |
| `<GETCPS>>` | 2 bytes | mask `0x8000`; 0x8001 means 1 count/s |
| `<GETMAXCPS>>` | 2 bytes | highest CPS since power-on |
| `<GETVOLT>>` | 1 byte | value × 0.1 V (0x26 = 3.8 V) |
| `<GETSERIAL>>` | 7 bytes | raw hex serial number |
| `<GETDATETIME>>` | 7 bytes | `YY MM DD hh mm ss AA` (binary values) |
| `<GETCFG>>` | 256 bytes | full config dump |
| `<SPIR A2 A1 A0 L1 L0>>` | raw bytes | flash history, max length 4096; records start `55 aa` |
| `<POWERON>>` | 0 or 1 byte | wakes the unit; config byte 0 = 0 means ON |
| `<POWEROFF>>` | none | unit goes quiet; use `on` to wake |
| `<REBOOT>>` | 1 byte `00` | soft reboot |
| `<HEARTBEAT1>>` | 2 bytes/s | streams CPS until `<HEARTBEAT0>>` |
| `<HEARTBEAT0>>` | none | must be sent to stop the stream |
| `<KEY0>>`…`<KEY3>>` | none | S1..S4; S1 = back, S2/S3 navigate, S4 = power-saving toggle |
| `<ALARM1>>` / `<ALARM0>>` | `aa` | changes config byte 1 |
| `<SPEAKER1>>` / `<SPEAKER0>>` | `aa` | changes config byte 2 |
| `<EchoON>>` / `<EchoOFF>>` | none on 300E | debug echo; no reply here |
| `<CFGUPDATE>>` | `aa` | reload config after writes |
| `<WCFG A0 D0>>` | `aa` | write config byte; cannot flip 0→1 without erase |
| `<ECFG>>` | `aa` | erase config to `FF` — destructive |
| `<FACTORYRESET>>` | `aa` | restore factory defaults — destructive |
| `<SETDATETIME YY MM DD hh mm ss>>` | `aa`/`00` | six **raw bytes** (see below) |
| `<SETDATEYY D0>>` etc. | `aa` | `SETDATEMM`, `SETDATEDD`, `SETTIMEHH`, `SETTIMEMM`, `SETTIMESS` |

## The raw-byte trap (important)

Parameters of `SETDATETIME`, `SETDATE*`, `SETTIME*`, `WCFG` and `SPIR` are
**binary byte values**, not ASCII hex characters.

- Correct: `<SETDATETIME` + bytes `1a 0b 13 0a 1c 27` + `>>` →
  `2026-11-19 10:28:39`.
- Wrong: `<SETDATETIME1a0b130a1c27>>` (12 ASCII chars) — the unit answers `aa`
  but stores a nonsense date/time.

In bash, `printf -v cmd '<SETDATETIME%02x...>>' 26 11 ...` produces the *wrong*
form. Build the format string with `\xHH` escapes and write directly to the fd
(`tx_bytes()` in the script); this also allows a `0x00` byte, which a bash
variable cannot hold.

## GETCFG offsets used by this skill

| Offset | Meaning |
| --- | --- |
| 0 | Power state: `0x00` = ON, `0xFF` = OFF (300 series) |
| 1 | Alarm enabled |
| 2 | Speaker enabled |
| 3 | **LCD radiation display unit**: `0` = CPM, `1` = µSv/h, `2` = mR/h (confirmed against the physical display on this unit) |
| 4 | Backlight timeout |
| 6–7 | Alarm CPM threshold (big-endian) |
| 8–9, 10–13 | Calibration point 1: CPM (uint16 BE), µSv/h (float32 **little-endian**) |
| 14–15, 16–19 | Calibration point 2 (same layout) |
| 20–21, 22–25 | Calibration point 3 (same layout) |
| 32 | History save type: 0 off, 1 CPS/s, 2 CPM/min, … |
| 49–50 | Max CPM |
| 56 | Battery type: 0 rechargeable, 1 non-rechargeable |
| 57 | Baud-rate code (252 = the configured 57600) |
| 62–65 | Threshold settings |

The three calibration points on this unit are effectively linear
(60 CPM = 0.39 µSv/h, 240 = 1.56, 1000 = 6.5 → ~153.8 CPM per µSv/h). The script
reproduces the device-style conversion exactly: prepend a `(0,0)` point, then
use piecewise-linear interpolation between successive calibration points; mR/h
is µSv/h ÷ 10, matching the instrument's 328 µSv/h = 32.8 mR/h range relationship.
`displayunit` prints byte 3, `displayvalue` converts the instrument's current
CPM, and `status` includes both the unit and converted value.

`live` deliberately does **not** use its heartbeat-derived running CPM for the
display value. That value is a cumulative average since the start of the run,
whereas the LCD uses the instrument's own rolling CPM. `live` therefore polls
`<GETCPM>>` every second and converts that CPM; heartbeat packets seen during
the poll have bit 15 set (`0x8000`–`0xBFFF`) and are fed back into the stream so
no counts are lost. `live` prints both the LCD-style 2-decimal value and a
3-decimal `display_precise` value, because with this calibration ~15 CPM is one
0.01 mR/h display step and the rounded value can stay unchanged for many
seconds. One-shot reads use `ask_expect()` to retry a missing reply up to three
times; the counter occasionally drops the first request after the port opens.

## Live row colour levels

Classification is done on the µSv/h value before it is divided for mR/h:

| Colour | `level_label` text | Default cut-off | mR/h equivalent | Meaning |
| --- | --- | --- | --- | --- |
| green | `NORMAL[<1µSv/h]` | `< GMC_SAFE_MAX_USV` (1 µSv/h) | < 0.1 mR/h | normal/low |
| yellow | `CAUTION[>=1µSv/h]` | `>= GMC_SAFE_MAX_USV` and `< GMC_DANGER_MIN_USV` | 0.1 to < 1 mR/h | elevated; do not linger |
| red | `EVACUATE[>=10µSv/h]` | `>= GMC_DANGER_MIN_USV` (10 µSv/h) | >= 1 mR/h | leave now |

The bracketed value is the cut-off **for that level** and is built from the
live `GMC_SAFE_MAX_USV` / `GMC_DANGER_MIN_USV` values, so it changes with env
overrides. ANSI colour is enabled automatically when stdout is a TTY. `--color`
forces it, `--no-color` or `NO_COLOR` disables it, and `--csv` always disables
it. Human rows carry the full `level_label`; CSV has both lowercase `level`
(green/yellow/red) and `level_label` columns. These are conservative
alert/logging thresholds, not radiation-safety advice.

## Commands probed but not useful on this model

- `<GETTEMP>>` → `00 00 00 aa` (no sensor fitted).
- `<GETGYRO>>` → meaningless bytes (no sensor fitted).
- WiFi/server commands from RFC1801 are for GMC-500/600+, not this unit.

## Behavioural quirks

- Zero counts can persist for a while after power-on or after a high dose; GQ
  calls this "tube recovery time" and suggests a couple of minutes, or powering
  off and on again.
- `key 2`/`key 3` can leave the unit in a menu where serial requests are not
  answered until `key 0` returns to the main screen.
- Repeated `od`-based reads with `timeout` are safe: with `min 0 time 2` the
  tty returns after ~0.2 s of silence, then `timeout` stops the reader.

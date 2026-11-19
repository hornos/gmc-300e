# Troubleshooting the GMC-300E

Work through these in order. Stop as soon as the symptom is explained; do not
declare hardware failure after a single zero reading.

## 1. Device not found / no `/dev/ttyUSB*`

```sh
lsusb | grep -i 1a86
ls -l /dev/serial/by-id/usb-1a86_USB_Serial-if00-port0
dmesg | tail -30        # may need sudo
```

- Expect `QinHeng Electronics CH340 serial converter`, `1a86:7523`, driver
  `ch341` bound to interface 0.
- If the adapter appears but no tty node exists, check `modprobe ch341` (needs
  root) and try a different USB cable/port — many cheap USB cables are
  power-only. The front-panel ports of this iMac share a hub.
- If the device disappears under load, suspect the USB hub or cable before the
  counter.

## 2. `Permission denied` on the port

The node is `root:uucp`. This session often lacks the `uucp` supplementary group
even though `sysop` is a member in `/etc/group`:

```sh
id -nG                 # current session groups
getent group uucp      # membership in /etc/group
```

The controller auto-reexecs with `newgrp uucp`; verbose mode shows the line
`no write access, re-executing with the uucp group`. If that fails:

- log out and back in, or run `newgrp uucp` in the shell before the script;
- permanently: `sudo usermod -aG uucp "$USER"` then re-login;
- last resort: run the script with sudo (not preferred for live tracking).

Do not `chmod` the device node — the group is the intended mechanism.

## 3. Wrong baud / no reply

The unit is **57600 8N1**. A port can be opened at any baud, so "open works" is
not proof of communication. The proof is a `GMC-300Re ...` reply to `GETVER`.

```sh
"$C" version
"$C" -v raw '<GETCPM>>'
```

If `GETVER` times out but the port opens, try `-b 115200` once (newer firmware
variants) — but on this unit 57600 is correct. Check that no other process has
the port open; only one reader can own it. The controller retries fixed-length
reads up to three times (`ask_expect()`); a one-off empty reply to `GETCFG` or
`GETCPM` should no longer abort `displayvalue`. If it still fails, run with
`-v` and look for `retry 1/3 ...` lines.

## 4. Zero CPM / no counts

This was observed on this unit and resolved. Follow this sequence:

1. **Confirm the device is on**: `"$C" -v config | grep -A3 decoded` — config
   byte 0 must be `0` (ON). If it is `255`, send `"$C" on`.
2. **Check battery**: `"$C" volt`. Expect ~3.8–3.9 V. Below ~3.4 V the HV
   converter may not reach the tube's operating voltage; charge/replace.
3. **Wake it**: `"$C" on`, or `"$C" key 1` (a button press), then wait 1 s and
   re-read `"$C" cpm`. Press `key 0` afterwards if the unit entered a menu.
4. **Check the hardware heartbeat**: run `"$C" live 10`. Real packets look like
   `80 00` / `80 01` …; `0x8000` means zero counts in that second, but a mix
   including `0x8001`/`0x8002` means the tube is counting.
5. **Clear a latched state**: `"$C" reboot`, wait 3–5 s, re-read `version` and
   `cpm`. A soft reboot was accepted by firmware 4.70.
6. **Allow recovery time**: GQ documents a tube recovery period of a couple of
   minutes after a high dose. Leave it alone and re-measure.
7. **Cross-check the display**: if the LCD also shows 0 CPM and there are no
   clicks with the speaker on, the fault is on the detector side, not USB.
8. Only then consider HV/tube/electronics failure or seek a repair. Do not run
   `ecfg`, `wcfg` or `factoryreset` as a "fix" for zero counts — configuration
   was not the cause here.

Reference point: this unit's own background spec is ≈0.2 pulses/s (≈12 CPM).
A sustained exact `0` over many minutes is abnormal; `14–21 CPM` is normal here.

## 4a. LCD mR/h is slightly higher than the `live` value

This is usually not a conversion error. The LCD uses the instrument's own
rolling CPM; the heartbeat CPM column in `live` is a running average since the
start of the run. With this calibration ~15 CPM is one 0.01 mR/h display step,
so a few counts near a rounding boundary can flip the LCD by one step. `live`
now polls `<GETCPM>>` every second for the converted value, and `displayvalue`
does a one-shot conversion of the same CPM:

```sh
"$C" displayvalue
"$C" live 30
```

If `displayvalue` itself differs from the LCD by more than one display step,
check the calibration bytes (config offsets 8, 14 and 20) with `"$C" -v config`;
they were intact on this unit (60/0.39, 240/1.56, 1000/6.5 CPM per µSv/h). Do
not rewrite them without the user's explicit intent.

## 5. Device clock wrong / history timestamps nonsense

```sh
date '+%Y-%m-%d %H:%M:%S'
"$C" datetime
"$C" synctime
"$C" datetime
```

- `synctime` uses computer local time, writes it, reads it back and reports the
  difference; `setdatetime` with no arguments is the same write without the
  verification. Check the machine's own timezone/NTP first — a wrong host clock
  just copies the error.
- Never write the date with `raw` using ASCII hex text; see
  `protocol.md#the-raw-byte-trap-important`. This corrupted the clock once and
  required the proper raw-byte command to repair.
- Changing the clock does not rewrite old history records.

## 6. `live` leaves the device streaming

If `live` was killed with SIGKILL, the `HEARTBEAT0` trap could not run and later
reads will contain extra 2-byte packets.

```sh
"$C" raw '<HEARTBEAT0>>'
"$C" cpm
```

`cpm` should return a single integer promptly once the heartbeat is off.

## 7. Commands stop answering after `key N`

Virtual keys are real button presses. S2/S3 can move the unit into a menu where
the serial parser ignores requests.

```sh
"$C" key 0      # S1 = back
"$C" version
```

## 8. Unusual readings after a config write

- Compare with a saved `"$C" config` dump.
- `cfgupdate` reloads configuration after `wcfg`.
- Flash cannot flip a bit 0→1 without `ecfg`; a "successful" `wcfg` that does
  not change a byte is expected in that case.
- Wrong baud-code or calibration bytes need a factory reset on the device's own
  menu — do not attempt more blind writes.

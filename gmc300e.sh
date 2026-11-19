#!/usr/bin/env bash
#
# gmc300e.sh - control a GQ GMC-300E / GMC-300Re Geiger-Muller counter over USB
#
# Protocol references: GQ-RFC1201 (300 series), GQ-RFC1801 (commands shared
# with 500/600 series).  The counter presents a CH340 USB-serial adapter
# (1a86:7523) and talks 57600 8N1.  Always host-initiated; commands start
# with '<' and end with '>>'.  Multi-byte integers are big-endian.
#
# No pyserial, no sudo helper: just bash + coreutils (stty, od, timeout).
#
set -o pipefail

PROG=${0##*/}
SELF=$(readlink -f "$0" 2>/dev/null || printf '%s' "$0")
ORIGINAL_ARGS=("$@")

PORT=${GMC_PORT:-}
BAUD=${GMC_BAUD:-57600}
FD=""
VERBOSE=${GMC_VERBOSE:-0}
CSV=0
REEXEC=${GMC_REEXEC:-0}

# Row colouring for live tracking. Thresholds are in uSv/h:
#   green  < SAFE_MAX_USV          (normal/low background)
#   yellow >= SAFE_MAX_USV         (elevated - do not linger)
#   red    >= DANGER_MIN_USV       (leave now)
SAFE_MAX_USV=${GMC_SAFE_MAX_USV:-1}
DANGER_MIN_USV=${GMC_DANGER_MIN_USV:-10}
COLOR=0
[[ -t 1 && -z ${NO_COLOR:-} ]] && COLOR=1
[[ ${GMC_COLOR:-} == 1 ]] && COLOR=1
[[ ${GMC_COLOR:-} == 0 ]] && COLOR=0

# ---------------------------------------------------------------------------
# output helpers
# ---------------------------------------------------------------------------
log()  { printf '%s\n' "$*"; }
note() { (( VERBOSE )) && printf '[i] %s\n' "$*" >&2; return 0; }
warn() { printf 'warning: %s\n' "$*" >&2; return 0; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

color_code() {
    case $1 in
        green)  printf '\033[32m' ;;
        yellow) printf '\033[33m' ;;
        red)    printf '\033[31m' ;;
        *)      printf '\033[0m' ;;
    esac
}
RESET_COLOR=$'\033[0m'

colored_log() {
    local level=$1; shift
    if (( COLOR )); then
        printf '%s%s%s\n' "$(color_code "$level")" "$*" "$RESET_COLOR"
    else
        printf '%s\n' "$*"
    fi
}

# Classify a dose rate in uSv/h into green / yellow / red.
level_for_usv() {
    awk -v u="$1" -v s="$SAFE_MAX_USV" -v d="$DANGER_MIN_USV" 'BEGIN {
        if (u >= d) print "red"
        else if (u >= s) print "yellow"
        else print "green"
    }'
}

# Radiation-safety wording for a level, with that level's cut-off in brackets.
# Use %s, not %g: bash printf honours the locale's decimal separator.
level_label() {
    case ${1:-} in
        green)  printf 'NORMAL[<%sµSv/h]' "$SAFE_MAX_USV" ;;
        yellow) printf 'CAUTION[>=%sµSv/h]' "$SAFE_MAX_USV" ;;
        red)    printf 'EVACUATE[>=%sµSv/h]' "$DANGER_MIN_USV" ;;
        *)      printf 'UNKNOWN' ;;
    esac
}

# ---------------------------------------------------------------------------
# help
# ---------------------------------------------------------------------------
help() {
cat <<EOF
$PROG - GQ GMC-300E Geiger counter controller (USB / CH340, 57600 8N1)

USAGE
  $PROG [global options] COMMAND [command options]

GLOBAL OPTIONS
  -p, --port DEVICE     serial device (default: auto-detect via
                        /dev/serial/by-id/usb-1a86_USB_Serial-if00-port0,
                        falling back to /dev/ttyUSB0)
  -b, --baud RATE       baud rate (default: 57600, env GMC_BAUD)
  -v, --verbose         trace every command and raw hex response
      --csv             machine-readable output for 'live' and 'status'
      --color           force ANSI colour in live rows (auto on a TTY)
      --no-color        disable ANSI colour in live rows
  -h, --help            show this help
      --version         show script version

INFO / READ
  version               model + firmware string            (<GETVER>>, 14 ASCII)
  status                one-shot summary: model, power, battery, CPM, CPS,
                        display unit + converted value, clock, serial number
  cpm                   current counts per minute           (<GETCPM>>, 2 bytes)
  cps                   current counts per second           (<GETCPS>>, 2 bytes,
                        top bit masked)
  maxcps                highest CPS since power-on          (<GETMAXCPS>>)
  volt                  battery voltage                     (<GETVOLT>>, 1 byte)
  serial                serial number as hex                (<GETSERIAL>>, 7 bytes)
  datetime              counter's date and time             (<GETDATETIME>>)
  config                full 256-byte config hex dump; with -v also decodes
                        the known fields                    (<GETCFG>>)
  displayunit           read the unit the LCD is set to: CPM, µSv/h or mR/h
                        (config byte 3). 'live' uses this same unit.
  displayvalue          read the instrument's CPM and print the converted value
                        in the LCD's unit (2 decimals, like the screen) plus a
                        3-decimal 'precise' value so small changes are visible.
  temp                  raw temperature response (not fitted on 300E)
  gyro                  raw gyro response (not fitted on 300E)
  history [ADDR] [LEN]  read flash history, hex dump        (<SPIR...>>);
                        ADDR default 0, LEN default 64

LIVE / STREAMING
  live [SECONDS]        real-time tracking using the hardware heartbeat.
                        Reads the unit set on the device LCD (config byte 3)
                        before starting and shows a converted rate in that
                        same unit, so the live view matches the display.
                        The converted value uses the instrument's own CPM
                        (polled every second), not the heartbeat-derived
                        average, because the LCD uses the instrument's CPM too.
                        Prints timestamp, CPS, cumulative counts, heartbeat
                        CPM, device CPM, the LCD-style value (2 decimals), a
                        more precise 3-decimal value and a LEVEL once per
                        second. Rows are green/NORMAL, yellow/CAUTION or
                        red/EVACUATE, with the cut-off shown in brackets in
                        the LEVEL column. Defaults are NORMAL < 1 uSv/h,
                        CAUTION >= 1 uSv/h, EVACUATE >= 10 uSv/h; change them
                        with GMC_SAFE_MAX_USV / GMC_DANGER_MIN_USV. Runs until
                        Ctrl-C, or for SECONDS if given. Heartbeat is always
                        switched off on exit.
                        Example:  $PROG live 60
                                  $PROG --csv live > counts.csv

POWER / DEVICE
  on                    power on / wake the tube             (<POWERON>>)
  off                   power off the unit                   (<POWEROFF>>)
  reboot                soft reboot                          (<REBOOT>>)
  key N                 press virtual key S1..S4 (N = 0..3)  (<KEYn>>)

MEASUREMENT SETTINGS
  alarm on|off          enable/disable the alarm             (<ALARM1/0>>)
  speaker on|off        enable/disable the clicks            (<SPEAKER1/0>>)
  echo on|off           serial debug echo (no reply on 300E) (<EchoON/OFF>>)

CLOCK
  synctime              sync the counter's clock to this host's local date and
                        time, then read it back and report the difference.
                        This is the preferred way to set the clock.
  setdatetime [YY MM DD hh mm ss]
                        set the counter's clock to an explicit date/time. With
                        no arguments, the computer's local time is used (same
                        as 'synctime', without the read-back check). Bytes are
                        sent raw, not as an ASCII parameter string.
                                                              (<SETDATETIME...>>)
  setyear NN            set year (00 = 2000)                 (<SETDATEYY..>>)
  setmonth NN           set month                            (<SETDATEMM..>>)
  setday NN             set day                              (<SETDATEDD..>>)
  sethour NN            set hour                             (<SETTIMEHH..>>)
  setminute NN          set minute                           (<SETTIMEMM..>>)
  setsecond NN          set second                           (<SETTIMESS..>>)

ADVANCED / DESTRUCTIVE (require --yes)
  cfgupdate             reload configuration                 (<CFGUPDATE>>)
  wcfg ADDR VALUE       write one config byte (0..255)       (<WCFG..>>)
  ecfg                  erase all configuration (0xFF)       (<ECFG>>)
  factoryreset          restore factory defaults             (<FACTORYRESET>>)

RAW ACCESS
  raw '<COMMAND>>'      send any protocol command and print the response as
                        both hex and ASCII. Use this for commands this script
                        does not wrap. Example: $PROG raw '<GETVER>>'

PERMISSIONS
  The device node is owned by root:uucp. If your login session does not have
  the uucp group yet, this script automatically re-executes itself through
  'newgrp uucp'. If that fails, log out and back in, or run:
      sudo usermod -aG uucp "$USER"
  You can also run the script with sudo, or point GMC_PORT at a device node
  your account can already write.

EXAMPLES
  $PROG status
  $PROG cpm
  $PROG displayunit
  $PROG displayvalue
  $PROG live
  $PROG live 30
  $PROG --csv live 3600 > hour.csv
  $PROG synctime
  $PROG setdatetime
  $PROG speaker off
  $PROG --yes wcfg 1 0
  $PROG raw '<GETCPM>>'

NOTES
  * Commands are host-initiated only. Reading a command's reply with a short
    timeout is normal: the counter sends exactly the documented byte count.
  * GETCPS replies have bit 15 set as a marker; bit 14..0 are the count.
  * GETCFG byte 0 is power state on the 300 series: 0 = ON, 255 = OFF.
  * GETCFG byte 3 selects the LCD radiation unit: 0 = CPM, 1 = uSv/h,
    2 = mR/h. Conversion uses the device's own three calibration points
    (config offsets 8, 14, 20) with piecewise-linear interpolation, then
    divides by 10 for mR/h, matching the instrument's 328 uSv/h = 32.8 mR/h
    ranges. 'displayvalue' converts the instrument's current CPM exactly the
    same way; use it to compare directly with the LCD.
  * The LCD and 'live' can differ slightly because the instrument's CPM is a
    rolling value that changes every few seconds, while the heartbeat CPM is
    a running average since 'live' started. On this calibration ~15 CPM is one
    0.01 mR/h display step; near a rounding boundary only a few counts are
    needed to flip the displayed step, so small differences are normal.
  * Live row colours use dose rate: green/NORMAL < GMC_SAFE_MAX_USV (default
    1 uSv/h = 0.1 mR/h), yellow/CAUTION from there to GMC_DANGER_MIN_USV
    (default 10 uSv/h = 1 mR/h), red/EVACUATE at or above that. The LEVEL
    column spells this out with the cut-off in brackets. These are
    conservative log/alert aids, not radiation-safety advice; adjust the env
    vars for your own limits. Use --no-color for plain logs; --csv is always
    uncoloured.
  * A freshly woken counter can report 0 CPM for a short time. The tube can
    also need a couple of minutes to recover after a high dose. 'on' or a
    physical key press wakes it; 'reboot' clears latched states.
  * After 'off' the unit stops answering until you send 'on' (or press a key).
  * 'key N' really presses a button: S2/S3 can open an on-screen menu where
    serial commands are ignored until you press S1 ('key 0') to go back.
  * ECFG, WCFG and FACTORYRESET change stored configuration. Wrong values can
    make the counter misbehave; use --yes only when you intend the change.

Version 1.0. Protocol: GQ-RFC1201 v1.40 / GQ-RFC1801 v1.00.
EOF
}

# ---------------------------------------------------------------------------
# port handling
# ---------------------------------------------------------------------------
detect_port() {
    [[ -n "$PORT" ]] && return 0
    local p=/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0
    if [[ -e "$p" ]]; then PORT=$p; return 0; fi
    for p in /dev/ttyUSB*; do
        [[ -e "$p" ]] && { PORT=$p; return 0; }
    done
    PORT=/dev/ttyUSB0
}

group_has_user() {
    local grp=$1 user members line
    user=$(id -un)
    line=$(getent group "$grp" 2>/dev/null) || return 1
    IFS=: read -r _ _ _ members <<< "$line"
    [[ ",$members," == *",$user,"* ]]
}

ensure_access() {
    detect_port
    [[ -e "$PORT" ]] || die "serial port $PORT does not exist (is the counter plugged in?)"
    [[ -w "$PORT" ]] && return 0

    if [[ $REEXEC != 1 ]] && command -v newgrp >/dev/null 2>&1 && group_has_user uucp; then
        local cmd
        printf -v cmd '%q ' "$SELF" "${ORIGINAL_ARGS[@]}"
        note "no write access, re-executing with the uucp group"
        export GMC_REEXEC=1
        exec newgrp uucp -c "$cmd"
    fi

    die "no write access to $PORT (owner/group: $(stat -Lc '%U:%G' "$PORT" 2>/dev/null)).
     Log out and back in, use 'newgrp uucp', run with sudo, or set GMC_PORT
     to a device node your account can write."
}

open_port() {
    [[ -n "$FD" ]] && return 0
    ensure_access
    exec {FD}<>"$PORT" || die "cannot open $PORT"
    stty -F "$PORT" raw -echo min 0 time 2 "$BAUD" cs8 -parenb -cstopb 2>/dev/null \
        || die "cannot configure $PORT at $BAUD 8N1"
    note "opened $PORT at $BAUD 8N1"
    # GeigerLog uses a GETCPM nudge first; it helps a just-plugged-in unit.
    printf '%s' '<GETCPM>>' >&$FD
    drain 0.3
}

close_port() {
    [[ -n "$FD" ]] || return 0
    exec {FD}>&- 2>/dev/null || true
    FD=""
    note "port closed"
}

drain() {
    local secs=${1:-0.3}
    [[ -n "$FD" ]] || return 0
    timeout "$secs" od -An -v -tx1 <&$FD >/dev/null 2>&1 || true
}

tx() {
    printf '%s' "$1" >&$FD
    note "TX  $1"
}

# Send a list of decimal byte values as genuine raw bytes (not hex text).
# Needed for SETDATETIME / SETDATE* / SETTIME* / WCFG, whose parameters are
# binary bytes. Writing straight to the fd also allows a NUL (0x00) byte,
# which a bash variable cannot hold.
tx_bytes() {
    local fmt='' hex v
    for v in "$@"; do
        printf -v hex '%02x' "$v"
        fmt+="\\x$hex"
    done
    printf "$fmt" >&$FD
    note "TX  <raw bytes: $*>"
}

rx_hex() {
    local secs=${1:-0.6} h
    h=$(timeout "$secs" od -An -v -tx1 <&$FD 2>/dev/null | tr -d ' \n')
    note "RX  ${h:-<empty>}"
    printf '%s' "$h"
}

ask() {
    local cmd=$1 secs=${2:-0.6}
    tx "$cmd"
    rx_hex "$secs"
}

# Like ask(), but retry a fixed-length reply a few times. The counter sometimes
# misses the first request just after the port is opened; one-shot commands
# should not fail because of that.
ask_expect() {
    local cmd=$1 bytes=$2 secs=${3:-0.8} tries=${4:-3} h='' i
    for (( i = 1; i <= tries; i++ )); do
        h=$(ask "$cmd" "$secs")
        if (( ${#h} >= bytes * 2 )); then
            printf '%s' "$h"
            return 0
        fi
        note "retry $i/$tries for $cmd (got $(( ${#h} / 2 )) byte(s))"
        sleep 0.2
    done
    printf '%s' "$h"
}

# ---------------------------------------------------------------------------
# hex/byte helpers
# ---------------------------------------------------------------------------
hex_to_str() {
    local h=$1 out="" h2 c
    while (( ${#h} >= 2 )); do
        h2=${h:0:2}
        printf -v c "\\x$h2"
        out+="$c"
        h=${h:2}
    done
    printf '%s' "$out"
}

hex_u16() {
    local h=$1
    [[ ${#h} -ge 4 ]] || return 1
    printf '%d' "$(( 16#${h:0:4} ))"
}

# GMC config byte 3 is the LCD's radiation display unit:
#   0 = CPM, 1 = uSv/h, 2 = mR/h (confirmed on GMC-300Re 4.70 firmware).
display_unit_name() {
    case ${1:-} in
        0) printf 'CPM' ;;
        1) printf 'µSv/h' ;;
        2) printf 'mR/h' ;;
        *) printf 'unknown(0x%02x)' "$1" ;;
    esac
}

# Decode a little-endian IEEE-754 float held as 8 hex chars (4 bytes).
# od prints the host's native float format, which is little-endian on x86_64.
hex_float_le() {
    local h=$1 esc='' i
    for (( i = 0; i < 8; i += 2 )); do esc+="\\x${h:i:2}"; done
    LC_ALL=C printf '%b' "$esc" | LC_ALL=C od -An -f | tr -d ' \n'
}

# Calibration pairs (CPM, uSv/h) read from config offsets 8, 14 and 20.
# A leading (0,0) point is added so the device's piecewise-linear conversion
# can be reproduced exactly (the same approach used by pyGMC).
CAL_POINTS=()
DISPLAY_FACTOR=154     # representative CPM per uSv/h, for the info line only

read_calibration() {
    local h=$1 i off cpm usv
    CAL_POINTS=( "0 0" )
    for i in 0 1 2; do
        off=$(( 8 + i * 6 ))
        cpm=$(( 16#${h:off*2:4} ))
        usv=$(hex_float_le "${h:(off+2)*2:8}")
        [[ "$usv" =~ ^[0-9]+(\.[0-9]+)?$ ]] || continue
        (( cpm > 0 )) || continue
        CAL_POINTS+=( "$cpm $usv" )
    done
    if (( ${#CAL_POINTS[@]} > 1 )); then
        DISPLAY_FACTOR=$(printf '%s\n' "${CAL_POINTS[@]:1}" | awk '
            { split($0, a, " "); if (a[2] > 0) { s += a[1] / a[2]; n++ } }
            END { if (n) printf "%.6f", s / n; else printf "154" }')
    fi
}

# Piecewise-linear CPM -> uSv/h conversion using the device calibration points.
cpm_to_usv() {
    local cpm=$1
    printf '%s\n' "${CAL_POINTS[@]}" | awk -v c="$cpm" '
        { split($0, a, " "); ec = a[1] + 0; eu = a[2] + 0
          if (!found && c <= ec) {
              if (ec == pc) { printf "%.6f", eu }
              else {
                  m = (eu - pu) / (ec - pc)
                  printf "%.6f", c * m + (eu - m * ec)
              }
              found = 1
              exit
          }
          pc = ec; pu = eu
        }
        END {
          if (!found) {
              if (pc > 0) printf "%.6f", c * (pu / pc)
              else printf "%.6f", c / 153.846154
          }
        }'
}

# Express a CPM value in the unit the device LCD is set to.
# Decimals default to 2, matching the LCD's 0.00 resolution for uSv/h and mR/h.
format_display_value() {
    local cpm=$1 decimals=${2:-2} usv
    case $DISPLAY_UNIT in
        0) printf '%d' "$cpm" ;;
        1|2)
            usv=$(cpm_to_usv "$cpm")
            if (( DISPLAY_UNIT == 2 )); then usv=$(awk -v u="$usv" 'BEGIN{ printf "%.6f", u / 10 }'); fi
            awk -v u="$usv" -v d="$decimals" 'BEGIN{ printf "%.*f", d, u }'
            ;;
        *) printf '?' ;;
    esac
}

# Poll the instrument's own CPM while the heartbeat is streaming. Heartbeat
# packets have bit 15 set (0x8000..0xBFFF); pass them back in POLL_HEARTBEAT so
# the live count is not short by the bytes consumed during the poll.
POLL_CPM=""
POLL_HEARTBEAT=""
poll_device_cpm() {
    local secs=${1:-0.25} h word val cpm='' hb='' attempt
    for attempt in 1 2; do
        tx '<GETCPM>>'
        h=$(rx_hex "$secs")
        while (( ${#h} >= 4 )); do
            word=${h:0:4}; h=${h:4}
            if (( 16#${word:0:1} >= 8 )); then
                hb+=$word
            else
                val=$(( 16#$word ))
                [[ -z "$cpm" ]] && cpm=$val
            fi
        done
        [[ -n "$cpm" ]] && break
    done
    POLL_CPM=$cpm
    POLL_HEARTBEAT=$hb
    note "polled device CPM: ${cpm:-none} (heartbeat bytes preserved: ${#hb}/2)"
}

need_bytes() {
    local h=$1 n=$2 what=$3
    if (( ${#h} < n * 2 )); then
        die "incomplete/no response to $what: got $(( ${#h} / 2 )) byte(s), expected $n (hex: ${h:-none})"
    fi
}

ack_ok() {
    local h=$1 what=$2
    [[ "$h" == aa || "$h" == 00 ]] || die "$what failed (expected aa or 00, got '${h:-none}')"
}

confirm_danger() {
    local desc=$1
    if [[ $YES != 1 ]]; then
        die "$desc is destructive. Re-run with --yes at the end to confirm."
    fi
}

# ---------------------------------------------------------------------------
# commands
# ---------------------------------------------------------------------------
cmd_version() {
    local h
    h=$(ask_expect '<GETVER>>' 14 0.8)
    need_bytes "$h" 14 GETVER
    log "$(hex_to_str "${h:0:28}")"
}

cmd_cpm() {
    local h
    h=$(ask_expect '<GETCPM>>' 2 0.8)
    need_bytes "$h" 2 GETCPM
    log "$(hex_u16 "$h")"
}

cmd_cps() {
    local h v
    h=$(ask_expect '<GETCPS>>' 2 0.8)
    need_bytes "$h" 2 GETCPS
    v=$(( $(hex_u16 "$h") & 0x3fff ))
    log "$v"
}

cmd_maxcps() {
    local h
    h=$(ask_expect '<GETMAXCPS>>' 2 0.8)
    need_bytes "$h" 2 GETMAXCPS
    log "$(hex_u16 "$h")"
}

cmd_volt() {
    local h v
    h=$(ask_expect '<GETVOLT>>' 1 0.8)
    need_bytes "$h" 1 GETVOLT
    v=$(( 16#${h:0:2} ))
    log "$(( v / 10 )).$(( v % 10 )) V"
}

cmd_serial() {
    local h
    h=$(ask_expect '<GETSERIAL>>' 7 0.8)
    need_bytes "$h" 7 GETSERIAL
    log "$h"
}

cmd_datetime() {
    local h
    h=$(ask_expect '<GETDATETIME>>' 7 0.8)
    need_bytes "$h" 7 GETDATETIME
    local yy=$((16#${h:0:2})) mm=$((16#${h:2:2})) dd=$((16#${h:4:2}))
    local hh=$((16#${h:6:2})) mi=$((16#${h:8:2})) ss=$((16#${h:10:2}))
    printf '20%02d-%02d-%02d %02d:%02d:%02d\n' "$yy" "$mm" "$dd" "$hh" "$mi" "$ss"
}

cmd_config() {
    local h i off
    h=$(ask_expect '<GETCFG>>' 256 2.0)
    need_bytes "$h" 256 GETCFG
    local chunk
    for (( off = 0; off < 256; off += 16 )); do
        chunk=${h:off*2:32}
        printf '%3d: %s\n' "$off" "$(printf '%s' "$chunk" | sed 's/../& /g; s/ $//')"
    done
    if (( VERBOSE )); then
        local p=$((16#${h:0:2})) a=$((16#${h:2:2})) s=$((16#${h:4:2}))
        log "-- decoded --"
        log "power      : $( [[ $p == 0 ]] && echo ON || echo OFF )  (raw $p; 0=ON,255=OFF)"
        log "alarm      : $a"
        log "speaker    : $s"
        log "display    : $(display_unit_name "$((16#${h:6:2}))")  (config byte 3; 0=CPM,1=uSv/h,2=mR/h)"
        log "save type  : $((16#${h:64:2}))  (0=off,1=CPS/s,2=CPM/min,...)"
        log "battery    : $((16#${h:112:2}))  (0=rechargeable,1=non-rechargeable)"
        log "baud code  : $((16#${h:114:2}))"
    fi
}

cmd_status() {
    local ver power volt cpm cps dt serial addr unit cpmn disp precise
    open_port
    ver=$(hex_to_str "$(ask_expect '<GETVER>>' 14 0.8)")
    addr=$(ask_expect '<GETCFG>>' 256 2.0)
    need_bytes "$addr" 256 GETCFG
    power=$((16#${addr:0:2}))
    DISPLAY_UNIT=$((16#${addr:6:2}))
    unit=$(display_unit_name "$DISPLAY_UNIT")
    read_calibration "$addr"
    volt=$(ask_expect '<GETVOLT>>' 1 0.8); need_bytes "$volt" 1 GETVOLT
    cpm=$(ask_expect '<GETCPM>>' 2 0.8);   need_bytes "$cpm" 2 GETCPM
    cps=$(ask_expect '<GETCPS>>' 2 0.8);   need_bytes "$cps" 2 GETCPS
    dt=$(ask_expect '<GETDATETIME>>' 7 0.8); need_bytes "$dt" 7 GETDATETIME
    serial=$(ask_expect '<GETSERIAL>>' 7 0.8); need_bytes "$serial" 7 GETSERIAL
    local vv=$((16#${volt:0:2}))
    local cpsv=$(( $(hex_u16 "$cps") & 0x3fff ))
    cpmn=$(hex_u16 "$cpm")
    disp=$(format_display_value "$cpmn" 2)
    precise=$(format_display_value "$cpmn" 3)
    if (( CSV )); then
        printf 'model,power,battery_v,cpm,cps,display_unit,display_value,display_precise,datetime,serial\n'
        printf '"%s",%s,%d.%d,%s,%d,%s,%s,%s,"20%02d-%02d-%02d %02d:%02d:%02d",%s\n' \
            "$ver" "$([[ $power == 0 ]] && echo ON || echo OFF)" \
            "$((vv/10))" "$((vv%10))" "$cpmn" "$cpsv" "$unit" "$disp" "$precise" \
            "$((16#${dt:0:2}))" "$((16#${dt:2:2}))" "$((16#${dt:4:2}))" \
            "$((16#${dt:6:2}))" "$((16#${dt:8:2}))" "$((16#${dt:10:2}))" "$serial"
    else
        log "model    : $ver"
        log "power    : $( [[ $power == 0 ]] && echo ON || echo OFF )"
        log "battery  : $((vv/10)).$((vv%10)) V"
        log "radiation: $cpmn CPM   $cpsv CPS"
        log "display  : $unit  ($disp, precise $precise)"
        printf 'clock    : 20%02d-%02d-%02d %02d:%02d:%02d\n' \
            "$((16#${dt:0:2}))" "$((16#${dt:2:2}))" "$((16#${dt:4:2}))" \
            "$((16#${dt:6:2}))" "$((16#${dt:8:2}))" "$((16#${dt:10:2}))"
        log "serial   : $serial"
    fi
}

cmd_displayunit() {
    local h u
    h=$(ask_expect '<GETCFG>>' 256 2.0)
    need_bytes "$h" 256 GETCFG
    u=$(( 16#${h:6:2} ))
    log "$(display_unit_name "$u") (config byte 3 = 0x$(printf '%02x' "$u"))"
}

cmd_displayvalue() {
    local cfg h cpmn unitname disp precise usv level label
    cfg=$(ask_expect '<GETCFG>>' 256 2.0)
    need_bytes "$cfg" 256 GETCFG
    DISPLAY_UNIT=$(( 16#${cfg:6:2} ))
    unitname=$(display_unit_name "$DISPLAY_UNIT")
    read_calibration "$cfg"
    h=$(ask_expect '<GETCPM>>' 2 0.8)
    need_bytes "$h" 2 GETCPM
    cpmn=$(hex_u16 "$h")
    disp=$(format_display_value "$cpmn" 2)
    precise=$(format_display_value "$cpmn" 3)
    usv=$(cpm_to_usv "$cpmn")
    level=$(level_for_usv "$usv")
    label=$(level_label "$level")
    if (( CSV )); then
        printf 'device_cpm,display_unit,display_value,display_precise,level,level_label\n'
        printf '%d,%s,%s,%s,%s,%s\n' "$cpmn" "$unitname" "$disp" "$precise" "$level" "$label"
    else
        colored_log "$level" "device CPM $cpmn  |  $unitname $disp  (precise $precise)  |  LEVEL $label"
    fi
}

cmd_live() {
    local secs=${1:-0} stop=0 carry="" h pair v total=0 n=0 started now
    local cfg ver unitname avg_cpm disp precise dev_cpm next_poll disp_cpm
    local usv level label row stamp
    local poll_secs=1
    [[ "$secs" =~ ^[0-9]+$ ]] || die "live: SECONDS must be a non-negative integer"
    open_port

    # Read the display unit and calibration from the device before streaming,
    # so the live view uses the same unit as the LCD.
    ver=$(hex_to_str "$(ask_expect '<GETVER>>' 14 0.8)")
    cfg=$(ask_expect '<GETCFG>>' 256 2.0)
    need_bytes "$cfg" 256 GETCFG
    DISPLAY_UNIT=$(( 16#${cfg:6:2} ))
    unitname=$(display_unit_name "$DISPLAY_UNIT")
    read_calibration "$cfg"
    note "device display unit: $unitname (config byte 3 = $(printf '%02x' "$DISPLAY_UNIT")), $DISPLAY_FACTOR CPM/uSv/h representative"

    tx '<HEARTBEAT1>>'
    drain 0.4
    started=$(date +%s)
    poll_device_cpm 0.4
    carry+=$POLL_HEARTBEAT
    dev_cpm=$POLL_CPM
    next_poll=$(( started + poll_secs ))
    trap 'stop=1' INT TERM

    if (( CSV )); then
        log 'epoch,time,cps,cumulative,avg_cpm,device_cpm,display_unit,display_value,display_precise,level,level_label'
    else
        log "Live tracking - $ver"
        log "device display unit: $unitname  (calibration: $DISPLAY_FACTOR CPM/uSv/h representative)"
        log "device CPM polled every ${poll_secs}s; 'precise' shows sub-display-step movement"
        printf 'levels: %s | %s | %s\n' \
            "$(level_label green)" "$(level_label yellow)" "$(level_label red)"
        printf 'timestamp  CPS  total  hb CPM  dev CPM    %8s    %8s  %-20s\n' \
            "$unitname" "$unitname+" LEVEL
        printf '%s\n' '----------------------------------------------------------------------------------------'
    fi

    while (( stop == 0 )); do
        now=$(date +%s)
        if (( secs > 0 )) && (( now - started >= secs )); then break; fi
        if (( now >= next_poll )); then
            poll_device_cpm 0.25
            carry+=$POLL_HEARTBEAT
            [[ -n "$POLL_CPM" ]] && dev_cpm=$POLL_CPM
            next_poll=$(( now + poll_secs ))
        fi
        h="$carry$(rx_hex 1.1)"
        while (( ${#h} >= 4 )); do
            pair=${h:0:4}; h=${h:4}
            v=$(( (16#$pair) & 0x3fff ))
            total=$(( total + v )); n=$(( n + 1 ))
            avg_cpm=$(( total * 60 / n ))
            if [[ -n "$dev_cpm" ]]; then
                disp_cpm=$dev_cpm
            else
                disp_cpm=$avg_cpm
            fi
            disp=$(format_display_value "$disp_cpm" 2)
            precise=$(format_display_value "$disp_cpm" 3)
            usv=$(cpm_to_usv "$disp_cpm")
            level=$(level_for_usv "$usv")
            label=$(level_label "$level")
            stamp=$(date +%H:%M:%S)
            if (( CSV )); then
                log "$(date +%s),$stamp,$v,$total,$avg_cpm,${dev_cpm:-},$unitname,$disp,$precise,$level,$label"
            else
                printf -v row '%s   %3d  %5d  %5d  %7s    %8s    %8s  %-20s' \
                    "$stamp" "$v" "$total" "$avg_cpm" \
                    "${dev_cpm:--}" "$disp" "$precise" "$label"
                colored_log "$level" "$row"
            fi
        done
        carry=$h
    done

    trap - INT TERM
    tx '<HEARTBEAT0>>'
    note "heartbeat stopped; $n samples, $total counts"
    close_port
}

cmd_on() {
    local h
    h=$(ask '<POWERON>>' 0.8)
    [[ -z "$h" || "$h" == 00 || "$h" == aa ]] || warn "unexpected POWERON reply: $h"
    log "power ON sent"
}

cmd_off() {
    tx '<POWEROFF>>'
    log "power OFF sent (the counter may go quiet now)"
    sleep 0.3
}

cmd_reboot() {
    local h
    h=$(ask '<REBOOT>>' 2.0)
    [[ -z "$h" || "$h" == 00 || "$h" == aa ]] || warn "unexpected REBOOT reply: $h"
    log "reboot command sent"
}

cmd_key() {
    local n=${1:-}
    [[ "$n" =~ ^[0-3]$ ]] || die "key: N must be 0..3 (S1..S4)"
    tx "<KEY${n}>>"
    log "key S$((n+1)) sent"
}

cmd_alarm() {
    local a=${1:-}
    case $a in
        on)  ack_ok "$(ask '<ALARM1>>' 0.6)" ALARM1 ;;
        off) ack_ok "$(ask '<ALARM0>>' 0.6)" ALARM0 ;;
        *) die "usage: $PROG alarm on|off" ;;
    esac
    log "alarm $a"
}

cmd_speaker() {
    local a=${1:-}
    case $a in
        on)  ack_ok "$(ask '<SPEAKER1>>' 0.6)" SPEAKER1 ;;
        off) ack_ok "$(ask '<SPEAKER0>>' 0.6)" SPEAKER0 ;;
        *) die "usage: $PROG speaker on|off" ;;
    esac
    log "speaker $a"
}

cmd_echo() {
    local a=${1:-} h
    case $a in
        on)  h=$(ask '<EchoON>>' 0.5) ;;
        off) h=$(ask '<EchoOFF>>' 0.5) ;;
        *) die "usage: $PROG echo on|off" ;;
    esac
    log "echo $a (reply: ${h:-none}; 300E usually does not answer)"
}

write_datetime() {
    local yy=$1 mm=$2 dd=$3 hh=$4 mi=$5 ss=$6
    local vals=()
    vals+=( "$((10#$yy))" "$((10#$mm))" "$((10#$dd))" )
    vals+=( "$((10#$hh))" "$((10#$mi))" "$((10#$ss))" )
    printf '%s' '<SETDATETIME' >&$FD
    tx_bytes "${vals[@]}"
    printf '%s' '>>' >&$FD
    local h
    h=$(rx_hex 0.8)
    h=${h#00}          # some firmware prefixes a 0x00 status byte
    [[ -z "$h" || "$h" == 00 || "$h" == aa ]] || warn "unexpected SETDATETIME reply: $h"
}

cmd_setdatetime() {
    local yy mm dd hh mi ss
    if (( $# >= 6 )); then
        yy=$1 mm=$2 dd=$3 hh=$4 mi=$5 ss=$6
    else
        read -r yy mm dd hh mi ss < <(date +'%y %m %d %H %M %S')
        note "using computer time: 20$yy-$mm-$dd $hh:$mi:$ss"
    fi
    write_datetime "$yy" "$mm" "$dd" "$hh" "$mi" "$ss"
    log "clock set to 20$yy-$mm-$dd $hh:$mi:$ss"
}

cmd_synctime() {
    local yy mm dd hh mi ss h dev dev_epoch now_epoch delta
    # Capture host local time as late as possible, then write it immediately.
    read -r yy mm dd hh mi ss < <(date +'%y %m %d %H %M %S')
    note "host local time: 20$yy-$mm-$dd $hh:$mi:$ss"
    write_datetime "$yy" "$mm" "$dd" "$hh" "$mi" "$ss"

    h=$(ask '<GETDATETIME>>' 0.8)
    need_bytes "$h" 7 GETDATETIME
    printf -v dev '20%02d-%02d-%02d %02d:%02d:%02d' \
        "$((16#${h:0:2}))" "$((16#${h:2:2}))" "$((16#${h:4:2}))" \
        "$((16#${h:6:2}))" "$((16#${h:8:2}))" "$((16#${h:10:2}))"

    now_epoch=$(date +%s)
    dev_epoch=$(date -d "$dev" +%s 2>/dev/null || printf '')
    log "host : $(date -d "@$now_epoch" '+%Y-%m-%d %H:%M:%S')"
    log "unit : $dev"
    if [[ -n "$dev_epoch" ]]; then
        delta=$(( now_epoch - dev_epoch ))
        (( delta < 0 )) && delta=$(( -delta ))
        if (( delta > 3 )); then
            warn "unit clock still differs from host by ${delta}s"
        else
            log "clocks agree (within ${delta}s)"
        fi
    fi
}

cmd_setfield() {
    local name=$1 cmdword=$2 val=${3:-}
    [[ "$val" =~ ^[0-9]+$ ]] && (( val >= 0 && val <= 255 )) || die "$name: value must be 0..255"
    printf '%s' "<$cmdword" >&$FD
    tx_bytes "$val"
    printf '%s' '>>' >&$FD
    local h
    h=$(rx_hex 0.8)
    [[ -z "$h" || "$h" == 00 || "$h" == aa ]] || warn "unexpected $cmdword reply: $h"
    log "$name = $val"
}

cmd_history() {
    local addr=${1:-0} len=${2:-64} cmd
    [[ "$addr" =~ ^[0-9]+$ ]] || die "history: ADDR must be an integer"
    [[ "$len" =~ ^[0-9]+$ ]] || die "history: LEN must be an integer"
    (( len > 0 && len <= 4096 )) || die "history: LEN must be 1..4096"
    printf '%s' '<SPIR' >&$FD
    tx_bytes "$(( (addr >> 16) & 255 ))" "$(( (addr >> 8) & 255 ))" "$(( addr & 255 ))" \
             "$(( (len >> 8) & 255 ))" "$(( len & 255 ))"
    printf '%s' '>>' >&$FD
    local h
    h=$(rx_hex 2.0)
    [[ -n "$h" ]] || die "no history data returned"
    log "history @ 0x$(printf '%06x' "$addr"), $((${#h}/2)) byte(s):"
    local off
    for (( off = 0; off < ${#h}; off += 32 )); do
        printf '%06x: %s\n' "$((addr + off/2))" "${h:off:32}"
    done
}

cmd_cfgupdate() { ack_ok "$(ask '<CFGUPDATE>>' 0.8)" CFGUPDATE; log "configuration reloaded"; }
cmd_ecfg()      { confirm_danger "ECFG (erase configuration)"; ack_ok "$(ask '<ECFG>>' 0.8)" ECFG; log "configuration erased"; }
cmd_factoryreset() { confirm_danger "FACTORYRESET"; ack_ok "$(ask '<FACTORYRESET>>' 0.8)" FACTORYRESET; log "factory defaults restored"; }

cmd_wcfg() {
    local addr=${1:-} val=${2:-}
    [[ "$addr" =~ ^[0-9]+$ ]] && (( addr >= 0 && addr <= 255 )) || die "wcfg: ADDR must be 0..255"
    [[ "$val" =~ ^[0-9]+$ ]] && (( val >= 0 && val <= 255 )) || die "wcfg: VALUE must be 0..255"
    confirm_danger "WCFG (write config byte $addr = $val)"
    printf '%s' '<WCFG' >&$FD
    tx_bytes "$addr" "$val"
    printf '%s' '>>' >&$FD
    ack_ok "$(rx_hex 0.8)" WCFG
    log "config[$addr] = $val"
}

cmd_raw() {
    local cmd=${1:-}
    [[ -n "$cmd" ]] || die "usage: $PROG raw '<COMMAND>>'"
    [[ "$cmd" == \<* && "$cmd" == *\>\> ]] || warn "command does not look like '<CMD>>'"
    local h
    h=$(ask "$cmd" 1.0)
    log "hex  : ${h:-<empty>}"
    log "ascii: $(hex_to_str "$h")"
}

# ---------------------------------------------------------------------------
# argument parsing
# ---------------------------------------------------------------------------
while (( $# )); do
    case $1 in
        -h|--help)      help; exit 0 ;;
        --version)      log "$PROG 1.0"; exit 0 ;;
        -v|--verbose)   VERBOSE=1; shift ;;
        --csv)          CSV=1; COLOR=0; shift ;;
        --color)        COLOR=1; shift ;;
        --no-color)     COLOR=0; shift ;;
        --yes)          YES=1; shift ;;
        -p|--port)      [[ $# -ge 2 ]] || die "$1 needs an argument"; PORT=$2; shift 2 ;;
        -b|--baud)      [[ $# -ge 2 ]] || die "$1 needs an argument"; BAUD=$2; shift 2 ;;
        --)             shift; break ;;
        -*)             die "unknown option: $1 (try --help)" ;;
        *)              break ;;
    esac
done

(( $# )) || { help; exit 0; }
CMD=$1; shift

case $CMD in
    version|ver)        open_port; cmd_version ;;
    status)             cmd_status ;;
    cpm)                open_port; cmd_cpm ;;
    cps)                open_port; cmd_cps ;;
    maxcps)             open_port; cmd_maxcps ;;
    volt|battery)       open_port; cmd_volt ;;
    serial|--serial)    open_port; cmd_serial ;;
    datetime|date)      open_port; cmd_datetime ;;
    config|cfg)         open_port; cmd_config ;;
    displayunit|unit)   open_port; cmd_displayunit ;;
    displayvalue|dose)  open_port; cmd_displayvalue ;;
    history|hist)       open_port; cmd_history "$@" ;;
    temp)               open_port; cmd_raw '<GETTEMP>>' ;;
    gyro)               open_port; cmd_raw '<GETGYRO>>' ;;
    live|track|monitor) cmd_live "${1:-0}" ;;
    on|wake)            open_port; cmd_on ;;
    off)                open_port; cmd_off ;;
    reboot)             open_port; cmd_reboot ;;
    key)                open_port; cmd_key "$@" ;;
    alarm)              open_port; cmd_alarm "$@" ;;
    speaker)            open_port; cmd_speaker "$@" ;;
    echo)               open_port; cmd_echo "$@" ;;
    synctime)           open_port; cmd_synctime ;;
    setdatetime)        open_port; cmd_setdatetime "$@" ;;
    setyear)            open_port; cmd_setfield setyear SETDATEYY "$@" ;;
    setmonth)           open_port; cmd_setfield setmonth SETDATEMM "$@" ;;
    setday)             open_port; cmd_setfield setday SETDATEDD "$@" ;;
    sethour)            open_port; cmd_setfield sethour SETTIMEHH "$@" ;;
    setminute)          open_port; cmd_setfield setminute SETTIMEMM "$@" ;;
    setsecond)          open_port; cmd_setfield setsecond SETTIMESS "$@" ;;
    cfgupdate)          open_port; cmd_cfgupdate ;;
    wcfg)               open_port; cmd_wcfg "$@" ;;
    ecfg)               open_port; cmd_ecfg ;;
    factoryreset)       open_port; cmd_factoryreset ;;
    raw)                open_port; cmd_raw "$@" ;;
    help)               help; exit 0 ;;
    *)                  die "unknown command '$CMD' (try --help)" ;;
esac

close_port

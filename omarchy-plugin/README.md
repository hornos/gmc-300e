# GMC-300E Radiation Monitor

[![Built for Omarchy: Plugin](https://raw.githubusercontent.com/tcballard/omarchy-badges/75975e5b5bf75e7ede3764bcd2950046f7abfe2c/badges/v1/omarchy-plugin.svg)](https://github.com/tcballard/omarchy-badges)

Live Geiger counter reading from a GMC-300E/300Re on the Omarchy bar

Kinds: service, bar-widget

A hosted service opens the serial port once and keeps one Rust controller
running for every bar instance. The bar item shows the controller's LCD
`display_value` in the device's own unit, coloured green, yellow or red from the
same level the controller reports. No reading is recomputed in QML.

## Requirements

- Omarchy 4 with Quattro shell-plugin support (tested on Omarchy 4.5.0-3).
- The Rust controller from this repository, built with `cargo build --release`
  in `rust/`.
- Either `gmc300e` on the shell's `PATH`, or a `binaryPath` setting pointing at
  the built binary.
- Serial access to the counter. The usual `uucp`/`dialout` group or a
  `/dev/serial/by-id` path works; see the repository README.

## Installation

From the repository root, the helper builds/installs the Rust binary and adds
the plugin:

```bash
./scripts/install-omarchy-plugin.sh
```

Or install the plugin directory manually:

```bash
omarchy plugin add "$PWD/omarchy-plugin" --enable --yes
```

The default `binaryPath` is `gmc300e`. To use a binary that is not on `PATH`,
set it inline on the bar entry in `~/.config/omarchy/shell.json`:

```json
{
  "id": "io.github.hornos.gmc-300e",
  "binaryPath": "/home/sysop/gmc-300e/rust/target/release/gmc300e"
}
```

Other inline settings:

| Key | Default | Meaning |
| --- | --- | --- |
| `binaryPath` | `gmc300e` | Rust controller executable. |
| `devicePath` | empty | Sets `GMC_PORT`; blank uses by-id auto-detection. |
| `safeMaxUsv` | `1` | Sets `GMC_SAFE_MAX_USV` (green/yellow boundary). |
| `dangerMinUsv` | `10` | Sets `GMC_DANGER_MIN_USV` (yellow/red boundary). |

## Behaviour

- The service runs `gmc300e --csv live`, which streams:
  `epoch,time,cps,cumulative,avg_cpm,device_cpm,display_unit,display_value,display_precise,level,level_label`.
- `display_value` is printed verbatim. `level` selects the colour:
  `green` `<1 µSv/h`, `yellow` `>=1 µSv/h`, `red` `>=10 µSv/h`.
- Setting the Quickshell `Process.running` to `false` sends SIGTERM. The Rust
  controller handles it, sends `<HEARTBEAT0>>` and closes the port cleanly.
- If the controller exits unexpectedly, the service restarts it with a bounded
  backoff. A silent stream dims the bar item as stale.
- The widget is compact on a vertical bar and reserves space on a horizontal
  bar so the layout does not jitter as the number changes.

## IPC

The service exposes target `io.github.hornos.gmc-300e` with `start`, `stop`,
`restart` and `status` functions. `status()` returns a JSON string with
monitoring state, last value/unit/level, sample count and error text.

## Validation and tests

```bash
cd omarchy-plugin
./tests/run
omarchy plugin validate .
```

`tests/parse_model.js` checks the CSV parser against green, yellow and red
rows, the header and malformed input. It runs when Node.js is available.

## Update

```bash
omarchy plugin update io.github.hornos.gmc-300e --yes
```

## Removal

```bash
omarchy plugin remove io.github.hornos.gmc-300e --yes
```

Removing or disabling the plugin stops the process, which clears the hardware
heartbeat before exit.

## Security

Omarchy plugins run as unsandboxed code inside `omarchy-shell`. Review this
repository and its dependencies before enabling it. The plugin does not use
`sudo` or `pkexec`, and all external text is capped and rendered as plain text.

## License

MIT © 2026 hornos

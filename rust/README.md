# gmc300e (Rust)

A dependency-free Rust port of the root [`gmc300e.sh`](../gmc300e.sh)
controller for GQ GMC-300E / GMC-300Re Geiger counters. It speaks the same
GQ-RFC1201 serial protocol on the CH340 USB adapter (57600 8N1) and implements
the same commands, output formats, colour levels and safety guards.

This lives on the `rust-controller` branch and has **not** been pushed.

## Build

```sh
cd rust
cargo build --release          # no crates.io dependencies
cargo test --release           # conversion / level unit tests
```

The binary is `rust/target/release/gmc300e`. Copy it anywhere, or run it
through Cargo:

```sh
cargo run --release -- status
cargo run --release -- --csv live 60 > counts.csv
```

## Differences from the bash version

- No external programs at run time (`stty`, `od`, `timeout`, `awk`, `date`).
  Serial configuration uses `termios` and reads use `poll()` through small
  direct libc bindings, so the release binary is self-contained apart from
  glibc.
- Environment variables and command-line options are identical
  (`GMC_PORT`, `GMC_BAUD`, `GMC_VERBOSE`, `GMC_COLOR`, `GMC_SAFE_MAX_USV`,
  `GMC_DANGER_MIN_USV`, `--csv`, `--color`, `--no-color`, `--yes`, ...).
- Automatic `newgrp uucp` re-execution is preserved for the root-owned
  device node.
- `live` uses a signal handler for Ctrl-C/SIGTERM and always sends
  `<HEARTBEAT0>>` before exiting, just like the shell trap.
- Calibration floats are read as little-endian IEEE-754 and rounded to the
  same six significant digits as `od -An -f` in the bash script, so the
  displayed values and `CPM/uSv/h` factor match.

## Verification against the device

The following were run side by side with `../gmc300e.sh` on a
`GMC-300Re 4.70` and produced equivalent output (apart from naturally
changing CPM values):

```sh
version cpm cps maxcps volt serial datetime displayunit displayvalue
status --csv status --csv displayvalue config -v config raw '<GETVER>>'
history 0 16 --no-color live 3 --csv live 2
```

Destructive commands (`wcfg`, `ecfg`, `factoryreset`) still require `--yes`.

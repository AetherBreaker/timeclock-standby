# timeclock-standby

The host side of the timeclock kiosk's standby screen, from
[the design](https://github.com/AetherBreaker/timeclock-system/blob/main/docs/superpowers/specs/2026-10-29-standby-screen-design.md):

- `watcher/`: a Rust program that shows the clock's terminal (1) or the standby screen's (7), deciding only from what it
  observes on the machine. Written once, then frozen.
- `status/`: a single-file Python program (Debian's 3.13, standard library only) that collects live starting and
  updating steps and serves the standby page. Edited on a clock when debugging.
- `display/`, `systemd/`, `plymouth/`, `install.sh`: the standby display, its units, the boot splash, and the installer
  the clock images run.

Both run on Linux only (terminal ioctls, `/proc`), so their tests do too, in containers:

```sh
docker run --rm -v "$PWD":/src -w /src -e UV_PROJECT_ENVIRONMENT=/tmp/venv ghcr.io/astral-sh/uv:python3.13-trixie-slim uv run --frozen pytest
docker run --rm -v "$PWD":/src -w /src/watcher rust:1 cargo test
```

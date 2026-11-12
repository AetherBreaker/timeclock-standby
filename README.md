# timeclock-standby

The standby screen of the timeclock kiosk, from
[the design](https://github.com/AetherBreaker/timeclock-system/blob/main/docs/superpowers/specs/2026-10-29-standby-screen-design.md),
installed by the [timeclock-host](https://github.com/AetherBreaker/timeclock-host) image (Fedora CoreOS, bootc):

- `watcher/`: a Rust program that shows the clock's window or the standby's, deciding only from what it observes on the
  machine. Screen logic only: it does not update the host.
- `status/`: a single-file Python program (standard library only) that collects live starting and updating steps and
  serves the standby page.
- `display/standby.sh`: the standby browser's launcher.
- `systemd/`, `config/`: the standby, status and watcher units, the settings templates. The image installs `display/`
  and `status/` to `/usr/libexec/timeclock-standby` and the watcher to `/usr/bin`.

It depends on timeclock-host for the display it sits in: sway's socket `/run/timeclock-display/sway.sock`, the clock
window's app id `local.timeclock.Clock`, `timeclock-clock.service`, `timeclock-display.service` (which
`timeclock-standby.service` binds to) and the kiosk browser `/usr/libexec/timeclock/kiosk.py`.

Both programs run on Linux only (terminal ioctls, `/proc`), so their tests do too, in containers:

```sh
docker run --rm -v "$PWD":/src -w /src -e UV_PROJECT_ENVIRONMENT=/tmp/venv ghcr.io/astral-sh/uv:python3.13-trixie-slim uv run --frozen pytest
docker run --rm -v "$PWD":/src -w /src/watcher rust:1 cargo test
```

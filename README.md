# timeclock-standby

The host side of the timeclock kiosk's standby screen, from
[the design](https://github.com/AetherBreaker/timeclock-system/blob/main/docs/superpowers/specs/2026-10-29-standby-screen-design.md),
installed by the [timeclock-host](https://github.com/AetherBreaker/timeclock-host) image (Fedora CoreOS, bootc):

- `watcher/`: a Rust program that shows the clock's window or the standby's, deciding only from what it observes on the
  machine, and keeps the bootc host updated (`watcher/src/update.rs`).
- `status/`: a single-file Python program (standard library only) that collects live starting and updating steps and
  serves the standby page.
- `display/`: sway's config, the kiosk browser (`kiosk.py`, WebKitGTK) and its two launchers, the display's check-in,
  RustDesk's unpacking from the clock's data folder.
- `systemd/`, `plymouth/`, `config/`: the units, the boot splash theme, the settings templates. The image installs
  `display/` and `status/` to `/usr/libexec/timeclock-standby` and the watcher to `/usr/bin`.

Both programs run on Linux only (terminal ioctls, `/proc`), so their tests do too, in containers:

```sh
docker run --rm -v "$PWD":/src -w /src -e UV_PROJECT_ENVIRONMENT=/tmp/venv ghcr.io/astral-sh/uv:python3.13-trixie-slim uv run --frozen pytest
docker run --rm -v "$PWD":/src -w /src/watcher rust:1 cargo test
```

#!/bin/sh
# The standby browser (timeclock-standby.service): the kiosk on the status page, its window tiled under the clock's
# (spec §3). It starts on fallback.html from disk, which moves to the live page once the status app answers; a failed load
# goes back there, so it never shows an error page.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
# sway's Wayland socket, in the session's runtime directory; the unit starts this only once sway answers.
export XDG_RUNTIME_DIR=/run/user/0 GDK_BACKEND=wayland GTK_A11Y=none
for s in "$XDG_RUNTIME_DIR"/wayland-*; do [ -S "$s" ] && export WAYLAND_DISPLAY="${s##*/}"; done
exec python3 "$HERE/kiosk.py" local.timeclock.Standby "file://$HERE/fallback.html"

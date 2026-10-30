#!/bin/sh
# The standby browser (timeclock-standby.service): cog on the status page, the window tiled under the clock's (spec §3).
# It starts on fallback.html from disk, which moves to the live page as soon as the status app answers, so it never
# shows an error page.
set -eu
# sway's Wayland socket, in the session's runtime directory; the unit starts this only once sway answers.
export XDG_RUNTIME_DIR=/run/user/0
for s in "$XDG_RUNTIME_DIR"/wayland-*; do [ -S "$s" ] && export WAYLAND_DISPLAY="${s##*/}"; done
exec cog --gapplication-app-id=local.timeclock.Standby --platform=wl --bg-color=black --webprocess-failure=restart \
  file:///opt/timeclock-standby/fallback.html

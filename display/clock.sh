#!/bin/sh
# The clock browser (timeclock-clock.service): cog on the clock app, hidden by sway until the watcher shows it (spec
# §3.3). It starts on clock-start.html, which moves to the clock once the app answers. A failed load ends it and systemd
# starts it again on that page: cog shows its error page then, logged as a load of about:blank, which no page of ours
# ever loads. Invisible either way: the clock window is hidden whenever the clock isn't live.
set -eu
CLOCK_URL=${CLOCK_URL:-http://127.0.0.1:8080/?kiosk}
export XDG_RUNTIME_DIR=/run/user/0
for s in "$XDG_RUNTIME_DIR"/wayland-*; do [ -S "$s" ] && export WAYLAND_DISPLAY="${s##*/}"; done
log=$(mktemp -u /run/timeclock-clock.XXXXXX)
mkfifo "$log"
trap 'rm -f "$log"' EXIT
start="file:///opt/timeclock-standby/clock-start.html?url=$(printf %s "$CLOCK_URL" | sed 's/%/%25/g; s/&/%26/g; s/?/%3F/g')"
cog --gapplication-app-id=local.timeclock.Clock --platform=wl --bg-color=black --webprocess-failure=restart "$start" \
  >"$log" 2>&1 &
cog=$!
while IFS= read -r line; do
  printf '%s\n' "$line"
  case "$line" in *"<about:blank> Loading"*)
    echo "clock.sh: a load failed; starting over"
    kill "$cog"
    ;;
  esac
done <"$log"
wait "$cog"

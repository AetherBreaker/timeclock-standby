#!/bin/sh
# The standby display (timeclock-standby.service): Firefox on the status app's page, under cage, on terminal 7.
# It checks in on its own terminal and hands back once cage is up (spec §3.3), like the kiosk does on terminal 1.
set -eu
OWN=7 # the watcher's standby_vt
export XDG_RUNTIME_DIR=/run/timeclock-standby
# A restart keeps /run: seatd refuses to start over its old socket. Debian's own seatd.service is disabled (install.sh).
rm -rf /run/seatd.sock "$XDG_RUNTIME_DIR"
mkdir -m 700 "$XDG_RUNTIME_DIR"
# A fresh profile every start: a kept one remembers the unclean stop and offers to restore the old session.
PROFILE=/tmp/standby-profile
rm -rf "$PROFILE"
mkdir -p "$PROFILE"
# Check in only from the clock's terminal or this one; a console in use (2-6) is waited out, never switched away from.
while :; do
  prev=$(sed 's/^tty//' /sys/class/tty/tty0/active)
  case "$prev" in 1 | "$OWN") break ;; esac
  sleep 1
done
perl -e 'open(T, "<", "/dev/tty0") or die "tty0: $!"; ioctl(T, 0x5606, $ARGV[0] + 0) or die "VT_ACTIVATE: $!";
  ioctl(T, 0x5607, $ARGV[0] + 0) or die "VT_WAITACTIVE: $!"' "$OWN"
exec seatd-launch -- cage -s -- /opt/timeclock-standby/handback.sh "$OWN" "$prev" \
  firefox-esr --profile "$PROFILE" --kiosk http://127.0.0.1:8091/

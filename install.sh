#!/bin/sh
# Install or update the standby screen, the screen watcher and the boot splash on a clock (timeclock-system's
# docs/superpowers/specs/2026-10-29-standby-screen-design.md, §8-9). Run as root from an unpacked release, or from this
# repository with the watcher built. Safe to run again; settings in /etc/timeclock/*.conf are never overwritten. The
# image's installer runs it in its chroot, where nothing is started (no systemd there).
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
WATCHER=$HERE/bin/timeclock-watcher # in a release
[ -f "$WATCHER" ] || WATCHER=$HERE/watcher/target/x86_64-unknown-linux-musl/release/timeclock-watcher
[ -f "$WATCHER" ] || { echo "install.sh: no watcher binary; build it or use a release" >&2; exit 1; }
RUNNING=false
[ ! -d /run/systemd/system ] || RUNNING=true

# The image's preseed installs these; a clock installed before the standby existed needs them now.
missing=""
for p in python3 perl cage seatd firefox-esr plymouth plymouth-themes librsvg2-bin; do
  dpkg-query -W -f='${Status}' "$p" 2>/dev/null | grep -q "ok installed" || missing="$missing $p"
done
# shellcheck disable=SC2086 # one word per package
[ -z "$missing" ] || DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends $missing
# Debian's seatd package runs a system seatd holding /run/seatd.sock; the standby starts its own with seatd-launch.
systemctl disable seatd.service >/dev/null 2>&1 || true
! $RUNNING || systemctl stop seatd.service >/dev/null 2>&1 || true

# The programs, the page, the standby display and its Firefox settings (the host's Firefox serves only the standby).
install -m 755 "$WATCHER" /usr/local/sbin/timeclock-watcher
install -d /opt/timeclock-standby
install -m 644 "$HERE"/status/status.py "$HERE"/status/standby.html "$HERE"/status/standby.css \
  "$HERE"/status/fallback.html /opt/timeclock-standby/
install -m 755 "$HERE"/display/standby.sh "$HERE"/display/handback.sh /opt/timeclock-standby/
install -D -m 644 "$HERE"/display/firefox/policies.json /usr/lib/firefox-esr/distribution/policies.json
install -D -m 644 "$HERE"/display/firefox/autoconfig.js /usr/lib/firefox-esr/defaults/pref/autoconfig.js
install -m 644 "$HERE"/display/firefox/standby.cfg /usr/lib/firefox-esr/standby.cfg

# Settings: written once, then left to the clock.
install -d /etc/timeclock
for f in watcher.conf standby.conf; do
  [ -e "/etc/timeclock/$f" ] || install -m 644 "$HERE/config/$f" "/etc/timeclock/$f"
done

# The units, in boot order: watcher, status app, standby display, then Docker.
install -m 644 "$HERE"/systemd/*.service /etc/systemd/system/
install -D -m 644 "$HERE"/systemd/docker-after-standby.conf /etc/systemd/system/docker.service.d/10-after-standby.conf

# The console: no login prompt on the kiosk's terminal, a 5-minute logout on 2-6, no kernel messages on screen.
systemctl mask getty@tty1.service >/dev/null
! $RUNNING || systemctl stop getty@tty1.service
install -m 644 "$HERE"/config/timeclock-tmout.sh /etc/profile.d/timeclock-tmout.sh
printf 'kernel.printk = 3 4 1 3\n' >/etc/sysctl.d/90-timeclock-quiet.conf
! $RUNNING || sysctl -q --load /etc/sysctl.d/90-timeclock-quiet.conf

# GRUB: no menu (holding Esc during its 1 s shows it), a quiet kernel and the splash.
set_grub() {
  if grep -q "^$1=" /etc/default/grub; then
    sed -i "s|^$1=.*|$1=$2|" /etc/default/grub
  else
    echo "$1=$2" >>/etc/default/grub
  fi
}
set_grub GRUB_TIMEOUT 1
set_grub GRUB_TIMEOUT_STYLE hidden
set_grub GRUB_CMDLINE_LINUX_DEFAULT '"quiet splash loglevel=3 i915.fastboot=1"'
# Cosmetic, so a failure warns rather than failing a clock's whole install; running this again on the clock fixes it.
if update-grub 2>/dev/null; then
  [ ! -x /usr/local/sbin/sync-esp ] || /usr/local/sbin/sync-esp
else
  echo "install.sh: update-grub failed; the boot menu may still show (run install.sh again)" >&2
fi

# The splash: Debian's spinner frames, with the clock's logo above them when the device has one (it stays out of git).
T=/usr/share/plymouth/themes/timeclock
install -d "$T"
cp /usr/share/plymouth/themes/spinner/animation-*.png "$T"/
install -m 644 "$HERE"/plymouth/timeclock.plymouth "$T"/
logo=$(find /data/timeclock_system_files/branding -maxdepth 1 -name '*.svg' 2>/dev/null | sort | head -n1)
if [ -n "$logo" ]; then
  rsvg-convert -h 140 "$logo" >"$T/watermark.png"
else
  rm -f "$T/watermark.png"
fi
plymouth-set-default-theme -R timeclock ||
  echo "install.sh: the splash theme didn't apply; boot may show text (run install.sh again)" >&2

units="timeclock-watcher.service timeclock-status.service timeclock-standby.service timeclock-tty1.service"
# shellcheck disable=SC2086 # one word per unit
systemctl enable $units >/dev/null 2>&1
if $RUNNING; then
  systemctl daemon-reload
  # shellcheck disable=SC2086
  systemctl restart $units
fi
echo "install.sh: done"

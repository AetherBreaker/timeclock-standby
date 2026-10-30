#!/bin/sh
# Install or update the clock's display (sway with the standby and clock browsers), the screen watcher, the status app,
# remote access and the boot splash (timeclock-system's docs/superpowers/specs/2026-10-29-standby-screen-design.md,
# §8-9). Run as root from an unpacked release, or from this repository with the watcher built. Safe to run again;
# settings in /etc/timeclock/*.conf are kept once edited. The image's installer runs it in its chroot, where nothing is
# started (no systemd there).
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
WATCHER=$HERE/bin/timeclock-watcher # in a release
[ -f "$WATCHER" ] || WATCHER=$HERE/watcher/target/x86_64-unknown-linux-musl/release/timeclock-watcher
[ -f "$WATCHER" ] || { echo "install.sh: no watcher binary; build it or use a release" >&2; exit 1; }
RUNNING=false
[ ! -d /run/systemd/system ] || RUNNING=true

# The image's preseed installs these; a clock installed before needs them now.
missing=""
for p in python3 perl sway swaybg cog pipewire wireplumber xdg-desktop-portal xdg-desktop-portal-wlr dbus-user-session \
  plymouth plymouth-themes librsvg2-bin; do
  dpkg-query -W -f='${Status}' "$p" 2>/dev/null | grep -q "ok installed" || missing="$missing $p"
done
# shellcheck disable=SC2086 # one word per package
[ -z "$missing" ] || DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends $missing
# sway takes the screen through logind's session (timeclock-display.service); a running seatd would come first.
systemctl disable seatd.service >/dev/null 2>&1 || true
! $RUNNING || systemctl stop seatd.service >/dev/null 2>&1 || true

# The programs, the pages and the display.
install -m 755 "$WATCHER" /usr/local/sbin/timeclock-watcher
install -d /opt/timeclock-standby
install -m 644 "$HERE"/status/status.py "$HERE"/status/standby.html "$HERE"/status/standby.css \
  "$HERE"/status/fallback.html "$HERE"/display/clock-start.html "$HERE"/display/sway.conf /opt/timeclock-standby/
install -m 755 "$HERE"/display/display.sh "$HERE"/display/standby.sh "$HERE"/display/clock.sh "$HERE"/display/remote.sh \
  /opt/timeclock-standby/
install -m 644 "$HERE"/systemd/timeclock-display.pam /etc/pam.d/timeclock-display
# Screen sharing for RustDesk, in root's session beside sway.
install -D -m 644 "$HERE"/display/xdpw.conf /etc/xdg/xdg-desktop-portal-wlr/config
for u in pipewire.service pipewire.socket; do
  install -D -m 644 "$HERE"/systemd/pipewire-root.conf "/etc/systemd/user/$u.d/timeclock-root.conf"
done
# RustDesk's own unit, from its package: after the display, so it finds sway's session.
install -D -m 644 "$HERE"/systemd/sweetfiresupportdesk-after-display.conf \
  /etc/systemd/system/sweetfiresupportdesk.service.d/10-timeclock.conf
# The first version's files: its hand-back script, its Docker ordering, the host Firefox's settings (before 0.2.0).
rm -f /opt/timeclock-standby/handback.sh /etc/systemd/system/docker.service.d/10-after-standby.conf \
  /usr/lib/firefox-esr/distribution/policies.json /usr/lib/firefox-esr/defaults/pref/autoconfig.js \
  /usr/lib/firefox-esr/standby.cfg

# Settings: written when absent or still the untouched template (every line a comment), then left to the clock.
install -d /etc/timeclock
for f in watcher.conf standby.conf; do
  if [ ! -e "/etc/timeclock/$f" ] || ! grep -qv '^[[:space:]]*\(#.*\)\?$' "/etc/timeclock/$f"; then
    install -m 644 "$HERE/config/$f" "/etc/timeclock/$f"
  else
    # An edited file keeps its settings, but not the keys 0.2.0 retired: the status app refuses unknown keys and the
    # watcher falls back to its defaults.
    sed -i 's/^\([[:space:]]*\)\(clock_vt\|standby_vt\|browser_process\|kiosk_service\)\([[:space:]]*=\)/\1# retired in 0.2.0: \2\3/' \
      "/etc/timeclock/$f"
  fi
done

install -m 644 "$HERE"/systemd/*.service /etc/systemd/system/

# The console: no login prompt on terminal 1, a 5-minute logout on 2-6, no kernel messages on screen.
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
# No i915.fastboot: kernel 6.9 made fastboot the default on every Intel GPU and dropped the option ("unknown parameter").
set_grub GRUB_CMDLINE_LINUX_DEFAULT '"quiet splash loglevel=3"'
# GRUB prints "Loading Linux ..." and "Loading initial ramdisk ..." at every boot, menu hidden or not, and Debian has no
# setting for it. Its 10_linux, diverted where package updates still reach it, runs behind a wrapper that drops those
# lines; editing it in place would hold back GRUB's updates on a changed config file.
G=/usr/local/lib/timeclock/10_linux.distrib
if [ "$(dpkg-divert --truename /etc/grub.d/10_linux)" != "$G" ]; then
  install -d /usr/local/lib/timeclock
  dpkg-divert --quiet --add --rename --divert "$G" /etc/grub.d/10_linux
fi
install -m 755 "$HERE"/grub/10_linux /etc/grub.d/10_linux
# Cosmetic, so a failure warns rather than failing a clock's whole install; running this again on the clock fixes it.
if update-grub 2>/dev/null; then
  [ ! -x /usr/local/sbin/sync-esp ] || /usr/local/sbin/sync-esp
else
  echo "install.sh: update-grub failed; the boot menu may still show (run install.sh again)" >&2
fi

# The splash: Debian's spinner theme's images, with the clock's logo centred when the device has one (it stays out of
# git). All of them, not just the frames: two-step refuses to show without its prompt images (lock.png and the rest) and
# Plymouth falls back to text mode. sway's background is the same logo, so splash to sway changes only the spinner;
# without a logo it's an empty image, which keeps sway's config valid.
T=/usr/share/plymouth/themes/timeclock
install -d "$T"
cp /usr/share/plymouth/themes/spinner/*.png "$T"/
install -m 644 "$HERE"/plymouth/timeclock.plymouth "$T"/
logo=$(find /data/timeclock_system_files/branding -maxdepth 1 -name '*.svg' 2>/dev/null | sort | head -n1)
if [ -n "$logo" ]; then
  rsvg-convert -h 140 "$logo" >"$T/watermark.png"
  # The standby's start page from disk shows it too, as the status app's page does (fallback.html).
  install -m 644 "$logo" /opt/timeclock-standby/logo.svg
else
  echo '<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>' | rsvg-convert >"$T/watermark.png"
  rm -f /opt/timeclock-standby/logo.svg
fi
plymouth-set-default-theme -R timeclock ||
  echo "install.sh: the splash theme didn't apply; boot may show text (run install.sh again)" >&2

# Remote access: in the chroot this installs RustDesk's dependencies; its service comes at first boot (remote.sh).
/opt/timeclock-standby/remote.sh || echo "install.sh: remote access didn't install (timeclock-remote retries at boot)" >&2

units="timeclock-watcher.service timeclock-status.service timeclock-display.service timeclock-standby.service
  timeclock-clock.service timeclock-remote.service timeclock-tty1.service"
# The first version enabled the standby into multi-user.target; it now starts with the display only.
systemctl disable timeclock-standby.service >/dev/null 2>&1 || true
# shellcheck disable=SC2086 # one word per unit
systemctl enable $units >/dev/null 2>&1
if $RUNNING; then
  systemctl daemon-reload
  # Only now, after the slow steps above, so the screen is down for seconds, not the whole install. Stopping the
  # display stops both browsers (BindsTo=), and stops the first version's cage standby, which held terminal 7.
  systemctl stop timeclock-standby.service timeclock-clock.service timeclock-display.service
  systemctl restart timeclock-watcher.service timeclock-status.service
  # shellcheck disable=SC2086
  systemctl start $units
  # Root's session manager, if it was already running (an SSH login starts one), loaded PipeWire's units before the
  # drop-in that lets them run as root: screen sharing stays off until it reloads (main-office, 2026-10-30).
  if XDG_RUNTIME_DIR=/run/user/0 systemctl --user --machine=root@ daemon-reload 2>/dev/null; then
    XDG_RUNTIME_DIR=/run/user/0 systemctl --user --machine=root@ start pipewire.socket wireplumber.service 2>/dev/null || true
    # A portal already running started without its capture backend (it failed without PipeWire): "missing a
    # capability" in RustDesk until both start again.
    XDG_RUNTIME_DIR=/run/user/0 systemctl --user --machine=root@ reset-failed xdg-desktop-portal-wlr.service 2>/dev/null || true
    XDG_RUNTIME_DIR=/run/user/0 systemctl --user --machine=root@ try-restart xdg-desktop-portal.service 2>/dev/null || true
  fi
  # RustDesk picks the screen's session once, at start: restarted now, with the display up.
  systemctl daemon-reload
  systemctl try-restart sweetfiresupportdesk.service
fi
echo "install.sh: done"

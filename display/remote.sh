#!/bin/sh
# Remote access (timeclock-remote.service at boot, and install.sh): RustDesk's package from the clock's folder, where the
# image puts it (it stays out of git), installed when it isn't yet or differs; its settings, ID included, kept in that
# folder as the kiosk container kept them, so a clock keeps its RustDesk ID (spec §3.4). No package, no remote access.
set -eu
DIR=/data/timeclock_system_files/remote
PKG=$DIR/sweetfiresupportdesk.deb
[ -f "$PKG" ] || { echo "remote.sh: no $PKG; running without remote access"; exit 0; }
install -d -m 700 "$DIR/config" /root/.config
if [ ! -L /root/.config/sweetfiresupportdesk ]; then
  rm -rf /root/.config/sweetfiresupportdesk
  ln -s "$DIR/config" /root/.config/sweetfiresupportdesk
fi
want=$(dpkg-deb -f "$PKG" Version)
have=$(dpkg-query -W -f='${Version}' sweetfiresupportdesk 2>/dev/null || true)
# Its install script sets up and starts its service only when systemd runs, so after an install in the image's chroot
# the first boot installs it again.
if [ "$want" != "$have" ] || [ ! -e /usr/lib/systemd/system/sweetfiresupportdesk.service ]; then
  DEBIAN_FRONTEND=noninteractive apt-get install -y -q --reinstall --allow-downgrades "$PKG"
fi

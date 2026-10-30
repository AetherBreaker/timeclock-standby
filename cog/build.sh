#!/bin/sh
# Debian's cog 0.18.5-1 (forky/sid) rebuilt for Debian 13, into $1: Debian 13's cog 0.18.4 turns every mouse click
# into an auxclick, so page buttons react to touch only (cog #759, fixed in 0.18.5). Run in a debian:trixie container
# (the release workflow does). The version sorts below Debian's own 0.18.5-1, so a later Debian release replaces it.
set -eu
OUT=$1
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends devscripts dpkg-dev equivs curl ca-certificates xz-utils >/dev/null
mkdir -p /build && cd /build
P=http://deb.debian.org/debian/pool/main/c/cog
for f in cog_0.18.5-1.dsc cog_0.18.5.orig.tar.xz cog_0.18.5.orig.tar.xz.asc cog_0.18.5-1.debian.tar.xz; do
  curl -fsSLO "$P/$f"
done
dpkg-source -x cog_0.18.5-1.dsc >/dev/null
cd cog-0.18.5
mk-build-deps -i -r -t "apt-get -y -qq --no-install-recommends" debian/control >/dev/null
DEBFULLNAME=timeclock DEBEMAIL=timeclock@localhost dch -b -v 0.18.5-1~timeclock13+1 -D trixie --force-distribution \
  "Rebuild for Debian 13: mouse clicks fire click, not auxclick (cog #759)."
dpkg-buildpackage -us -uc -b >/build/build.log 2>&1 || { tail -40 /build/build.log; exit 1; }
mkdir -p "$OUT"
cp /build/cog_0.18.5-1~timeclock13+1_amd64.deb "$OUT"/

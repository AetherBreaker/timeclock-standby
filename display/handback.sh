#!/bin/sh
# Run by cage as its client, which cage 0.2.0 starts only once its display is up: hand the screen back to the terminal
# that showed before this display checked in, if a display still holds it, then run the browser.
# Usage: handback.sh <own terminal> <previous terminal> <command...>
# Perl, not chvt: it's in every Debian (perl-base), and the switch mustn't wait for the other display (no VT_WAITACTIVE).
set -eu
own=$1 prev=$2
shift 2
claimed() { # in graphics mode (KDGETMODE == KD_GRAPHICS)
  perl -e 'open(T, "<", "/dev/tty$ARGV[0]") or exit 1; my $m = pack("i", 0); ioctl(T, 0x4B3B, $m) or exit 1;
    exit(unpack("i", $m) == 1 ? 0 : 1)' "$1"
}
if [ "$prev" != "$own" ] && claimed "$prev"; then
  perl -e 'open(T, "<", "/dev/tty0") or die "tty0: $!"; ioctl(T, 0x5606, $ARGV[0] + 0) or die "VT_ACTIVATE: $!"' "$prev"
fi
exec "$@"

#!/usr/bin/python3
"""The clock's kiosk browser: one WebKitGTK window with no browser UI, under its own Wayland app id, by which sway and
the watcher tell the clock window from the standby's. sway places and shows the window; this only loads pages. A failed
load goes back to the start page after a second, so no error page ever shows; a crashed web process reloads it.

Usage: kiosk.py <app-id> <start-url>
"""

# Standard library imports
import sys

# Third party imports
import gi

gi.require_version("Gdk", "4.0")
gi.require_version("Gtk", "4.0")
gi.require_version("WebKit", "6.0")
from gi.repository import Gdk, Gio, GLib, Gtk, WebKit  # noqa: E402

RETRY_MS = 1000


def main() -> int:  # noqa: D103
  app_id, start = sys.argv[1], sys.argv[2]
  # NON_UNIQUE: the clock and the standby are separate processes, each under its own systemd unit (the watcher counts
  # the clock unit's connections), never one merged instance.
  app = Gtk.Application(application_id=app_id, flags=Gio.ApplicationFlags.NON_UNIQUE)

  def activate(app: Gtk.Application) -> None:
    view = WebKit.WebView()
    black = Gdk.RGBA()
    black.parse("black")
    view.set_background_color(black)
    view.connect("context-menu", lambda *_: True)  # no right-click or long-press menu

    def back_to_start() -> bool:
      view.load_uri(start)
      return False

    def changed(view: WebKit.WebView, event: WebKit.LoadEvent) -> None:
      if event == WebKit.LoadEvent.FINISHED:
        print(f"<{view.get_uri()}> loaded", flush=True)

    def failed(_view: WebKit.WebView, _event: WebKit.LoadEvent, uri: str, error: GLib.Error) -> bool:
      print(f"<{uri}> failed ({error.message}); back to the start page", flush=True)
      GLib.timeout_add(RETRY_MS, back_to_start)
      return True  # handled: no error page

    def terminated(_view: WebKit.WebView, reason: WebKit.WebProcessTerminationReason) -> None:
      print(f"web process ended ({reason.value_nick}); reloading the start page", flush=True)
      back_to_start()

    view.connect("load-changed", changed)
    view.connect("load-failed", failed)
    view.connect("web-process-terminated", terminated)
    window = Gtk.ApplicationWindow(application=app)
    window.set_decorated(False)
    window.set_child(view)
    window.present()
    view.load_uri(start)

  app.connect("activate", activate)
  return app.run([sys.argv[0]])


if __name__ == "__main__":
  sys.exit(main())

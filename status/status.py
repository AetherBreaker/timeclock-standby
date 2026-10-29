"""The standby screen's status app: collects live starting and updating steps and serves the standby page.

Spec: timeclock-system's docs/superpowers/specs/2026-10-29-standby-screen-design.md, §5. It runs on Debian 13's own
Python 3.13 with the standard library only (not the clock app's 3.14), so a clock can be fixed by editing this file.
"""

import fcntl
import json
import os
import socket
import struct
import subprocess
import tempfile
import threading
import time
import urllib.parse
import urllib.request
from collections.abc import Callable, Iterator
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

CONFIG_PATH = Path("/etc/timeclock/standby.conf")
DEFAULTS = {
  "port": "8091",
  "banner_text": "An update is being installed. The clock will restart in about a minute.",
  "status_dir": "/data/timeclock_system_files/status",
  "branding_dir": "/data/timeclock_system_files/branding",
  "docker_socket": "/var/run/docker.sock",
  "app_service": "timeclock-system",
  "kiosk_service": "kiosk",
  "helper_image": "coollabsio/coolify-helper",
  "app_port": "8080",
  "browser_process": "firefox-esr",
  "clock_vt": "1",
}
KDGETMODE, KD_GRAPHICS = 0x4B3B, 1
# Columns of a `/proc/net/tcp` line: sl, local address, remote address, state.
REMOTE, STATE = 2, 3
VERSION_LABEL = "org.opencontainers.image.version"


def load_config(path: Path = CONFIG_PATH) -> dict[str, str]:
  """`key = value` lines and `#` comments, every key optional; a missing file means the defaults."""
  config = dict(DEFAULTS)
  try:
    text = path.read_text()
  except FileNotFoundError:
    return config
  for n, raw in enumerate(text.splitlines(), 1):
    line = raw.split("#", 1)[0].strip()
    if not line:
      continue
    key, sep, value = line.partition("=")
    if not sep or key.strip() not in DEFAULTS:
      raise ValueError(f"{path}:{n}: expected a known `key = value`: {raw!r}")
    config[key.strip()] = value.strip()
  return config


def has_established_to(table: str, port: int) -> bool:
  """Whether a `/proc/net/tcp` or `tcp6` table holds an ESTABLISHED (`01`) connection to remote `port`."""
  remote_port = f":{port:04X}"
  for line in table.splitlines()[1:]:
    fields = line.split()
    if len(fields) > STATE and fields[STATE] == "01" and fields[REMOTE].endswith(remote_port):
      return True
  return False


def image_matches(image: str, helper: str) -> bool:
  """Whether an image reference names `helper`, with any registry, tag or digest (the watcher's rule)."""
  name = image.split("@", 1)[0]
  i = name.rfind(":")
  if i != -1 and "/" not in name[i:]:  # a ':' before the last '/' is a registry's port
    name = name[:i]
  return name == helper or (name.endswith(helper) and name[: -len(helper)].endswith("/"))


def apply_banner(path: Path, *, wanted: bool, text: str) -> None:
  """Write the banner file atomically, readable by the app's user, or remove it.

  Never creates the folder: the app makes it as its own user, and one made by root would lock the app out of its
  status file. No folder means no app has run yet, so nobody to show a banner to.
  """
  if not wanted:
    path.unlink(missing_ok=True)
    return
  if not path.parent.is_dir():
    return
  fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=".banner-")
  with os.fdopen(fd, "w") as f:
    json.dump({"text": text}, f)
  os.chmod(tmp, 0o644)
  os.replace(tmp, path)


class Tracker:
  """Every fact the page shows. The sources report into it; `revision` counts changes and `changed` wakes waiters."""

  def __init__(self, app_service: str = DEFAULTS["app_service"], helper_image: str = DEFAULTS["helper_image"]) -> None:
    """Start with nothing known: every step not done."""
    self.app_service, self.helper_image = app_service, helper_image
    self.changed = threading.Condition()
    self.revision = 0
    self._f: dict[str, Any] = {
      "network_ok": False,
      "docker_active": False,
      "app_exists": False,
      "app_running": False,
      "app_step": None,
      "serving": False,
      "live": False,
      "helper": False,
      "updating": False,
      "version": None,
      "built": False,
      "old_stopped": False,
      "new_started": False,
    }

  def _change(self, fn: Callable[[dict[str, Any]], object]) -> None:
    with self.changed:
      f = self._f
      before = dict(f)
      fn(f)
      # A deploy ends once the new version is live, or when the helper leaves without the old app ever stopping.
      if f["updating"] and ((f["live"] and f["new_started"]) or (not f["helper"] and not f["old_stopped"])):
        f.update(updating=False, version=None, built=False, old_stopped=False, new_started=False)
      if f != before:
        self.revision += 1
        self.changed.notify_all()

  def on_system(self, *, network_ok: bool, docker_active: bool) -> None:
    """NetworkManager's connectivity and Docker's unit state."""
    self._change(lambda f: f.update(network_ok=network_ok, docker_active=docker_active))

  def on_app_serving(self, *, serving: bool) -> None:
    """Whether the app answers its favicon."""
    self._change(lambda f: f.update(serving=serving))

  def on_clock_live(self, *, live: bool) -> None:
    """Whether the kiosk shows a working clock (the watcher's test)."""
    self._change(lambda f: f.update(live=live))

  def on_app_status(self, step: str, version: str) -> None:
    """The app's own startup step, display-ready, from its status file."""

    def apply(f: dict[str, Any]) -> None:
      f["app_step"] = step
      if f["updating"] and f["new_started"] and not f["version"]:
        f["version"] = version

    self._change(apply)

  def on_new_image(self, version: str) -> None:
    """A newly built app image's version label (the release tag, like v0.7.0)."""
    self._change(lambda f: f.update(version=version.removeprefix("v"), built=True) if f["updating"] else None)

  def on_containers(self, containers: list[dict[str, Any]]) -> None:
    """Resync from Docker's full list: at start, and after the event stream drops."""
    apps = [c for c in containers if (c.get("Labels") or {}).get("com.docker.compose.service") == self.app_service]
    running = any(c.get("State") == "running" for c in apps)
    helper = any(c.get("State") == "running" and image_matches(c.get("Image", ""), self.helper_image) for c in containers)

    def apply(f: dict[str, Any]) -> None:
      f.update(helper=helper, app_exists=bool(apps), app_running=running)
      if helper:
        f["updating"] = True
        f["old_stopped"] = f["old_stopped"] or not running
      if not running:
        f["serving"] = False

    self._change(apply)

  def on_event(self, event: dict[str, Any]) -> None:
    """One container event from Docker's stream."""
    if event.get("Type") != "container":
      return
    action = event.get("Action", "")
    attrs = (event.get("Actor") or {}).get("Attributes") or {}
    if image_matches(attrs.get("image", ""), self.helper_image):
      self._on_helper(action)
      return
    if attrs.get("com.docker.compose.service") != self.app_service:
      return
    version = (attrs.get(VERSION_LABEL) or "").removeprefix("v")

    def start(f: dict[str, Any]) -> None:
      f.update(app_exists=True, app_running=True, app_step=None, serving=False)
      if f["updating"] and f["old_stopped"]:
        f["new_started"] = True
      if f["updating"] and version:
        f["version"] = version

    if action == "start":
      self._change(start)
    elif action == "die":
      self._change(lambda f: f.update(app_running=False, serving=False, old_stopped=f["old_stopped"] or f["updating"]))

  def _on_helper(self, action: str) -> None:
    # Split out of on_event for C901. A helper starting is a deploy starting, from scratch even if one was under way.
    if action == "start":
      self._change(lambda f: f.update(helper=True, updating=True, version=None, built=False, old_stopped=False, new_started=False))
    elif action == "die":
      self._change(lambda f: f.update(helper=False))

  @property
  def banner_wanted(self) -> bool:
    """Whether the banner should show: during the build only.

    Once the old app stops the standby shows, and a banner left behind would show on the new clock's first page.
    """
    with self.changed:
      f = self._f
      return f["updating"] and f["helper"] and not f["old_stopped"]

  def snapshot(self) -> dict[str, Any]:
    """The page's content: mode, title, and each step as done, active or pending."""
    with self.changed:
      f = dict(self._f)
      revision = self.revision
    if f["updating"]:
      v = f["version"]
      title = f"Updating to version {v}" if v else "Updating"
      items = [
        ("Preparing an update", True),
        (f"Building version {v}" if v else "Building the update", f["built"] or f["old_stopped"]),
        ("Stopping the old version", f["old_stopped"]),
        (f"Starting version {v}" if v else "Starting the new version", f["new_started"] and f["app_running"]),
      ]
      if f["new_started"] and f["app_step"]:
        items.append((f["app_step"], f["serving"]))
      items.append(("Opening the clock", f["live"] and f["new_started"]))
    else:
      title = "Starting up"
      items = [("Starting up", True), ("Connecting to the network", f["network_ok"]), ("Starting Docker", f["docker_active"])]
      if f["docker_active"] and not f["app_exists"]:
        title = "Waiting for setup"
        items.append(("Waiting for setup", False))
      else:
        items.append(("Starting the time clock", f["app_running"]))
        if f["app_step"]:
          items.append((f["app_step"], f["serving"]))
        items.append(("Opening the clock", f["live"]))
    steps, active_given = [], False
    for text, done in items:
      state = "done" if done else "pending" if active_given else "active"
      active_given = active_given or not done
      steps.append({"text": text, "state": state})
    return {"mode": "updating" if f["updating"] else "starting", "title": title, "steps": steps, "revision": revision}


# -- sources: each runs in its own thread and reports into the tracker ---------------------------------------------


def docker_get(sock_path: str, path: str, timeout: float = 5) -> bytes:
  """One request over Docker's socket. HTTP/1.0, so the answer comes whole, not chunked, and the socket closes."""
  with socket.socket(socket.AF_UNIX) as s:
    s.settimeout(timeout)
    s.connect(sock_path)
    s.sendall(f"GET {path} HTTP/1.0\r\nHost: docker\r\n\r\n".encode())
    chunks = []
    while chunk := s.recv(65536):
      chunks.append(chunk)
  head, _, body = b"".join(chunks).partition(b"\r\n\r\n")
  status_line = head.split(b"\r\n", 1)[0]
  if not status_line.startswith(b"HTTP/1.") or status_line.split()[1:2] != [b"200"]:
    raise OSError(f"docker {path}: {status_line!r}")
  return body


def docker_events(sock_path: str) -> Iterator[dict[str, Any]]:
  """Docker's container and image events, one dict each, until the connection ends."""
  filters = urllib.parse.quote(json.dumps({"type": ["container", "image"]}))
  with socket.socket(socket.AF_UNIX) as s:
    s.connect(sock_path)
    s.sendall(f"GET /events?filters={filters} HTTP/1.0\r\nHost: docker\r\n\r\n".encode())
    stream = s.makefile("rb")
    for line in stream:
      if not line.strip():
        break  # end of the headers
    for line in stream:
      if line.strip():
        yield json.loads(line)


def follow_docker(tracker: Tracker, config: dict[str, str]) -> None:
  """Resync from the container list, then follow the event stream; after any end or error, wait 1 s and again."""
  sock, service = config["docker_socket"], config["app_service"]
  while True:
    try:
      tracker.on_containers(json.loads(docker_get(sock, "/containers/json?all=1")))
      for event in docker_events(sock):
        name = ((event.get("Actor") or {}).get("Attributes") or {}).get("name", "")
        if event.get("Type") == "image" and event.get("Action") == "tag":
          # Coolify names images `<app uuid>_<service>:<commit>`.
          if f"_{service}:" in name or f"-{service}:" in name:
            image = json.loads(docker_get(sock, f"/images/{urllib.parse.quote(name, safe='')}/json"))
            if version := ((image.get("Config") or {}).get("Labels") or {}).get(VERSION_LABEL):
              tracker.on_new_image(version)
        else:
          tracker.on_event(event)
    except (OSError, ValueError):
      pass
    time.sleep(1)


def poll_system(tracker: Tracker) -> None:
  """Docker's unit state and NetworkManager's connectivity, every 2 s."""

  def run(*cmd: str) -> str:
    try:
      return subprocess.run(cmd, capture_output=True, text=True, timeout=5, check=False).stdout.strip()
    except (OSError, subprocess.SubprocessError):
      return ""

  while True:
    network = run("nmcli", "-t", "-f", "CONNECTIVITY", "general")
    docker = run("systemctl", "is-active", "docker")
    tracker.on_system(network_ok=network in ("full", "limited", "portal"), docker_active=docker == "active")
    time.sleep(2)


def poll_app_status(tracker: Tracker, status_dir: Path) -> None:
  """The app's own startup step, from the file it writes; reported when the file changes."""
  path, seen = status_dir / "app.json", None
  while True:
    try:
      mtime = path.stat().st_mtime_ns
      if mtime != seen:
        data = json.loads(path.read_text())
        tracker.on_app_status(str(data["step"]), str(data.get("version", "")))
        seen = mtime
    except (OSError, ValueError, KeyError, TypeError):
      pass
    time.sleep(1)


def is_claimed(vt: int) -> bool:
  """Whether a display holds terminal `vt`: it's in graphics mode."""
  try:
    fd = os.open(f"/dev/tty{vt}", os.O_RDONLY | os.O_NOCTTY)
  except OSError:
    return False
  try:
    mode = fcntl.ioctl(fd, KDGETMODE, struct.pack("i", 0))
    return struct.unpack("i", mode)[0] == KD_GRAPHICS
  except OSError:
    return False
  finally:
    os.close(fd)


def clock_connection(browser: str, port: int) -> bool:
  """The kiosk's browser (outside the host's network namespace) holds a connection to the app (the watcher's rule)."""
  try:
    host_ns = os.readlink("/proc/1/ns/net")
  except OSError:
    return False
  for proc in Path("/proc").iterdir():
    try:
      if (proc / "comm").read_text().rstrip("\n") != browser or os.readlink(proc / "ns/net") == host_ns:
        continue
      if any(has_established_to((proc / t).read_text(), port) for t in ("net/tcp", "net/tcp6")):
        return True
    except OSError:
      continue
  return False


def app_serving(config: dict[str, str]) -> bool:
  """The app answers `/favicon.ico` at its container's address (from Docker); no address or answer is no."""
  filters = urllib.parse.quote(json.dumps({"label": [f"com.docker.compose.service={config['app_service']}"]}))
  try:
    for c in json.loads(docker_get(config["docker_socket"], f"/containers/json?filters={filters}")):
      for net in ((c.get("NetworkSettings") or {}).get("Networks") or {}).values():
        if ip := net.get("IPAddress"):
          with urllib.request.urlopen(f"http://{ip}:{config['app_port']}/favicon.ico", timeout=1):
            return True
  except (OSError, ValueError):
    pass
  return False


def poll_clock(tracker: Tracker, config: dict[str, str]) -> None:
  """Whether the clock is live (as the watcher sees it), and while it isn't, whether the app already serves; every 1 s."""
  vt, port = int(config["clock_vt"]), int(config["app_port"])
  while True:
    live = is_claimed(vt) and clock_connection(config["browser_process"], port)
    tracker.on_clock_live(live=live)
    tracker.on_app_serving(serving=live or app_serving(config))
    time.sleep(1)


def keep_banner(tracker: Tracker, path: Path, text: str) -> None:
  """Write or remove the banner file as `banner_wanted` changes; at start, clear one a crash may have left."""
  last = None
  while True:
    wanted = tracker.banner_wanted
    if wanted != last:
      try:
        apply_banner(path, wanted=wanted, text=text)
        last = wanted
      except OSError:
        pass
    with tracker.changed:
      tracker.changed.wait(timeout=5)


# -- the page ----------------------------------------------------------------------------------------------------

FILES = {"/": ("standby.html", "text/html; charset=utf-8"), "/standby.css": ("standby.css", "text/css; charset=utf-8")}
KEEPALIVE_SECS = 15


class StandbyServer(ThreadingHTTPServer):
  """The page's server: what it serves from, and the tracker its event streams follow."""

  daemon_threads = True

  def __init__(self, port: int, tracker: Tracker, files: Path, branding: Path) -> None:
    """Bind to localhost only; the workstation reaches it through an SSH tunnel."""
    super().__init__(("127.0.0.1", port), Handler)
    self.tracker, self.files, self.branding = tracker, files, branding


class Handler(BaseHTTPRequestHandler):
  """`/` and `/standby.css` from the files beside this one, `/logo.svg` from the branding folder, `/events` live."""

  server: StandbyServer

  def do_GET(self) -> None:
    """Serve one request."""
    path = urllib.parse.urlsplit(self.path).path
    if path == "/events":
      self._events()
      return
    if path in FILES:
      name, kind = FILES[path]
      self._send(self.server.files / name, kind)
    elif path == "/logo.svg" and (logos := sorted(self.server.branding.glob("*.svg"))):
      self._send(logos[0], "image/svg+xml")
    else:
      self.send_error(404)

  def _send(self, file: Path, kind: str) -> None:
    body = file.read_bytes()
    self.send_response(200)
    self.send_header("Content-Type", kind)
    self.send_header("Content-Length", str(len(body)))
    self.send_header("Cache-Control", "no-store")
    self.end_headers()
    self.wfile.write(body)

  def _events(self) -> None:
    # Server-Sent Events: the snapshot now and after each change, a comment every 15 s to keep the stream alive.
    tracker = self.server.tracker
    self.send_response(200)
    self.send_header("Content-Type", "text/event-stream")
    self.send_header("Cache-Control", "no-store")
    self.end_headers()
    self.close_connection = True
    sent = None
    try:
      while True:
        with tracker.changed:
          if tracker.revision == sent:
            tracker.changed.wait(timeout=KEEPALIVE_SECS)
        snap = tracker.snapshot()
        if snap["revision"] == sent:
          self.wfile.write(b": keepalive\n\n")
        else:
          self.wfile.write(f"data: {json.dumps(snap)}\n\n".encode())
          sent = snap["revision"]
        self.wfile.flush()
    except OSError:
      return  # the page went away

  def log_message(self, format: str, *args: object) -> None:  # noqa: A002 (the base class's name)
    """Quiet: the page polls nothing, and errors show in the journal through the sources."""


def make_server(config: dict[str, str], tracker: Tracker, files: Path) -> StandbyServer:
  """The page's server on the configured port, not yet serving."""
  return StandbyServer(int(config["port"]), tracker, files, Path(config["branding_dir"]))


def main() -> None:
  """Start every source in its own thread, then serve the page."""
  config = load_config()
  tracker = Tracker(config["app_service"], config["helper_image"])
  status_dir = Path(config["status_dir"])
  sources: list[tuple[Callable[..., None], tuple[Any, ...]]] = [
    (follow_docker, (tracker, config)),
    (poll_system, (tracker,)),
    (poll_app_status, (tracker, status_dir)),
    (poll_clock, (tracker, config)),
    (keep_banner, (tracker, status_dir / "banner.json", config["banner_text"])),
  ]
  for target, args in sources:
    threading.Thread(target=target, args=args, daemon=True, name=target.__name__).start()
  make_server(config, tracker, Path(__file__).resolve().parent).serve_forever()


if __name__ == "__main__":
  main()

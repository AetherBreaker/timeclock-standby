"""The standby page's server: the page, its files and the live event stream."""

import http.client
import json
import threading
from collections.abc import Iterator
from http import HTTPStatus
from pathlib import Path

import pytest

import status
from status import Tracker, load_config, make_server


@pytest.fixture
def served(tmp_path: Path) -> Iterator[tuple[int, Tracker, Path]]:
  branding = tmp_path / "branding"
  branding.mkdir()
  config = {**status.DEFAULTS, "port": "0", "branding_dir": str(branding)}
  tracker = Tracker()
  server = make_server(config, tracker, Path(status.__file__).parent)
  threading.Thread(target=server.serve_forever, daemon=True).start()
  yield server.server_address[1], tracker, branding
  server.shutdown()
  server.server_close()


def get(port: int, path: str) -> tuple[int, str, bytes]:
  conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
  conn.request("GET", path)
  resp = conn.getresponse()
  return resp.status, resp.getheader("Content-Type") or "", resp.read()


def read_event(resp: http.client.HTTPResponse) -> dict:
  while True:
    line = resp.readline().decode()
    if line.startswith("data: "):
      return json.loads(line.removeprefix("data: "))


def test_root_serves_the_page(served: tuple[int, Tracker, Path]) -> None:
  status_code, kind, body = get(served[0], "/")
  assert status_code == HTTPStatus.OK
  assert kind.startswith("text/html")
  assert b'new EventSource("/events")' in body
  assert b'href="/standby.css"' in body


def test_css_served(served: tuple[int, Tracker, Path]) -> None:
  status_code, kind, _ = get(served[0], "/standby.css")
  assert (status_code, kind.split(";")[0]) == (HTTPStatus.OK, "text/css")


def test_logo_served_when_present(served: tuple[int, Tracker, Path]) -> None:
  port, _, branding = served
  assert get(port, "/logo.svg")[0] == HTTPStatus.NOT_FOUND
  (branding / "SFT-Logo.svg").write_text("<svg/>")
  status_code, kind, body = get(port, "/logo.svg")
  assert (status_code, kind, body) == (HTTPStatus.OK, "image/svg+xml", b"<svg/>")


def test_unknown_path_is_404(served: tuple[int, Tracker, Path]) -> None:
  assert get(served[0], "/../status.py")[0] == HTTPStatus.NOT_FOUND
  assert get(served[0], "/updating")[0] == HTTPStatus.NOT_FOUND


def test_events_send_the_snapshot_first(served: tuple[int, Tracker, Path]) -> None:
  port, tracker, _ = served
  conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
  conn.request("GET", "/events")
  resp = conn.getresponse()
  assert resp.getheader("Content-Type") == "text/event-stream"
  assert read_event(resp) == tracker.snapshot()


def test_events_send_each_change(served: tuple[int, Tracker, Path]) -> None:
  port, tracker, _ = served
  conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
  conn.request("GET", "/events")
  resp = conn.getresponse()
  first = read_event(resp)
  tracker.on_system(network_ok=True, docker_active=False)
  second = read_event(resp)
  assert second["revision"] == first["revision"] + 1
  assert second["steps"][1] == {"text": "Connecting to the network", "state": "done"}


def test_binds_localhost_only(served: tuple[int, Tracker, Path]) -> None:
  # The fixture's server is the real one; only its port differs.
  config = {**status.DEFAULTS, "port": "0"}
  server = make_server(config, Tracker(), Path(status.__file__).parent)
  try:
    assert server.server_address[0] == "127.0.0.1"
  finally:
    server.server_close()


def test_config_overrides_port(tmp_path: Path) -> None:
  conf = tmp_path / "standby.conf"
  conf.write_text("# a comment\nport = 9191\nbanner_text = Back soon\n")
  config = load_config(conf)
  assert (config["port"], config["banner_text"]) == ("9191", "Back soon")
  assert load_config(tmp_path / "missing.conf") == status.DEFAULTS
  conf.write_text("prot = 9191\n")
  with pytest.raises(ValueError, match="prot"):
    load_config(conf)

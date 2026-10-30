"""The status app's state, its parsing helpers and the banner file (pure parts; the sources are checked on a clock)."""

import json
from pathlib import Path

import status
from status import Tracker, apply_banner, established_to, held

APP = "timeclock-system"
HELPER = "coollabsio/coolify-helper:1.0.19"


def container(service: str | None, state: str = "running", image: str = "x_timeclock-system:abc", **labels: str) -> dict:
  if service is not None:
    labels["com.docker.compose.service"] = service
  return {"Id": f"id-{service}-{state}", "Image": image, "State": state, "Labels": labels}


def event(action: str, *, service: str | None = None, image: str = "x_timeclock-system:abc", **attrs: str) -> dict:
  if service is not None:
    attrs["com.docker.compose.service"] = service
  return {"Type": "container", "Action": action, "Actor": {"ID": "c1", "Attributes": {"image": image, **attrs}}}


def steps(t: Tracker) -> list[tuple[str, str]]:
  return [(s["text"], s["state"]) for s in t.snapshot()["steps"]]


def running_clock() -> Tracker:
  t = Tracker()
  t.on_system(network_ok=True, docker_active=True)
  t.on_containers([container(APP)])
  t.on_app_serving(serving=True)
  t.on_clock_live(live=True)
  return t


def test_boot_walks_the_starting_steps() -> None:
  t = Tracker()
  assert t.snapshot()["mode"] == "starting"
  assert t.snapshot()["title"] == "Starting up"
  assert steps(t)[:3] == [("Starting up", "done"), ("Connecting to the network", "active"), ("Starting Docker", "pending")]
  t.on_system(network_ok=True, docker_active=True)
  t.on_containers([container(APP, state="created")])
  assert steps(t)[1:] == [
    ("Connecting to the network", "done"),
    ("Starting Docker", "done"),
    ("Starting the time clock", "active"),
    ("Opening the clock", "pending"),
  ]
  t.on_event(event("start", service=APP))
  t.on_app_serving(serving=True)
  assert steps(t)[-2:] == [("Starting the time clock", "done"), ("Opening the clock", "active")]
  t.on_clock_live(live=True)
  assert all(state == "done" for _, state in steps(t))


def test_app_status_step_appears_in_place() -> None:
  t = Tracker()
  t.on_system(network_ok=True, docker_active=True)
  t.on_containers([container(APP)])
  t.on_app_status("Loading employees and punches", "0.6.2")
  assert steps(t)[-3:] == [
    ("Starting the time clock", "done"),
    ("Loading employees and punches", "active"),
    ("Opening the clock", "pending"),
  ]
  t.on_app_status("Starting the fingerprint reader", "0.6.2")
  assert steps(t)[-2] == ("Starting the fingerprint reader", "active")
  t.on_app_serving(serving=True)
  assert steps(t)[-2:] == [("Starting the fingerprint reader", "done"), ("Opening the clock", "active")]


def test_no_app_container_waits_for_setup() -> None:
  t = Tracker()
  t.on_system(network_ok=True, docker_active=True)
  t.on_containers([container(None, image="ghcr.io/coollabsio/sentinel:1.1.3")])
  assert t.snapshot()["title"] == "Waiting for setup"
  assert steps(t)[-1] == ("Waiting for setup", "active")


def test_helper_switches_to_updating_and_wants_the_banner() -> None:
  t = running_clock()
  assert not t.banner_wanted
  t.on_event(event("start", image=HELPER))
  snap = t.snapshot()
  assert (snap["mode"], snap["title"]) == ("updating", "Updating")
  assert steps(t)[:2] == [("Preparing an update", "done"), ("Building the update", "active")]
  assert t.banner_wanted


def test_new_image_label_names_the_version() -> None:
  t = running_clock()
  t.on_event(event("start", image=HELPER))
  t.on_new_image("0.7.0")
  assert t.snapshot()["title"] == "Updating to version 0.7.0"
  assert steps(t)[1:3] == [("Building version 0.7.0", "done"), ("Stopping the old version", "active")]


def test_banner_dropped_when_the_old_app_stops() -> None:
  # The standby takes over then; a banner left in place would show on the new clock's first page.
  t = running_clock()
  t.on_event(event("start", image=HELPER))
  t.on_event(event("die", service=APP))
  assert not t.banner_wanted
  assert t.snapshot()["mode"] == "updating"


def test_clock_live_ends_updating_and_drops_the_banner() -> None:
  t = running_clock()
  t.on_event(event("start", image=HELPER))
  t.on_event(event("die", service=APP))
  t.on_clock_live(live=False)
  t.on_app_serving(serving=False)
  t.on_event(event("start", service=APP, **{"org.opencontainers.image.version": "0.7.0"}))
  assert steps(t)[3] == ("Starting version 0.7.0", "done")
  t.on_app_status("Ready", "0.7.0")
  t.on_app_serving(serving=True)
  assert t.snapshot()["mode"] == "updating"
  assert steps(t)[-1] == ("Opening the clock", "active")
  t.on_event(event("die", image=HELPER))
  t.on_clock_live(live=True)
  assert t.snapshot()["mode"] == "starting"
  assert not t.banner_wanted


def test_helper_gone_without_restart_drops_the_banner() -> None:
  # Review Focus 4: the build failed; the old app never stopped.
  t = running_clock()
  t.on_event(event("start", image=HELPER))
  t.on_event(event("die", image=HELPER))
  assert t.snapshot()["mode"] == "starting"
  assert not t.banner_wanted


def test_resync_mid_deploy_restores_updating() -> None:
  # Review Focus 3: the status app restarts while the helper runs and the old app is already down.
  t = Tracker()
  t.on_system(network_ok=True, docker_active=True)
  t.on_containers([container(None, image=HELPER), container(APP, state="exited")])
  assert t.snapshot()["mode"] == "updating"
  assert ("Stopping the old version", "done") in steps(t)
  assert not t.banner_wanted


def test_resync_after_stream_loss_matches_containers() -> None:
  # Review Focus 2: the event stream dropped mid-deploy and the deploy failed meanwhile; the resync must end it.
  t = running_clock()
  t.on_event(event("start", image=HELPER))
  t.on_containers([container(APP)])
  assert t.snapshot()["mode"] == "starting"
  assert not t.banner_wanted


def test_revision_counts_only_changes() -> None:
  t = running_clock()
  before = t.revision
  t.on_clock_live(live=True)
  t.on_app_serving(serving=True)
  assert t.revision == before
  t.on_clock_live(live=False)
  assert t.revision == before + 1


def test_tcp6_and_close_wait() -> None:
  header = "  sl  local_address                         remote_address                        st tx_queue ... inode\n"
  line = (
    "   1: 0000000000000000FFFF0000030012AC:C944 0000000000000000FFFF0000020012AC:1F90 {} 00000000:00000000"
    " 02:00000306 00000000     0        0 49018 2 00000000d979db27 20 4 28 10 -1\n"
  )
  assert established_to(header + line.format("01"), 8080) == {49018}
  assert not established_to(header + line.format("08"), 8080)
  assert not established_to(header + line.format("01"), 8081)


def test_a_connection_counts_once_it_has_held() -> None:
  first_seen: dict[int, float] = {}
  assert not held(first_seen, 0.0, {7})
  assert held(first_seen, 1.0, {7})


def test_brief_connections_never_count() -> None:
  # Docker's proxy accepting a probe while the new app starts: a new socket each check, gone by the next.
  first_seen: dict[int, float] = {}
  for t in range(5):
    assert not held(first_seen, float(t), {100 + t} if t % 2 == 0 else set())


def test_a_dropped_connection_starts_over() -> None:
  first_seen: dict[int, float] = {}
  held(first_seen, 0.0, {7})
  held(first_seen, 0.5, set())
  assert not held(first_seen, 1.0, {7})
  assert held(first_seen, 2.0, {7})


def test_banner_written_atomically_and_removed(tmp_path: Path) -> None:
  path = tmp_path / "status" / "banner.json"
  path.parent.mkdir()  # the app's folder: the app makes it, as its own user
  apply_banner(path, wanted=True, text="An update is being installed.")
  assert json.loads(path.read_text()) == {"text": "An update is being installed."}
  assert [p.name for p in path.parent.iterdir()] == ["banner.json"]
  apply_banner(path, wanted=False, text="")
  assert not path.exists()
  apply_banner(path, wanted=False, text="")  # already gone: no error


def test_image_matches_like_the_watcher() -> None:
  assert status.image_matches("ghcr.io/coollabsio/coolify-helper:1.0.20", "coollabsio/coolify-helper")
  assert not status.image_matches("ghcr.io/coollabsio/sentinel:1.1.3", "coollabsio/coolify-helper")


def test_banner_needs_the_apps_folder(tmp_path: Path) -> None:
  # Made by root it would lock the app (an ordinary user) out of its own status file; no folder means no app yet.
  path = tmp_path / "status" / "banner.json"
  apply_banner(path, wanted=True, text="An update is being installed.")
  assert not path.parent.exists()


def test_version_label_loses_its_v() -> None:
  # GIT_TAG is the label's value: "v0.7.0" reads "version 0.7.0".
  t = running_clock()
  t.on_event(event("start", image=HELPER))
  t.on_new_image("v0.7.0")
  assert t.snapshot()["title"] == "Updating to version 0.7.0"


def test_docker_is_done_once_the_app_runs() -> None:
  # Docker starts the app's container while docker.service is still activating (it reports ready only after restoring
  # containers): the clock's step must not finish before Docker's.
  t = Tracker()
  t.on_system(network_ok=True, docker_active=False)
  t.on_event(event("start", service=APP))
  assert steps(t)[2:4] == [("Starting Docker", "done"), ("Starting the time clock", "done")]

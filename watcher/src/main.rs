//! The screen watcher's loop: observe, decide, act, every 100 ms or sooner on input. Nothing talks to it; it acts
//! through sway's control socket (the clock window) and the kernel's terminal calls (consoles).
use std::time::{Duration, Instant};
use timeclock_watcher::config::{Config, Flip, parse_config};
use timeclock_watcher::decide::{Action, Observation, State, decide};
use timeclock_watcher::keys::Inputs;
use timeclock_watcher::{observe, sway};

const CONFIG: &str = "/etc/timeclock/watcher.conf";
const TICK: Duration = Duration::from_millis(100);
const RESCAN_EVERY: Duration = Duration::from_secs(2);
const LIVE_EVERY: Duration = Duration::from_millis(500);
const DOCKER_EVERY: Duration = Duration::from_secs(2);

fn main() {
  let cfg = match std::fs::read_to_string(CONFIG) {
    Ok(text) => parse_config(&text).unwrap_or_else(|e| {
      eprintln!("{CONFIG}: {e}; using the defaults");
      Config::default()
    }),
    Err(_) => Config::default(),
  };
  eprintln!("watching: {cfg:?}");
  let start = Instant::now();
  let mut inputs = Inputs::default();
  let mut state = State::new(&cfg, observe::active_vt().unwrap_or(cfg.display_vt), Duration::ZERO);
  let mut held = observe::Held::default();
  let (mut last_input, mut last_vt_key) = (None, None);
  let (mut clock_live, mut deploying) = (false, false);
  let (mut rescanned, mut live_checked, mut docker_checked) = (start, None::<Instant>, None::<Instant>);
  let mut seen: Option<(u16, bool, Option<bool>, bool)> = None;

  loop {
    // Sleep until input arrives or the tick is up.
    let mut fds: Vec<libc::pollfd> = inputs
      .fds()
      .into_iter()
      .map(|fd| libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
      })
      .collect();
    // SAFETY: `fds` is a live, correctly sized array for the duration of the call.
    unsafe {
      libc::poll(
        fds.as_mut_ptr(),
        fds.len() as libc::nfds_t,
        TICK.as_millis() as libc::c_int,
      )
    };

    let now = start.elapsed();
    let (input, vt_key) = inputs.drain();
    if input {
      last_input = Some(now);
    }
    if let Some(n) = vt_key {
      last_vt_key = Some((now, n));
    }
    if rescanned.elapsed() >= RESCAN_EVERY {
      inputs.rescan();
      rescanned = Instant::now();
    }
    if live_checked.is_none_or(|t| t.elapsed() >= LIVE_EVERY) {
      clock_live = held.update(now, &observe::unit_connections(&cfg.clock_unit, cfg.app_port));
      live_checked = Some(Instant::now());
    }
    if cfg.flip == Flip::DeployStart && docker_checked.is_none_or(|t| t.elapsed() >= DOCKER_EVERY) {
      deploying = observe::helper_running(&cfg.docker_socket, &cfg.helper_image);
      docker_checked = Some(Instant::now());
    }
    let Ok(active_vt) = observe::active_vt() else { continue };
    // sway's tree only matters while its terminal shows; an unanswered socket reads as no clock window.
    let clock_window = if active_vt == cfg.display_vt {
      sway::clock_window(&cfg.sway_socket, &cfg.clock_app_id).unwrap_or(None)
    } else {
      None
    };

    let obs = Observation {
      now,
      active_vt,
      display_claimed: observe::is_claimed(cfg.display_vt),
      active_claimed: observe::is_claimed(active_vt),
      clock_window,
      clock_live,
      deploying,
      last_input,
      last_vt_key,
    };
    if seen != Some((active_vt, clock_live, clock_window, deploying)) {
      eprintln!(
        "terminal {active_vt}, clock live: {clock_live}, clock window shown: {clock_window:?}, deploying: {deploying}"
      );
      seen = Some((active_vt, clock_live, clock_window, deploying));
    }
    let result = match decide(&cfg, &obs, &mut state) {
      Action::Stay => continue,
      Action::Switch(vt) => observe::activate_vt(vt).map(|()| format!("switching to terminal {vt}")),
      Action::ShowClock => {
        sway::run(&cfg.sway_socket, &sway::show_command(&cfg.clock_app_id)).map(|()| "showing the clock".into())
      }
      Action::HideClock => {
        sway::run(&cfg.sway_socket, &sway::hide_command(&cfg.clock_app_id)).map(|()| "hiding the clock".into())
      }
    };
    match result {
      Ok(done) => eprintln!("{done}"),
      Err(e) => eprintln!("acting failed: {e}"),
    }
  }
}

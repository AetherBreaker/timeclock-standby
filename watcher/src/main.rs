//! The screen watcher's loop: observe, decide, switch, every 100 ms or sooner on input. It talks to nothing.
use std::time::{Duration, Instant};
use timeclock_watcher::config::{Config, Flip, parse_config};
use timeclock_watcher::decide::{Action, Observation, State, decide};
use timeclock_watcher::keys::Inputs;
use timeclock_watcher::observe;

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
  let mut state = State::new(observe::active_vt().unwrap_or(cfg.clock_vt));
  let (mut last_input, mut last_vt_key) = (None, None);
  let (mut clock_live, mut deploying) = (false, false);
  let (mut rescanned, mut live_checked, mut docker_checked) = (start, None::<Instant>, None::<Instant>);
  let mut seen: Option<(u16, bool, bool)> = None;

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
      clock_live = observe::is_claimed(cfg.clock_vt) && observe::clock_connection(&cfg.browser_process, cfg.app_port);
      live_checked = Some(Instant::now());
    }
    if cfg.flip == Flip::DeployStart && docker_checked.is_none_or(|t| t.elapsed() >= DOCKER_EVERY) {
      deploying = observe::helper_running(&cfg.docker_socket, &cfg.helper_image);
      docker_checked = Some(Instant::now());
    }
    let Ok(active_vt) = observe::active_vt() else { continue };

    let obs = Observation {
      now,
      active_vt,
      clock_claimed: observe::is_claimed(cfg.clock_vt),
      standby_claimed: observe::is_claimed(cfg.standby_vt),
      clock_live,
      deploying,
      last_input,
      last_vt_key,
    };
    if seen != Some((active_vt, clock_live, deploying)) {
      eprintln!("terminal {active_vt}, clock live: {clock_live}, deploying: {deploying}");
      seen = Some((active_vt, clock_live, deploying));
    }
    if let Action::Switch(vt) = decide(&cfg, &obs, &mut state) {
      match observe::activate_vt(vt) {
        Ok(()) => eprintln!("switching to terminal {vt}"),
        Err(e) => eprintln!("switching to terminal {vt} failed: {e}"),
      }
    }
  }
}

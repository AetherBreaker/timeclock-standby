//! The screen watcher's loop: observe, decide, act, every 100 ms or sooner on input. Nothing talks to it; it acts
//! through sway's control socket (the clock window) and the kernel's terminal calls (consoles).
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use timeclock_watcher::config::{Config, Flip, parse_config};
use timeclock_watcher::decide::{Action, Observation, State, decide};
use timeclock_watcher::keys::Inputs;
use timeclock_watcher::{observe, sway, update};

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
  // The host updater runs beside the loop; it reads when input last came (ms after `start`) to reboot only when idle.
  let last_input_ms = Arc::new(AtomicU64::new(u64::MAX));
  {
    let (cfg, last_input_ms) = (cfg.clone(), Arc::clone(&last_input_ms));
    thread::spawn(move || update::run_updater(cfg, start, last_input_ms));
  }
  let mut inputs = Inputs::default();
  let mut state = State::new(&cfg, observe::active_vt().unwrap_or(cfg.display_vt), Duration::ZERO);
  let mut held = observe::Held::default();
  let (mut last_input, mut last_vt_key) = (None, None);
  let (mut clock_live, mut deploying) = (false, false);
  let (mut rescanned, mut live_checked, mut docker_checked) = (start, None::<Instant>, None::<Instant>);
  let mut seen: Option<(u16, bool, Option<bool>, bool)> = None;
  let mut had_window: Option<bool> = None;

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
      last_input_ms.store(now.as_millis() as u64, Ordering::Relaxed);
    }
    if let Some(n) = vt_key {
      last_vt_key = Some((now, n));
    }
    if rescanned.elapsed() >= RESCAN_EVERY {
      inputs.rescan();
      rescanned = Instant::now();
    }
    // sway answers on a console too (it only stops drawing); an unanswered socket reads as no clock window.
    let clock_window = sway::clock_window(&cfg.sway_socket, &cfg.clock_app_id).unwrap_or(None);
    // A new clock window (its browser restarted) gets a fresh look at the connection: a result up to LIVE_EVERY old
    // still counts the old browser's, and would show the new window black, before its page loads.
    if had_window == Some(false) && clock_window.is_some() {
      live_checked = None;
    }
    had_window = Some(clock_window.is_some());
    if live_checked.is_none_or(|t| t.elapsed() >= LIVE_EVERY) {
      clock_live = held.update(now, &observe::unit_connections(&cfg.clock_unit, cfg.app_port));
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

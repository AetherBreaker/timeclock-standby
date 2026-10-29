//! The switching decision: observations in, an action out. Pure, so every rule is unit-tested.
use crate::config::{Config, Flip};
use std::time::Duration;

/// One look at the machine. Times are monotonic, from the watcher's start.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Observation {
  pub now: Duration,
  pub active_vt: u16,
  pub clock_claimed: bool,
  pub standby_claimed: bool,
  /// The clock's terminal is claimed and the kiosk's browser holds a connection to the app.
  pub clock_live: bool,
  /// Coolify's helper container is running (looked at in deploy-start mode only).
  pub deploying: bool,
  pub last_input: Option<Duration>,
  /// The last Ctrl+Alt+F<n> press: when, and n.
  pub last_vt_key: Option<(Duration, u16)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
  Stay,
  Switch(u16),
}

#[derive(Clone, Copy, Debug)]
enum Mode {
  Auto,
  /// A person switched with Ctrl+Alt+F<n>: hands off until they've been idle for `idle_return`.
  Manual {
    since: Duration,
  },
  /// Anyone else switched (a display checking in or out, `chvt` over SSH): hands off until `until`.
  Settling {
    until: Duration,
  },
}

#[derive(Debug)]
pub struct State {
  seen_vt: u16,
  /// The watcher's own switch not yet seen to land: its target and when it was asked for.
  pending: Option<(u16, Duration)>,
  mode: Mode,
  not_live_since: Option<Duration>,
}

impl State {
  /// The terminal showing when the watcher starts isn't anyone's switch.
  pub fn new(active_vt: u16) -> Self {
    Self {
      seen_vt: active_vt,
      pending: None,
      mode: Mode::Auto,
      not_live_since: None,
    }
  }
}

/// A switch that hasn't landed is asked for again after this long.
const RETRY: Duration = Duration::from_secs(1);

/// Decide whether to switch, and remember what this observation tells about who switched last.
pub fn decide(cfg: &Config, obs: &Observation, state: &mut State) -> Action {
  let now = obs.now;
  if obs.active_vt != state.seen_vt {
    let own = state.pending.is_some_and(|(vt, _)| vt == obs.active_vt);
    let by_key = obs
      .last_vt_key
      .is_some_and(|(at, n)| n == obs.active_vt && now.saturating_sub(at) <= cfg.user_key_window);
    state.mode = if own {
      Mode::Auto
    } else if by_key {
      Mode::Manual { since: now }
    } else {
      Mode::Settling {
        until: now + cfg.settle,
      }
    };
    state.pending = None;
    state.seen_vt = obs.active_vt;
  }
  if obs.clock_live {
    state.not_live_since = None;
  } else if state.not_live_since.is_none() {
    state.not_live_since = Some(now);
  }

  // The clock when live; the standby once it's been down for the delay, or at once for a deploy in deploy-start mode.
  let down_for = state.not_live_since.map_or(Duration::ZERO, |t| now.saturating_sub(t));
  let wanted = if cfg.flip == Flip::DeployStart && obs.deploying {
    Some(cfg.standby_vt)
  } else if obs.clock_live {
    Some(cfg.clock_vt)
  } else {
    (down_for >= cfg.standby_delay).then_some(cfg.standby_vt)
  };

  match state.mode {
    Mode::Manual { since } => {
      // The key press counts as input, so idle time runs from the last input; `since` covers a missing one.
      let idle = now.saturating_sub(obs.last_input.unwrap_or(since));
      if wanted != Some(obs.active_vt) && idle < cfg.idle_return {
        return Action::Stay;
      }
      state.mode = Mode::Auto;
    }
    Mode::Settling { until } => {
      if now < until {
        return Action::Stay;
      }
      state.mode = Mode::Auto;
    }
    Mode::Auto => {}
  }

  let Some(target) = wanted else { return Action::Stay };
  let claimed = if target == cfg.clock_vt {
    obs.clock_claimed
  } else {
    obs.standby_claimed
  };
  if target == obs.active_vt || !claimed {
    return Action::Stay;
  }
  if state
    .pending
    .is_some_and(|(vt, at)| vt == target && now.saturating_sub(at) < RETRY)
  {
    return Action::Stay;
  }
  state.pending = Some((target, now));
  Action::Switch(target)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::config::{Config, Flip};
  use std::time::Duration;

  fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
  }

  /// Clock and standby both claimed; nothing live, no input, no deploy.
  fn obs(now: u64, active: u16) -> Observation {
    Observation {
      now: ms(now),
      active_vt: active,
      clock_claimed: true,
      standby_claimed: true,
      ..Default::default()
    }
  }

  fn live(now: u64, active: u16) -> Observation {
    Observation {
      clock_live: true,
      ..obs(now, active)
    }
  }

  fn deploy_start() -> Config {
    Config {
      flip: Flip::DeployStart,
      ..Config::default()
    }
  }

  #[test]
  fn first_observation_is_not_a_switch() {
    let c = Config::default();
    let mut s = State::new(7);
    // Live at once: switches, rather than treating the first look at terminal 7 as someone else's switch.
    assert_eq!(decide(&c, &live(0, 7), &mut s), Action::Switch(1));
  }

  #[test]
  fn live_clock_shows_terminal_1() {
    let c = Config::default();
    let mut s = State::new(7);
    assert_eq!(decide(&c, &obs(0, 7), &mut s), Action::Stay);
    assert_eq!(decide(&c, &live(100, 7), &mut s), Action::Switch(1));
  }

  #[test]
  fn not_live_waits_standby_delay_then_shows_7() {
    let c = Config::default();
    let mut s = State::new(1);
    assert_eq!(decide(&c, &live(0, 1), &mut s), Action::Stay);
    assert_eq!(decide(&c, &obs(1_000, 1), &mut s), Action::Stay);
    assert_eq!(decide(&c, &obs(3_900, 1), &mut s), Action::Stay);
    assert_eq!(decide(&c, &obs(4_000, 1), &mut s), Action::Switch(7));
  }

  #[test]
  fn live_again_within_delay_cancels_the_flip() {
    let c = Config::default();
    let mut s = State::new(1);
    decide(&c, &live(0, 1), &mut s);
    assert_eq!(decide(&c, &obs(1_000, 1), &mut s), Action::Stay);
    assert_eq!(decide(&c, &live(2_000, 1), &mut s), Action::Stay);
    assert_eq!(decide(&c, &obs(3_000, 1), &mut s), Action::Stay);
    assert_eq!(decide(&c, &obs(5_900, 1), &mut s), Action::Stay); // 2.9 s since it went down again
    assert_eq!(decide(&c, &obs(6_000, 1), &mut s), Action::Switch(7));
  }

  #[test]
  fn deploy_start_mode_flips_at_once_while_deploying() {
    let c = deploy_start();
    let mut s = State::new(1);
    decide(&c, &live(0, 1), &mut s);
    let o = Observation {
      deploying: true,
      ..live(100, 1)
    };
    assert_eq!(decide(&c, &o, &mut s), Action::Switch(7));
  }

  #[test]
  fn app_stopped_mode_ignores_deploying() {
    let c = Config::default();
    let mut s = State::new(1);
    let o = Observation {
      deploying: true,
      ..live(0, 1)
    };
    assert_eq!(decide(&c, &o, &mut s), Action::Stay);
  }

  #[test]
  fn deploy_failed_returns_to_clock() {
    let c = deploy_start();
    let mut s = State::new(1);
    let deploying = Observation {
      deploying: true,
      ..live(0, 1)
    };
    assert_eq!(decide(&c, &deploying, &mut s), Action::Switch(7));
    decide(
      &c,
      &Observation {
        deploying: true,
        ..live(100, 7)
      },
      &mut s,
    ); // own switch landed
    // The helper exited and the old app never stopped: back to the clock.
    assert_eq!(decide(&c, &live(5_000, 7), &mut s), Action::Switch(1));
  }

  #[test]
  fn never_switches_to_unclaimed() {
    let c = Config::default();
    let mut s = State::new(1);
    let o = |now| Observation {
      standby_claimed: false,
      ..obs(now, 1)
    };
    decide(&c, &o(0), &mut s);
    assert_eq!(decide(&c, &o(10_000), &mut s), Action::Stay);
    let o = |now| Observation {
      clock_claimed: false,
      ..live(now, 7)
    };
    let mut s = State::new(7);
    assert_eq!(decide(&c, &o(0), &mut s), Action::Stay);
  }

  #[test]
  fn own_switch_is_not_foreign() {
    let c = Config::default();
    let mut s = State::new(7);
    assert_eq!(decide(&c, &live(0, 7), &mut s), Action::Switch(1));
    // The switch lands; the clock then dies: the standby follows the normal delay, no settling.
    assert_eq!(decide(&c, &live(100, 1), &mut s), Action::Stay);
    assert_eq!(decide(&c, &obs(200, 1), &mut s), Action::Stay);
    assert_eq!(decide(&c, &obs(3_200, 1), &mut s), Action::Switch(7));
  }

  #[test]
  fn retries_a_switch_that_didnt_land_after_1s() {
    let c = Config::default();
    let mut s = State::new(7);
    assert_eq!(decide(&c, &live(0, 7), &mut s), Action::Switch(1));
    assert_eq!(decide(&c, &live(500, 7), &mut s), Action::Stay);
    assert_eq!(decide(&c, &live(1_000, 7), &mut s), Action::Switch(1));
  }

  #[test]
  fn user_switch_holds_until_idle() {
    let c = Config::default();
    let mut s = State::new(1);
    decide(&c, &live(0, 1), &mut s);
    let console = |now, input: u64| Observation {
      last_input: Some(ms(input)),
      last_vt_key: Some((ms(9_900), 2)),
      ..live(now, 2)
    };
    assert_eq!(decide(&c, &console(10_000, 9_900), &mut s), Action::Stay);
    assert_eq!(decide(&c, &console(129_800, 9_900), &mut s), Action::Stay);
    assert_eq!(decide(&c, &console(129_900, 9_900), &mut s), Action::Switch(1));
  }

  #[test]
  fn input_resets_the_idle_timer() {
    let c = Config::default();
    let mut s = State::new(1);
    decide(&c, &live(0, 1), &mut s);
    let console = |now, input: u64| Observation {
      last_input: Some(ms(input)),
      last_vt_key: Some((ms(9_900), 2)),
      ..live(now, 2)
    };
    decide(&c, &console(10_000, 9_900), &mut s);
    assert_eq!(decide(&c, &console(130_000, 100_000), &mut s), Action::Stay);
    assert_eq!(decide(&c, &console(219_900, 100_000), &mut s), Action::Stay);
    assert_eq!(decide(&c, &console(220_000, 100_000), &mut s), Action::Switch(1));
  }

  #[test]
  fn user_switch_onto_wanted_resumes_at_once() {
    let c = Config::default();
    let mut s = State::new(1);
    decide(&c, &live(0, 1), &mut s);
    let key = |now: u64, active, n| Observation {
      last_input: Some(ms(now - 50)),
      last_vt_key: Some((ms(now - 50), n)),
      ..live(now, active)
    };
    assert_eq!(decide(&c, &key(1_000, 2, 2), &mut s), Action::Stay); // into a console: hands off
    assert_eq!(decide(&c, &key(2_000, 1, 1), &mut s), Action::Stay); // back onto the clock: resumed
    // Resumed means the normal rules run again at once: the clock dies, the standby follows after the delay.
    assert_eq!(decide(&c, &obs(2_100, 1), &mut s), Action::Stay);
    assert_eq!(decide(&c, &obs(5_100, 1), &mut s), Action::Switch(7));
  }

  #[test]
  fn key_for_another_terminal_is_not_a_user_switch() {
    let c = Config::default();
    let mut s = State::new(1);
    decide(&c, &live(0, 1), &mut s);
    // F2 was pressed, but terminal 7 came up: someone else's switch, so settle 5 s rather than hold 2 minutes.
    let o = |now| Observation {
      last_input: Some(ms(950)),
      last_vt_key: Some((ms(950), 2)),
      ..live(now, 7)
    };
    assert_eq!(decide(&c, &o(1_000), &mut s), Action::Stay);
    assert_eq!(decide(&c, &o(5_900), &mut s), Action::Stay);
    assert_eq!(decide(&c, &o(6_000), &mut s), Action::Switch(1));
  }

  #[test]
  fn stale_key_is_not_a_user_switch() {
    let c = Config::default();
    let mut s = State::new(1);
    decide(&c, &live(0, 1), &mut s);
    // Ctrl+Alt+F2 a whole second earlier doesn't explain a switch now.
    let o = |now| Observation {
      last_input: Some(ms(0)),
      last_vt_key: Some((ms(0), 2)),
      ..live(now, 2)
    };
    assert_eq!(decide(&c, &o(1_000), &mut s), Action::Stay);
    assert_eq!(decide(&c, &o(6_000), &mut s), Action::Switch(1));
  }

  #[test]
  fn other_switch_settles_5s() {
    let c = Config::default();
    let mut s = State::new(7);
    decide(&c, &obs(0, 7), &mut s);
    // A display checking in: terminal 1 comes up with no key pressed.
    assert_eq!(decide(&c, &obs(1_000, 1), &mut s), Action::Stay);
    // It hands back within the settle; the watcher waits out 5 s from the last change.
    assert_eq!(decide(&c, &obs(1_600, 7), &mut s), Action::Stay);
    assert_eq!(decide(&c, &live(4_000, 7), &mut s), Action::Stay);
    assert_eq!(decide(&c, &live(6_600, 7), &mut s), Action::Switch(1));
  }
}

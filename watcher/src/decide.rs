//! The decision: observations in, an action out. Pure, so every rule is unit-tested.
use crate::config::{Config, Flip};
use std::time::Duration;

/// One look at the machine. Times are monotonic, from the watcher's start.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Observation {
  pub now: Duration,
  pub active_vt: u16,
  /// sway holds its terminal (graphics mode).
  pub display_claimed: bool,
  /// The terminal on screen is in graphics mode: a display or the boot splash holds it, so it isn't a console.
  pub active_claimed: bool,
  /// The clock window: `None` when there's none (or sway didn't answer), else whether it shows.
  pub clock_window: Option<bool>,
  /// The clock browser holds a connection to the app (observe::Held).
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
  ShowClock,
  HideClock,
  Switch(u16),
}

#[derive(Clone, Copy, Debug)]
enum Mode {
  Auto,
  /// A person is on a console: hands off until they've been idle for `idle_return`.
  Manual {
    since: Duration,
  },
  /// Anyone else switched (`chvt` over SSH): hands off until `until`.
  Settling {
    until: Duration,
  },
}

#[derive(Debug)]
pub struct State {
  seen_vt: u16,
  /// The watcher's own switch not yet seen to land: its target and when it was asked for.
  pending: Option<(u16, Duration)>,
  /// The last window command and when: not sent again before `RETRY` while sway's tree catches up.
  sent: Option<(Action, Duration)>,
  mode: Mode,
  not_live_since: Option<Duration>,
}

impl State {
  /// A watcher (re)started on a console treats it as in use: it never pulls someone off a console on its own start.
  pub fn new(cfg: &Config, active_vt: u16, now: Duration) -> Self {
    Self {
      seen_vt: active_vt,
      pending: None,
      sent: None,
      mode: if active_vt == cfg.display_vt {
        Mode::Auto
      } else {
        Mode::Manual { since: now }
      },
      not_live_since: None,
    }
  }
}

/// A switch or window command that hasn't taken effect is sent again after this long.
const RETRY: Duration = Duration::from_secs(1);

/// Decide what to do, and remember what this observation tells about who switched last.
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

  if obs.active_vt != cfg.display_vt {
    // Off sway's terminal: a console, or the splash at boot. Back to sway's once hands-off ends.
    match state.mode {
      Mode::Manual { since } => {
        // The key press counts as input, so idle time runs from the last input; `since` covers a missing one.
        if now.saturating_sub(obs.last_input.unwrap_or(since)) < cfg.idle_return {
          return Action::Stay;
        }
      }
      Mode::Settling { until } if now < until => return Action::Stay,
      _ => {}
    }
    state.mode = Mode::Auto;
    // To sway's terminal when sway holds it, and from a console even when it doesn't: the display's check-in waits for
    // terminal 7 while a console shows, so otherwise each would wait for the other. Never away from a terminal in
    // graphics mode that sway doesn't hold: at boot that's the splash, which the display's check-in takes over itself.
    if !obs.display_claimed && obs.active_claimed {
      return Action::Stay;
    }
    if state
      .pending
      .is_some_and(|(vt, at)| vt == cfg.display_vt && now.saturating_sub(at) < RETRY)
    {
      return Action::Stay;
    }
    state.pending = Some((cfg.display_vt, now));
    return Action::Switch(cfg.display_vt);
  }
  state.mode = Mode::Auto;

  let Some(shown) = obs.clock_window else {
    return Action::Stay; // no clock window: the standby is what shows
  };
  // Shown when live; hidden once it's been down for the delay (a connection blip shouldn't flicker the screen), or at
  // once for a deploy in deploy-start mode. Within the delay, left as it is.
  let down_for = state.not_live_since.map_or(Duration::ZERO, |t| now.saturating_sub(t));
  let want = if cfg.flip == Flip::DeployStart && obs.deploying {
    false
  } else if obs.clock_live {
    true
  } else if down_for >= cfg.standby_delay {
    false
  } else {
    return Action::Stay;
  };
  if want == shown {
    state.sent = None;
    return Action::Stay;
  }
  let action = if want { Action::ShowClock } else { Action::HideClock };
  if state
    .sent
    .is_some_and(|(a, at)| a == action && now.saturating_sub(at) < RETRY)
  {
    return Action::Stay;
  }
  state.sent = Some((action, now));
  action
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::config::{Config, Flip};
  use std::time::Duration;

  fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
  }

  /// On sway's terminal, sway running, the clock window hidden; nothing live, no input, no deploy.
  fn obs(now: u64) -> Observation {
    Observation {
      now: ms(now),
      active_vt: 7,
      display_claimed: true,
      active_claimed: true,
      clock_window: Some(false),
      ..Default::default()
    }
  }

  fn live(now: u64) -> Observation {
    Observation {
      clock_live: true,
      ..obs(now)
    }
  }

  fn shown(o: Observation) -> Observation {
    Observation {
      clock_window: Some(true),
      ..o
    }
  }

  /// On console `vt` (text mode), with sway running.
  fn console(o: Observation, vt: u16) -> Observation {
    Observation {
      active_vt: vt,
      active_claimed: false,
      ..o
    }
  }

  fn state(c: &Config) -> State {
    State::new(c, 7, ms(0))
  }

  fn deploy_start() -> Config {
    Config {
      flip: Flip::DeployStart,
      ..Config::default()
    }
  }

  #[test]
  fn live_clock_is_shown() {
    let c = Config::default();
    let mut s = state(&c);
    assert_eq!(decide(&c, &obs(0), &mut s), Action::Stay);
    assert_eq!(decide(&c, &live(100), &mut s), Action::ShowClock);
    assert_eq!(decide(&c, &shown(live(200)), &mut s), Action::Stay);
  }

  #[test]
  fn not_live_waits_the_delay_then_hides() {
    let c = Config::default();
    let mut s = state(&c);
    assert_eq!(decide(&c, &shown(live(0)), &mut s), Action::Stay);
    assert_eq!(decide(&c, &shown(obs(1_000)), &mut s), Action::Stay);
    assert_eq!(decide(&c, &shown(obs(3_900)), &mut s), Action::Stay);
    assert_eq!(decide(&c, &shown(obs(4_000)), &mut s), Action::HideClock);
    assert_eq!(decide(&c, &obs(4_100), &mut s), Action::Stay);
  }

  #[test]
  fn live_again_within_the_delay_cancels_the_hide() {
    let c = Config::default();
    let mut s = state(&c);
    decide(&c, &shown(live(0)), &mut s);
    assert_eq!(decide(&c, &shown(obs(1_000)), &mut s), Action::Stay);
    assert_eq!(decide(&c, &shown(live(2_000)), &mut s), Action::Stay);
    assert_eq!(decide(&c, &shown(obs(3_000)), &mut s), Action::Stay);
    assert_eq!(decide(&c, &shown(obs(5_900)), &mut s), Action::Stay); // 2.9 s since it went down again
    assert_eq!(decide(&c, &shown(obs(6_000)), &mut s), Action::HideClock);
  }

  #[test]
  fn hidden_and_not_live_stays_hidden() {
    let c = Config::default();
    let mut s = state(&c);
    for t in [0, 3_000, 10_000] {
      assert_eq!(decide(&c, &obs(t), &mut s), Action::Stay);
    }
  }

  #[test]
  fn no_clock_window_does_nothing() {
    let c = Config::default();
    let mut s = state(&c);
    let gone = |t| Observation {
      clock_window: None,
      ..live(t)
    };
    assert_eq!(decide(&c, &gone(0), &mut s), Action::Stay);
    assert_eq!(decide(&c, &gone(5_000), &mut s), Action::Stay);
  }

  #[test]
  fn deploy_start_mode_hides_at_once_while_deploying() {
    let c = deploy_start();
    let mut s = state(&c);
    decide(&c, &shown(live(0)), &mut s);
    let o = Observation {
      deploying: true,
      ..shown(live(100))
    };
    assert_eq!(decide(&c, &o, &mut s), Action::HideClock);
  }

  #[test]
  fn app_stopped_mode_ignores_deploying() {
    let c = Config::default();
    let mut s = state(&c);
    let o = Observation {
      deploying: true,
      ..shown(live(0))
    };
    assert_eq!(decide(&c, &o, &mut s), Action::Stay);
  }

  #[test]
  fn a_deploy_that_never_stopped_the_app_returns_to_the_clock() {
    let c = deploy_start();
    let mut s = state(&c);
    let deploying = |o| Observation { deploying: true, ..o };
    assert_eq!(decide(&c, &deploying(shown(live(0))), &mut s), Action::HideClock);
    assert_eq!(decide(&c, &deploying(live(100)), &mut s), Action::Stay);
    // The helper exited and the old app never stopped.
    assert_eq!(decide(&c, &live(5_000), &mut s), Action::ShowClock);
  }

  #[test]
  fn a_command_is_sent_again_only_after_1s() {
    let c = Config::default();
    let mut s = state(&c);
    assert_eq!(decide(&c, &live(0), &mut s), Action::ShowClock);
    assert_eq!(decide(&c, &live(500), &mut s), Action::Stay); // sway's tree not caught up yet
    assert_eq!(decide(&c, &live(1_000), &mut s), Action::ShowClock);
  }

  #[test]
  fn user_on_a_console_is_left_alone_until_idle() {
    let c = Config::default();
    let mut s = state(&c);
    decide(&c, &shown(live(0)), &mut s);
    let o = |now, input: u64| Observation {
      last_input: Some(ms(input)),
      last_vt_key: Some((ms(9_900), 2)),
      ..console(shown(live(now)), 2)
    };
    assert_eq!(decide(&c, &o(10_000, 9_900), &mut s), Action::Stay);
    assert_eq!(decide(&c, &o(129_800, 9_900), &mut s), Action::Stay);
    assert_eq!(decide(&c, &o(129_900, 9_900), &mut s), Action::Switch(7));
  }

  #[test]
  fn input_resets_the_idle_timer() {
    let c = Config::default();
    let mut s = state(&c);
    decide(&c, &live(0), &mut s);
    let o = |now, input: u64| Observation {
      last_input: Some(ms(input)),
      last_vt_key: Some((ms(9_900), 2)),
      ..console(live(now), 2)
    };
    decide(&c, &o(10_000, 9_900), &mut s);
    assert_eq!(decide(&c, &o(130_000, 100_000), &mut s), Action::Stay);
    assert_eq!(decide(&c, &o(219_900, 100_000), &mut s), Action::Stay);
    assert_eq!(decide(&c, &o(220_000, 100_000), &mut s), Action::Switch(7));
  }

  #[test]
  fn user_back_on_sways_terminal_resumes_at_once() {
    let c = Config::default();
    let mut s = state(&c);
    decide(&c, &shown(live(0)), &mut s);
    let key = |o: Observation, n| Observation {
      last_input: Some(o.now - ms(50)),
      last_vt_key: Some((o.now - ms(50), n)),
      ..o
    };
    assert_eq!(
      decide(&c, &key(console(shown(live(1_000)), 2), 2), &mut s),
      Action::Stay
    );
    assert_eq!(decide(&c, &key(shown(live(2_000)), 7), &mut s), Action::Stay);
    // Resumed: the clock dies, it's hidden after the delay.
    assert_eq!(decide(&c, &shown(obs(2_100)), &mut s), Action::Stay);
    assert_eq!(decide(&c, &shown(obs(5_100)), &mut s), Action::HideClock);
  }

  #[test]
  fn someone_elses_switch_settles_5s() {
    let c = Config::default();
    let mut s = state(&c);
    decide(&c, &live(0), &mut s);
    // `chvt 2` over SSH: no key pressed.
    assert_eq!(decide(&c, &console(live(1_000), 2), &mut s), Action::Stay);
    assert_eq!(decide(&c, &console(live(5_900), 2), &mut s), Action::Stay);
    assert_eq!(decide(&c, &console(live(6_000), 2), &mut s), Action::Switch(7));
  }

  #[test]
  fn key_for_another_terminal_is_not_a_user_switch() {
    let c = Config::default();
    let mut s = state(&c);
    decide(&c, &live(0), &mut s);
    // F3 was pressed, but terminal 2 came up: someone else's switch, so settle 5 s rather than hold 2 minutes.
    let o = |now| Observation {
      last_input: Some(ms(950)),
      last_vt_key: Some((ms(950), 3)),
      ..console(live(now), 2)
    };
    assert_eq!(decide(&c, &o(1_000), &mut s), Action::Stay);
    assert_eq!(decide(&c, &o(6_000), &mut s), Action::Switch(7));
  }

  #[test]
  fn stale_key_is_not_a_user_switch() {
    let c = Config::default();
    let mut s = state(&c);
    decide(&c, &live(0), &mut s);
    // Ctrl+Alt+F2 a whole second earlier doesn't explain a switch now.
    let o = |now| Observation {
      last_input: Some(ms(0)),
      last_vt_key: Some((ms(0), 2)),
      ..console(live(now), 2)
    };
    assert_eq!(decide(&c, &o(1_000), &mut s), Action::Stay);
    assert_eq!(decide(&c, &o(6_000), &mut s), Action::Switch(7));
  }

  #[test]
  fn from_a_console_an_unclaimed_terminal_7_is_still_shown() {
    // sway restarted while a console showed: the display's check-in waits for 7, so the watcher takes the screen there
    // even before sway holds it, or each would wait for the other.
    let c = Config::default();
    let mut s = state(&c);
    decide(&c, &live(0), &mut s);
    let o = |now| Observation {
      display_claimed: false,
      clock_window: None,
      ..console(obs(now), 2)
    };
    assert_eq!(decide(&c, &o(1_000), &mut s), Action::Stay);
    assert_eq!(decide(&c, &o(6_000), &mut s), Action::Switch(7));
  }

  #[test]
  fn never_away_from_the_splash_to_an_unclaimed_terminal_7() {
    // Boot: the splash holds terminal 1 in graphics mode; the display's check-in takes it over itself.
    let c = Config::default();
    let splash = |now| Observation {
      active_vt: 1,
      display_claimed: false,
      active_claimed: true,
      clock_window: None,
      ..obs(now)
    };
    let mut s = State::new(&c, 1, ms(0));
    for t in [0, 6_000, 200_000] {
      assert_eq!(decide(&c, &splash(t), &mut s), Action::Stay);
    }
    // Once sway holds 7, a graphics-mode terminal elsewhere is left for it after the hands-off.
    let claimed = Observation {
      display_claimed: true,
      ..splash(200_100)
    };
    assert_eq!(decide(&c, &claimed, &mut s), Action::Switch(7));
  }

  #[test]
  fn started_on_a_console_treats_it_as_in_use() {
    let c = Config::default();
    let mut s = State::new(&c, 2, ms(0));
    let o = |now| console(live(now), 2);
    assert_eq!(decide(&c, &o(100), &mut s), Action::Stay);
    assert_eq!(decide(&c, &o(119_900), &mut s), Action::Stay);
    assert_eq!(decide(&c, &o(120_000), &mut s), Action::Switch(7));
  }

  #[test]
  fn own_switch_is_not_foreign_and_is_retried() {
    let c = Config::default();
    let mut s = state(&c);
    decide(&c, &live(0), &mut s);
    let o = |now| console(live(now), 2);
    decide(&c, &o(1_000), &mut s);
    assert_eq!(decide(&c, &o(6_000), &mut s), Action::Switch(7));
    assert_eq!(decide(&c, &o(6_500), &mut s), Action::Stay);
    assert_eq!(decide(&c, &o(7_000), &mut s), Action::Switch(7)); // didn't land: asked again
    // It lands: no settling, the window rules run at once.
    assert_eq!(decide(&c, &live(7_100), &mut s), Action::ShowClock);
  }

  #[test]
  fn windows_are_left_alone_off_sways_terminal() {
    let c = Config::default();
    let mut s = State::new(&c, 2, ms(0));
    // Live and hidden, but a console shows: no window command (sway isn't on screen).
    assert_eq!(decide(&c, &console(live(100), 2), &mut s), Action::Stay);
  }
}

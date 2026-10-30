//! `/etc/timeclock/watcher.conf`: `key = value` lines and `#` comments; every key is optional.
use std::path::PathBuf;
use std::time::Duration;

/// When the standby screen takes over during a deploy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flip {
  /// When the clock stops being live (the default).
  AppStopped,
  /// As soon as Coolify's helper container appears.
  DeployStart,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
  pub flip: Flip,
  /// sway's terminal.
  pub display_vt: u16,
  /// sway's control socket (`SWAYSOCK`, set by the display unit).
  pub sway_socket: PathBuf,
  /// The clock window's Wayland app id.
  pub clock_app_id: String,
  /// The clock browser's systemd unit: its processes' connections tell whether the clock is live.
  pub clock_unit: String,
  pub idle_return: Duration,
  pub settle: Duration,
  pub standby_delay: Duration,
  pub user_key_window: Duration,
  pub app_port: u16,
  pub helper_image: String,
  pub docker_socket: PathBuf,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      flip: Flip::AppStopped,
      display_vt: 7,
      sway_socket: "/run/timeclock-display/sway.sock".into(),
      clock_app_id: "local.timeclock.Clock".into(),
      clock_unit: "timeclock-clock.service".into(),
      idle_return: Duration::from_secs(120),
      settle: Duration::from_secs(5),
      standby_delay: Duration::from_secs(3),
      user_key_window: Duration::from_millis(500),
      app_port: 8080,
      helper_image: "coollabsio/coolify-helper".into(),
      docker_socket: "/var/run/docker.sock".into(),
    }
  }
}

/// Parse the config file's text; an unknown key, a bad value or a line without `=` is an error naming the line.
pub fn parse_config(text: &str) -> Result<Config, String> {
  let mut c = Config::default();
  for (i, raw) in text.lines().enumerate() {
    let line = raw.split('#').next().unwrap_or("").trim();
    if line.is_empty() {
      continue;
    }
    let n = i + 1;
    let Some((key, value)) = line.split_once('=') else {
      return Err(format!("line {n}: expected `key = value`"));
    };
    let (key, value) = (key.trim(), value.trim());
    let secs = || {
      value
        .parse::<u64>()
        .map_err(|_| format!("line {n}: {key}: not a whole number: {value}"))
    };
    let small = || {
      value
        .parse::<u16>()
        .map_err(|_| format!("line {n}: {key}: not a number up to 65535: {value}"))
    };
    match key {
      "flip" => {
        c.flip = match value {
          "app-stopped" => Flip::AppStopped,
          "deploy-start" => Flip::DeployStart,
          _ => {
            return Err(format!(
              "line {n}: flip: expected app-stopped or deploy-start, not {value}"
            ));
          }
        }
      }
      "display_vt" => c.display_vt = small()?,
      "sway_socket" => c.sway_socket = value.into(),
      "clock_app_id" => c.clock_app_id = value.into(),
      "clock_unit" => c.clock_unit = value.into(),
      "idle_return_secs" => c.idle_return = Duration::from_secs(secs()?),
      "settle_secs" => c.settle = Duration::from_secs(secs()?),
      "standby_delay_secs" => c.standby_delay = Duration::from_secs(secs()?),
      "user_key_window_ms" => c.user_key_window = Duration::from_millis(secs()?),
      "app_port" => c.app_port = small()?,
      "helper_image" => c.helper_image = value.into(),
      "docker_socket" => c.docker_socket = value.into(),
      _ => return Err(format!("line {n}: unknown key {key}")),
    }
  }
  Ok(c)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;
  use std::time::Duration;

  #[test]
  fn defaults_when_empty() {
    let c = parse_config("").unwrap();
    assert_eq!(c, Config::default());
    assert_eq!(c.flip, Flip::AppStopped);
    assert_eq!(c.display_vt, 7);
    assert_eq!(c.sway_socket, PathBuf::from("/run/timeclock-display/sway.sock"));
    assert_eq!(c.clock_app_id, "local.timeclock.Clock");
    assert_eq!(c.clock_unit, "timeclock-clock.service");
    assert_eq!(c.idle_return, Duration::from_secs(120));
    assert_eq!(c.settle, Duration::from_secs(5));
    assert_eq!(c.standby_delay, Duration::from_secs(3));
    assert_eq!(c.user_key_window, Duration::from_millis(500));
    assert_eq!(c.app_port, 8080);
    assert_eq!(c.helper_image, "coollabsio/coolify-helper");
    assert_eq!(c.docker_socket, PathBuf::from("/var/run/docker.sock"));
  }

  #[test]
  fn parses_every_key() {
    let c = parse_config(
      "# comment\n\nflip = deploy-start\ndisplay_vt = 8  # trailing\nsway_socket = /run/s.sock\n\
       clock_app_id = x.Clock\nclock_unit = x.service\nidle_return_secs = 60\nsettle_secs = 4\n\
       standby_delay_secs = 1\nuser_key_window_ms = 250\napp_port = 9000\nhelper_image = x/helper\n\
       docker_socket = /run/docker.sock\n",
    )
    .unwrap();
    assert_eq!(c.flip, Flip::DeployStart);
    assert_eq!(c.display_vt, 8);
    assert_eq!(c.sway_socket, PathBuf::from("/run/s.sock"));
    assert_eq!(
      (c.clock_app_id.as_str(), c.clock_unit.as_str()),
      ("x.Clock", "x.service")
    );
    assert_eq!(c.idle_return, Duration::from_secs(60));
    assert_eq!(c.settle, Duration::from_secs(4));
    assert_eq!(c.standby_delay, Duration::from_secs(1));
    assert_eq!(c.user_key_window, Duration::from_millis(250));
    assert_eq!(c.app_port, 9000);
    assert_eq!(c.helper_image, "x/helper");
    assert_eq!(c.docker_socket, PathBuf::from("/run/docker.sock"));
  }

  #[test]
  fn rejects_unknown_key() {
    assert!(parse_config("flipp = app-stopped").unwrap_err().contains("flipp"));
    // The first version's keys are gone with its terminal switching.
    assert!(parse_config("clock_vt = 1").is_err());
  }

  #[test]
  fn rejects_bad_flip() {
    assert!(parse_config("flip = sometimes").unwrap_err().contains("sometimes"));
  }

  #[test]
  fn rejects_bad_number_and_missing_equals() {
    assert!(parse_config("app_port = eighty").is_err());
    assert!(parse_config("app_port 8080").is_err());
  }
}

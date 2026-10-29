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
  pub clock_vt: u16,
  pub standby_vt: u16,
  pub idle_return: Duration,
  pub settle: Duration,
  pub standby_delay: Duration,
  pub user_key_window: Duration,
  pub browser_process: String,
  pub app_port: u16,
  pub helper_image: String,
  pub docker_socket: PathBuf,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      flip: Flip::AppStopped,
      clock_vt: 1,
      standby_vt: 7,
      idle_return: Duration::from_secs(120),
      settle: Duration::from_secs(5),
      standby_delay: Duration::from_secs(3),
      user_key_window: Duration::from_millis(500),
      browser_process: "firefox-esr".into(),
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
      "clock_vt" => c.clock_vt = small()?,
      "standby_vt" => c.standby_vt = small()?,
      "idle_return_secs" => c.idle_return = Duration::from_secs(secs()?),
      "settle_secs" => c.settle = Duration::from_secs(secs()?),
      "standby_delay_secs" => c.standby_delay = Duration::from_secs(secs()?),
      "user_key_window_ms" => c.user_key_window = Duration::from_millis(secs()?),
      "browser_process" => c.browser_process = value.into(),
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
    assert_eq!((c.clock_vt, c.standby_vt), (1, 7));
    assert_eq!(c.idle_return, Duration::from_secs(120));
    assert_eq!(c.settle, Duration::from_secs(5));
    assert_eq!(c.standby_delay, Duration::from_secs(3));
    assert_eq!(c.user_key_window, Duration::from_millis(500));
    assert_eq!(c.browser_process, "firefox-esr");
    assert_eq!(c.app_port, 8080);
    assert_eq!(c.helper_image, "coollabsio/coolify-helper");
    assert_eq!(c.docker_socket, PathBuf::from("/var/run/docker.sock"));
  }

  #[test]
  fn parses_every_key() {
    let c = parse_config(
      "# comment\n\nflip = deploy-start\nclock_vt = 2\nstandby_vt = 8  # trailing\nidle_return_secs = 60\n\
       settle_secs = 4\nstandby_delay_secs = 1\nuser_key_window_ms = 250\nbrowser_process = firefox\n\
       app_port = 9000\nhelper_image = x/helper\ndocker_socket = /run/docker.sock\n",
    )
    .unwrap();
    assert_eq!(c.flip, Flip::DeployStart);
    assert_eq!((c.clock_vt, c.standby_vt), (2, 8));
    assert_eq!(c.idle_return, Duration::from_secs(60));
    assert_eq!(c.settle, Duration::from_secs(4));
    assert_eq!(c.standby_delay, Duration::from_secs(1));
    assert_eq!(c.user_key_window, Duration::from_millis(250));
    assert_eq!(c.browser_process, "firefox");
    assert_eq!(c.app_port, 9000);
    assert_eq!(c.helper_image, "x/helper");
    assert_eq!(c.docker_socket, PathBuf::from("/run/docker.sock"));
  }

  #[test]
  fn rejects_unknown_key() {
    assert!(parse_config("flipp = app-stopped").unwrap_err().contains("flipp"));
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

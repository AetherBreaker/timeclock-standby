//! What the watcher reads from the machine: the kernel's terminal state, the kiosk's connection, Docker's containers.
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

const KDGETMODE: u64 = 0x4B3B;
const KD_GRAPHICS: libc::c_int = 1;
const VT_ACTIVATE: u64 = 0x5606;

fn open_tty(path: &str) -> io::Result<File> {
  File::options().read(true).custom_flags(libc::O_NOCTTY).open(path)
}

/// The terminal on screen, from `/sys/class/tty/tty0/active` ("tty7").
pub fn active_vt() -> io::Result<u16> {
  let text = fs::read_to_string("/sys/class/tty/tty0/active")?;
  text
    .trim()
    .strip_prefix("tty")
    .and_then(|n| n.parse().ok())
    .ok_or_else(|| io::Error::other(text))
}

/// Whether a display holds terminal `vt`: it's in graphics mode. Any error reads as not claimed.
pub fn is_claimed(vt: u16) -> bool {
  let Ok(tty) = open_tty(&format!("/dev/tty{vt}")) else {
    return false;
  };
  let mut mode: libc::c_int = 0;
  // SAFETY: KDGETMODE writes one int through the pointer, which outlives the call.
  let ok = unsafe { libc::ioctl(tty.as_raw_fd(), KDGETMODE as libc::Ioctl, &mut mode) } == 0;
  ok && mode == KD_GRAPHICS
}

/// Ask the kernel to switch to terminal `vt`; it completes once the current terminal's owner lets go.
pub fn activate_vt(vt: u16) -> io::Result<()> {
  let tty = open_tty("/dev/tty0")?;
  // SAFETY: VT_ACTIVATE takes the terminal number by value.
  if unsafe { libc::ioctl(tty.as_raw_fd(), VT_ACTIVATE as libc::Ioctl, libc::c_int::from(vt)) } == 0 {
    Ok(())
  } else {
    Err(io::Error::last_os_error())
  }
}

/// Whether a `/proc/net/tcp` or `tcp6` table holds an ESTABLISHED (state `01`) connection to remote `port`.
pub fn has_established_to(table: &str, port: u16) -> bool {
  let remote_port = format!(":{port:04X}");
  table.lines().skip(1).any(|line| {
    let mut fields = line.split_whitespace().skip(2); // sl, local address
    let remote = fields.next();
    fields.next() == Some("01") && remote.is_some_and(|r| r.ends_with(&remote_port))
  })
}

/// The kiosk's browser holds a connection to the app: a process named `browser` outside the host's network
/// namespace (the standby's Firefox runs in the host's) with an ESTABLISHED connection to `port`.
pub fn clock_connection(browser: &str, port: u16) -> bool {
  let Ok(host_ns) = fs::read_link("/proc/1/ns/net") else {
    return false;
  };
  let Ok(procs) = fs::read_dir("/proc") else { return false };
  procs.flatten().map(|e| e.path()).any(|dir| {
    fs::read_to_string(dir.join("comm")).is_ok_and(|c| c.trim_end() == browser)
      && fs::read_link(dir.join("ns/net")).is_ok_and(|ns| ns != host_ns)
      && ["net/tcp", "net/tcp6"]
        .iter()
        .any(|t| fs::read_to_string(dir.join(t)).is_ok_and(|s| has_established_to(&s, port)))
  })
}

/// Whether an image reference names `helper`, with any registry, tag or digest.
pub fn image_matches(image: &str, helper: &str) -> bool {
  let name = image.split('@').next().unwrap_or(image);
  // A tag follows the last ':' after the last '/'; a ':' before that is a registry's port.
  let name = match name.rfind(':') {
    Some(i) if !name[i..].contains('/') => &name[..i],
    _ => name,
  };
  name == helper || name.strip_suffix(helper).is_some_and(|rest| rest.ends_with('/'))
}

/// Whether Docker's `/containers/json` answer lists a container running `helper`. No JSON library: every
/// `"Image":"…"` value is checked (the exact key, so `"ImageID"` never matches).
pub fn helper_in_containers(json: &str, helper: &str) -> bool {
  json
    .split(r#""Image":""#)
    .skip(1)
    .any(|rest| rest.split('"').next().is_some_and(|image| image_matches(image, helper)))
}

/// Ask Docker for its running containers over its socket; any failure reads as no helper.
pub fn helper_running(socket: &Path, helper: &str) -> bool {
  let ask = || -> io::Result<String> {
    let mut s = UnixStream::connect(socket)?;
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    s.set_write_timeout(Some(Duration::from_secs(2)))?;
    // HTTP/1.0, so Docker answers without chunked encoding and closes when done.
    s.write_all(b"GET /containers/json HTTP/1.0\r\nHost: docker\r\n\r\n")?;
    let mut answer = String::new();
    s.read_to_string(&mut answer)?;
    Ok(answer)
  };
  ask().is_ok_and(|a| helper_in_containers(&a, helper))
}

#[cfg(test)]
mod tests {
  use super::*;

  const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 8759 1 0000000064d2ef3b 100 0 0 10 0
  15: 030012AC:C93C 020012AC:1F90 01 00000000:00000000 02:0000028B 00000000     0        0 49773 2 00000000651cc880 20 4 30 10 -1
";
  const TCP6_HEADER: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n";

  fn tcp6(remote_port: &str, state: &str) -> String {
    format!(
      "{TCP6_HEADER}   1: 0000000000000000FFFF0000030012AC:C944 0000000000000000FFFF0000020012AC:{remote_port} {state} \
       00000000:00000000 02:00000306 00000000     0        0 49018 2 00000000d979db27 20 4 28 10 -1\n"
    )
  }

  #[test]
  fn established_ipv4_counts() {
    assert!(has_established_to(TCP, 8080));
  }

  #[test]
  fn established_tcp6_counts() {
    assert!(has_established_to(&tcp6("1F90", "01"), 8080));
  }

  #[test]
  fn close_wait_does_not_count() {
    assert!(!has_established_to(&tcp6("1F90", "08"), 8080));
    assert!(!has_established_to(&TCP.replace(" 01 ", " 06 "), 8080)); // TIME_WAIT
  }

  #[test]
  fn other_port_does_not_count() {
    assert!(!has_established_to(TCP, 8081));
    assert!(!has_established_to(&tcp6("0016", "01"), 8080));
    // A local socket on 8080, listening or connected to elsewhere, isn't a connection to it.
    let local_only: String = TCP.lines().take(2).map(|l| format!("{l}\n")).collect();
    assert!(!has_established_to(
      &local_only.replace(":0016 00000000:0000 0A", ":1F90 00000000:0000 0A"),
      8080
    ));
    assert!(!has_established_to(
      &TCP.replace("030012AC:C93C 020012AC:1F90", "030012AC:1F90 020012AC:C93C"),
      8080
    ));
  }

  const CONTAINERS: &str = r#"[{"Id":"a1","Names":["/coolify-sentinel"],"Image":"ghcr.io/coollabsio/sentinel:1.1.3","ImageID":"sha256:81f8","Command":"/app/sentinel"},{"Id":"b2","Names":["/px0q"],"Image":"coollabsio/coolify-helper:1.0.19","ImageID":"sha256:ab12"}]"#;

  #[test]
  fn helper_tag_and_registry_match() {
    assert!(image_matches(
      "coollabsio/coolify-helper:1.0.19",
      "coollabsio/coolify-helper"
    ));
    assert!(image_matches("coollabsio/coolify-helper", "coollabsio/coolify-helper"));
    assert!(image_matches(
      "ghcr.io/coollabsio/coolify-helper:1.0.20",
      "coollabsio/coolify-helper"
    ));
    assert!(image_matches(
      "coollabsio/coolify-helper@sha256:abc",
      "coollabsio/coolify-helper"
    ));
    assert!(helper_in_containers(CONTAINERS, "coollabsio/coolify-helper"));
  }

  #[test]
  fn sentinel_does_not_match() {
    assert!(!image_matches(
      "ghcr.io/coollabsio/sentinel:1.1.3",
      "coollabsio/coolify-helper"
    ));
    assert!(!image_matches(
      "other/coollabsio/coolify-helper-x:1",
      "coollabsio/coolify-helper"
    ));
    let only_sentinel = CONTAINERS.split(r#",{"Id":"b2""#).next().unwrap().to_string() + "]";
    assert!(!helper_in_containers(&only_sentinel, "coollabsio/coolify-helper"));
  }

  #[test]
  fn imageid_key_is_ignored() {
    // "ImageID" starts like "Image"; only the Image value names the image.
    let json = r#"[{"Id":"c3","Image":"python:3.13-slim","ImageID":"coollabsio/coolify-helper"}]"#;
    assert!(!helper_in_containers(json, "coollabsio/coolify-helper"));
  }

  #[test]
  fn empty_list_is_no_helper() {
    assert!(!helper_in_containers("[]", "coollabsio/coolify-helper"));
    assert!(!helper_in_containers("", "coollabsio/coolify-helper"));
  }
}

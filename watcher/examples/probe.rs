//! Prints what the watcher observes, for checking on a clock: `probe` once a second for 10 s; `probe keys N` input
//! for N seconds, rescanning devices every 2 s as the watcher does.
use std::path::Path;
use std::time::{Duration, Instant};
use timeclock_watcher::{keys::Inputs, observe};

fn main() {
  let args: Vec<String> = std::env::args().collect();
  if args.get(1).map(String::as_str) == Some("keys") {
    let secs = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let (start, mut inputs, mut last_scan) = (Instant::now(), Inputs::default(), Instant::now());
    while start.elapsed() < Duration::from_secs(secs) {
      if last_scan.elapsed() >= Duration::from_secs(2) {
        inputs.rescan();
        last_scan = Instant::now();
      }
      let (input, vt_key) = inputs.drain();
      if input || vt_key.is_some() {
        println!("{:>6.1}s devices={} input={input} vt_key={vt_key:?}", start.elapsed().as_secs_f32(), inputs.fds().len());
      }
      std::thread::sleep(Duration::from_millis(100));
    }
    return;
  }
  for _ in 0..10 {
    println!(
      "active={:?} claimed1={} claimed7={} clock_connection={} helper={}",
      observe::active_vt(),
      observe::is_claimed(1),
      observe::is_claimed(7),
      observe::clock_connection("firefox-esr", 8080),
      observe::helper_running(Path::new("/var/run/docker.sock"), "coollabsio/coolify-helper"),
    );
    std::thread::sleep(Duration::from_secs(1));
  }
}

//! The screen watcher: shows or hides the clock window from what it observes, and nothing talks to it. It does not
//! update the host (timeclock-host's updater does).
pub mod config;
pub mod decide;
pub mod keys;
pub mod observe;
pub mod sway;

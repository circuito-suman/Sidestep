//! Owns the DPI bypass engine child process (GoodbyeDPI on Windows, ByeDPI/ciadpi on Linux & macOS).
//!
//! Honesty guarantees:
//! * Windows: child is placed in a Job Object with KILL_ON_JOB_CLOSE.
//! * Linux: child is spawned with prctl(PR_SET_PDEATHSIG, SIGTERM) so the kernel
//!   kills the child if this app exits or crashes.
//! * macOS: child is supervised by a kqueue EVFILT_PROC (NOTE_EXIT) watchdog
//!   plus Drop cleanup so the child never outlives Sidestep.
//! * In all cases, if supervision cannot be established, start() fails.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

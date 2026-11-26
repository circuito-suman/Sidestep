#![cfg_attr(windows, windows_subsystem = "windows")]

pub use sidestep_core::{flags, probe, Ev};

pub mod autostart;
pub mod config;
pub mod engine;
#[cfg(not(windows))]
pub mod gui;
#[cfg(windows)]
pub mod kit;
#[cfg(target_os = "linux")]
pub mod proxy;
#[cfg(target_os = "macos")]
#[path = "proxy_macos.rs"]
pub mod proxy;
pub mod setup;

#[cfg(windows)]
mod win_main;

#[cfg(target_os = "linux")]
mod linux_main;

#[cfg(target_os = "macos")]
mod macos_main;

fn main() {
    #[cfg(windows)]
    win_main::main();

    #[cfg(target_os = "linux")]
    linux_main::main();

    #[cfg(target_os = "macos")]
    macos_main::main();
}

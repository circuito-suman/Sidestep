//! Unix Native GUI front-end using eframe (egui) and platform system tray
//! (FreeDesktop StatusNotifierItem via `ksni` on Linux, `NSStatusBar` via `tray-icon` on macOS).
//! Faithfully matches the Windows 4-tab interface, colors, icons, and honesty contract.

pub mod app;
pub mod icons;
pub mod ipc;
pub mod state;
#[cfg(target_os = "linux")]
pub mod tray;
#[cfg(target_os = "macos")]
pub mod tray_macos;
pub mod views;

pub use app::SidestepApp;
pub use ipc::send_instance_cmd;

use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
#[cfg(target_os = "linux")]
use std::time::Duration;

use crate::config::Config;
use crate::setup::Mode;
use state::{AppState, UiCmd};

/// Main GUI entrypoint.
pub fn run_gui(mode: Mode, cfg: Config, exe: PathBuf) {
    let (cmd_tx, cmd_rx) = channel();
    let state = Arc::new(Mutex::new(AppState::new(mode, cfg)));

    // Check single-instance
    if let Some(resp) = ipc::spawn_instance_listener(state.clone(), cmd_tx.clone()) {
        println!("Another Sidestep instance is already running ({resp}). Focused existing window.");
        return;
    }

    // Spawn supervisor background thread
    let sup_handle = state::Supervisor::spawn(state.clone(), cmd_rx, exe);

    // Spawn FreeDesktop system tray service on Linux in background thread
    #[cfg(target_os = "linux")]
    spawn_linux_tray(state.clone(), cmd_tx.clone());

    // Set application window icon from app.ico
    let mut viewport_builder = eframe::egui::ViewportBuilder::default()
        .with_inner_size([600.0, 560.0])
        .with_min_inner_size([540.0, 500.0])
        .with_title("Sidestep")
        .with_visible(true)
        .with_resizable(true)
        .with_decorations(true)
        .with_close_button(true);

    if let Ok(img) = image::load_from_memory(icons::APP_ICO) {
        let rgba = img
            .resize_exact(48, 48, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        viewport_builder = viewport_builder.with_icon(Arc::new(eframe::egui::IconData {
            rgba: rgba.into_raw(),
            width: 48,
            height: 48,
        }));
    }

    let options = eframe::NativeOptions {
        viewport: viewport_builder,
        ..Default::default()
    };

    let app_state = state.clone();
    let app_tx = cmd_tx.clone();
    let res = eframe::run_native(
        "Sidestep",
        options,
        Box::new(move |cc| {
            #[cfg(target_os = "macos")]
            let mac_tray = match tray_macos::MacTray::new(app_tx.clone(), cc.egui_ctx.clone()) {
                Ok(t) => {
                    if let Ok(mut st) = app_state.lock() {
                        st.has_tray = true;
                    }
                    Some(t)
                }
                Err(e) => {
                    eprintln!("[WARN] Failed to create macOS menu bar item: {e}");
                    None
                }
            };
            #[cfg(not(target_os = "macos"))]
            let _ = cc;

            let app = SidestepApp::new(
                app_state,
                app_tx,
                #[cfg(target_os = "macos")]
                mac_tray,
            );
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = res {
        eprintln!("[ERROR] eframe::run_native failed: {e}");
    }

    // Ensure supervisor terminates child engine and restores system proxy before process exit.
    if let Ok(mut st) = state.lock() {
        st.should_exit = true;
    }
    let _ = cmd_tx.send(UiCmd::Quit);
    let _ = sup_handle.join();
    #[cfg(target_os = "macos")]
    ipc::cleanup_instance_socket();
}

#[cfg(target_os = "linux")]
fn spawn_linux_tray(tray_state: Arc<Mutex<AppState>>, tray_tx: std::sync::mpsc::Sender<UiCmd>) {
    std::thread::Builder::new()
        .name("sidestep-tray".into())
        .spawn(move || {
            use ksni::blocking::TrayMethods;
            let tray = tray::SidestepTray::new(tray_state.clone(), tray_tx);
            match tray.assume_sni_available(true).spawn() {
                Ok(handle) => {
                    if let Ok(mut st) = tray_state.lock() {
                        st.has_tray = true;
                    }
                    let mut last_view = None;
                    loop {
                        std::thread::sleep(Duration::from_millis(100));
                        let view = match tray_state.lock() {
                            Ok(st) => {
                                if st.should_exit {
                                    break;
                                }
                                st.view
                            }
                            Err(_) => break,
                        };
                        if Some(view) != last_view {
                            last_view = Some(view);
                            handle.update(|_| {});
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[WARN] Failed to spawn FreeDesktop system tray: {e:?}");
                }
            }
        })
        .ok();
}

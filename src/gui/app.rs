//! Application controller and eframe lifecycle for Linux GUI.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::icons::AppTextures;
#[cfg(target_os = "macos")]
use super::state::View;
use super::state::{AppState, UiCmd};
#[cfg(target_os = "macos")]
use super::tray_macos::MacTray;
use eframe::egui::{self, Color32};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Status,
    Flags,
    Log,
    Settings,
}

pub struct SidestepApp {
    pub(crate) state: Arc<Mutex<AppState>>,
    pub(crate) cmd_tx: Sender<UiCmd>,
    pub(crate) selected_tab: Tab,
    pub(crate) flags_edit: String,
    pub(crate) sites_edit: String,
    pub(crate) check_secs_edit: String,
    pub(crate) autotune_val: bool,
    pub(crate) autostart_val: bool,
    pub(crate) confirm_uninstall: bool,
    pub(crate) status_help: bool,
    pub(crate) log_wrap: bool,
    pub(crate) is_window_shown: bool,
    pub(crate) textures: Option<AppTextures>,
    #[cfg(target_os = "macos")]
    pub(crate) mac_tray: Option<MacTray>,
}

impl SidestepApp {
    pub fn new(
        state: Arc<Mutex<AppState>>,
        cmd_tx: Sender<UiCmd>,
        #[cfg(target_os = "macos")] mac_tray: Option<MacTray>,
    ) -> Self {
        let (flags, sites, secs, autotune, log_wrap) = {
            let st = state.lock().unwrap();
            (
                st.config.flags.clone(),
                st.config.sites.clone(),
                st.config.check_secs.to_string(),
                st.config.autotune,
                st.config.log_wrap,
            )
        };
        let autostart_val = crate::autostart::is_registered();

        Self {
            state,
            cmd_tx,
            selected_tab: Tab::Status,
            flags_edit: flags,
            sites_edit: sites,
            check_secs_edit: secs,
            autotune_val: autotune,
            autostart_val,
            confirm_uninstall: false,
            status_help: false,
            log_wrap,
            is_window_shown: false,
            textures: None,
            #[cfg(target_os = "macos")]
            mac_tray,
        }
    }
}

impl eframe::App for SidestepApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply clean Windows-matching light theme
        let mut visuals = egui::Visuals::light();
        visuals.window_fill = Color32::from_rgb(250, 250, 252);
        visuals.panel_fill = Color32::from_rgb(250, 250, 252);
        visuals.faint_bg_color = Color32::from_rgb(242, 244, 247);
        visuals.extreme_bg_color = Color32::WHITE;
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(250, 250, 252);
        ctx.set_visuals(visuals);

        if self.textures.is_none() {
            self.textures = Some(AppTextures::load(ctx));
        }

        // Handle window close event: cleanly shut down application, engine, and tray
        if ctx.input(|i| i.viewport().close_requested()) {
            let mut st = self.state.lock().unwrap();
            st.should_exit = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            let _ = self.cmd_tx.send(UiCmd::Quit);
        }

        // Check if supervisor requested window presentation or shutdown
        let (should_exit, show_window, view, verify) = {
            let mut st = self.state.lock().unwrap();
            let show = if cfg!(target_os = "macos") {
                let s = st.window_visible;
                if s {
                    st.window_visible = false;
                    self.is_window_shown = true;
                }
                s
            } else {
                let s = st.window_visible && !self.is_window_shown;
                if s {
                    self.is_window_shown = true;
                }
                s
            };
            #[cfg(target_os = "macos")]
            if let Some(ref mut tray) = self.mac_tray {
                let is_running = st.engine_pid.is_some() || st.view != View::Off;
                tray.sync(st.view, is_running);
            }
            (st.should_exit, show, st.view, st.verify)
        };

        if should_exit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if show_window {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // Request repaint every 250ms for uptime, timers, and state transitions
        ctx.request_repaint_after(Duration::from_millis(250));

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(ctx.style().visuals.window_fill()))
            .show(ctx, |ui| {
                let tex = self.textures.as_ref().unwrap().clone();

                // 1. Header Banner
                self.render_header(ui, view, verify, &tex);

                ui.add_space(4.0);

                // 2. Tab Navigation Bar
                self.render_tab_bar(ui, &tex);

                ui.separator();

                // 3. Active Tab Content
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(8.0);
                        match self.selected_tab {
                            Tab::Status => self.render_status_tab(ui, view, verify, &tex),
                            Tab::Flags => self.render_flags_tab(ui, &tex),
                            Tab::Log => self.render_log_tab(ui, &tex),
                            Tab::Settings => self.render_settings_tab(ui, &tex),
                        }
                    });
            });
    }
}

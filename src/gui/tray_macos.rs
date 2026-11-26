//! macOS Menu Bar status item (`NSStatusBar`) via `tray-icon` and `muda`.

use std::sync::mpsc::Sender;

use eframe::egui;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use super::icons::{ico_to_rgba, BUSY_ICO, OFF_ICO, ON_ICO};
use super::state::{UiCmd, View};

fn make_icon(ico_bytes: &[u8]) -> Result<Icon, String> {
    let size = 32;
    let rgba = ico_to_rgba(ico_bytes, size);
    Icon::from_rgba(rgba, size, size).map_err(|e| format!("creating tray icon: {e}"))
}

fn tooltip_for(view: View) -> &'static str {
    match view {
        View::On => "Sidestep — Bypass is ON (verified)",
        View::Off => "Sidestep — Bypass is OFF",
        View::Starting => "Sidestep — Starting bypass engine...",
        View::Checking => "Sidestep — Checking test sites...",
        View::Tuning => "Sidestep — Tuning TTL for network...",
        View::Blocked => "Sidestep — Bypass is ON, but DPI is blocking",
        View::Unreachable => "Sidestep — Test sites unreachable",
    }
}

pub struct MacTray {
    tray: TrayIcon,
    toggle_item: MenuItem,
    icon_on: Icon,
    icon_off: Icon,
    icon_busy: Icon,
    last_view: Option<View>,
    last_running: Option<bool>,
}

impl MacTray {
    /// Must be called on the main thread after `NSApplication` has been initialized.
    pub fn new(cmd_tx: Sender<UiCmd>, ctx: egui::Context) -> Result<Self, String> {
        let icon_on = make_icon(ON_ICO)?;
        let icon_off = make_icon(OFF_ICO)?;
        let icon_busy = make_icon(BUSY_ICO)?;

        let toggle_item = MenuItem::new("Bypass on", true, None);
        let open_item = MenuItem::new("Open Sidestep", true, None);
        let tune_item = MenuItem::new("Find TTL now", true, None);
        let quit_item = MenuItem::new("Quit Sidestep", true, None);

        let menu = Menu::new();
        menu.append_items(&[
            &toggle_item,
            &open_item,
            &tune_item,
            &PredefinedMenuItem::separator(),
            &quit_item,
        ])
        .map_err(|e| format!("building menu bar items: {e}"))?;

        let toggle_id = toggle_item.id().clone();
        let open_id = open_item.id().clone();
        let tune_id = tune_item.id().clone();
        let quit_id = quit_item.id().clone();

        MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
            let cmd = if ev.id == toggle_id {
                Some(UiCmd::Toggle)
            } else if ev.id == open_id {
                Some(UiCmd::ShowWindow)
            } else if ev.id == tune_id {
                Some(UiCmd::TuneNow)
            } else if ev.id == quit_id {
                Some(UiCmd::Quit)
            } else {
                None
            };
            if let Some(c) = cmd {
                let _ = cmd_tx.send(c);
                ctx.request_repaint();
            }
        }));

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(true)
            .with_tooltip(tooltip_for(View::Off))
            .with_icon(icon_off.clone())
            .build()
            .map_err(|e| format!("building macOS menu bar item: {e}"))?;

        Ok(Self {
            tray,
            toggle_item,
            icon_on,
            icon_off,
            icon_busy,
            last_view: None,
            last_running: None,
        })
    }

    /// Synchronizes icon, tooltip, and toggle label on the main thread when state changes.
    pub fn sync(&mut self, view: View, is_running: bool) {
        if self.last_view == Some(view) && self.last_running == Some(is_running) {
            return;
        }
        self.last_view = Some(view);
        self.last_running = Some(is_running);

        let icon = match view {
            View::On => self.icon_on.clone(),
            View::Off => self.icon_off.clone(),
            _ => self.icon_busy.clone(),
        };
        let _ = self.tray.set_icon(Some(icon));
        let _ = self.tray.set_tooltip(Some(tooltip_for(view)));
        self.toggle_item.set_text(if is_running {
            "Bypass off"
        } else {
            "Bypass on"
        });
    }
}

//! View renderers for the Sidestep GUI tabs (Header, Status, Flags, Log, Settings).

use std::path::PathBuf;

use eframe::egui::{self, Color32, Image, RichText, Stroke, Vec2};

use super::app::{SidestepApp, Tab};
use super::icons::AppTextures;
use super::state::{UiCmd, Verify, View};
use crate::flags;

impl SidestepApp {
    pub(crate) fn render_header(
        &mut self,
        ui: &mut egui::Ui,
        view: View,
        _verify: Verify,
        tex: &AppTextures,
    ) {
        let (band_bg, headline_color, sub_color, headline, subtitle, badge) = match view {
            View::Off => (
                Color32::from_rgb(241, 243, 245), // BAND_OFF (0x00F5F3F1)
                Color32::from_rgb(73, 79, 87),    // Slate text (0x00574F49)
                Color32::from_rgb(115, 120, 130),
                "Bypass is off",
                "Nothing is filtering.",
                &tex.badge_off,
            ),
            View::Starting => (
                Color32::from_rgb(255, 243, 191), // BAND_BUSY (0x00BFF3FF)
                Color32::from_rgb(156, 74, 0),    // Dark amber text (0x00004A9C)
                Color32::from_rgb(175, 95, 20),
                "Starting...",
                "Launching bypass engine...",
                &tex.badge_busy,
            ),
            View::Checking => (
                Color32::from_rgb(255, 243, 191),
                Color32::from_rgb(156, 74, 0),
                Color32::from_rgb(175, 95, 20),
                "Checking...",
                "Probing test sites through bypass...",
                &tex.badge_busy,
            ),
            View::Tuning => (
                Color32::from_rgb(255, 243, 191),
                Color32::from_rgb(156, 74, 0),
                Color32::from_rgb(175, 95, 20),
                "Tuning...",
                "Finding optimal TTL for this network...",
                &tex.badge_busy,
            ),
            View::On => (
                Color32::from_rgb(211, 249, 216), // BAND_ON (0x00D8F9D3)
                Color32::from_rgb(43, 138, 62),   // Dark green text (0x003E8A2B)
                Color32::from_rgb(50, 120, 60),
                "Bypass is on",
                "Verified: test sites respond through the bypass.",
                &tex.badge_on,
            ),
            View::Blocked => (
                Color32::from_rgb(255, 243, 191),
                Color32::from_rgb(156, 74, 0),
                Color32::from_rgb(175, 95, 20),
                "Bypass is on, but DPI is blocking",
                "Test sites failed to respond. Try tuning TTL.",
                &tex.badge_busy,
            ),
            View::Unreachable => (
                Color32::from_rgb(241, 243, 245),
                Color32::from_rgb(73, 79, 87),
                Color32::from_rgb(115, 120, 130),
                "Test sites unreachable",
                "Check your internet connection.",
                &tex.badge_off,
            ),
        };

        let is_running = view != View::Off;
        let is_busy = matches!(view, View::Starting | View::Checking | View::Tuning);

        let band_frame = egui::Frame::NONE
            .fill(band_bg)
            .inner_margin(egui::Margin::symmetric(16, 12));

        band_frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                // 40x40 Authentic Icon Badge
                ui.add(Image::new(badge).fit_to_exact_size(Vec2::splat(40.0)));

                ui.add_space(10.0);

                let right_btn_w = 120.0;
                let text_w = (ui.available_width() - right_btn_w).max(120.0);

                ui.allocate_ui_with_layout(
                    Vec2::new(text_w, 42.0),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        ui.label(
                            RichText::new(headline)
                                .size(18.0)
                                .strong()
                                .color(headline_color),
                        );
                        ui.add_space(1.0);
                        ui.add(
                            egui::Label::new(RichText::new(subtitle).size(12.0).color(sub_color))
                                .wrap(),
                        );
                    },
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let btn_text = if is_running { " Turn off" } else { " Turn on" };
                    let btn = egui::Button::image_and_text(
                        Image::new(&tex.power).fit_to_exact_size(Vec2::splat(16.0)),
                        RichText::new(btn_text).size(14.0).strong(),
                    )
                    .min_size(Vec2::new(114.0, 36.0));

                    if ui.add(btn).clicked() {
                        let _ = self.cmd_tx.send(UiCmd::Toggle);
                    }
                });
            });

            if is_busy {
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(0.5).animate(true));
            }
        });
    }

    pub(crate) fn render_tab_bar(&mut self, ui: &mut egui::Ui, tex: &AppTextures) {
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            let tabs = [
                (Tab::Status, " Status", &tex.tab_status),
                (Tab::Flags, " Flags", &tex.tab_flags),
                (Tab::Log, " Log", &tex.tab_log),
                (Tab::Settings, " Settings", &tex.tab_settings),
            ];

            for (tab, name, icon) in tabs {
                let is_selected = self.selected_tab == tab;
                let text = if is_selected {
                    RichText::new(name)
                        .strong()
                        .color(ui.visuals().strong_text_color())
                } else {
                    RichText::new(name).color(ui.visuals().text_color())
                };

                let mut btn = egui::Button::image_and_text(
                    Image::new(icon).fit_to_exact_size(Vec2::splat(16.0)),
                    text,
                )
                .min_size(Vec2::new(88.0, 28.0));

                if is_selected {
                    btn = btn.stroke(Stroke::new(1.5_f32, ui.visuals().selection.bg_fill));
                }
                if ui.add(btn).clicked() {
                    self.selected_tab = tab;
                }
            }
        });
    }

    pub(crate) fn render_status_tab(
        &mut self,
        ui: &mut egui::Ui,
        _view: View,
        verify: Verify,
        tex: &AppTextures,
    ) {
        let (
            pid_str,
            uptime,
            ttl_str,
            flags_str,
            last_detail,
            passed,
            blocked,
            is_running,
            is_busy_job,
        ) = {
            let st = self.state.lock().unwrap();
            let pid = st.engine_pid.map_or("none".into(), |p| p.to_string());
            let ttl = st.current_ttl.map_or("none".into(), |t| t.to_string());
            (
                pid,
                st.uptime_str(),
                ttl,
                st.active_flags.clone(),
                st.last_check_detail.clone(),
                st.checks_passed,
                st.checks_blocked,
                st.engine_pid.is_some() || st.view != View::Off,
                st.probing || st.tuning,
            )
        };

        egui::Grid::new("status_grid")
            .num_columns(2)
            .spacing([18.0, 10.0])
            .striped(true)
            .show(ui, |ui| {
                // Row 0: Engine
                ui.horizontal(|ui| {
                    ui.add(Image::new(&tex.engine).fit_to_exact_size(Vec2::splat(16.0)));
                    ui.label(RichText::new("Engine").color(Color32::from_rgb(100, 105, 115)));
                });
                if is_running {
                    let engine_label = if cfg!(target_os = "macos") {
                        "ciadpi (running)"
                    } else {
                        "goodbyedpi (running)"
                    };
                    ui.label(
                        RichText::new(engine_label)
                            .color(Color32::from_rgb(40, 167, 69))
                            .strong(),
                    );
                } else {
                    ui.label("Not running");
                }
                ui.end_row();

                // Row 1: PID
                ui.horizontal(|ui| {
                    ui.add(Image::new(&tex.pid).fit_to_exact_size(Vec2::splat(16.0)));
                    ui.label(RichText::new("PID").color(Color32::from_rgb(100, 105, 115)));
                });
                ui.label(&pid_str);
                ui.end_row();

                // Row 2: Uptime
                ui.horizontal(|ui| {
                    ui.add(Image::new(&tex.clock).fit_to_exact_size(Vec2::splat(16.0)));
                    ui.label(RichText::new("Uptime").color(Color32::from_rgb(100, 105, 115)));
                });
                ui.label(&uptime);
                ui.end_row();

                // Row 3: TTL in use
                ui.horizontal(|ui| {
                    ui.add(Image::new(&tex.ttl).fit_to_exact_size(Vec2::splat(16.0)));
                    ui.label(RichText::new("TTL in use").color(Color32::from_rgb(100, 105, 115)));
                });
                ui.label(&ttl_str);
                ui.end_row();

                // Row 4: Active flags
                ui.horizontal(|ui| {
                    ui.add(Image::new(&tex.flag).fit_to_exact_size(Vec2::splat(16.0)));
                    ui.label(RichText::new("Active flags").color(Color32::from_rgb(100, 105, 115)));
                });
                ui.add(egui::Label::new(RichText::new(&flags_str).monospace()).wrap());
                ui.end_row();

                // Row 5: Last check (with ok/bad/pending badge)
                ui.horizontal(|ui| {
                    ui.add(Image::new(&tex.check).fit_to_exact_size(Vec2::splat(16.0)));
                    ui.label(RichText::new("Last check").color(Color32::from_rgb(100, 105, 115)));
                });
                ui.horizontal(|ui| {
                    let badge = match verify {
                        Verify::Passed => &tex.ok,
                        Verify::Blocked => &tex.bad,
                        _ => &tex.pending,
                    };
                    ui.add(Image::new(badge).fit_to_exact_size(Vec2::splat(14.0)));
                    ui.add(egui::Label::new(&last_detail).wrap());
                });
                ui.end_row();

                // Row 6: Checks
                ui.horizontal(|ui| {
                    ui.add(Image::new(&tex.chart).fit_to_exact_size(Vec2::splat(16.0)));
                    ui.label(RichText::new("Checks").color(Color32::from_rgb(100, 105, 115)));
                });
                ui.label(format!("{passed} passed, {blocked} blocked"));
                ui.end_row();
            });

        ui.add_space(14.0);
        ui.separator();
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            let check_btn = egui::Button::image_and_text(
                Image::new(&tex.check).fit_to_exact_size(Vec2::splat(16.0)),
                " Check now",
            )
            .min_size(Vec2::new(110.0, 32.0));
            if ui
                .add_enabled(is_running && !is_busy_job, check_btn)
                .clicked()
            {
                let _ = self.cmd_tx.send(UiCmd::CheckNow);
            }

            let tune_btn = egui::Button::image_and_text(
                Image::new(&tex.tune).fit_to_exact_size(Vec2::splat(16.0)),
                " Find TTL now",
            )
            .min_size(Vec2::new(120.0, 32.0));
            if ui.add_enabled(!is_busy_job, tune_btn).clicked() {
                let _ = self.cmd_tx.send(UiCmd::TuneNow);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let help_btn = egui::Button::image_and_text(
                    Image::new(&tex.help).fit_to_exact_size(Vec2::splat(16.0)),
                    if self.status_help {
                        " Hide help"
                    } else {
                        " Help"
                    },
                )
                .min_size(Vec2::new(88.0, 32.0));
                if ui.add(help_btn).clicked() {
                    self.status_help = !self.status_help;
                }
            });
        });

        if self.status_help {
            ui.add_space(8.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(RichText::new("How Sidestep Status Works:").strong());
                ui.label("• Engine: Checked continuously. If Sidestep crashes, kernel terminates the engine instantly.");
                ui.label("• TTL in use: Hop count where decoy packets expire after the firewall but before destination server.");
                ui.label("• Last check: Sends TLS ClientHello every 90s to verify game stores actually open.");
            });
        }
    }

    pub(crate) fn render_flags_tab(&mut self, ui: &mut egui::Ui, tex: &AppTextures) {
        ui.label(RichText::new("DPI Bypass Flags").strong());
        ui.label("Flags passed directly to ciadpi (ByeDPI) when bypass starts.");

        ui.add_space(6.0);

        // 1. Text edit taking full available width (matching Windows layout)
        let avail_w = ui.available_width();
        ui.add(
            egui::TextEdit::singleline(&mut self.flags_edit)
                .desired_width(avail_w)
                .font(egui::TextStyle::Monospace),
        );

        ui.add_space(6.0);

        // 2. Action buttons row underneath text edit (matching Windows win_main.rs)
        ui.horizontal(|ui| {
            let apply_btn = egui::Button::image_and_text(
                Image::new(&tex.apply).fit_to_exact_size(Vec2::splat(16.0)),
                " Apply",
            )
            .min_size(Vec2::new(100.0, 30.0));
            if ui.add(apply_btn).clicked() {
                let _ = self
                    .cmd_tx
                    .send(UiCmd::ApplyFlags(self.flags_edit.trim().to_string()));
            }

            let undo_btn = egui::Button::image_and_text(
                Image::new(&tex.undo).fit_to_exact_size(Vec2::splat(16.0)),
                " Restore defaults",
            )
            .min_size(Vec2::new(140.0, 30.0));
            if ui.add(undo_btn).clicked() {
                self.flags_edit = flags::DEFAULT_FLAGS.to_string();
                let _ = self.cmd_tx.send(UiCmd::ApplyFlags(self.flags_edit.clone()));
            }
        });

        ui.add_space(14.0);
        ui.separator();
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.add(Image::new(&tex.help).fit_to_exact_size(Vec2::splat(16.0)));
            ui.label(RichText::new("All ByeDPI flags. Select one to add or remove:").strong());
        });

        ui.add_space(6.0);

        let catalog = [
            (
                "--tlsrec 1 -s 1+s -d 1+s",
                "Split & Disorder",
                "Splits TLS record header at byte 1 and TCP payload at SNI, sending packets out-of-order. Bypasses Indian ISPs and Cloudflare cleanly.",
                "--tlsrec 1 -s 1+s -d 1+s",
            ),
            (
                "--tlsrec 1+s -s 1+s",
                "Split at SNI",
                "Splits the TLS record right at the site name (SNI). Closest ByeDPI gets to --frag-by-sni.",
                "--tlsrec 1+s -s 1+s",
            ),
            (
                "-s 1+s -d 1+s",
                "Disorder Stream",
                "Sends payload split packets in reverse/disordered sequence to confuse inspection engines.",
                "-s 1+s -d 1+s",
            ),
            (
                "--tlsrec 1 -f 1+s -t 1",
                "Decoy Packet (Fake)",
                "Sends a fake decoy packet with low TTL at SNI offset before the real TLS payload.",
                "--tlsrec 1 -f 1+s -t 1",
            ),
        ];

        for (flag, group, desc, example) in catalog {
            let active = flags::uses(&self.flags_edit, flag.split_whitespace().next().unwrap());

            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(flag).monospace().strong());
                        ui.label(
                            RichText::new(format!("[{group}]"))
                                .color(ui.visuals().weak_text_color()),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (icon, label) = if active {
                                (&tex.minus, " Remove")
                            } else {
                                (&tex.plus, " Add")
                            };
                            let btn = egui::Button::image_and_text(
                                Image::new(icon).fit_to_exact_size(Vec2::splat(14.0)),
                                label,
                            )
                            .min_size(Vec2::new(84.0, 26.0));
                            if ui.add(btn).clicked() {
                                let base = flag.split_whitespace().next().unwrap();
                                if active {
                                    self.flags_edit = flags::remove(&self.flags_edit, base);
                                } else {
                                    self.flags_edit = flags::add(&self.flags_edit, example);
                                }
                                let _ = self
                                    .cmd_tx
                                    .send(UiCmd::ApplyFlags(self.flags_edit.trim().to_string()));
                            }
                        });
                    });

                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(RichText::new(desc).color(ui.visuals().weak_text_color()))
                            .wrap(),
                    );
                });

            ui.add_space(4.0);
        }
    }

    pub(crate) fn render_log_tab(&mut self, ui: &mut egui::Ui, tex: &AppTextures) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Engine & Prober Event Log").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let clear_btn = egui::Button::image_and_text(
                    Image::new(&tex.clear).fit_to_exact_size(Vec2::splat(16.0)),
                    " Clear",
                );
                if ui.add(clear_btn).clicked() {
                    let mut st = self.state.lock().unwrap();
                    st.log_lines.clear();
                }

                if ui.checkbox(&mut self.log_wrap, "Wrap lines").changed() {
                    let mut st = self.state.lock().unwrap();
                    st.log_wrap = self.log_wrap;
                    st.config.log_wrap = self.log_wrap;
                    let _ = st.config.save();
                }
            });
        });

        ui.add_space(4.0);

        let lines: Vec<String> = {
            let st = self.state.lock().unwrap();
            st.log_lines.iter().cloned().collect()
        };

        egui::Frame::canvas(ui.style())
            .fill(Color32::from_rgb(18, 18, 20))
            .inner_margin(8.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in &lines {
                            let text = RichText::new(line)
                                .monospace()
                                .size(11.5)
                                .color(Color32::from_rgb(220, 220, 225));
                            if self.log_wrap {
                                ui.label(text);
                            } else {
                                ui.add(
                                    egui::Label::new(text).wrap_mode(egui::TextWrapMode::Extend),
                                );
                            }
                        }
                    });
            });
    }

    pub(crate) fn render_settings_tab(&mut self, ui: &mut egui::Ui, tex: &AppTextures) {
        ui.label(RichText::new("Block Detection").strong());
        ui.horizontal(|ui| {
            ui.add(Image::new(&tex.globe).fit_to_exact_size(Vec2::splat(16.0)));
            ui.label(RichText::new("Test sites:").strong());
        });

        ui.add_space(2.0);

        ui.horizontal(|ui| {
            let undo_btn_w = 88.0;
            let edit_w = (ui.available_width() - undo_btn_w - 12.0).max(120.0);
            if ui
                .add(egui::TextEdit::singleline(&mut self.sites_edit).desired_width(edit_w))
                .lost_focus()
            {
                let mut st = self.state.lock().unwrap();
                st.config.sites = self.sites_edit.clone();
                let _ = st.config.save();
            }

            let undo_btn = egui::Button::image_and_text(
                Image::new(&tex.undo).fit_to_exact_size(Vec2::splat(14.0)),
                " Reset",
            )
            .min_size(Vec2::new(undo_btn_w, 28.0));
            if ui.add(undo_btn).clicked() {
                self.sites_edit =
                    "epicgames.com, store.steampowered.com, gog.com, torrentgalaxy.info".into();
                let mut st = self.state.lock().unwrap();
                st.config.sites = self.sites_edit.clone();
                let _ = st.config.save();
            }
        });

        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.add(Image::new(&tex.rocket).fit_to_exact_size(Vec2::splat(16.0)));
            if ui
                .checkbox(
                    &mut self.autotune_val,
                    "Search for a working TTL when a check is blocked",
                )
                .changed()
            {
                let mut st = self.state.lock().unwrap();
                st.config.autotune = self.autotune_val;
                let _ = st.config.save();
            }
        });

        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.add(Image::new(&tex.clock).fit_to_exact_size(Vec2::splat(16.0)));
            ui.label("Check interval (seconds):");
            if ui
                .add(egui::TextEdit::singleline(&mut self.check_secs_edit).desired_width(60.0))
                .lost_focus()
            {
                if let Ok(secs) = self.check_secs_edit.trim().parse::<u64>() {
                    let mut st = self.state.lock().unwrap();
                    st.config.check_secs = secs.max(10);
                    let _ = st.config.save();
                }
            }
        });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label(RichText::new("Startup").strong());
        ui.horizontal(|ui| {
            ui.add(Image::new(&tex.clock).fit_to_exact_size(Vec2::splat(16.0)));
            if ui
                .checkbox(
                    &mut self.autostart_val,
                    if cfg!(target_os = "macos") {
                        "Launch Sidestep at logon (LaunchAgent)"
                    } else {
                        "Launch Sidestep at logon (tray icon always visible)"
                    },
                )
                .changed()
            {
                let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("sidestep"));
                if self.autostart_val {
                    if let Err(e) = crate::autostart::register(&exe) {
                        eprintln!("Failed to register autostart: {e}");
                    }
                } else {
                    let _ = crate::autostart::unregister();
                }
            }
        });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        let (mode_str, dir_str) = {
            let st = self.state.lock().unwrap();
            (
                if st.mode.is_portable() {
                    "Portable"
                } else {
                    "Installed"
                },
                st.mode.dir().display().to_string(),
            )
        };

        ui.label(RichText::new("Installation").strong());
        ui.horizontal(|ui| {
            ui.add(Image::new(&tex.folder).fit_to_exact_size(Vec2::splat(16.0)));
            ui.add(egui::Label::new(format!("Mode: {mode_str} | Location: {dir_str}")).wrap());
        });

        ui.add_space(8.0);

        if !self.confirm_uninstall {
            let trash_btn = egui::Button::image_and_text(
                Image::new(&tex.trash).fit_to_exact_size(Vec2::splat(16.0)),
                " Uninstall Sidestep",
            )
            .min_size(Vec2::new(140.0, 30.0));
            if ui.add(trash_btn).clicked() {
                self.confirm_uninstall = true;
            }
        } else {
            ui.colored_label(
                Color32::from_rgb(220, 53, 69),
                "Are you sure you want to remove Sidestep?",
            );
            ui.horizontal(|ui| {
                if ui.button("Yes, Uninstall").clicked() {
                    let st = self.state.lock().unwrap();
                    let _ = crate::setup::uninstall(&st.mode);
                    let _ = crate::autostart::unregister();
                    let _ = self.cmd_tx.send(UiCmd::Quit);
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_uninstall = false;
                }
            });
        }
    }
}

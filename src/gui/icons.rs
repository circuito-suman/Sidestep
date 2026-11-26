//! Icon management and image texture decoders for the Linux GUI and FreeDesktop tray.

use eframe::egui::{ColorImage, Context, TextureHandle, TextureOptions};

// Main application and badge icons
pub const APP_ICO: &[u8] = include_bytes!("../../icons/app.ico");
pub const ON_ICO: &[u8] = include_bytes!("../../icons/on.ico");
pub const OFF_ICO: &[u8] = include_bytes!("../../icons/off.ico");
pub const BUSY_ICO: &[u8] = include_bytes!("../../icons/busy.ico");

// UI monotone glyphs
pub const ICO_STATUS: &[u8] = include_bytes!("../../icons/ui/status.ico");
pub const ICO_FLAGS: &[u8] = include_bytes!("../../icons/ui/flags.ico");
pub const ICO_LOG: &[u8] = include_bytes!("../../icons/ui/log.ico");
pub const ICO_SETTINGS: &[u8] = include_bytes!("../../icons/ui/settings.ico");

pub const ICO_POWER: &[u8] = include_bytes!("../../icons/ui/power.ico");
pub const ICO_CHECK: &[u8] = include_bytes!("../../icons/ui/check.ico");
pub const ICO_TUNE: &[u8] = include_bytes!("../../icons/ui/tune.ico");
pub const ICO_HELP: &[u8] = include_bytes!("../../icons/ui/help.ico");
pub const ICO_APPLY: &[u8] = include_bytes!("../../icons/ui/apply.ico");
pub const ICO_UNDO: &[u8] = include_bytes!("../../icons/ui/undo.ico");
pub const ICO_CLEAR: &[u8] = include_bytes!("../../icons/ui/clear.ico");
pub const ICO_PLUS: &[u8] = include_bytes!("../../icons/ui/plus.ico");
pub const ICO_MINUS: &[u8] = include_bytes!("../../icons/ui/minus.ico");
pub const ICO_TRASH: &[u8] = include_bytes!("../../icons/ui/trash.ico");
pub const ICO_FOLDER: &[u8] = include_bytes!("../../icons/ui/folder.ico");
pub const ICO_ENGINE: &[u8] = include_bytes!("../../icons/ui/engine.ico");
pub const ICO_PID: &[u8] = include_bytes!("../../icons/ui/pid.ico");
pub const ICO_CLOCK: &[u8] = include_bytes!("../../icons/ui/clock.ico");
pub const ICO_TTL: &[u8] = include_bytes!("../../icons/ui/ttl.ico");
pub const ICO_FLAG: &[u8] = include_bytes!("../../icons/ui/flag.ico");
pub const ICO_CHART: &[u8] = include_bytes!("../../icons/ui/chart.ico");
pub const ICO_GLOBE: &[u8] = include_bytes!("../../icons/ui/globe.ico");
pub const ICO_OK: &[u8] = include_bytes!("../../icons/ui/ok.ico");
pub const ICO_BAD: &[u8] = include_bytes!("../../icons/ui/bad.ico");
pub const ICO_PENDING: &[u8] = include_bytes!("../../icons/ui/pending.ico");
pub const ICO_ROCKET: &[u8] = include_bytes!("../../icons/ui/rocket.ico");

/// Decodes an embedded ICO file into raw RGBA bytes at `size x size`.
pub fn ico_to_rgba(ico_bytes: &[u8], size: u32) -> Vec<u8> {
    let img = image::load_from_memory(ico_bytes)
        .unwrap_or_else(|_| image::DynamicImage::new_rgba8(size, size));
    img.resize_exact(size, size, image::imageops::FilterType::Lanczos3)
        .to_rgba8()
        .into_raw()
}

/// Decodes an embedded ICO file into an `egui::ColorImage` at the requested dimension.
pub fn decode_ico(ico_bytes: &[u8], size: u32) -> ColorImage {
    let rgba = ico_to_rgba(ico_bytes, size);
    ColorImage::from_rgba_unmultiplied([size as usize, size as usize], &rgba)
}

/// Converts embedded ICO bytes into a `ksni::Icon` (ARGB32 network byte order) for FreeDesktop tray.
#[cfg(target_os = "linux")]
pub fn ico_to_ksni_icon(ico_bytes: &[u8], size: u32) -> ksni::Icon {
    let img = image::load_from_memory(ico_bytes)
        .unwrap_or_else(|_| image::DynamicImage::new_rgba8(size, size));
    let resized = img
        .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for pixel in resized.pixels() {
        let [r, g, b, a] = pixel.0;
        // ksni expects ARGB in network byte order: A, R, G, B
        data.push(a);
        data.push(r);
        data.push(g);
        data.push(b);
    }
    ksni::Icon {
        width: size as i32,
        height: size as i32,
        data,
    }
}

/// Cache of UI textures for egui rendering.
#[derive(Clone)]
pub struct AppTextures {
    // 40x40 Header state badges
    pub badge_off: TextureHandle,
    pub badge_on: TextureHandle,
    pub badge_busy: TextureHandle,

    // 16x16 Tab icons
    pub tab_status: TextureHandle,
    pub tab_flags: TextureHandle,
    pub tab_log: TextureHandle,
    pub tab_settings: TextureHandle,

    // 16x16 Button & row glyphs
    pub power: TextureHandle,
    pub check: TextureHandle,
    pub tune: TextureHandle,
    pub help: TextureHandle,
    pub apply: TextureHandle,
    pub undo: TextureHandle,
    pub clear: TextureHandle,
    pub plus: TextureHandle,
    pub minus: TextureHandle,
    pub trash: TextureHandle,
    pub folder: TextureHandle,
    pub engine: TextureHandle,
    pub pid: TextureHandle,
    pub clock: TextureHandle,
    pub ttl: TextureHandle,
    pub flag: TextureHandle,
    pub chart: TextureHandle,
    pub globe: TextureHandle,
    pub ok: TextureHandle,
    pub bad: TextureHandle,
    pub pending: TextureHandle,
    pub rocket: TextureHandle,
}

impl AppTextures {
    pub fn load(ctx: &Context) -> Self {
        let opt = TextureOptions::LINEAR;
        let load = |name: &str, bytes: &[u8], size: u32| {
            let img = decode_ico(bytes, size);
            ctx.load_texture(name, img, opt)
        };

        Self {
            badge_off: load("badge_off", OFF_ICO, 40),
            badge_on: load("badge_on", ON_ICO, 40),
            badge_busy: load("badge_busy", BUSY_ICO, 40),

            tab_status: load("tab_status", ICO_STATUS, 16),
            tab_flags: load("tab_flags", ICO_FLAGS, 16),
            tab_log: load("tab_log", ICO_LOG, 16),
            tab_settings: load("tab_settings", ICO_SETTINGS, 16),

            power: load("power", ICO_POWER, 16),
            check: load("check", ICO_CHECK, 16),
            tune: load("tune", ICO_TUNE, 16),
            help: load("help", ICO_HELP, 16),
            apply: load("apply", ICO_APPLY, 16),
            undo: load("undo", ICO_UNDO, 16),
            clear: load("clear", ICO_CLEAR, 16),
            plus: load("plus", ICO_PLUS, 16),
            minus: load("minus", ICO_MINUS, 16),
            trash: load("trash", ICO_TRASH, 16),
            folder: load("folder", ICO_FOLDER, 16),
            engine: load("engine", ICO_ENGINE, 16),
            pid: load("pid", ICO_PID, 16),
            clock: load("clock", ICO_CLOCK, 16),
            ttl: load("ttl", ICO_TTL, 16),
            flag: load("flag", ICO_FLAG, 16),
            chart: load("chart", ICO_CHART, 16),
            globe: load("globe", ICO_GLOBE, 16),
            ok: load("ok", ICO_OK, 16),
            bad: load("bad", ICO_BAD, 16),
            pending: load("pending", ICO_PENDING, 16),
            rocket: load("rocket", ICO_ROCKET, 16),
        }
    }
}

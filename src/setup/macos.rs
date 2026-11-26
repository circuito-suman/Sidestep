//! macOS installation, portable mode detection, payload extraction, and uninstallation.

use std::path::{Path, PathBuf};

pub const PAYLOAD: [(&str, &[u8]); 1] = [("ciadpi", include_bytes!("../../vendor/ciadpi-darwin"))];
pub const APP_NAME: &str = "Sidestep";
const APP_EXE: &str = "sidestep";
const PORTABLE_MARKER: &str = "PORTABLE.txt";
const PORTABLE_FLAG: &str = "sidestep.portable";
const PORTABLE_DIR: &str = "Sidestep-data";
const OWN_FILES: [&str; 6] = [
    "ciadpi",
    "config.ini",
    "daemon.pid",
    "proxy.env",
    "Sidestep.log",
    PORTABLE_MARKER,
];

#[derive(Clone)]
pub enum Mode {
    Installed(PathBuf),
    Portable(PathBuf),
}

impl Mode {
    pub fn dir(&self) -> &Path {
        match self {
            Mode::Installed(d) | Mode::Portable(d) => d,
        }
    }

    pub fn is_portable(&self) -> bool {
        matches!(self, Mode::Portable(_))
    }

    pub fn app_exe(&self) -> PathBuf {
        match self {
            Mode::Installed(_) => user_bin_dir().join(APP_EXE),
            Mode::Portable(_) => std::env::current_exe().unwrap_or_else(|_| PathBuf::from(APP_EXE)),
        }
    }

    pub fn engine_exe(&self) -> PathBuf {
        let local = self.dir().join("ciadpi");
        if local.is_file() {
            return local;
        }
        if let Some(sys) = find_system_ciadpi() {
            return sys;
        }
        local
    }
}

/// Locates a pre-installed `ciadpi` binary on macOS (Homebrew, `/usr/local/bin`, or `PATH`).
pub fn find_system_ciadpi() -> Option<PathBuf> {
    for p in [
        "/opt/homebrew/bin/ciadpi",
        "/usr/local/bin/ciadpi",
        "/usr/bin/ciadpi",
        "/bin/ciadpi",
    ] {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("ciadpi");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Standard Apple Application Support directory: `~/Library/Application Support/Sidestep/`.
pub fn user_data_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library")
        .join("Application Support")
        .join(APP_NAME)
}

/// User-writable CLI directory: `~/.local/bin`.
pub fn user_bin_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".local/bin"))
        .unwrap_or_else(|| PathBuf::from("/usr/local/bin"))
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn portable_dir() -> PathBuf {
    exe_dir().join(PORTABLE_DIR)
}

/// Detects portable mode from `sidestep.portable` (file or directory) adjacent to the binary,
/// or a `Sidestep-data/PORTABLE.txt` marker; otherwise defaults to Application Support.
pub fn detect_in(base_dir: &Path) -> Mode {
    let p = base_dir.join(PORTABLE_DIR);
    if base_dir.join(PORTABLE_FLAG).exists() || p.join(PORTABLE_MARKER).is_file() {
        return Mode::Portable(p);
    }
    Mode::Installed(user_data_dir())
}

pub fn detect() -> Mode {
    detect_in(&exe_dir())
}

fn set_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
}

pub fn write_payload(dir: &Path) -> Result<Vec<&'static str>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let dest = dir.join("ciadpi");
    if dest.is_file() {
        return Ok(vec![]);
    }
    // Prefer system binary; fall back to embedded Universal 2 Darwin binary.
    let wrote = if let Some(sys) = find_system_ciadpi() {
        std::fs::copy(&sys, &dest).is_ok()
    } else {
        false
    };
    if !wrote {
        std::fs::write(&dest, PAYLOAD[0].1).map_err(|e| format!("writing ciadpi: {e}"))?;
    }
    set_executable(&dest);
    Ok(vec!["ciadpi"])
}

pub fn install(dir: &Path) -> Result<Mode, String> {
    write_payload(dir)?;

    let bin_dir = user_bin_dir();
    std::fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("creating {}: {e}", bin_dir.display()))?;
    let target = bin_dir.join(APP_EXE);

    if let Ok(me) = std::env::current_exe() {
        if std::fs::canonicalize(&me).ok() != std::fs::canonicalize(&target).ok() {
            let tmp_target = bin_dir.join(format!(".{APP_EXE}.tmp.{}", std::process::id()));
            std::fs::copy(&me, &tmp_target)
                .map_err(|e| format!("copying app to {}: {e}", tmp_target.display()))?;
            set_executable(&tmp_target);
            std::fs::rename(&tmp_target, &target)
                .map_err(|e| format!("installing app to {}: {e}", target.display()))?;
        }
    }

    Ok(Mode::Installed(dir.to_path_buf()))
}

pub fn uninstall(mode: &Mode) -> Vec<String> {
    let mut done = Vec::new();
    let _ = crate::autostart::unregister();
    done.push("LaunchAgent autostart entry removed".into());

    let bin = user_bin_dir().join(APP_EXE);
    if bin.exists() && std::fs::remove_file(&bin).is_ok() {
        done.push(format!("Binary {} removed", bin.display()));
    }

    let dir = mode.dir();
    for f in OWN_FILES {
        let p = dir.join(f);
        if p.exists() {
            let _ = std::fs::remove_file(&p);
        }
    }
    let _ = std::fs::remove_dir(dir);
    done.push(format!("Files in {} removed", dir.display()));
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_portable_flag_file_and_directory() {
        let root = std::env::temp_dir().join(format!("sidestep-mac-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        assert!(!detect_in(&root).is_portable());

        // File marker: sidestep.portable
        std::fs::write(root.join(PORTABLE_FLAG), b"").unwrap();
        let mode = detect_in(&root);
        assert!(mode.is_portable());
        assert_eq!(mode.dir(), root.join(PORTABLE_DIR));

        // Directory marker: sidestep.portable/
        std::fs::remove_file(root.join(PORTABLE_FLAG)).unwrap();
        std::fs::create_dir_all(root.join(PORTABLE_FLAG)).unwrap();
        assert!(detect_in(&root).is_portable());

        let _ = std::fs::remove_dir_all(&root);
    }
}

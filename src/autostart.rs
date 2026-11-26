//! Autostart configuration:
//! * Windows: Scheduled Task ("Sidestep") via schtasks.exe running at logon with highest privileges.
//! * Linux: FreeDesktop XDG Autostart entry (~/.config/autostart/sidestep.desktop).

use std::path::Path;

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const TASK: &str = "Sidestep";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn schtasks(args: &[&str]) -> Result<String, String> {
        let out = Command::new("schtasks")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("running schtasks: {e}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if out.status.success() {
            Ok(text)
        } else {
            Err(text.trim().to_string())
        }
    }

    pub fn is_registered() -> bool {
        schtasks(&["/Query", "/TN", TASK]).is_ok()
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    pub fn register(exe: &Path) -> Result<(), String> {
        let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
            (Ok(d), Ok(u)) => format!("{d}\\{u}"),
            (_, Ok(u)) => u,
            _ => return Err("cannot determine current user".into()),
        };
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>Starts the Sidestep tray app at logon. The tray icon is always visible while it runs.</Description></RegistrationInfo>
  <Triggers>
    <LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId><Delay>PT5S</Delay></LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec><Command>"{exe}"</Command><Arguments>--autostart</Arguments></Exec>
  </Actions>
</Task>"#,
            user = xml_escape(&user),
            exe = xml_escape(&exe.display().to_string()),
        );

        let mut bytes = vec![0xFF, 0xFE];
        for u in xml.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let tmp = std::env::temp_dir().join("Sidestep-task.xml");
        std::fs::write(&tmp, bytes).map_err(|e| format!("writing task xml: {e}"))?;
        let tmp_s = tmp.display().to_string();
        let res = schtasks(&["/Create", "/TN", TASK, "/XML", &tmp_s, "/F"]);
        let _ = std::fs::remove_file(&tmp);
        res.map(|_| ())
    }

    pub fn unregister() -> Result<(), String> {
        if !is_registered() {
            return Ok(());
        }
        schtasks(&["/Delete", "/TN", TASK, "/F"]).map(|_| ())
    }

    pub fn unregister_task(name: &str) -> Result<(), String> {
        if schtasks(&["/Query", "/TN", name]).is_err() {
            return Ok(());
        }
        schtasks(&["/Delete", "/TN", name, "/F"]).map(|_| ())
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::path::PathBuf;

    fn autostart_dir() -> Option<PathBuf> {
        std::env::var("HOME").ok().map(|h| {
            std::env::var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(h).join(".config"))
                .join("autostart")
        })
    }

    fn autostart_path() -> Option<PathBuf> {
        autostart_dir().map(|d| d.join("sidestep.desktop"))
    }

    pub fn is_registered() -> bool {
        autostart_path().is_some_and(|p| p.is_file())
    }

    pub fn register(exe: &Path) -> Result<(), String> {
        let dir = autostart_dir().ok_or("cannot determine user home directory")?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("creating autostart dir: {e}"))?;
        let path = dir.join("sidestep.desktop");
        let content = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Sidestep\n\
             Comment=Autonomous DPI Bypass\n\
             Exec=\"{}\" daemon\n\
             Icon=sidestep\n\
             Terminal=false\n\
             Categories=Network;\n\
             X-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        std::fs::write(&path, content).map_err(|e| format!("writing desktop file: {e}"))
    }

    pub fn unregister() -> Result<(), String> {
        if let Some(p) = autostart_path() {
            if p.exists() {
                std::fs::remove_file(&p).map_err(|e| format!("removing autostart file: {e}"))?;
            }
        }
        Ok(())
    }

    pub fn unregister_task(_name: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    const LABEL: &str = "com.cosmicinfinity.sidestep";

    fn launch_agents_dir() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/LaunchAgents"))
    }

    fn plist_path() -> Option<PathBuf> {
        launch_agents_dir().map(|d| d.join(format!("{LABEL}.plist")))
    }

    fn gui_domain() -> String {
        let uid = unsafe { libc::getuid() };
        format!("gui/{uid}")
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    pub fn is_registered() -> bool {
        plist_path().is_some_and(|p| p.is_file())
    }

    pub fn register(exe: &Path) -> Result<(), String> {
        let dir = launch_agents_dir().ok_or("cannot determine user home directory")?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("creating LaunchAgents dir: {e}"))?;
        let path = dir.join(format!("{LABEL}.plist"));
        let log_dir = crate::setup::user_data_dir();
        let _ = std::fs::create_dir_all(&log_dir);
        let stdout_log = log_dir.join("Sidestep.log");

        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \t<key>Label</key>\n\
             \t<string>{LABEL}</string>\n\
             \t<key>ProgramArguments</key>\n\
             \t<array>\n\
             \t\t<string>{exe}</string>\n\
             \t\t<string>daemon</string>\n\
             \t</array>\n\
             \t<key>RunAtLoad</key>\n\
             \t<true/>\n\
             \t<key>KeepAlive</key>\n\
             \t<false/>\n\
             \t<key>StandardOutPath</key>\n\
             \t<string>{log}</string>\n\
             \t<key>StandardErrorPath</key>\n\
             \t<string>{log}</string>\n\
             </dict>\n\
             </plist>\n",
            exe = xml_escape(&exe.display().to_string()),
            log = xml_escape(&stdout_log.display().to_string()),
        );

        std::fs::write(&path, plist).map_err(|e| format!("writing LaunchAgent plist: {e}"))?;
        // Ensure service target is enabled in launchd without spawning a duplicate daemon immediately.
        let target = format!("{}/{LABEL}", gui_domain());
        let _ = Command::new("launchctl").args(["enable", &target]).output();
        Ok(())
    }

    pub fn unregister() -> Result<(), String> {
        if let Some(p) = plist_path() {
            if p.exists() {
                let target = format!("{}/{LABEL}", gui_domain());
                let _ = Command::new("launchctl")
                    .args(["bootout", &target])
                    .output();
                let _ = Command::new("launchctl")
                    .args(["unload", "-w"])
                    .arg(&p)
                    .output();
                std::fs::remove_file(&p).map_err(|e| format!("removing LaunchAgent plist: {e}"))?;
            }
        }
        Ok(())
    }

    pub fn unregister_task(_name: &str) -> Result<(), String> {
        Ok(())
    }
}

pub use imp::*;

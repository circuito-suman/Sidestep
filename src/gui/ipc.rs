//! Single-instance Unix domain socket IPC for Linux.
//! Allows running CLI commands (open, toggle, on, off, status) or focusing
//! an already-running GUI instance.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::state::{AppState, UiCmd, View};

/// Removes the single-instance Unix domain socket on clean shutdown.
#[cfg(target_os = "macos")]
pub fn cleanup_instance_socket() {
    let uid = unsafe { libc::getuid() };
    let _ = std::fs::remove_file(format!("/tmp/sidestep-{uid}.sock"));
}

/// Sends a command to an existing running Sidestep instance via Unix domain socket.
pub fn send_instance_cmd(cmd: &str) -> Option<String> {
    let uid = unsafe { libc::getuid() };
    let sock_path = format!("/tmp/sidestep-{uid}.sock");
    let mut s = UnixStream::connect(&sock_path).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok();
    s.write_all(format!("{cmd}\n").as_bytes()).ok()?;
    let mut resp = String::new();
    let mut buf = [0u8; 1024];
    if let Ok(n) = s.read(&mut buf) {
        resp.push_str(String::from_utf8_lossy(&buf[..n]).trim());
    }
    Some(resp)
}

/// Spawns a Unix domain socket listener to implement single-instance wake/focus/control.
pub fn spawn_instance_listener(
    state: Arc<Mutex<AppState>>,
    cmd_tx: Sender<UiCmd>,
) -> Option<String> {
    let uid = unsafe { libc::getuid() };
    let sock_path = format!("/tmp/sidestep-{uid}.sock");

    // Try connecting to existing instance
    if let Some(resp) = send_instance_cmd("open") {
        return Some(resp);
    }

    // Unlink any stale socket
    let _ = std::fs::remove_file(&sock_path);

    let listener = match UnixListener::bind(&sock_path) {
        Ok(l) => l,
        Err(_) => return None,
    };

    std::thread::Builder::new()
        .name("sidestep-socket-listener".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut s = stream;
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = [0u8; 128];
                if let Ok(n) = s.read(&mut buf) {
                    let line = String::from_utf8_lossy(&buf[..n]);
                    let cmd = line.trim();
                    match cmd {
                        "open" | "show" => {
                            let _ = cmd_tx.send(UiCmd::ShowWindow);
                            let _ = s.write_all(b"OK: window focused\n");
                        }
                        "toggle" => {
                            let _ = cmd_tx.send(UiCmd::Toggle);
                            let _ = s.write_all(b"OK: toggled\n");
                        }
                        "on" => {
                            let is_off = {
                                let st = state.lock().unwrap();
                                st.view == View::Off
                            };
                            if is_off {
                                let _ = cmd_tx.send(UiCmd::Toggle);
                                let _ = s.write_all(b"OK: turned on\n");
                            } else {
                                let _ = s.write_all(b"OK: already active\n");
                            }
                        }
                        "off" => {
                            let is_on = {
                                let st = state.lock().unwrap();
                                st.view != View::Off
                            };
                            if is_on {
                                let _ = cmd_tx.send(UiCmd::Toggle);
                                let _ = s.write_all(b"OK: turned off\n");
                            } else {
                                let _ = s.write_all(b"OK: already off\n");
                            }
                        }
                        "status" => {
                            let st = state.lock().unwrap();
                            let info = format!(
                                "STATUS: view={:?} engine_pid={:?} ttl={:?} proxy={}\n",
                                st.view,
                                st.engine_pid,
                                st.current_ttl,
                                crate::proxy::is_socks_proxy_active()
                            );
                            let _ = s.write_all(info.as_bytes());
                        }
                        _ => {
                            let _ = s.write_all(b"ERR: unknown command\n");
                        }
                    }
                }
            }
        })
        .ok();

    None
}

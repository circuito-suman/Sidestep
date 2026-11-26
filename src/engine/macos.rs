//! macOS (Darwin) ciadpi process supervision.
//! Replaces Linux's `PR_SET_PDEATHSIG` with an out-of-process `kqueue`
//! (`EVFILT_PROC` / `NOTE_EXIT`) watchdog plus `Drop` cleanup so `ciadpi`
//! never outlives Sidestep, even on unexpected termination.

use std::ffi::CStr;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::Ev;

const PROC_ALL_PIDS: u32 = 1;

extern "C" {
    fn proc_listpids(
        kind: u32,
        typeinfo: u32,
        buffer: *mut libc::c_void,
        buffersize: libc::c_int,
    ) -> libc::c_int;
    fn proc_name(pid: libc::c_int, buffer: *mut libc::c_void, buffersize: u32) -> libc::c_int;
}

#[derive(Default)]
pub struct Engine {
    child: Option<Child>,
    watchdog: Option<Child>,
    /// Incremented on every start so stale reader-thread events can be ignored.
    pub generation: u64,
    pub started_at: Option<Instant>,
    pub active_flags: String,
    /// true once the engine has survived its start-up.
    pub confirmed: bool,
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Engine {
    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }

    /// Starts the DPI bypass engine (e.g. ciadpi) with `args` under kqueue watchdog supervision.
    pub fn start(&mut self, exe: &Path, args: &[String], tx: &Sender<Ev>) -> Result<u32, String> {
        self.stop();

        let exe_abs = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
        let cur_dir = match exe_abs.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };

        let mut child = Command::new(&exe_abs)
            .args(args)
            .current_dir(cur_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to launch {}: {e}", exe.display()))?;

        let child_pid = child.id();

        // Spawn out-of-process kqueue EVFILT_PROC watchdog in the Sidestep binary
        // (skipped in lib/bin unit tests where current_exe() is the cargo test runner).
        let watchdog = if !cfg!(test) {
            let me = std::env::current_exe()
                .map_err(|e| format!("locating Sidestep binary for watchdog: {e}"))?;
            let my_pid = std::process::id();
            match Command::new(me)
                .args(["__watch", &my_pid.to_string(), &child_pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(w) => Some(w),
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "could not start kqueue watchdog; refused to run engine unsupervised: {e}"
                    ));
                }
            }
        } else {
            None
        };

        self.generation += 1;
        let gen = self.generation;
        if let Some(out) = child.stdout.take() {
            spawn_reader(out, gen, tx.clone(), true);
        }
        if let Some(err) = child.stderr.take() {
            spawn_reader(err, gen, tx.clone(), false);
        }

        self.child = Some(child);
        self.watchdog = watchdog;
        self.started_at = Some(Instant::now());
        self.active_flags = crate::flags::join(args);
        self.confirmed = false;
        Ok(child_pid)
    }

    /// Terminates the child process and reaps both child and watchdog.
    pub fn stop(&mut self) -> Option<String> {
        let mut child = self.child.take()?;
        let pid = child.id();
        let _ = child.kill();
        let status = child.wait();

        if let Some(mut w) = self.watchdog.take() {
            reap_watchdog(&mut w);
        }

        self.started_at = None;
        self.active_flags.clear();
        self.confirmed = false;
        Some(match status {
            Ok(_) => format!("bypass engine (PID {pid}) terminated"),
            Err(e) => format!("bypass engine (PID {pid}) kill issued, wait failed: {e}"),
        })
    }

    /// Non-blocking liveness check.
    pub fn poll(&mut self) -> Option<String> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => {
                let pid = child.id();
                self.child = None;
                if let Some(mut w) = self.watchdog.take() {
                    reap_watchdog(&mut w);
                }
                self.started_at = None;
                self.active_flags.clear();
                self.confirmed = false;
                Some(format!(
                    "bypass engine (PID {pid}) exited on its own: {status}"
                ))
            }
            Ok(None) => None,
            Err(e) => Some(format!("could not query bypass engine status: {e}")),
        }
    }
}

fn reap_watchdog(w: &mut Child) {
    for _ in 0..10 {
        if let Ok(Some(_)) = w.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = w.kill();
    let _ = w.wait();
}

fn spawn_reader<R: Read + Send + 'static>(stream: R, gen: u64, tx: Sender<Ev>, is_stdout: bool) {
    std::thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.split(b'\n') {
            let Ok(bytes) = line else { break };
            let text = String::from_utf8_lossy(&bytes).trim_end().to_string();
            if text.is_empty() {
                continue;
            }
            let _ = tx.send(Ev::GdpiLine(gen, text));
        }
        if is_stdout {
            let _ = tx.send(Ev::GdpiEof(gen));
        }
    });
}

fn terminate_child(child_pid: u32) {
    signal_pid(child_pid, libc::SIGTERM);
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(50));
        if !signal_pid(child_pid, 0) {
            return;
        }
    }
    signal_pid(child_pid, libc::SIGKILL);
}

fn register_exit_watch(kq: libc::c_int, pid: u32) -> Result<(), i32> {
    unsafe {
        let mut ev: libc::kevent = std::mem::zeroed();
        ev.ident = pid as usize;
        ev.filter = libc::EVFILT_PROC;
        ev.flags = libc::EV_ADD | libc::EV_ONESHOT;
        ev.fflags = libc::NOTE_EXIT;
        if libc::kevent(kq, &ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) < 0 {
            let err = *libc::__error();
            return Err(err);
        }
    }
    Ok(())
}

/// Blocks via `kqueue` (`EVFILT_PROC` / `NOTE_EXIT`) on `parent_pid` and `child_pid`.
/// Returns `true` if `parent_pid` exited first and the watchdog terminated `child_pid`.
pub fn run_kqueue_watchdog(parent_pid: u32, child_pid: u32) -> bool {
    let kq = unsafe { libc::kqueue() };
    if kq < 0 {
        return false;
    }

    if register_exit_watch(kq, parent_pid).is_err() {
        unsafe { libc::close(kq) };
        terminate_child(child_pid);
        return true;
    }
    if register_exit_watch(kq, child_pid).is_err() {
        unsafe { libc::close(kq) };
        return false;
    }

    if !signal_pid(parent_pid, 0) {
        unsafe { libc::close(kq) };
        terminate_child(child_pid);
        return true;
    }

    let mut parent_died = false;
    let mut events: [libc::kevent; 2] = unsafe { std::mem::zeroed() };
    loop {
        let n = unsafe {
            libc::kevent(
                kq,
                std::ptr::null(),
                0,
                events.as_mut_ptr(),
                events.len() as libc::c_int,
                std::ptr::null(),
            )
        };
        if n < 0 {
            let err = unsafe { *libc::__error() };
            if err == libc::EINTR {
                continue;
            }
            break;
        }
        for ev in &events[..(n as usize)] {
            if ev.ident == child_pid as usize {
                unsafe { libc::close(kq) };
                return false;
            }
            if ev.ident == parent_pid as usize {
                parent_died = true;
            }
        }
        if parent_died {
            terminate_child(child_pid);
            break;
        }
    }

    unsafe { libc::close(kq) };
    parent_died
}

/// Scans Darwin processes via `libproc` to find any running `ciadpi` or `goodbyedpi` instances.
pub fn find_goodbyedpi_pids() -> Vec<u32> {
    let mut pids = Vec::new();
    let my_pid = std::process::id();

    let needed = unsafe { proc_listpids(PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0) };
    if needed <= 0 {
        return pids;
    }

    let cap = (needed as usize / std::mem::size_of::<libc::pid_t>()) + 64;
    let mut buf: Vec<libc::pid_t> = vec![0; cap];
    let bytes = unsafe {
        proc_listpids(
            PROC_ALL_PIDS,
            0,
            buf.as_mut_ptr() as *mut libc::c_void,
            (buf.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int,
        )
    };
    if bytes <= 0 {
        return pids;
    }

    let count = (bytes as usize) / std::mem::size_of::<libc::pid_t>();
    let mut name_buf = [0u8; 64];
    for &pid in &buf[..count] {
        if pid <= 0 || pid as u32 == my_pid {
            continue;
        }
        name_buf.fill(0);
        let n = unsafe {
            proc_name(
                pid,
                name_buf.as_mut_ptr() as *mut libc::c_void,
                name_buf.len() as u32,
            )
        };
        if n > 0 {
            if let Ok(cstr) = CStr::from_bytes_until_nul(&name_buf) {
                let name = cstr.to_string_lossy();
                if name == "ciadpi" || name == "goodbyedpi" {
                    pids.push(pid as u32);
                }
            }
        }
    }

    pids
}

pub fn signal_pid(pid: u32, sig: libc::c_int) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, sig) == 0 }
}

pub fn kill_pid(pid: u32) -> bool {
    signal_pid(pid, libc::SIGKILL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn engine_lifecycle() {
        let mut eng = Engine::default();
        assert!(!eng.is_running());
        assert_eq!(eng.pid(), None);

        let (tx, _rx) = channel();
        let res = eng.start(Path::new("sleep"), &["5".into()], &tx);
        assert!(res.is_ok());
        assert!(eng.is_running());
        assert!(eng.pid().is_some());

        let stop_msg = eng.stop();
        assert!(stop_msg.is_some());
        assert!(!eng.is_running());
        assert_eq!(eng.pid(), None);
    }

    #[test]
    fn libproc_scan_runs_cleanly() {
        let found = find_goodbyedpi_pids();
        assert!(!found.contains(&std::process::id()));
    }
}

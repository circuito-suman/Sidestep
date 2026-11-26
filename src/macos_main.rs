//! Sidestep macOS (Darwin) CLI & Daemon Orchestrator.
//!
//! Provides CLI controls, autonomous background monitoring, TTL tuning,
//! and kqueue process supervision matching Sidestep's honesty contract.

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::engine::{self, Engine};
use crate::proxy;
use crate::setup::{self, Mode};
use sidestep_core::flags;
use sidestep_core::probe::{self, Verdict};
use sidestep_core::Ev;

static RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn handle_signal(_: libc::c_int) {
    RUNNING.store(false, Ordering::SeqCst);
}

fn install_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handle_signal as *const () as libc::sighandler_t;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGHUP, &sa, std::ptr::null_mut());
    }
}

fn init_mode() -> Mode {
    let mode = setup::detect();
    crate::config::set_data_dir(mode.dir().to_path_buf());
    mode
}

fn pid_file_path() -> PathBuf {
    crate::config::data_dir().join("daemon.pid")
}

fn read_daemon_pid() -> Option<u32> {
    let p = pid_file_path();
    let text = std::fs::read_to_string(&p).ok()?;
    let pid: u32 = text.trim().parse().ok()?;
    if engine::signal_pid(pid, 0) {
        Some(pid)
    } else {
        let _ = std::fs::remove_file(&p);
        None
    }
}

fn write_daemon_pid(pid: u32) {
    let _ = std::fs::create_dir_all(crate::config::data_dir());
    let _ = std::fs::write(pid_file_path(), pid.to_string());
}

fn remove_daemon_pid() {
    let _ = std::fs::remove_file(pid_file_path());
}

fn ensure_engine_binary(mode: &Mode) -> Result<PathBuf, String> {
    let p = mode.dir();
    setup::write_payload(p)?;
    Ok(mode.engine_exe())
}

/// Tries each TTL in `cfg.ttl_min..=cfg.ttl_max`, starts the engine for each,
/// and returns the first ttl that probes as `Verdict::Pass`.
fn find_working_ttl(
    eng: &mut Engine,
    exe: &Path,
    cfg: &mut Config,
    tx: &Sender<Ev>,
    sites: &[String],
    proxy_addr: SocketAddr,
) -> Option<u8> {
    for ttl in cfg.ttl_min..=cfg.ttl_max {
        let test_flags = flags::with_ttl(&cfg.flags, ttl);
        let args = flags::tokenize(&test_flags);
        if eng.start(exe, &args, tx).is_ok() {
            std::thread::sleep(Duration::from_millis(500));
            let (verdict, _) = probe::probe_all_via(sites, Some(proxy_addr));
            if verdict == Verdict::Pass {
                cfg.flags = test_flags;
                let _ = cfg.save();
                return Some(ttl);
            }
        }
    }
    None
}

pub fn main() {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();

    if raw_args.first().map(String::as_str) == Some("__watch") {
        if let (Some(ppid), Some(cpid)) = (
            raw_args.get(1).and_then(|s| s.parse::<u32>().ok()),
            raw_args.get(2).and_then(|s| s.parse::<u32>().ok()),
        ) {
            let _ = init_mode();
            if engine::run_kqueue_watchdog(ppid, cpid) {
                proxy::clear_socks_proxy();
            }
        }
        return;
    }

    let subcommands: Vec<String> = raw_args
        .iter()
        .filter(|a| !matches!(a.as_str(), "--gui" | "--debug" | "-v" | "--verbose"))
        .cloned()
        .collect();

    if subcommands.is_empty() {
        let mode = init_mode();
        let cfg = Config::load();
        let exe = match ensure_engine_binary(&mode) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Error preparing engine binary: {e}");
                std::process::exit(1);
            }
        };
        crate::gui::run_gui(mode, cfg, exe);
        return;
    }

    let cmd = subcommands[0].as_str();

    match cmd {
        "on" => cmd_on(),
        "off" => cmd_off(),
        "status" => cmd_status(),
        "check" => cmd_check(),
        "tune" => cmd_tune(),
        "daemon" | "--daemon" | "--autostart" => cmd_daemon(),
        "autostart" => cmd_autostart(&subcommands),
        "install" => cmd_install(),
        "uninstall" | "--uninstall" => cmd_uninstall(),
        "help" | "-h" | "--help" => cmd_help(),
        other => {
            eprintln!("Unknown command: {other}");
            eprintln!("Run 'sidestep help' for usage.");
            std::process::exit(1);
        }
    }
}

fn cmd_autostart(args: &[String]) {
    let sub = args.get(1).map(|s| s.as_str()).unwrap_or("status");
    match sub {
        "enable" | "on" => {
            let me = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("sidestep"));
            match crate::autostart::register(&me) {
                Ok(()) => println!(
                    "Autostart enabled: ~/Library/LaunchAgents/com.cosmicinfinity.sidestep.plist created."
                ),
                Err(e) => {
                    eprintln!("Failed to enable autostart: {e}");
                    std::process::exit(1);
                }
            }
        }
        "disable" | "off" => match crate::autostart::unregister() {
            Ok(()) => println!(
                "Autostart disabled: ~/Library/LaunchAgents/com.cosmicinfinity.sidestep.plist removed."
            ),
            Err(e) => {
                eprintln!("Failed to disable autostart: {e}");
                std::process::exit(1);
            }
        },
        _ => {
            println!(
                "Autostart is currently {}",
                if crate::autostart::is_registered() {
                    "ENABLED"
                } else {
                    "DISABLED"
                }
            );
            println!("Usage: sidestep autostart [enable|disable]");
        }
    }
}

fn cmd_on() {
    let mode = init_mode();
    if let Err(e) = ensure_engine_binary(&mode) {
        eprintln!("Error preparing engine binary: {e}");
        std::process::exit(1);
    }

    let cfg = Config::load();
    let proxy_addr = flags::proxy_addr(&cfg.flags);
    let port = flags::parse_proxy_port(&cfg.flags);

    if let Some(resp) = crate::gui::send_instance_cmd("on") {
        println!("Sidestep GUI instance active ({resp}).");
        println!("Testing connectivity to test sites...");
        std::thread::sleep(Duration::from_millis(500));
        let sites = probe::parse_sites(&cfg.sites);
        let (verdict, results) = probe::probe_all_via(&sites, Some(proxy_addr));
        print_probe_results(&results, verdict);
        println!("\nBypass is ON. Run 'sidestep off' to disable or 'sidestep status' for health.");
        return;
    }

    if let Some(pid) = read_daemon_pid() {
        println!("Sidestep daemon is already active (PID {pid}).");
        let sites = probe::parse_sites(&cfg.sites);
        let (verdict, results) = probe::probe_all_via(&sites, Some(proxy_addr));
        print_probe_results(&results, verdict);
        return;
    }

    println!("Starting Sidestep daemon in background...");
    let me = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("sidestep"));
    let spawned = Command::new(&me)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    if let Err(e) = spawned {
        eprintln!("Failed to launch daemon: {e}");
        std::process::exit(1);
    }

    // Wait briefly for daemon to initialize and bind SOCKS5 listener.
    let started = (0..25).any(|_| {
        std::thread::sleep(Duration::from_millis(150));
        read_daemon_pid().is_some()
            && TcpStream::connect_timeout(&proxy_addr, Duration::from_millis(150)).is_ok()
    });
    if !started {
        eprintln!("Warning: daemon did not confirm startup within 3.75 s.");
    }

    println!("Bypass engine started. System proxy configured: SOCKS5 127.0.0.1:{port}");
    println!("Testing connectivity to test sites...");
    let sites = probe::parse_sites(&cfg.sites);
    let (verdict, results) = probe::probe_all_via(&sites, Some(proxy_addr));
    print_probe_results(&results, verdict);

    println!("\nBypass is ON. Run 'sidestep off' to disable or 'sidestep status' for health.");
}

fn cmd_off() {
    let _mode = init_mode();

    if let Some(resp) = crate::gui::send_instance_cmd("off") {
        println!("Sidestep GUI instance: {resp}.");
        println!("System proxy restored. Bypass is OFF.");
        return;
    }

    println!("Stopping Sidestep bypass...");
    proxy::clear_socks_proxy();

    if let Some(pid) = read_daemon_pid() {
        engine::signal_pid(pid, libc::SIGTERM);
        for _ in 0..15 {
            std::thread::sleep(Duration::from_millis(50));
            if !engine::signal_pid(pid, 0) {
                break;
            }
        }
        println!("  Daemon PID {pid}: terminated");
    }
    remove_daemon_pid();

    let pids = engine::find_goodbyedpi_pids();
    for pid in pids {
        let ok = engine::kill_pid(pid);
        println!(
            "  Engine PID {pid}: {}",
            if ok {
                "terminated"
            } else {
                "failed to terminate"
            }
        );
    }

    println!("System proxy restored. Bypass is OFF.");
}

fn cmd_status() {
    let mode = init_mode();
    let cfg = Config::load();

    let daemon_pid = read_daemon_pid();
    let gui_status = if daemon_pid.is_none() {
        crate::gui::send_instance_cmd("status")
    } else {
        None
    };
    let engine_pids = engine::find_goodbyedpi_pids();
    let ttl = flags::get_ttl(&cfg.flags);
    let mut proxy_active = proxy::is_socks_proxy_active();
    if daemon_pid.is_none() && gui_status.is_none() && engine_pids.is_empty() && proxy_active {
        proxy::clear_socks_proxy();
        proxy_active = false;
    }
    let port = flags::parse_proxy_port(&cfg.flags);

    println!("==================================================");
    println!("               SIDESTEP MACOS STATUS              ");
    println!("==================================================");
    if let Some(dpid) = daemon_pid {
        println!(
            "Status:       ON (Daemon PID {dpid}, Engine PID {:?})",
            engine_pids
        );
    } else if let Some(ref gs) = gui_status {
        println!("Status:       GUI Active ({gs})");
    } else if !engine_pids.is_empty() {
        println!(
            "Status:       ON (Engine PID {:?}, unmonitored)",
            engine_pids
        );
    } else {
        println!("Status:       OFF");
    }
    println!(
        "Mode:         {}",
        if mode.is_portable() {
            "Portable"
        } else {
            "Installed"
        }
    );
    println!("Location:     {}", mode.dir().display());
    println!(
        "System Proxy: {}",
        if proxy_active {
            format!("Active (SOCKS5 127.0.0.1:{port})")
        } else {
            "Direct (none)".into()
        }
    );
    println!(
        "Active TTL:   {}",
        ttl.map_or("none".into(), |t| t.to_string())
    );
    println!("Active Flags: {}", cfg.flags);
    println!("Test Sites:   {}", cfg.sites);
    println!(
        "Autotune:     {}",
        if cfg.autotune { "Enabled" } else { "Disabled" }
    );
    println!(
        "Autostart:    {}",
        if crate::autostart::is_registered() {
            "Enabled"
        } else {
            "Disabled"
        }
    );
    println!("==================================================");
}

fn cmd_check() {
    let _mode = init_mode();
    let cfg = Config::load();

    let pids = engine::find_goodbyedpi_pids();
    let proxy = if !pids.is_empty() || read_daemon_pid().is_some() {
        Some(flags::proxy_addr(&cfg.flags))
    } else {
        None
    };

    let sites = probe::parse_sites(&cfg.sites);
    println!(
        "Probing {} sites {}...",
        sites.len(),
        if proxy.is_some() {
            "through bypass"
        } else {
            "(direct connection)"
        }
    );
    let (verdict, results) = probe::probe_all_via(&sites, proxy);
    print_probe_results(&results, verdict);
}

fn cmd_tune() {
    let mode = init_mode();
    if read_daemon_pid().is_some() || !engine::find_goodbyedpi_pids().is_empty() {
        eprintln!("Bypass engine is already running. Run 'sidestep off' before running 'tune'.");
        std::process::exit(1);
    }
    let exe = match ensure_engine_binary(&mode) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };
    let mut cfg = Config::load();
    let sites = probe::parse_sites(&cfg.sites);
    let proxy_addr = flags::proxy_addr(&cfg.flags);

    println!(
        "Starting autonomous TTL search against {} ...",
        sites.join(", ")
    );
    let (tx, _rx) = channel();
    let mut eng = Engine::default();

    match find_working_ttl(&mut eng, &exe, &mut cfg, &tx, &sites, proxy_addr) {
        Some(ttl) => {
            println!("\nSUCCESS: Found working TTL {ttl}!");
            println!("Saved working flags to config: {}", cfg.flags);
        }
        None => {
            println!(
                "\nTTL search failed: No TTL in range {}-{} passed the firewall.",
                cfg.ttl_min, cfg.ttl_max
            );
        }
    }
    eng.stop();
}

fn cmd_daemon() {
    install_signal_handlers();
    let mode = init_mode();
    if crate::gui::send_instance_cmd("status").is_some() || read_daemon_pid().is_some() {
        return;
    }
    let exe = match ensure_engine_binary(&mode) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Error: {e}");
            return;
        }
    };
    let mut cfg = Config::load();
    let proxy_addr = flags::proxy_addr(&cfg.flags);
    let port = flags::parse_proxy_port(&cfg.flags);

    let (tx, rx) = channel();
    let mut eng = Engine::default();
    let is_debug = std::env::var("SIDESTEP_DEBUG")
        .map(|v| v != "0")
        .unwrap_or(false)
        || std::env::args().any(|a| a == "--debug" || a == "-v" || a == "--verbose");

    let effective_flags = if is_debug && !flags::uses(&cfg.flags, "-x") {
        format!("{} -x 1", cfg.flags)
    } else {
        cfg.flags.clone()
    };
    let args = flags::tokenize(&effective_flags);
    if is_debug {
        eprintln!("[DEBUG/daemon] Spawning: {} {:?}", exe.display(), args);
    }

    let _pid = match eng.start(&exe, &args, &tx) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to launch bypass engine: {e}");
            return;
        }
    };
    proxy::set_socks_proxy(port);
    write_daemon_pid(std::process::id());

    let mut next_check = Instant::now() + Duration::from_secs(cfg.check_secs);
    while RUNNING.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(500));

        while let Ok(ev) = rx.try_recv() {
            if let Ev::GdpiLine(_, line) = ev {
                if is_debug {
                    eprintln!("[DEBUG/daemon] ciadpi: {line}");
                }
            }
        }

        if let Some(_msg) = eng.poll() {
            break;
        }

        if Instant::now() >= next_check {
            let sites = probe::parse_sites(&cfg.sites);
            let (verdict, _) = probe::probe_all_via(&sites, Some(proxy_addr));
            match verdict {
                Verdict::Pass => {
                    next_check = Instant::now() + Duration::from_secs(cfg.check_secs);
                }
                Verdict::Blocked => {
                    if cfg.autotune {
                        if let Some(ttl) =
                            find_working_ttl(&mut eng, &exe, &mut cfg, &tx, &sites, proxy_addr)
                        {
                            if is_debug {
                                eprintln!("[DEBUG/daemon] Autotune found working TTL {ttl}");
                            }
                        }
                    }
                    next_check = Instant::now() + Duration::from_secs(cfg.check_secs);
                }
                Verdict::Unreachable => {
                    next_check = Instant::now() + Duration::from_secs(cfg.check_secs);
                }
            }
        }
    }

    proxy::clear_socks_proxy();
    eng.stop();
    remove_daemon_pid();
}

fn cmd_install() {
    let mode = init_mode();
    match setup::install(mode.dir()) {
        Ok(m) => {
            println!("Sidestep successfully installed to:");
            println!(
                "  Binary: {}",
                setup::user_bin_dir().join("sidestep").display()
            );
            println!("  Data:   {}", m.dir().display());
            println!("\nYou can now run 'sidestep on' or launch the Sidestep GUI.");
        }
        Err(e) => {
            eprintln!("Installation failed: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_uninstall() {
    cmd_off();
    let mode = init_mode();
    let notes = setup::uninstall(&mode);
    println!("Sidestep uninstalled successfully:");
    for n in notes {
        println!("  - {n}");
    }
}

fn print_probe_results(results: &[probe::SiteResult], overall: Verdict) {
    for r in results {
        let tag = match r.verdict {
            Verdict::Pass => "PASSED",
            Verdict::Blocked => "BLOCKED",
            Verdict::Unreachable => "UNREACHABLE",
        };
        println!("  {:<26} [{:^11}] {}", r.host, tag, r.detail);
    }
    println!("Overall Verdict: {:?}", overall);
}

fn cmd_help() {
    println!(
        "Sidestep - Autonomous DPI Bypass for Game Launchers & Web Services\n\n\
         USAGE:\n\
         \x20 sidestep <COMMAND>\n\n\
         COMMANDS:\n\
         \x20 on           Turn bypass ON, configure system proxy, and verify\n\
         \x20 off          Turn bypass OFF and clear system proxy\n\
         \x20 status       Show current bypass state, active TTL, flags, and health\n\
         \x20 check        Send a TLS ClientHello probe to test sites through the bypass\n\
         \x20 tune         Run autonomous TTL search (1..12) to discover working TTL\n\
         \x20 daemon       Run the background monitor (90s prober, auto-tuning, supervision)\n\
         \x20 autostart    Manage LaunchAgent autostart [enable|disable|status]\n\
         \x20 install      Install binary to ~/.local/bin and set up Application Support data\n\
         \x20 uninstall    Remove Sidestep binary, configuration, and LaunchAgent\n\
         \x20 help         Print this help message\n"
    );
}

//! System proxy integration for macOS (Darwin) via `route` and `networksetup`,
//! plus shell environment file (`all_proxy` / `ALL_PROXY`).

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn run_cmd(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

fn networksetup(args: &[&str]) -> bool {
    Command::new("networksetup")
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Extracts the default route interface name (e.g. `en0`) from `route -n get default`.
pub fn parse_default_interface(route_out: &str) -> Option<String> {
    for line in route_out.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("interface:") {
            let iface = rest.trim();
            if !iface.is_empty() {
                return Some(iface.to_string());
            }
        }
    }
    None
}

/// Parses `networksetup -listnetworkserviceorder` into `(service_name, optional_device)`.
/// Disabled services prefixed with `(*)` are skipped.
pub fn parse_network_services(order_out: &str) -> Vec<(String, Option<String>)> {
    let mut services = Vec::new();
    let mut current_service: Option<String> = None;

    for line in order_out.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            current_service = None;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('(') {
            if let Some((prefix, name)) = rest.split_once(')') {
                if prefix == "*" {
                    current_service = None;
                    continue;
                }
                if prefix.chars().all(|c| c.is_ascii_digit()) && !prefix.is_empty() {
                    let svc = name.trim();
                    if !svc.is_empty() {
                        current_service = Some(svc.to_string());
                    }
                    continue;
                }
            }
        }
        if let Some(svc) = current_service.take() {
            let device = trimmed
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(')'))
                .and_then(|inner| inner.rsplit_once("Device:").map(|(_, d)| d))
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(str::to_string);
            services.push((svc, device));
        }
    }

    services
}

/// Parses `networksetup -getsocksfirewallproxy <service>` output into `(enabled, server, port)`.
pub fn parse_socks_proxy_status(out: &str) -> (bool, Option<String>, Option<u16>) {
    let mut enabled = false;
    let mut server = None;
    let mut port = None;

    for line in out.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim();
            match k.trim() {
                "Enabled" => enabled = v.eq_ignore_ascii_case("Yes"),
                "Server" if !v.is_empty() => server = Some(v.to_string()),
                "Port" => port = v.parse::<u16>().ok(),
                _ => {}
            }
        }
    }

    (enabled, server, port)
}

fn all_enabled_services() -> Vec<(String, Option<String>)> {
    run_cmd("networksetup", &["-listnetworkserviceorder"])
        .map(|out| parse_network_services(&out))
        .unwrap_or_default()
}

/// Discovers active macOS network service names matching the default route interface,
/// falling back to enabled Ethernet/Wi-Fi hardware services when offline or tunneled.
fn active_services() -> Vec<String> {
    let services = all_enabled_services();
    let default_iface = run_cmd("route", &["-n", "get", "default"])
        .or_else(|| run_cmd("route", &["get", "default"]))
        .and_then(|out| parse_default_interface(&out));

    if let Some(ref iface) = default_iface {
        let matched: Vec<String> = services
            .iter()
            .filter(|(_, dev)| dev.as_deref() == Some(iface.as_str()))
            .map(|(svc, _)| svc.clone())
            .collect();
        if !matched.is_empty() {
            return matched;
        }
    }

    let hw: Vec<String> = services
        .iter()
        .filter(|(_, dev)| dev.as_deref().is_some_and(|d| d.starts_with("en")))
        .map(|(svc, _)| svc.clone())
        .collect();
    if !hw.is_empty() {
        return hw;
    }

    vec!["Wi-Fi".to_string()]
}

fn proxy_env_dir() -> PathBuf {
    crate::setup::detect().dir().to_path_buf()
}

/// Configures the macOS SOCKS5 system proxy on the active network service and writes `proxy.env`.
pub fn set_socks_proxy(port: u16) -> bool {
    let port_str = port.to_string();
    let mut any_ok = false;

    for service in active_services() {
        let ok_addr = networksetup(&["-setsocksfirewallproxy", &service, "127.0.0.1", &port_str]);
        let ok_state = networksetup(&["-setsocksfirewallproxystate", &service, "on"]);
        if ok_addr && ok_state {
            any_ok = true;
        }
    }

    let dir = proxy_env_dir();
    let _ = fs::create_dir_all(&dir);
    let content = format!(
        "export all_proxy=\"socks5h://127.0.0.1:{port}\"\n\
         export ALL_PROXY=\"socks5h://127.0.0.1:{port}\"\n"
    );
    let _ = fs::write(dir.join("proxy.env"), content);

    any_ok
}

/// Disables the macOS SOCKS5 system proxy and removes `proxy.env`, restoring direct connectivity.
pub fn clear_socks_proxy() -> bool {
    let mut targets: BTreeSet<String> = active_services().into_iter().collect();
    for (svc, _) in all_enabled_services() {
        if let Some(out) = run_cmd("networksetup", &["-getsocksfirewallproxy", &svc]) {
            let (enabled, server, _) = parse_socks_proxy_status(&out);
            if enabled && server.as_deref() == Some("127.0.0.1") {
                targets.insert(svc);
            }
        }
    }

    let mut any_ok = false;
    for service in targets {
        if let Some(out) = run_cmd("networksetup", &["-getsocksfirewallproxy", &service]) {
            let (enabled, _, _) = parse_socks_proxy_status(&out);
            if !enabled {
                continue;
            }
        }
        if networksetup(&["-setsocksfirewallproxystate", &service, "off"]) {
            any_ok = true;
        }
    }

    let env_dir = proxy_env_dir();
    let _ = fs::remove_file(env_dir.join("proxy.env"));
    let app_support = crate::setup::user_data_dir();
    if env_dir != app_support {
        let _ = fs::remove_file(app_support.join("proxy.env"));
    }

    any_ok
}

/// Checks whether Sidestep's SOCKS5 proxy is currently enabled on an active macOS network service.
pub fn is_socks_proxy_active() -> bool {
    for service in active_services() {
        if let Some(out) = run_cmd("networksetup", &["-getsocksfirewallproxy", &service]) {
            let (enabled, server, _) = parse_socks_proxy_status(&out);
            if enabled && server.as_deref() == Some("127.0.0.1") {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_route_default_interface() {
        let sample = "\
   route to: default
destination: default
       mask: default
    gateway: 192.168.1.1
  interface: en0
      flags: <UP,GATEWAY,DONE,STATIC,PRCLONING,GLOBAL>
";
        assert_eq!(parse_default_interface(sample).as_deref(), Some("en0"));
    }

    #[test]
    fn parses_network_service_order() {
        let sample = "\
An asterisk (*) denotes that a network service is disabled.
(1) USB 10/100 LAN
(Hardware Port: USB 10/100 LAN, Device: en5)

(*) Disabled Ethernet
(Hardware Port: Ethernet, Device: en7)

(4) Wi-Fi
(Hardware Port: Wi-Fi, Device: en0)

(5) Tailscale
(Hardware Port: io.tailscale.ipn.macos, Device: )
";
        let svcs = parse_network_services(sample);
        assert_eq!(
            svcs,
            vec![
                ("USB 10/100 LAN".to_string(), Some("en5".to_string())),
                ("Wi-Fi".to_string(), Some("en0".to_string())),
                ("Tailscale".to_string(), None),
            ]
        );
    }

    #[test]
    fn parses_socks_proxy_output() {
        let sample = "\
Enabled: Yes
Server: 127.0.0.1
Port: 1080
Authenticated Proxy Enabled: 0
";
        assert_eq!(
            parse_socks_proxy_status(sample),
            (true, Some("127.0.0.1".to_string()), Some(1080))
        );
    }
}

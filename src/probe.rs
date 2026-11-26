//! Block detector: opens a raw TCP connection to <site>:443 and sends a TLS
//! ClientHello carrying the site's SNI. Because goodbyedpi intercepts all
//! outbound 443 traffic, this hello goes through the bypass exactly like a
//! browser's/launcher's would.
//!
//!   server answers with a TLS record     -> passed (DPI let it through)
//!   RST / close / silence after hello    -> blocked (what the campus DPI does)
//!   DNS or TCP connect fails             -> unreachable (network/IP issue, not SNI DPI)
//!
//! No TLS library: we only need the first 5 bytes of the server's reply.

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    Pass,
    Blocked,
    Unreachable,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SiteResult {
    pub host: String,
    pub verdict: Verdict,
    pub detail: String,
}

/// Accepts "epicgames.com", "https://store.steampowered.com/app/1/x", "gog.com:443"...
pub fn normalize_host(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s.split("://").last().unwrap_or(s);
    let s = s.split(['/', '?', '#']).next().unwrap_or("");
    let s = s.rsplit('@').next().unwrap_or(s);
    let s = s.split(':').next().unwrap_or("");
    let s = s.trim().trim_end_matches('.').to_ascii_lowercase();
    if s.is_empty() || !s.contains('.') {
        None
    } else {
        Some(s)
    }
}

pub fn parse_sites(list: &str) -> Vec<String> {
    list.split([',', ' ', ';', '\n'])
        .filter_map(normalize_host)
        .collect()
}

/// Overall verdict: any block => Blocked; else any pass => Pass; else Unreachable.
pub fn probe_all(sites: &[String]) -> (Verdict, Vec<SiteResult>) {
    probe_all_via(sites, None)
}

pub fn probe_all_via(sites: &[String], proxy: Option<SocketAddr>) -> (Verdict, Vec<SiteResult>) {
    let results: Vec<SiteResult> = if sites.len() <= 1 {
        sites.iter().map(|h| probe_site_via(h, proxy)).collect()
    } else {
        std::thread::scope(|scope| {
            let handles: Vec<_> = sites
                .iter()
                .map(|h| scope.spawn(move || probe_site_via(h, proxy)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| SiteResult {
                        host: String::new(),
                        verdict: Verdict::Unreachable,
                        detail: "probe thread panicked".into(),
                    })
                })
                .collect()
        })
    };
    let v = if results.iter().any(|r| r.verdict == Verdict::Blocked) {
        Verdict::Blocked
    } else if results.iter().any(|r| r.verdict == Verdict::Pass) {
        Verdict::Pass
    } else {
        Verdict::Unreachable
    };
    (v, results)
}

pub fn probe_site(host: &str) -> SiteResult {
    probe_site_via(host, None)
}

fn socks5_connect(
    proxy: &SocketAddr,
    host: &str,
    port: u16,
) -> Result<TcpStream, (Verdict, String)> {
    let mut s = match TcpStream::connect_timeout(proxy, Duration::from_secs(3)) {
        Ok(s) => s,
        Err(e) => {
            return Err((
                Verdict::Unreachable,
                format!("connect to proxy {proxy} failed: {e}"),
            ))
        }
    };
    let _ = s.set_nodelay(true);
    let _ = s.set_read_timeout(Some(Duration::from_secs(6)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(5)));

    // SOCKS5 greeting: [VER=5, NMETHODS=1, METHOD=0 (no auth)]
    if let Err(e) = s.write_all(&[0x05, 0x01, 0x00]) {
        return Err((
            Verdict::Unreachable,
            format!("SOCKS5 greeting send failed: {e}"),
        ));
    }
    let mut resp = [0u8; 2];
    if let Err(e) = s.read_exact(&mut resp) {
        return Err((
            Verdict::Unreachable,
            format!("SOCKS5 greeting read failed: {e}"),
        ));
    }
    if resp != [0x05, 0x00] {
        return Err((
            Verdict::Unreachable,
            format!("SOCKS5 auth rejected: {resp:?}"),
        ));
    }

    // SOCKS5 connect request: [VER=5, CMD=1, RSV=0, ATYP=3 (domain), LEN, host, PORT]
    let h_bytes = host.as_bytes();
    if h_bytes.len() > 255 {
        return Err((Verdict::Unreachable, "host name too long for SOCKS5".into()));
    }
    let mut req = Vec::with_capacity(7 + h_bytes.len());
    req.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, h_bytes.len() as u8]);
    req.extend_from_slice(h_bytes);
    req.extend_from_slice(&port.to_be_bytes());
    if let Err(e) = s.write_all(&req) {
        return Err((
            Verdict::Unreachable,
            format!("SOCKS5 connect request failed: {e}"),
        ));
    }

    let mut reply = [0u8; 4];
    if let Err(e) = s.read_exact(&mut reply) {
        return Err((
            Verdict::Unreachable,
            format!("SOCKS5 connect reply read failed: {e}"),
        ));
    }
    if reply[0] != 0x05 || reply[1] != 0x00 {
        let verdict = match reply[1] {
            0x01 | 0x03..=0x06 => Verdict::Unreachable,
            _ => Verdict::Blocked,
        };
        return Err((
            verdict,
            format!("SOCKS5 connect rejected with status 0x{:02x}", reply[1]),
        ));
    }
    let to_skip = match reply[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut len = [0u8; 1];
            if let Err(e) = s.read_exact(&mut len) {
                return Err((
                    Verdict::Unreachable,
                    format!("SOCKS5 bnd.addr len read failed: {e}"),
                ));
            }
            len[0] as usize + 2
        }
        _ => return Err((Verdict::Unreachable, "SOCKS5 unknown ATYP in reply".into())),
    };
    let mut skip_buf = vec![0u8; to_skip];
    if let Err(e) = s.read_exact(&mut skip_buf) {
        return Err((
            Verdict::Unreachable,
            format!("SOCKS5 bnd.addr read failed: {e}"),
        ));
    }
    Ok(s)
}

pub fn probe_site_via(host: &str, proxy: Option<SocketAddr>) -> SiteResult {
    let r = |verdict, detail: String| SiteResult {
        host: host.to_string(),
        verdict,
        detail,
    };

    let (mut s, target_info) = if let Some(p) = proxy {
        match socks5_connect(&p, host, 443) {
            Ok(s) => (s, format!("via SOCKS5 {p}")),
            Err((v, d)) => return r(v, d),
        }
    } else {
        let addrs: Vec<SocketAddr> = match (host, 443).to_socket_addrs() {
            Ok(a) => a.filter(|a| a.is_ipv4()).collect(),
            Err(e) => return r(Verdict::Unreachable, format!("DNS lookup failed: {e}")),
        };
        let Some(addr) = addrs.first().copied() else {
            return r(Verdict::Unreachable, "no IPv4 address".into());
        };

        let s = match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
            Ok(s) => s,
            Err(e) => {
                return r(
                    Verdict::Unreachable,
                    format!("TCP connect to {addr} failed: {e}"),
                )
            }
        };
        let _ = s.set_nodelay(true);
        let _ = s.set_read_timeout(Some(Duration::from_secs(6)));
        let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
        (s, format!("from {addr}"))
    };

    if let Err(e) = s.write_all(&client_hello(host)) {
        return r(Verdict::Blocked, format!("send failed after connect: {e}"));
    }

    let mut buf = [0u8; 5];
    match s.read(&mut buf) {
        Ok(0) => r(
            Verdict::Blocked,
            "connection closed right after ClientHello".into(),
        ),
        Ok(_) if buf[0] == 0x16 => r(Verdict::Pass, format!("ServerHello {target_info}")),
        Ok(_) if buf[0] == 0x15 => r(
            Verdict::Pass,
            format!("TLS alert {target_info} (server reached)"),
        ),
        Ok(_) => r(
            Verdict::Pass,
            format!("non-TLS reply {target_info} (server reached)"),
        ),
        Err(e)
            if matches!(
                e.kind(),
                ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
            ) =>
        {
            r(Verdict::Blocked, "reset after ClientHello (DPI RST)".into())
        }
        Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => r(
            Verdict::Blocked,
            "no reply 6s after ClientHello (dropped)".into(),
        ),
        Err(e) => r(
            Verdict::Blocked,
            format!("read error after ClientHello: {e}"),
        ),
    }
}

// ---------------------------------------------------------------- ClientHello

fn rand32() -> [u8; 32] {
    static CTR: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    let mut x = t
        ^ CTR.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
        ^ (std::process::id() as u64) << 32;
    let mut out = [0u8; 32];
    for chunk in out.chunks_mut(8) {
        // xorshift64*
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        chunk.copy_from_slice(&x.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes());
    }
    out
}

fn u16be(v: &mut Vec<u8>, n: usize) {
    v.extend_from_slice(&(n as u16).to_be_bytes());
}

fn ext(v: &mut Vec<u8>, ty: u16, body: &[u8]) {
    u16be(v, ty as usize);
    u16be(v, body.len());
    v.extend_from_slice(body);
}

fn client_hello(host: &str) -> Vec<u8> {
    let h = host.as_bytes();
    let mut e = Vec::new();

    // server_name
    let mut sni = Vec::new();
    u16be(&mut sni, h.len() + 3);
    sni.push(0);
    u16be(&mut sni, h.len());
    sni.extend_from_slice(h);
    ext(&mut e, 0x0000, &sni);
    // supported_groups: x25519, secp256r1
    ext(&mut e, 0x000a, &[0x00, 0x04, 0x00, 0x1d, 0x00, 0x17]);
    // ec_point_formats: uncompressed
    ext(&mut e, 0x000b, &[0x01, 0x00]);
    // signature_algorithms
    let mut sa = Vec::new();
    let algs: [u16; 8] = [
        0x0403, 0x0804, 0x0401, 0x0503, 0x0805, 0x0501, 0x0806, 0x0601,
    ];
    u16be(&mut sa, algs.len() * 2);
    for a in algs {
        u16be(&mut sa, a as usize);
    }
    ext(&mut e, 0x000d, &sa);
    // ALPN: http/1.1
    ext(&mut e, 0x0010, b"\x00\x09\x08http/1.1");
    // supported_versions: TLS 1.3, 1.2
    ext(&mut e, 0x002b, &[0x04, 0x03, 0x04, 0x03, 0x03]);
    // psk_key_exchange_modes: psk_dhe_ke
    ext(&mut e, 0x002d, &[0x01, 0x01]);
    // key_share: x25519 (any 32 bytes is a valid x25519 public value)
    let mut ks = Vec::new();
    u16be(&mut ks, 36);
    u16be(&mut ks, 0x001d);
    u16be(&mut ks, 32);
    ks.extend_from_slice(&rand32());
    ext(&mut e, 0x0033, &ks);

    let mut body = vec![0x03, 0x03];
    body.extend_from_slice(&rand32());
    body.push(32);
    body.extend_from_slice(&rand32());
    let suites: [u16; 7] = [0x1301, 0x1302, 0x1303, 0xc02b, 0xc02f, 0xc02c, 0xc030];
    u16be(&mut body, suites.len() * 2);
    for s in suites {
        u16be(&mut body, s as usize);
    }
    body.extend_from_slice(&[0x01, 0x00]);
    u16be(&mut body, e.len());
    body.extend_from_slice(&e);

    let mut hs = vec![0x01];
    hs.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    hs.extend_from_slice(&body);

    let mut rec = vec![0x16, 0x03, 0x01];
    u16be(&mut rec, hs.len());
    rec.extend_from_slice(&hs);
    rec
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hosts() {
        assert_eq!(
            normalize_host("https://store.steampowered.com/app/1/x").unwrap(),
            "store.steampowered.com"
        );
        assert_eq!(normalize_host("GOG.com:443").unwrap(), "gog.com");
        assert!(normalize_host("localhost").is_none());
    }
    #[test]
    fn hello_lengths_consistent() {
        let h = client_hello("epicgames.com");
        assert_eq!(h[0], 0x16);
        let rec_len = u16::from_be_bytes([h[3], h[4]]) as usize;
        assert_eq!(rec_len, h.len() - 5);
        let hs_len = u32::from_be_bytes([0, h[6], h[7], h[8]]) as usize;
        assert_eq!(hs_len, h.len() - 9);
    }

    #[test]
    fn socks5_unreachable_reply_is_unreachable() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut conn, _)) = listener.accept() {
                let mut buf = [0u8; 64];
                let _ = conn.read(&mut buf);
                let _ = conn.write_all(&[0x05, 0x00]);
                let _ = conn.read(&mut buf);
                // 0x04 = Host unreachable
                let _ = conn.write_all(&[0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
            }
        });
        let res = probe_site_via("epicgames.com", Some(addr));
        assert_eq!(res.verdict, Verdict::Unreachable);
    }
}

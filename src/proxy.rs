//! Proxy profiles, sing-box TUN config generation and a direct handshake test.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{paths, sys};

pub const TUN_NAME: &str = "ZapretTun";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    #[default]
    Socks5,
    Http,
}

impl ProxyKind {
    pub fn label(self) -> &'static str {
        match self {
            ProxyKind::Socks5 => "SOCKS5",
            ProxyKind::Http => "HTTP",
        }
    }
}

fn ser_secret<S: serde::Serializer>(v: &str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&sys::protect_str(v))
}
fn de_secret<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let hex = String::deserialize(d)?;
    Ok(sys::unprotect_str(&hex))
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub kind: ProxyKind,
    pub host: String,
    pub port: String,
    pub username: String,
    /// Stored encrypted with DPAPI (current Windows user only).
    #[serde(rename = "password_dpapi", serialize_with = "ser_secret", deserialize_with = "de_secret")]
    pub password: String,
}

impl Profile {
    pub fn display_name(&self) -> String {
        if !self.name.trim().is_empty() {
            self.name.trim().to_string()
        } else if !self.host.trim().is_empty() {
            format!("{}:{}", self.host.trim(), self.port.trim())
        } else {
            "Новый прокси".into()
        }
    }

    pub fn port_num(&self) -> Result<u16, String> {
        match self.port.trim().parse::<u16>() {
            Ok(p) if p > 0 => Ok(p),
            _ => Err("Укажите порт прокси (1–65535).".into()),
        }
    }

    pub fn validate(&self) -> Result<u16, String> {
        if self.host.trim().is_empty() {
            return Err("Укажите адрес прокси-сервера.".into());
        }
        self.port_num()
    }

    /// Resolves the proxy host (IPv4 preferred). Done by us so the kill switch can whitelist the IP.
    pub fn resolve(&self) -> Result<SocketAddr, String> {
        let port = self.validate()?;
        let host = self.host.trim().trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(SocketAddr::new(ip, port));
        }
        let addrs: Vec<SocketAddr> = (host, port)
            .to_socket_addrs()
            .map_err(|e| format!("не удалось найти адрес {host}: {e}"))?
            .collect();
        addrs
            .iter()
            .find(|a| a.is_ipv4())
            .or_else(|| addrs.first())
            .copied()
            .ok_or_else(|| format!("не удалось найти адрес {host}"))
    }
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Accepts `socks5://user:pass@host:port`, `http://host:port`, `user:pass@host:port`,
/// `host:port:user:pass` and plain `host:port`.
pub fn parse_line(input: &str) -> Option<Profile> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let mut p = Profile::default();
    let mut rest = s;
    if let Some(i) = s.find("://") {
        let scheme = s[..i].to_ascii_lowercase();
        p.kind = match scheme.as_str() {
            "socks" | "socks5" | "socks5h" => ProxyKind::Socks5,
            "http" => ProxyKind::Http,
            _ => return None,
        };
        rest = &s[i + 3..];
    }
    let rest = rest.split(['/', '?', '#']).next().unwrap_or(rest);

    let (auth, hostport) = match rest.rfind('@') {
        Some(i) => (Some(&rest[..i]), &rest[i + 1..]),
        None => (None, rest),
    };

    // host:port, [v6]:port or host:port:user:pass
    let (host, port, extra): (String, String, Option<(String, String)>) = if let Some(stripped) = hostport.strip_prefix('[') {
        let end = stripped.find(']')?;
        let host = stripped[..end].to_string();
        let port = stripped[end + 1..].trim_start_matches(':').to_string();
        (host, port, None)
    } else {
        let parts: Vec<&str> = hostport.split(':').collect();
        match parts.len() {
            2 => (parts[0].into(), parts[1].into(), None),
            4 if auth.is_none() => (parts[0].into(), parts[1].into(), Some((parts[2].into(), parts[3].into()))),
            _ => return None,
        }
    };
    if host.is_empty() || port.parse::<u16>().is_err() {
        return None;
    }
    p.host = host;
    p.port = port;
    if let Some(a) = auth {
        let (u, pw) = a.split_once(':').unwrap_or((a, ""));
        p.username = pct_decode(u);
        p.password = pct_decode(pw);
    } else if let Some((u, pw)) = extra {
        p.username = u;
        p.password = pw;
    }
    Some(p)
}

// ------------------------------------------------------------------ settings

pub const DNS_CHOICES: &[(&str, &str)] = &[
    ("1.1.1.1", "Cloudflare 1.1.1.1"),
    ("8.8.8.8", "Google 8.8.8.8"),
    ("9.9.9.9", "Quad9 9.9.9.9"),
    ("77.88.8.8", "Яндекс 77.88.8.8"),
];

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub profiles: Vec<Profile>,
    pub selected: usize,
    pub kill_switch: bool,
    pub allow_lan: bool,
    pub block_quic: bool,
    pub ipv6: bool,
    pub auto_reconnect: bool,
    pub connect_on_launch: bool,
    pub dns: String,
    pub bypass_processes: String,
    pub bypass_domains: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            profiles: Vec::new(),
            selected: 0,
            kill_switch: true,
            allow_lan: true,
            block_quic: true,
            ipv6: false,
            auto_reconnect: true,
            connect_on_launch: false,
            dns: "1.1.1.1".into(),
            bypass_processes: String::new(),
            bypass_domains: String::new(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read(paths::proxy_json())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(paths::proxy_json(), s);
        }
    }

    pub fn current(&self) -> Option<&Profile> {
        self.profiles.get(self.selected)
    }
}

fn split_list(s: &str) -> Vec<String> {
    s.split(['\n', '\r', ',', ';', ' '])
        .map(|x| x.trim().trim_start_matches("*.").trim_start_matches('.').to_string())
        .filter(|x| !x.is_empty() && !x.starts_with('#'))
        .collect()
}

/// Candidate /30 subnets for our TUN (first host address of each). Deliberately NOT sing-box's
/// default 172.19.0.1/30: other sing-box based VPNs (Sota Connect, Hiddify…) use it, and two adapters
/// with the same address make Windows hand our tunnel's packets to the other one — TCP then lives on
/// retransmits and ping jumps to 600–3000 ms.
const TUN_V4_CANDIDATES: &[[u8; 4]] = &[
    [172, 29, 53, 1],
    [10, 233, 137, 1],
    [192, 168, 243, 1],
    [172, 26, 199, 1],
    [10, 198, 71, 1],
];
const TUN_V6: &str = "fd5a:7e70:9c11::1/126";

/// Picks the first candidate /30 that does not overlap any adapter's subnet.
pub fn pick_tun_v4(used: &[(Ipv4Addr, Ipv4Addr)]) -> Ipv4Addr {
    let ours_mask = u32::MAX << 2;
    TUN_V4_CANDIDATES
        .iter()
        .map(|o| Ipv4Addr::from(*o))
        .find(|c| {
            let c = u32::from(*c);
            used.iter().all(|(a, m)| {
                let a = u32::from(*a);
                // Overlap if either network contains the other: compare under the wider (shorter) mask.
                let mask = u32::from(*m) & ours_mask;
                a == 0 || (a & mask) != (c & mask)
            })
        })
        .unwrap_or_else(|| Ipv4Addr::from(TUN_V4_CANDIDATES[0]))
}

/// Builds a sing-box (1.12+ schema) config: TUN with auto_route captures all traffic,
/// DNS goes over DoH through the proxy, optional direct rules for LAN / processes / domains.
pub fn singbox_config(p: &Profile, server: SocketAddr, s: &Settings, tun_v4: Ipv4Addr) -> Value {
    let mut out = json!({
        "type": match p.kind { ProxyKind::Socks5 => "socks", ProxyKind::Http => "http" },
        "tag": "proxy",
        "server": server.ip().to_string(),
        "server_port": server.port(),
    });
    if p.kind == ProxyKind::Socks5 {
        out["version"] = json!("5");
    }
    if !p.username.is_empty() {
        out["username"] = json!(p.username);
        out["password"] = json!(p.password);
    }

    let procs = split_list(&s.bypass_processes);
    let domains = split_list(&s.bypass_domains);

    let mut dns_rules = Vec::new();
    if !domains.is_empty() {
        dns_rules.push(json!({ "domain_suffix": domains, "server": "dns-local" }));
    }

    let mut rules = vec![
        // Catch ordinary DNS before sniffing (including TCP DNS).
        json!({ "port": 53, "action": "hijack-dns" }),
        json!({ "action": "sniff" }),
        json!({ "protocol": "dns", "action": "hijack-dns" }),
    ];
    if s.allow_lan {
        rules.push(json!({ "ip_is_private": true, "outbound": "direct" }));
    }
    if !procs.is_empty() {
        rules.push(json!({ "process_name": procs, "outbound": "direct" }));
    }
    if !domains.is_empty() {
        rules.push(json!({ "domain_suffix": domains, "outbound": "direct" }));
    }
    // HTTP CONNECT carries TCP only. Reject all remaining UDP, after DNS and
    // explicit direct exceptions, so clients can fall back instead of timing out.
    if p.kind == ProxyKind::Http {
        rules.push(json!({ "network": "udp", "action": "reject" }));
    } else if s.block_quic {
        rules.push(json!({ "network": "udp", "port": 443, "action": "reject" }));
    }
    if !s.ipv6 {
        rules.push(json!({ "ip_version": 6, "action": "reject" }));
    }

    let dns = if s.dns.trim().is_empty() { "1.1.1.1".to_string() } else { s.dns.trim().to_string() };

    json!({
        "log": { "level": "warn", "timestamp": false },
        "dns": {
            "servers": [
                { "type": "https", "tag": "dns-remote", "server": dns, "detour": "proxy" },
                { "type": "local", "tag": "dns-local" }
            ],
            "rules": dns_rules,
            "final": "dns-remote",
            "strategy": if s.ipv6 { "prefer_ipv4" } else { "ipv4_only" }
        },
        "inbounds": [{
            "type": "tun",
            "tag": "tun-in",
            "interface_name": TUN_NAME,
            "address": [format!("{tun_v4}/30"), TUN_V6],
            "mtu": 1500,
            "auto_route": true,
            "strict_route": true
        }],
        "outbounds": [ out, { "type": "direct", "tag": "direct" } ],
        "route": {
            "rules": rules,
            "final": "proxy",
            "auto_detect_interface": true,
            "default_domain_resolver": "dns-local"
        }
    })
}

// ---------------------------------------------------------------------- test

fn b64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in input.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

const TEST_HOST: &str = "cp.cloudflare.com";

fn read_exact_t(s: &mut TcpStream, n: usize) -> Result<Vec<u8>, String> {
    let mut b = vec![0u8; n];
    s.read_exact(&mut b).map_err(|e| format!("сервер оборвал соединение: {e}"))?;
    Ok(b)
}

fn socks5_connect(s: &mut TcpStream, p: &Profile) -> Result<(), String> {
    let auth = !p.username.is_empty();
    s.write_all(if auth { &[5, 2, 0, 2] } else { &[5, 1, 0] }).map_err(|e| e.to_string())?;
    let r = read_exact_t(s, 2)?;
    if r[0] != 5 {
        return Err("это не SOCKS5-сервер".into());
    }
    match r[1] {
        0 => {}
        2 => {
            let (u, pw) = (p.username.as_bytes(), p.password.as_bytes());
            let mut m = vec![1, u.len() as u8];
            m.extend_from_slice(u);
            m.push(pw.len() as u8);
            m.extend_from_slice(pw);
            s.write_all(&m).map_err(|e| e.to_string())?;
            let a = read_exact_t(s, 2)?;
            if a[1] != 0 {
                return Err("неверный логин или пароль".into());
            }
        }
        0xFF => return Err("сервер требует авторизацию — укажите логин и пароль".into()),
        m => return Err(format!("неподдерживаемый метод авторизации {m}")),
    }
    let mut req = vec![5, 1, 0, 3, TEST_HOST.len() as u8];
    req.extend_from_slice(TEST_HOST.as_bytes());
    req.extend_from_slice(&443u16.to_be_bytes());
    s.write_all(&req).map_err(|e| e.to_string())?;
    let h = read_exact_t(s, 4)?;
    if h[1] != 0 {
        return Err(format!("прокси отказал в соединении (код {})", h[1]));
    }
    let skip = match h[3] {
        1 => 4,
        4 => 16,
        3 => read_exact_t(s, 1)?[0] as usize,
        _ => 0,
    };
    read_exact_t(s, skip + 2)?;
    Ok(())
}

fn http_connect(s: &mut TcpStream, p: &Profile) -> Result<(), String> {
    let mut req = format!("CONNECT {TEST_HOST}:443 HTTP/1.1\r\nHost: {TEST_HOST}:443\r\n");
    if !p.username.is_empty() {
        req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", b64(format!("{}:{}", p.username, p.password).as_bytes())));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let head = read_head(s)?;
    let code = status_code(&head);
    match code {
        200 => Ok(()),
        407 => Err("неверный логин или пароль (407)".into()),
        0 => Err("это не HTTP-прокси".into()),
        c => Err(format!("прокси ответил кодом {c}")),
    }
}

fn read_head(s: &mut impl Read) -> Result<String, String> {
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    while buf.len() < 8192 {
        match s.read(&mut b) {
            Ok(0) => break,
            Ok(_) => {
                buf.push(b[0]);
                if buf.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            Err(e) => return Err(format!("нет ответа: {e}")),
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn status_code(head: &str) -> u16 {
    head.lines()
        .next()
        .filter(|l| l.starts_with("HTTP/"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0)
}

/// Checks CONNECT, certificate-verified TLS and HTTPS through the proxy.
pub fn test(p: &Profile) -> Result<u128, String> {
    let addr = p.resolve()?;
    let t0 = Instant::now();
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(6)).map_err(|e| format!("нет соединения с {addr}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(8))).ok();
    s.set_write_timeout(Some(Duration::from_secs(8))).ok();
    match p.kind {
        ProxyKind::Socks5 => socks5_connect(&mut s, p)?,
        ProxyKind::Http => http_connect(&mut s, p)?,
    }
    let connector = native_tls::TlsConnector::new().map_err(|e| format!("TLS: {e}"))?;
    let mut s = connector.connect(TEST_HOST, s).map_err(|e| format!("HTTPS через прокси: {e}"))?;
    s.write_all(format!("GET /generate_204 HTTP/1.1\r\nHost: {TEST_HOST}\r\nConnection: close\r\n\r\n").as_bytes())
        .map_err(|e| e.to_string())?;
    let head = read_head(&mut s)?;
    let code = status_code(&head);
    if code == 204 || code == 200 {
        Ok(t0.elapsed().as_millis())
    } else if code == 0 {
        Err("через прокси не пришёл ответ от сайта".into())
    } else {
        Err(format!("сайт ответил кодом {code} через прокси"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lines() {
        let p = parse_line("socks5://us%40er:p%3Ass@proxy.example.com:1080").unwrap();
        assert_eq!((p.kind, p.host.as_str(), p.port.as_str(), p.username.as_str(), p.password.as_str()),
                   (ProxyKind::Socks5, "proxy.example.com", "1080", "us@er", "p:ss"));
        let p = parse_line("http://1.2.3.4:3128").unwrap();
        assert_eq!((p.kind, p.host.as_str(), p.port.as_str()), (ProxyKind::Http, "1.2.3.4", "3128"));
        let p = parse_line("5.6.7.8:8000:login:pwd").unwrap();
        assert_eq!((p.username.as_str(), p.password.as_str()), ("login", "pwd"));
        let p = parse_line("user:pw@[2001:db8::1]:1080").unwrap();
        assert_eq!((p.host.as_str(), p.port.as_str(), p.username.as_str()), ("2001:db8::1", "1080", "user"));
        assert!(parse_line("vless://abc@host:443").is_none());
        assert!(parse_line("garbage").is_none());
    }

    #[test]
    fn tun_subnet_avoids_other_adapters() {
        let m30: Ipv4Addr = "255.255.255.252".parse().unwrap();
        let m16: Ipv4Addr = "255.255.0.0".parse().unwrap();
        // Another sing-box VPN on the old default subnet does not matter any more.
        let sota = ("172.19.0.1".parse().unwrap(), m30);
        assert_eq!(pick_tun_v4(&[sota]), Ipv4Addr::new(172, 29, 53, 1));
        // First candidate taken (/16 that contains it) -> next one.
        let busy = ("172.29.0.5".parse().unwrap(), m16);
        assert_eq!(pick_tun_v4(&[sota, busy]), Ipv4Addr::new(10, 233, 137, 1));
        // A neighbouring /30 in the same /24 does not overlap.
        let near = ("10.233.137.9".parse().unwrap(), m30);
        assert_eq!(pick_tun_v4(&[busy, near]), Ipv4Addr::new(10, 233, 137, 1));
    }

    #[test]
    fn writes_config() {
        let p = Profile { name: "t".into(), kind: ProxyKind::Socks5, host: "1.2.3.4".into(), port: "1080".into(),
                          username: "u".into(), password: "p".into() };
        let mut s = Settings::default();
        s.bypass_processes = "Discord.exe, steam.exe".into();
        s.bypass_domains = "gosuslugi.ru\n*.sber.ru".into();
        let v = singbox_config(&p, "1.2.3.4:1080".parse().unwrap(), &s, pick_tun_v4(&[]));
        let out = std::env::var("SB_TEST_OUT").unwrap_or_else(|_| std::env::temp_dir().join("sb-test.json").display().to_string());
        std::fs::write(&out, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        let mut h = p.clone(); h.kind = ProxyKind::Http;
        let v2 = singbox_config(&h, "1.2.3.4:3128".parse().unwrap(), &Settings::default(), pick_tun_v4(&[]));
        std::fs::write(format!("{out}.http.json"), serde_json::to_string_pretty(&v2).unwrap()).unwrap();
    }

    #[test]
    fn http_rejects_unsupported_udp_even_when_quic_toggle_is_off() {
        let p = Profile { kind: ProxyKind::Http, ..Profile::default() };
        let s = Settings { block_quic: false, bypass_processes: "Discord.exe".into(),
            bypass_domains: "example.org".into(), ..Settings::default() };
        let v = singbox_config(&p, "192.0.2.1:8000".parse().unwrap(), &s, pick_tun_v4(&[]));
        let rules = v["route"]["rules"].as_array().unwrap();
        let udp = rules.iter().position(|r| r["network"] == "udp" && r["action"] == "reject").unwrap();
        assert!(rules[..udp].iter().any(|r| r["action"] == "hijack-dns"));
        assert!(rules[..udp].iter().any(|r| r["process_name"][0] == "Discord.exe"));
        assert!(rules[..udp].iter().any(|r| r["domain_suffix"][0] == "example.org"));
        assert!(rules[udp].get("port").is_none(), "HTTP must reject UDP on every port");
        assert_eq!(v["dns"]["strategy"], "ipv4_only");
        assert!(rules.iter().any(|r| r["ip_version"] == 6 && r["action"] == "reject"));
    }

    #[test]
    fn socks_preserves_udp_and_ipv6_when_enabled() {
        let s = Settings { block_quic: false, ipv6: true, ..Settings::default() };
        let v = singbox_config(&Profile::default(), "192.0.2.1:1080".parse().unwrap(), &s, pick_tun_v4(&[]));
        assert_eq!(v["dns"]["strategy"], "prefer_ipv4");
        assert!(!v["route"]["rules"].as_array().unwrap().iter().any(|r| r["action"] == "reject"));
    }

    #[test]
    fn http_probe_uses_https_port_and_reports_auth_failure() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let head = read_head(&mut s).unwrap();
            assert!(head.starts_with("CONNECT cp.cloudflare.com:443 HTTP/1.1\r\n"));
            assert!(head.contains("Proxy-Authorization: Basic dTpw\r\n"));
            s.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n").unwrap();
        });
        let mut socket = TcpStream::connect(addr).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let p = Profile { username: "u".into(), password: "p".into(), ..Profile::default() };
        assert!(http_connect(&mut socket, &p).unwrap_err().contains("407"));
        server.join().unwrap();
    }

    #[test]
    #[ignore = "uses the local saved profile and the network; run explicitly"]
    fn live_profile_https() {
        let file = std::env::var("ZAPRET_LIVE_PROFILE").expect("set path to local settings");
        let settings: Settings = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        let ms = test(settings.current().expect("selected profile")).unwrap();
        println!("Saved proxy: verified HTTPS response in {ms} ms");
    }

    #[test]
    #[ignore = "runs the installed sing-box with loopback-only test listeners"]
    fn live_engine_https_and_dns() {
        let file = std::env::var("ZAPRET_LIVE_PROFILE").expect("set path to local settings");
        let mut settings: Settings = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        settings.ipv6 = false;
        let p = settings.current().unwrap();
        let mut config = singbox_config(p, p.resolve().unwrap(), &settings, pick_tun_v4(&[]));
        // This test changes no system routes. Follow the user's current route,
        // including an existing VPN; physical-interface isolation needs a real TUN test.
        config["route"]["auto_detect_interface"] = json!(false);
        let log_path = std::env::temp_dir().join(format!("zapret-engine-test-{}.log", std::process::id()));
        config["log"] = json!({"level":"debug", "output":log_path});
        let mixed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dns = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mixed_port = mixed.local_addr().unwrap().port();
        let dns_port = dns.local_addr().unwrap().port();
        config["inbounds"] = json!([
            {"type":"mixed", "tag":"test-proxy", "listen":"127.0.0.1", "listen_port":mixed_port},
            {"type":"direct", "tag":"test-dns", "listen":"127.0.0.1", "listen_port":dns_port,
             "override_address":"1.1.1.1", "override_port":53}
        ]);
        struct Engine(std::process::Child, std::path::PathBuf);
        impl Drop for Engine {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
                let _ = std::fs::remove_file(&self.1);
            }
        }
        let path = std::env::temp_dir().join(format!("zapret-engine-test-{}.json", std::process::id()));
        std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        drop((mixed, dns));
        let child = sys::hidden(std::env::var("ZAPRET_LIVE_ENGINE").expect("set installed sing-box path"))
            .args(["run", "-c"]).arg(&path).spawn().unwrap();
        let _engine = Engine(child, path);
        let addr = SocketAddr::from(([127, 0, 0, 1], mixed_port));
        let mut ready = false;
        for _ in 0..50 {
            if TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok() { ready = true; break; }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(ready, "sing-box test listener did not start");
        let local = Profile { host: "127.0.0.1".into(), port: mixed_port.to_string(), ..Profile::default() };
        let result = test(&local);
        if result.is_err() { println!("{}", std::fs::read_to_string(&log_path).unwrap_or_default()); }
        println!("HTTPS through sing-box and saved proxy: {} ms", result.unwrap());
        for qtype in [1u16, 28] {
            // TCP DNS query for example.com, through the generated DNS hijack and DoH detour.
            let mut query = vec![0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0,
                7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0];
            query.extend(qtype.to_be_bytes());
            query.extend(1u16.to_be_bytes());
            let mut socket = TcpStream::connect(("127.0.0.1", dns_port)).unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
            socket.write_all(&(query.len() as u16).to_be_bytes()).unwrap();
            socket.write_all(&query).unwrap();
            let length = read_exact_t(&mut socket, 2).unwrap();
            let reply = read_exact_t(&mut socket, u16::from_be_bytes([length[0], length[1]]) as usize).unwrap();
            assert_eq!(&reply[..2], &[0x12, 0x34]);
            assert_eq!(reply[3] & 15, 0, "DNS error response");
            let answers = u16::from_be_bytes([reply[6], reply[7]]);
            if qtype == 1 { assert!(answers > 0, "DoH must return IPv4 addresses"); }
            else { assert_eq!(answers, 0, "IPv4 mode must suppress AAAA answers"); }
            println!("DNS type {qtype}: {answers} answers, success");
        }
    }
}

//! Strategy .bat parsing and winws.exe helpers (mirrors what service.bat does).

use std::fs;
use std::path::Path;

use crate::{paths, sys};

// ---------------------------------------------------------------- game filter

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameMode {
    Off,
    All,
    Tcp,
    Udp,
}

impl GameMode {
    pub const ALL: [GameMode; 4] = [GameMode::Off, GameMode::All, GameMode::Tcp, GameMode::Udp];

    pub fn label(self) -> &'static str {
        match self {
            GameMode::Off => "выключен",
            GameMode::All => "TCP и UDP",
            GameMode::Tcp => "только TCP",
            GameMode::Udp => "только UDP",
        }
    }

    pub fn read() -> Self {
        GameFilter::read().mode
    }

    pub fn write(self) -> std::io::Result<()> {
        let path = paths::game_flag();
        let existing = match fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if let Some(text) = existing.as_deref().filter(|text| text.contains('=')) {
            let filter = GameFilter::parse(text);
            return fs::write(path, format!("mode={}\r\ntcp={}\r\nudp={}\r\n", self.key(), filter.tcp, filter.udp));
        }
        if self == Self::Off {
            return if existing.is_some() { fs::remove_file(path) } else { Ok(()) };
        }
        fs::create_dir_all(paths::utils())?;
        fs::write(path, format!("{}\r\n", self.key()))
    }

    fn key(self) -> &'static str {
        match self {
            Self::Off => "disabled",
            Self::All => "all",
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }

    fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "all" => Self::All,
            "tcp" => Self::Tcp,
            "udp" => Self::Udp,
            _ => Self::Off,
        }
    }
}

struct GameFilter {
    mode: GameMode,
    tcp: String,
    udp: String,
}

impl GameFilter {
    fn defaults(mode: GameMode) -> Self {
        Self {
            mode,
            tcp: "1024-65535".into(),
            udp: "1024-65535".into(),
        }
    }

    fn read() -> Self {
        match fs::read_to_string(paths::game_flag()) {
            Ok(text) => Self::parse(&text),
            Err(_) => Self::defaults(GameMode::Off),
        }
    }

    fn parse(text: &str) -> Self {
        let text = text.trim_start_matches('\u{feff}');
        if !text.contains('=') {
            let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
            let mode = match GameMode::parse(first) {
                GameMode::Off => GameMode::Udp, // Legacy presence flag, including an empty file.
                mode => mode,
            };
            return Self::defaults(mode);
        }
        let mut filter = Self::defaults(GameMode::Off);
        for line in text.lines().map(str::trim) {
            match line.split_once('=') {
                Some((key, value)) => match key.trim().to_ascii_lowercase().as_str() {
                    "mode" => filter.mode = GameMode::parse(value.trim()),
                    "tcp" => filter.tcp = Self::port_range(value),
                    "udp" => filter.udp = Self::port_range(value),
                    _ => {}
                },
                None => {
                    let mode = GameMode::parse(line);
                    if mode != GameMode::Off {
                        filter.mode = mode;
                    }
                }
            }
        }
        filter
    }

    fn port_range(value: &str) -> String {
        fn port(s: &str) -> Option<u16> {
            if s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            s.parse().ok()
        }

        let value = value.replace(' ', "");
        if value.split(',').all(|item| {
            let (first, last) = item.split_once('-').unwrap_or((item, item));
            matches!((port(first), port(last)), (Some(first), Some(last)) if first <= last)
        }) {
            value
        } else {
            "1024-65535".into()
        }
    }

    fn expand(&self, args: &str) -> String {
        let (tcp, udp, legacy) = match self.mode {
            GameMode::Off => ("12", "12", "12"),
            GameMode::All => (self.tcp.as_str(), self.udp.as_str(), self.tcp.as_str()),
            GameMode::Tcp => (self.tcp.as_str(), "12", self.tcp.as_str()),
            GameMode::Udp => ("12", self.udp.as_str(), self.udp.as_str()),
        };
        let args = replace_ci(args, "%GameFilterTCP%", tcp);
        let args = replace_ci(&args, "%GameFilterUDP%", udp);
        replace_ci(&args, "%GameFilter%", legacy)
    }
}

#[cfg(test)]
mod game_filter_tests {
    use super::{add_call_of_duty_hosts, GameFilter};

    const PLACEHOLDERS: &str = "%GameFilterTCP% %GameFilterUDP% %GameFilter%";

    #[test]
    fn legacy_game_filter_modes_still_expand() {
        for (flag, expanded) in [
            ("", "12 1024-65535 1024-65535"),
            ("udp\r\n", "12 1024-65535 1024-65535"),
            ("tcp", "1024-65535 12 1024-65535"),
            ("all", "1024-65535 1024-65535 1024-65535"),
        ] {
            assert_eq!(GameFilter::parse(flag).expand(PLACEHOLDERS), expanded);
        }
    }

    #[test]
    fn service_bat_ranges_expand_for_each_mode() {
        for (mode, expanded) in [
            ("disabled", "12 12 12"),
            ("all", "3074,27015 3074,27031-27036 3074,27015"),
            ("tcp", "3074,27015 12 3074,27015"),
            ("udp", "12 3074,27031-27036 3074,27031-27036"),
        ] {
            let flag = format!("mode={mode}\r\ntcp=3074, 27015\r\nudp=3074,27031-27036\r\n");
            assert_eq!(GameFilter::parse(&flag).expand(PLACEHOLDERS), expanded);
        }
    }

    #[test]
    fn invalid_ranges_fall_back_independently() {
        let filter = GameFilter::parse("mode=all\ntcp=3074,27036-27031\nudp=3074");
        assert_eq!(filter.expand(PLACEHOLDERS), "1024-65535 3074 1024-65535");
    }

    #[test]
    fn call_of_duty_domains_extend_only_general_hostlist_rules() {
        let base = r#"--filter-udp=443 --hostlist="%LISTS%list-general.txt" --new --filter-l7=discord --new --filter-tcp=443 --hostlist="%LISTS%list-general.txt""#;
        let args = add_call_of_duty_hosts(base).unwrap();
        assert_eq!(args.matches("--hostlist-domains=callofduty.com,demonware.net,s.activision.com").count(), 2);
        assert!(args.contains("--new --filter-l7=discord --new"));
        assert!(add_call_of_duty_hosts("--filter-l7=discord").is_err());
    }

    #[test]
    #[ignore = "uses the installed Flowseal strategy"]
    fn installed_general_alt_supports_call_of_duty_preset() {
        let root = std::path::PathBuf::from(std::env::var_os("ZAPRET_COD_TEST_ROOT").unwrap());
        crate::paths::set_root(root.clone(), root.clone());
        let bat = root.join("general (ALT).bat");
        let normal = super::build_args(&bat, false).unwrap();
        let cod = super::build_args(&bat, true).unwrap();
        assert!(!normal.contains("--hostlist-domains=callofduty.com"));
        assert!(cod.contains("--hostlist-domains=callofduty.com,demonware.net,s.activision.com"));
        assert!(cod.contains("--filter-l7=discord,stun"));
        assert!(cod.contains("--filter-udp=12 "));
    }
}


// ----------------------------------------------------------------- bat parser

fn replace_ci(s: &str, from: &str, to: &str) -> String {
    // ASCII lowercasing keeps byte offsets identical, so indices map back to `s`.
    let ls = s.to_ascii_lowercase();
    let lf = from.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(pos) = ls[i..].find(&lf) {
        out.push_str(&s[i..i + pos]);
        out.push_str(to);
        i += pos + lf.len();
    }
    out.push_str(&s[i..]);
    out
}

fn leftover_var(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let mut j = i + 1;
            if j < b.len() && (b[j].is_ascii_alphabetic() || b[j] == b'_') {
                j += 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                if j < b.len() && b[j] == b'%' {
                    return Some(s[i..=j].to_string());
                }
            }
        }
        i += 1;
    }
    None
}

/// Extracts the winws.exe command line from a strategy .bat and expands
/// %BIN%, %LISTS%, %~dp0 and the %GameFilterTCP%/%GameFilterUDP% placeholders.
pub const CALL_OF_DUTY: &str = "Call of Duty";
pub const CALL_OF_DUTY_HOSTLIST: &str = include_str!("../lists/list-call-of-duty.txt");
const GENERAL_HOSTLIST_ARG: &str = r#"--hostlist="%LISTS%list-general.txt""#;

pub fn call_of_duty_base(strategies: &[String]) -> Option<&str> {
    ["general (ALT).bat", "general.bat"]
        .iter()
        .find_map(|base| strategies.iter().find(|name| name.eq_ignore_ascii_case(base)).map(String::as_str))
}

pub fn build_args(bat: &Path, call_of_duty: bool) -> Result<String, String> {
    let bytes = fs::read(bat).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.to_ascii_lowercase().contains("winws.exe"))
        .ok_or("В файле не найдена команда запуска winws.exe.")?;

    let mut cmd = String::new();
    for l in &lines[start..] {
        let t = l.trim_end();
        let (t, cont) = match t.strip_suffix('^') {
            Some(x) => (x, true),
            None => (t, false),
        };
        cmd.push_str(t);
        cmd.push(' ');
        if !cont {
            break;
        }
    }

    let lower = cmd.to_ascii_lowercase();
    let mut p = lower.find("winws.exe").unwrap() + "winws.exe".len();
    if cmd.as_bytes().get(p) == Some(&b'"') {
        p += 1;
    }
    let mut args = cmd[p..].trim().to_string();
    if args.is_empty() {
        return Err("Аргументы winws.exe пусты.".into());
    }

    if call_of_duty {
        args = add_call_of_duty_hosts(&args)?;
    }
    let root = paths::win(paths::root());
    args = replace_ci(&args, "%BIN%", &format!("{}\\", paths::win(&paths::bin())));
    args = replace_ci(&args, "%LISTS%", &format!("{}\\", paths::win(&paths::lists())));
    args = replace_ci(&args, "%~dp0", &format!("{root}\\"));
    let filter = if call_of_duty { GameFilter::defaults(GameMode::Off) } else { GameFilter::read() };
    args = filter.expand(&args);

    if let Some(v) = leftover_var(&args) {
        return Err(format!("В стратегии осталась нераспознанная переменная {v}."));
    }
    Ok(args.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn add_call_of_duty_hosts(args: &str) -> Result<String, String> {
    if !args.to_ascii_lowercase().contains(&GENERAL_HOSTLIST_ARG.to_ascii_lowercase()) {
        return Err("В базовой стратегии нет списка list-general.txt для подключения доменов Call of Duty.".into());
    }
    let domains = CALL_OF_DUTY_HOSTLIST.lines().map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>().join(",");
    let argument = format!("{GENERAL_HOSTLIST_ARG} --hostlist-domains={domains}");
    Ok(replace_ci(args, GENERAL_HOSTLIST_ARG, &argument))
}

// ------------------------------------------------------------------ strategies

fn natural_key(s: &str) -> String {
    let mut out = String::new();
    let mut digits = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            if !digits.is_empty() {
                out.push_str(&format!("{digits:0>8}"));
                digits.clear();
            }
            out.extend(c.to_lowercase());
        }
    }
    if !digits.is_empty() {
        out.push_str(&format!("{digits:0>8}"));
    }
    out
}

pub fn list_strategies() -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(paths::root())
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| {
                    let l = n.to_ascii_lowercase();
                    l.ends_with(".bat") && !l.starts_with("service")
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|a| natural_key(a));
    v
}

pub fn strip_bat(s: &str) -> &str {
    if s.to_ascii_lowercase().ends_with(".bat") {
        &s[..s.len() - 4]
    } else {
        s
    }
}

pub fn ensure_user_lists() {
    let l = paths::lists();
    let _ = fs::create_dir_all(&l);
    let defaults = [
        ("ipset-exclude-user.txt", "203.0.113.113/32\r\n"),
        ("list-general-user.txt", "# Never leave this file empty\r\ndomain.example.abc\r\n"),
        ("list-exclude-user.txt", "domain.example.abc\r\n"),
    ];
    for (name, body) in defaults {
        let f = l.join(name);
        if !f.exists() {
            let _ = fs::write(f, body);
        }
    }
}

pub fn enable_tcp_timestamps() {
    sys::run(&sys::system32("netsh.exe"), "interface tcp set global timestamps=enabled");
}

pub fn tcp_timestamps_enabled() -> bool {
    let (_, out) = sys::run(&sys::system32("netsh.exe"), "interface tcp show global");
    out.lines().any(|l| {
        let l = l.to_lowercase();
        (l.contains("timestamps") || l.contains("метки")) && (l.contains("enabled") || l.contains("включ"))
    })
}

pub fn local_version() -> String {
    if let Ok(v) = fs::read_to_string(paths::version_txt()) {
        let v = v.trim();
        if !v.is_empty() {
            return v.to_string();
        }
    }
    if let Ok(b) = fs::read(paths::service_bat()) {
        let t = String::from_utf8_lossy(&b);
        let l = t.to_ascii_lowercase();
        if let Some(i) = l.find("set \"local_version=") {
            let rest = &t[i + "set \"local_version=".len()..];
            if let Some(end) = rest.find('"') {
                return rest[..end].trim().to_string();
            }
        }
    }
    "?".into()
}

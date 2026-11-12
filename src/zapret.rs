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
        let Ok(s) = fs::read_to_string(paths::game_flag()) else { return GameMode::Off };
        let first = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
        match first.to_ascii_lowercase().as_str() {
            "all" => GameMode::All,
            "tcp" => GameMode::Tcp,
            // service.bat falls back to UDP for any other content
            _ => GameMode::Udp,
        }
    }

    pub fn write(self) -> std::io::Result<()> {
        let f = paths::game_flag();
        if self == GameMode::Off {
            if f.exists() {
                fs::remove_file(f)?;
            }
            return Ok(());
        }
        fs::create_dir_all(paths::utils())?;
        let v = match self {
            GameMode::All => "all",
            GameMode::Tcp => "tcp",
            _ => "udp",
        };
        fs::write(f, format!("{v}\r\n"))
    }

    /// Mirrors :game_switch_status in service.bat
    pub fn ports(self) -> (&'static str, &'static str) {
        match self {
            GameMode::All => ("1024-65535", "1024-65535"),
            GameMode::Tcp => ("1024-65535", "12"),
            GameMode::Udp => ("12", "1024-65535"),
            GameMode::Off => ("12", "12"),
        }
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
pub fn build_args(bat: &Path, mode: GameMode) -> Result<String, String> {
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

    let (tcp, udp) = mode.ports();
    let root = paths::win(paths::root());
    args = replace_ci(&args, "%BIN%", &format!("{}\\", paths::win(&paths::bin())));
    args = replace_ci(&args, "%LISTS%", &format!("{}\\", paths::win(&paths::lists())));
    args = replace_ci(&args, "%~dp0", &format!("{root}\\"));
    args = replace_ci(&args, "%GameFilterTCP%", tcp);
    args = replace_ci(&args, "%GameFilterUDP%", udp);
    args = replace_ci(&args, "%GameFilter%", if mode == GameMode::Off { "12" } else { "1024-65535" });

    if let Some(v) = leftover_var(&args) {
        return Err(format!("В стратегии осталась нераспознанная переменная {v}."));
    }
    Ok(args.split_whitespace().collect::<Vec<_>>().join(" "))
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

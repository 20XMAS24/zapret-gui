//! TgWsProxy: process control, its %APPDATA%\TgWsProxy\config.json, tray icon visibility.

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::{paths, sys};

pub type Conf = Map<String, Value>;

pub fn is_running_in(procs: &[sys::Proc]) -> bool {
    procs.iter().any(|p| p.name.to_ascii_lowercase().starts_with("tgwsproxy"))
}

pub fn is_running() -> bool {
    is_running_in(&sys::processes())
}

pub fn start() -> Result<(), String> {
    let exe = paths::find_tgwsproxy().ok_or("TgWsProxy_windows.exe не найден в папке.")?;
    force_managed_by_gui();
    std::process::Command::new(&exe)
        .current_dir(paths::root())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn stop() {
    sys::kill_where(|p| p.name.to_ascii_lowercase().starts_with("tgwsproxy"));
}

pub fn local_file_version() -> Option<String> {
    paths::find_tgwsproxy().and_then(|p| sys::file_version(&p))
}

// --------------------------------------------------------------- config.json

const KEY_ORDER: &[&str] = &[
    "host", "port", "secret", "dc_ip", "cfproxy", "cfproxy_user_domain", "cfproxy_worker_domain",
    "force_test_dc", "buf_kb", "pool_size", "verbose", "log_max_mb", "check_updates", "autostart",
    "appearance",
];

pub fn dir() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(appdata).join("TgWsProxy")
}

pub fn file() -> PathBuf {
    dir().join("config.json")
}

pub fn defaults() -> Conf {
    let v = json!({
        "host": "127.0.0.1",
        "port": 1443,
        "secret": "",
        "dc_ip": ["2:149.154.167.220", "4:149.154.167.220"],
        "cfproxy": true,
        "cfproxy_user_domain": "",
        "cfproxy_worker_domain": "",
        "force_test_dc": false,
        "buf_kb": 256,
        "pool_size": 4,
        "verbose": false,
        "log_max_mb": 5.0,
        "check_updates": true,
        "autostart": false,
        "appearance": "auto"
    });
    v.as_object().cloned().unwrap_or_default()
}

/// Reads config.json as-is (unknown keys preserved). A missing or broken file gives defaults.
pub fn load() -> Conf {
    fs::read(file())
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_else(defaults)
}

pub fn save(c: &Conf) -> Result<(), String> {
    fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let mut ordered = Map::new();
    for k in KEY_ORDER {
        if let Some(v) = c.get(*k) {
            ordered.insert(k.to_string(), v.clone());
        }
    }
    for (k, v) in c {
        if !ordered.contains_key(k) {
            ordered.insert(k.clone(), v.clone());
        }
    }
    let txt = serde_json::to_string_pretty(&Value::Object(ordered)).map_err(|e| e.to_string())?;
    fs::write(file(), txt + "\n").map_err(|e| e.to_string())
}

pub fn get_str(c: &Conf, key: &str) -> String {
    match c.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => defaults().get(key).and_then(|v| v.as_str()).unwrap_or("").to_string(),
        Some(v) => v.to_string(),
    }
}

pub fn get_num(c: &Conf, key: &str) -> f64 {
    c.get(key)
        .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .or_else(|| defaults().get(key).and_then(|v| v.as_f64()))
        .unwrap_or(0.0)
}

pub fn get_bool(c: &Conf, key: &str) -> bool {
    c.get(key)
        .and_then(|v| v.as_bool())
        .or_else(|| defaults().get(key).and_then(|v| v.as_bool()))
        .unwrap_or(false)
}

/// String or array-of-strings value shown as "a, b". Newer TgWsProxy keeps CF domains as arrays.
fn get_list_str(c: &Conf, key: &str) -> String {
    match c.get(key) {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "),
        _ => get_str(c, key),
    }
}

/// Writes back in the same shape the file already uses; also keeps `<flag>` in sync when present.
fn set_list_str(c: &mut Conf, key: &str, flag: &str, text: &str) {
    let items: Vec<String> = text.split([',', ';', '\n', ' ']).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
    let v = if matches!(c.get(key), Some(Value::Array(_))) {
        json!(items)
    } else {
        json!(items.join(","))
    };
    c.insert(key.into(), v);
    if c.contains_key(flag) {
        c.insert(flag.into(), json!(!items.is_empty()));
    }
}

pub fn dc_list(c: &Conf) -> String {
    match c.get("dc_ip") {
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
            .collect::<Vec<_>>()
            .join(", "),
        Some(Value::String(s)) => s.clone(),
        _ => "2:149.154.167.220, 4:149.154.167.220".into(),
    }
}

pub fn set_dc_list(c: &mut Conf, text: &str) {
    let a: Vec<Value> = text
        .split([',', ';', '\n', '\r'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| Value::String(s.into()))
        .collect();
    c.insert("dc_ip".into(), Value::Array(a));
}

/// ZapretGUI owns autostart and updates, so TgWsProxy must not do them itself.
pub fn force_managed_by_gui() {
    let mut c = load();
    let mut changed = false;
    for k in ["check_updates", "autostart"] {
        if c.get(k).and_then(|v| v.as_bool()) != Some(false) {
            c.insert(k.into(), Value::Bool(false));
            changed = true;
        }
    }
    if changed {
        let _ = save(&c);
    }
}

pub fn telegram_link() -> String {
    let c = load();
    let mut host = get_str(&c, "host");
    if host.is_empty() || host == "0.0.0.0" {
        host = "127.0.0.1".into();
    }
    let port = get_num(&c, "port") as i64;
    let secret = normalize_secret(&get_str(&c, "secret"));
    let mut s = format!("tg://proxy?server={host}&port={port}");
    if !secret.is_empty() {
        // TgWsProxy runs in "secure" (padded) mode unless no_secure is set: the client then
        // needs the secret with a "dd" prefix, otherwise Telegram cannot connect.
        s.push_str("&secret=");
        if !get_bool(&c, "no_secure") {
            s.push_str("dd");
        }
        s.push_str(&secret);
    }
    s
}

/// The 32-hex secret as TgWsProxy stores it: lowercased, a pasted "dd"/"ee" client prefix removed.
pub fn normalize_secret(s: &str) -> String {
    let s = s.trim().to_ascii_lowercase();
    if s.len() == 34 && (s.starts_with("dd") || s.starts_with("ee")) {
        return s[2..].to_string();
    }
    s
}

// ------------------------------------------------------- tray icon visibility
// Windows 11 keeps per-exe tray icon visibility in HKCU\Control Panel\NotifyIconSettings\<id>\IsPromoted.
// On Windows 10 the equivalent is an opaque blob that cannot be edited safely.

const TRAY_KEY: &str = "Control Panel\\NotifyIconSettings";

pub fn tray_supported() -> bool {
    sys::windows_build() >= 22000
}

pub fn tray_set_promoted(fragment: &str, promoted: bool) -> Result<usize, String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    if !tray_supported() {
        return Err("Скрытие значка через реестр работает только в Windows 11. В Windows 10 перетащите значок мышкой в область скрытых значков.".into());
    }
    let root = winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(TRAY_KEY, KEY_READ)
        .map_err(|_| "Раздел реестра NotifyIconSettings не найден.".to_string())?;
    let frag = fragment.to_ascii_lowercase();
    let mut changed = 0;
    for sub in root.enum_keys().flatten() {
        let Ok(k) = root.open_subkey_with_flags(&sub, KEY_READ | KEY_WRITE) else { continue };
        let Ok(path) = k.get_value::<String, _>("ExecutablePath") else { continue };
        if !path.to_ascii_lowercase().contains(&frag) {
            continue;
        }
        if k.set_value("IsPromoted", &(if promoted { 1u32 } else { 0u32 })).is_ok() {
            changed += 1;
        }
    }
    if changed == 0 {
        return Err("Значок TgWsProxy ещё не зарегистрирован в системе. Запустите TgWsProxy хотя бы раз и повторите.".into());
    }
    Ok(changed)
}

pub fn tray_is_hidden(fragment: &str) -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    let Ok(root) = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey(TRAY_KEY) else { return false };
    let frag = fragment.to_ascii_lowercase();
    for sub in root.enum_keys().flatten() {
        let Ok(k) = root.open_subkey(&sub) else { continue };
        let Ok(path) = k.get_value::<String, _>("ExecutablePath") else { continue };
        if path.to_ascii_lowercase().contains(&frag) {
            if let Ok(p) = k.get_value::<u32, _>("IsPromoted") {
                if p == 0 {
                    return true;
                }
            }
        }
    }
    false
}

// ------------------------------------------------------------ editable form

/// Flat, UI-editable copy of the TgWsProxy settings.
#[derive(Clone, PartialEq)]
pub struct Form {
    pub host: String,
    pub port: String,
    pub secret: String,
    pub dc: String,
    pub cfproxy: bool,
    pub cf_user: String,
    pub cf_worker: String,
    pub buf_kb: String,
    pub pool: String,
    pub log_mb: String,
    pub appearance: String,
    pub verbose: bool,
    pub test_dc: bool,
    pub hide_tray: bool,
}

impl Form {
    pub fn load() -> Self {
        let c = load();
        let num = |k: &str| {
            let n = get_num(&c, k);
            if n.fract() == 0.0 { format!("{}", n as i64) } else { format!("{n}") }
        };
        Form {
            host: get_str(&c, "host"),
            port: num("port"),
            secret: get_str(&c, "secret"),
            dc: dc_list(&c),
            cfproxy: get_bool(&c, "cfproxy"),
            cf_user: get_list_str(&c, "cfproxy_user_domain"),
            cf_worker: get_list_str(&c, "cfproxy_worker_domain"),
            buf_kb: num("buf_kb"),
            pool: num("pool_size"),
            log_mb: format!("{:.1}", get_num(&c, "log_max_mb")),
            appearance: get_str(&c, "appearance"),
            verbose: get_bool(&c, "verbose"),
            test_dc: get_bool(&c, "force_test_dc"),
            hide_tray: tray_is_hidden("TgWsProxy"),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let parse_int = |s: &str, name: &str, min: i64, max: i64| -> Result<i64, String> {
            let v: i64 = s.trim().parse().map_err(|_| format!("«{name}»: нужно целое число"))?;
            if v < min || v > max {
                return Err(format!("«{name}»: допустимо от {min} до {max}"));
            }
            Ok(v)
        };
        let port = parse_int(&self.port, "Порт", 1, 65535)?;
        let buf = parse_int(&self.buf_kb, "Размер буфера", 16, 8192)?;
        let pool = parse_int(&self.pool, "Пул соединений", 1, 64)?;
        let log_mb: f64 = self
            .log_mb
            .trim()
            .replace(',', ".")
            .parse()
            .map_err(|_| "«Макс. размер лога»: нужно число".to_string())?;

        let secret = normalize_secret(&self.secret);
        if !secret.is_empty() && (secret.len() != 32 || !secret.bytes().all(|b| b.is_ascii_hexdigit())) {
            return Err("«Секрет»: нужно 32 шестнадцатеричных символа (кнопка 🎲 сгенерирует)".into());
        }

        let mut c = load();
        c.insert("host".into(), json!(self.host.trim()));
        c.insert("port".into(), json!(port));
        c.insert("secret".into(), json!(secret));
        set_dc_list(&mut c, &self.dc);
        c.insert("cfproxy".into(), json!(self.cfproxy));
        set_list_str(&mut c, "cfproxy_user_domain", "cfproxy_user_domain_enabled", &self.cf_user);
        set_list_str(&mut c, "cfproxy_worker_domain", "cfproxy_worker_enabled", &self.cf_worker);
        c.insert("force_test_dc".into(), json!(self.test_dc));
        c.insert("buf_kb".into(), json!(buf));
        c.insert("pool_size".into(), json!(pool));
        c.insert("verbose".into(), json!(self.verbose));
        c.insert("log_max_mb".into(), json!(log_mb.clamp(0.5, 500.0)));
        c.insert("appearance".into(), json!(self.appearance));
        c.insert("check_updates".into(), json!(false));
        c.insert("autostart".into(), json!(false));
        save(&c)
    }
}

pub fn random_secret() -> String {
    sys::to_hex(&sys::random_bytes(16))
}

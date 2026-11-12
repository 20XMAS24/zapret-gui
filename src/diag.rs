//! Native re-implementation of service.bat's :service_diagnostics, Discord cache cleanup, autostart.

use std::path::PathBuf;

use crate::logbuf::Kind;
use crate::{paths, sys, zapret};

pub fn run(log: &dyn Fn(&str, Kind)) {
    log("── Диагностика ──────────────────────────────", Kind::Head);

    match sys::service_state("BFE") {
        Some(sys::SVC_RUNNING) => log("[OK] Base Filtering Engine работает", Kind::Ok),
        _ => log("[X] Base Filtering Engine не запущен — без него zapret и kill switch не работают", Kind::Error),
    }

    // System proxy
    {
        use winreg::enums::HKEY_CURRENT_USER;
        let k = winreg::RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings");
        match k {
            Ok(k) => {
                let en: u32 = k.get_value("ProxyEnable").unwrap_or(0);
                if en == 1 {
                    let srv: String = k.get_value("ProxyServer").unwrap_or_else(|_| "(не указан)".into());
                    log(&format!("[?] Включён системный прокси: {srv}"), Kind::Warn);
                    log("    Отключите его, если не используете", Kind::Warn);
                } else {
                    log("[OK] Системный прокси выключен", Kind::Ok);
                }
            }
            Err(_) => log("[?] Не удалось проверить системный прокси", Kind::Warn),
        }
    }

    if zapret::tcp_timestamps_enabled() {
        log("[OK] TCP timestamps включены", Kind::Ok);
    } else {
        log("[?] TCP timestamps выключены — включаю...", Kind::Warn);
        zapret::enable_tcp_timestamps();
        if zapret::tcp_timestamps_enabled() {
            log("[OK] TCP timestamps включены", Kind::Ok);
        } else {
            log("[X] Не удалось включить TCP timestamps", Kind::Error);
        }
    }

    let procs = sys::processes();
    let has_proc = |n: &str| procs.iter().any(|p| p.name.to_ascii_lowercase().trim_end_matches(".exe") == n.to_ascii_lowercase());

    if has_proc("AdguardSvc") {
        log("[X] Найден процесс Adguard — может ломать Discord", Kind::Error);
        log("    https://github.com/Flowseal/zapret-discord-youtube/issues/417", Kind::Dim);
    } else {
        log("[OK] Adguard не обнаружен", Kind::Ok);
    }

    let names = sys::service_names();
    let has = |kw: &str| names.iter().any(|n| n.to_lowercase().contains(&kw.to_lowercase()));

    if has("Killer") {
        log("[X] Killer конфликтует с zapret", Kind::Error);
    } else {
        log("[OK] Killer не обнаружен", Kind::Ok);
    }
    if names.iter().any(|n| {
        let l = n.to_lowercase();
        l.contains("intel") && l.contains("connectivity")
    }) {
        log("[X] Intel Connectivity Network Service конфликтует с zapret", Kind::Error);
    } else {
        log("[OK] Intel Connectivity не обнаружен", Kind::Ok);
    }
    if has("TracSrvWrapper") || has("EPWD") {
        log("[X] Найдены службы Check Point — конфликтуют с zapret", Kind::Error);
    } else {
        log("[OK] Check Point не обнаружен", Kind::Ok);
    }
    if has("SmartByte") {
        log("[X] SmartByte конфликтует с zapret", Kind::Error);
    } else {
        log("[OK] SmartByte не обнаружен", Kind::Ok);
    }

    let sys_found = std::fs::read_dir(paths::bin())
        .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().to_ascii_lowercase().ends_with(".sys")))
        .unwrap_or(false);
    if sys_found {
        log("[OK] Драйвер WinDivert на месте", Kind::Ok);
    } else {
        log("[X] Файл WinDivert64.sys не найден в bin\\ (антивирус мог его удалить)", Kind::Error);
    }

    if paths::singbox().is_file() {
        log("[OK] sing-box.exe на месте (прокси-туннель доступен)", Kind::Ok);
    } else {
        log("[?] bin\\sing-box.exe не найден — прокси-туннель работать не будет", Kind::Warn);
    }

    let vpns: Vec<&String> = names.iter().filter(|n| n.to_uppercase().contains("VPN")).collect();
    if vpns.is_empty() {
        log("[OK] VPN-служб не обнаружено", Kind::Ok);
    } else {
        let list: Vec<&str> = vpns.iter().map(|s| s.as_str()).collect();
        log(&format!("[?] Найдены VPN-службы: {} — некоторые VPN конфликтуют с zapret", list.join(", ")), Kind::Warn);
    }

    let hosts = PathBuf::from(sys::system32("drivers\\etc\\hosts"));
    if let Ok(b) = std::fs::read(&hosts) {
        let t = String::from_utf8_lossy(&b).to_lowercase();
        if t.contains("youtube.com") || t.contains("youtu.be") {
            log("[?] В hosts есть записи для youtube.com / youtu.be — это может мешать доступу", Kind::Warn);
        } else {
            log("[OK] Файл hosts без записей YouTube", Kind::Ok);
        }
    }

    let winws = has_proc("winws");
    let wd = sys::service_state("WinDivert");
    if !winws && matches!(wd, Some(sys::SVC_RUNNING) | Some(sys::SVC_STOP_PENDING)) {
        log("[?] winws.exe не запущен, но служба WinDivert активна — её использует другой обход", Kind::Warn);
    }

    let found: Vec<&str> = ["GoodbyeDPI", "discordfix_zapret", "winws1", "winws2"]
        .into_iter()
        .filter(|c| sys::service_state(c).is_some())
        .collect();
    if found.is_empty() {
        log("[OK] Конфликтующих обходов не найдено", Kind::Ok);
    } else {
        log(&format!("[X] Найдены конфликтующие службы обхода: {}", found.join(", ")), Kind::Error);
    }

    if sys::service_state("zapret") == Some(sys::SVC_RUNNING) {
        log("[?] Служба zapret запущена — стратегия из неё работает независимо от этого окна", Kind::Warn);
    }

    log("── Диагностика завершена ────────────────────", Kind::Head);
}

pub fn clear_discord_cache(log: &dyn Fn(&str, Kind)) {
    let appdata = PathBuf::from(std::env::var("APPDATA").unwrap_or_default());
    let targets = [
        ("Discord.exe", "Discord", "discord"),
        ("DiscordPTB.exe", "Discord PTB", "discordptb"),
        ("DiscordCanary.exe", "Discord Canary", "discordcanary"),
        ("DiscordDevelopment.exe", "Discord Development", "discorddevelopment"),
    ];
    let mut any = false;
    for (exe, title, folder) in targets {
        let dir = appdata.join(folder);
        if !dir.is_dir() {
            continue;
        }
        any = true;
        if sys::kill_where(|p| p.name.eq_ignore_ascii_case(exe)) > 0 {
            log(&format!("{title} был запущен — закрыт"), Kind::Info);
        }
        for sub in ["Cache", "Code Cache", "GPUCache"] {
            let d = dir.join(sub);
            if !d.is_dir() {
                continue;
            }
            match std::fs::remove_dir_all(&d) {
                Ok(_) => log(&format!("Удалено: {}", d.display()), Kind::Ok),
                Err(e) => log(&format!("Не удалось удалить {}: {e}", d.display()), Kind::Warn),
            }
        }
    }
    if !any {
        log("Установленный Discord не найден", Kind::Warn);
    }
}

// ------------------------------------------------------------------ autostart
// Logon autostart via Task Scheduler: the app runs elevated, so a Run-key entry would hit UAC each logon.

const TASK: &str = "ZapretGUI";

pub fn autostart_enabled() -> bool {
    sys::run(&sys::system32("schtasks.exe"), &format!("/Query /TN \"{TASK}\"")).0 == 0
}

pub fn autostart_set(enable: bool) -> Result<(), String> {
    let out = if enable {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let args = format!(
            "/Create /F /TN \"{TASK}\" /SC ONLOGON /RL HIGHEST /TR \"\\\"{}\\\" -autostart\"",
            exe.display()
        );
        sys::run(&sys::system32("schtasks.exe"), &args).1
    } else {
        sys::run(&sys::system32("schtasks.exe"), &format!("/Delete /F /TN \"{TASK}\"")).1
    };
    if autostart_enabled() == enable {
        Ok(())
    } else {
        Err(out.trim().to_string())
    }
}

//! Connectivity report for reproducing Discord and Call of Duty failures.

use std::fs::{self, File};
use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{paths, sys, updater};

const MAX_TARGETS: usize = 12;
const DISCORD_GATEWAY: &str = "https://discord.com/api/v10/gateway";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Ip(IpAddr),
    Tcp(SocketAddr),
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ip(ip) => write!(f, "{ip}"),
            Self::Tcp(addr) => write!(f, "{addr}"),
        }
    }
}

pub fn parse_targets(input: &str) -> Result<Vec<Target>, String> {
    let mut targets = Vec::new();
    for item in input.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';')).filter(|s| !s.is_empty()) {
        let target = match item.parse::<IpAddr>() {
            Ok(ip) => Target::Ip(ip),
            Err(_) => {
                let addr = item.parse::<SocketAddr>().ok().filter(|addr| addr.port() != 0)
                    .ok_or_else(|| format!("Укажите IP или IP:TCP-порт. Проверьте «{item}»."))?;
                Target::Tcp(addr)
            }
        };
        if !targets.contains(&target) {
            targets.push(target);
            if targets.len() > MAX_TARGETS {
                return Err(format!("За один запуск можно проверить до {MAX_TARGETS} адресов."));
            }
        }
    }
    Ok(targets)
}

pub fn report_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(|dir| PathBuf::from(dir).join("ZapretGUI"))
        .unwrap_or_else(paths::cache_dir)
        .join("Diagnostics")
}

pub fn run(context: &str, input: &str) -> io::Result<PathBuf> {
    let folder = report_dir();
    fs::create_dir_all(&folder)?;
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let path = folder.join(format!("Call-of-Duty-{}-{unique}.txt", sys::now_stamp()));
    let mut report = File::create_new(&path)?;
    writeln!(report, "ZapretGUI {} | Диагностика Call of Duty и Discord", updater::APP_VERSION)?;
    writeln!(report, "{context}")?;
    writeln!(report, "Начало: {} (местное время)", sys::now_stamp())?;
    let discord_before = discord_process_count();
    writeln!(report, "Процессов Discord перед проверкой: {discord_before}")?;
    writeln!(report, "\n1. Discord — публичный HTTPS API Gateway")?;
    report.flush()?;
    let start = Instant::now();
    let discord = updater::get_string(DISCORD_GATEWAY).and_then(|body| check_gateway_response(&body));
    let elapsed = start.elapsed().as_millis();
    match discord {
        Ok(()) => {
            writeln!(report, "Доступен ({elapsed} мс). Вход в Discord и голос не проверялись.")?;
            writeln!(report, "\n2. Call of Duty — проверка IP")?;
            report.flush()?;
            check_targets(&mut report, input)?;
        }
        Err(error) => {
            writeln!(report, "Ошибка через {elapsed} мс: {error}")?;
            writeln!(report, "\n2. Call of Duty — пропущено после ошибки Discord.")?;
        }
    }
    let discord_after = discord_process_count();
    writeln!(report, "\nПроцессов Discord после проверки: {discord_after}")?;
    if discord_before > 0 && discord_after == 0 {
        writeln!(report, "Discord завершился во время проверки. Причину уточняйте в журнале приложений Windows.")?;
    }
    writeln!(report, "Конец: {}", sys::now_hms())?;
    report.flush()?;
    Ok(path)
}

fn check_gateway_response(body: &str) -> Result<(), String> {
    let payload: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| "Discord Gateway вернул ответ не в формате JSON.")?;
    let url = payload["url"].as_str().unwrap_or("");
    if url == "wss://gateway.discord.gg" || url.starts_with("wss://gateway.discord.gg/") {
        return Ok(());
    }
    Err("В ответе Discord Gateway отсутствует ожидаемый адрес.".into())
}

fn discord_process_count() -> usize {
    sys::processes().iter().filter(|p| {
        ["Discord.exe", "DiscordPTB.exe", "DiscordCanary.exe", "DiscordDevelopment.exe"]
            .iter().any(|name| p.name.eq_ignore_ascii_case(name))
    }).count()
}

fn check_targets(report: &mut File, input: &str) -> io::Result<()> {
    let mut targets = match parse_targets(input) {
        Ok(targets) => targets,
        Err(error) => return writeln!(report, "IP не проверены: {error}"),
    };
    if targets.is_empty() {
        targets = match discover_tcp_targets() {
            Ok(targets) if !targets.is_empty() => targets,
            Ok(_) => return writeln!(report, "IP не проверены: у запущенной игры нет обнаруженных TCP-соединений. Введите адреса вручную или повторите проверку с открытой игрой."),
            Err(error) => return writeln!(report, "IP не проверены: {error}"),
        };
        writeln!(report, "Источник: текущие TCP-соединения процесса Call of Duty.")?;
    } else {
        writeln!(report, "Источник: адреса, указанные пользователем.")?;
    }
    for target in targets {
        let ip = match target { Target::Ip(ip) => ip, Target::Tcp(addr) => addr.ip() };
        if let IpAddr::V4(ip) = ip {
            if let Some(route) = sys::route_iface(ip) {
                writeln!(report, "{target} — интерфейс Windows: {} ({})", route.alias, route.description)?;
            }
        }
        let start = Instant::now();
        match target {
            Target::Ip(ip) => writeln!(report, "{target} — ICMP: {}", ping(ip))?,
            Target::Tcp(addr) => match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
                Ok(_) => writeln!(report, "{target} — TCP: подключение за {} мс", start.elapsed().as_millis())?,
                Err(e) => writeln!(report, "{target} — TCP: {e}")?,
            },
        }
        report.flush()?;
    }
    writeln!(report, "Отсутствие ответа ICMP не подтверждает сбой игры. Игровой UDP и вход в матч не проверялись.")
}

fn powershell(script: &str) -> Result<String, String> {
    let output = sys::hidden(sys::system32("WindowsPowerShell\\v1.0\\powershell.exe"))
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("Сетевая команда Windows завершилась с кодом {}.", output.status));
    }
    Ok(sys::oem_decode(&output.stdout))
}

fn ping(ip: IpAddr) -> String {
    // The address comes from IpAddr, so it cannot inject PowerShell syntax.
    let script = format!(
        "$ErrorActionPreference='Stop'; $p=New-Object Net.NetworkInformation.Ping; try {{ $r=$p.Send([ipaddress]'{ip}', 1200); Write-Output \"$($r.Status) ($($r.RoundtripTime) ms)\" }} finally {{ $p.Dispose() }}"
    );
    powershell(&script).map(|output| output.trim().to_string()).unwrap_or_else(|e| e)
}

fn discover_tcp_targets() -> Result<Vec<Target>, String> {
    let script = r#"
        $game = @(Get-Process -Name cod,codhq,cod22,cod23,cod24,cod25,cod26 -ErrorAction SilentlyContinue)
        if ($game.Count) {
            Get-NetTCPConnection -OwningProcess $game.Id -State Established,SynSent -ErrorAction SilentlyContinue |
                ForEach-Object { '[{0}]:{1}' -f $_.RemoteAddress,$_.RemotePort }
        }
    "#;
    let output = powershell(script)?;
    let mut targets = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        // PowerShell brackets both families so the IPv6 case is unambiguous.
        let Some((ip, port)) = line.strip_prefix('[').and_then(|s| s.split_once("]:")) else { continue };
        let (Ok(ip), Ok(port)) = (ip.parse::<IpAddr>(), port.parse::<u16>()) else { continue };
        if ip.is_unspecified() || ip.is_loopback() || port == 0 {
            continue;
        }
        let target = Target::Tcp(SocketAddr::new(ip, port));
        if !targets.contains(&target) {
            targets.push(target);
            if targets.len() == MAX_TARGETS {
                break;
            }
        }
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_numeric_targets_and_distinguishes_icmp_from_tcp() {
        let targets = parse_targets("192.0.2.1, 192.0.2.1; 192.0.2.2:443\n2001:db8::1 [2001:db8::1]:443").unwrap();
        assert_eq!(targets.len(), 4);
        assert!(matches!(targets[0], Target::Ip(_)));
        assert!(matches!(targets[1], Target::Tcp(_)));
        assert!(matches!(targets[2], Target::Ip(_)));
        assert!(matches!(targets[3], Target::Tcp(_)));
        for invalid in ["example.com:443", "192.0.2.2:0", "https://192.0.2.1"] {
            assert!(parse_targets(invalid).is_err());
        }
    }

    #[test]
    fn accepts_current_and_documented_gateway_urls() {
        for url in ["wss://gateway.discord.gg", "wss://gateway.discord.gg/"] {
            assert!(check_gateway_response(&format!(r#"{{"url":"{url}"}}"#)).is_ok());
        }
        assert!(check_gateway_response(r#"{"url":"wss://gateway.discord.gg.example.com/"}"#).is_err());
        assert!(check_gateway_response("<html>blocked</html>").is_err());
    }

    #[test]
    #[ignore = "uses the public Discord API and loopback endpoints"]
    fn live_game_debug_report() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let input = format!("127.0.0.1, {}", listener.local_addr().unwrap());
        let path = run("Самопроверка с локальными IP, без изменения маршрутов.", &input).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        println!("{}", path.display());
        assert!(content.contains("Доступен ("), "{content}");
        assert!(content.contains("ICMP: Success"), "{content}");
        assert!(content.contains("TCP: подключение"), "{content}");
    }
}

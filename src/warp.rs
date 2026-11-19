//! Cloudflare One Client integration for the Call of Duty strategy.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::sys;

const TRAFFIC_AND_UDP_DNS: &str = "warp";

#[derive(Deserialize)]
struct Status {
    status: String,
}

#[derive(Deserialize)]
struct Settings {
    settings: OperationMode,
}

#[derive(Deserialize)]
struct OperationMode {
    operation_mode: String,
}

fn cli() -> Result<PathBuf, String> {
    ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|root| PathBuf::from(root).join(r"Cloudflare\Cloudflare WARP\warp-cli.exe"))
        .find(|path| path.is_file())
        .ok_or_else(|| "Установите Cloudflare One Client и завершите его первоначальную настройку.".into())
}

fn run(cli: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = sys::hidden(cli).args(args).output().map_err(|e| format!("Cloudflare WARP: {e}"))?;
    if !out.status.success() {
        let error = if out.stderr.is_empty() { out.stdout } else { out.stderr };
        return Err(format!("Cloudflare WARP ({}): {}", args.join(" "), String::from_utf8_lossy(&error).trim()));
    }
    Ok(out.stdout)
}

fn query<T: serde::de::DeserializeOwned>(cli: &Path, command: &str) -> Result<T, String> {
    serde_json::from_slice(&run(cli, &["--json", command])?)
        .map_err(|e| format!("Cloudflare WARP не вернул ожидаемые данные ({command}): {e}"))
}

/// Returns the CLI path when the preset needs to start WARP itself.
pub fn prepare() -> Result<Option<PathBuf>, String> {
    let cli = cli()?;
    let status: Status = query(&cli, "status")?;
    let settings: Settings = query(&cli, "settings")?;
    let mode = settings.settings.operation_mode;
    match (status.status.as_str(), mode.as_str()) {
        ("Connected", TRAFFIC_AND_UDP_DNS) => return Ok(None),
        ("Disconnected", _) => {}
        ("Connected", _) => return Err("Отключите Cloudflare One, чтобы профиль мог выбрать режим «Трафик и DNS (UDP)».".into()),
        (state, _) => return Err(format!("Cloudflare One сейчас меняет состояние ({state}). Повторите запуск после завершения.")),
    }
    if mode != TRAFFIC_AND_UDP_DNS {
        run(&cli, &["mode", TRAFFIC_AND_UDP_DNS])?;
        let settings: Settings = query(&cli, "settings")?;
        if settings.settings.operation_mode != TRAFFIC_AND_UDP_DNS {
            return Err("Cloudflare One не применил режим «Трафик и DNS (UDP)».".into());
        }
    }
    Ok(Some(cli))
}

pub fn connect(cli: &Path) -> Result<(), String> {
    run(cli, &["connect"])?;
    wait_for(cli, "Connected", Duration::from_secs(30))
}

pub fn disconnect(cli: &Path) -> Result<(), String> {
    let status: Status = query(cli, "status")?;
    if status.status != "Disconnected" {
        run(cli, &["disconnect"])?;
        wait_for(cli, "Disconnected", Duration::from_secs(10))?;
    }
    Ok(())
}

fn wait_for(cli: &Path, expected: &str, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        let status: Status = query(cli, "status")?;
        if status.status == expected {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            return Err(format!("Cloudflare One не перешёл в состояние {expected}. Текущее состояние: {}.", status.status));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the installed Cloudflare One Client; querying is read-only"]
    fn installed_warp_cli() {
        let cli = cli().unwrap();
        let status: Status = query(&cli, "status").unwrap();
        let settings: Settings = query(&cli, "settings").unwrap();
        assert!(!status.status.is_empty());
        assert!(!settings.settings.operation_mode.is_empty());
        let help = run(&cli, &["mode", "--help"]).unwrap();
        assert!(String::from_utf8_lossy(&help).contains("normal UDP DNS"));
    }
}

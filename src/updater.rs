//! Updates for zapret (Flowseal/zapret-discord-youtube) and TgWsProxy (Flowseal/tg-ws-proxy).

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::logbuf::Kind;
use crate::{paths, sys, tg};

const UA: &str = "ZapretGUI/2.0 (+https://github.com/Flowseal/zapret-discord-youtube)";

fn agent(read_timeout: u64) -> Result<ureq::Agent, String> {
    let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(read_timeout))
        .user_agent(UA)
        .build())
}

pub fn get_string(url: &str) -> Result<String, String> {
    agent(15)?
        .get(url)
        .set("Cache-Control", "no-cache")
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())
}

pub fn download(url: &str, dest: &Path, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    let resp = agent(120)?.get(url).call().map_err(|e| e.to_string())?;
    let total: u64 = resp.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut rd = resp.into_reader();
    let tmp = dest.with_extension("part");
    let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = rd.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress(done, total);
    }
    drop(f);
    if total > 0 && done < total {
        let _ = fs::remove_file(&tmp);
        return Err("загрузка оборвалась".into());
    }
    let _ = fs::remove_file(dest);
    fs::rename(&tmp, dest).map_err(|e| e.to_string())
}

// ------------------------------------------------------------------- zapret

const VERSION_URL: &str = "https://raw.githubusercontent.com/Flowseal/zapret-discord-youtube/main/.service/version.txt";
const RELEASE_FMT: &str = "https://github.com/Flowseal/zapret-discord-youtube/releases/download/{v}/zapret-discord-youtube-{v}.zip";

const PROTECTED: &[&str] = &[
    "lists\\list-general-user.txt",
    "lists\\list-exclude-user.txt",
    "lists\\ipset-exclude-user.txt",
    "lists\\ipset-all.txt",
    "lists\\ipset-all.txt.backup",
    "utils\\check_updates.enabled",
    "utils\\game_filter.enabled",
];

pub fn zapret_remote_version() -> Result<String, String> {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    Ok(get_string(&format!("{VERSION_URL}?t={t}"))?.trim().to_string())
}

/// true when remote is strictly newer than local (numeric compare per dot-separated part).
pub fn is_newer(local: &str, remote: &str) -> bool {
    if remote.is_empty() {
        return false;
    }
    if local.is_empty() || local == "?" {
        return true;
    }
    if local == remote {
        return false;
    }
    let num = |s: &str| -> u64 { s.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0) };
    let (a, b): (Vec<&str>, Vec<&str>) = (local.split('.').collect(), remote.split('.').collect());
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).map(|s| num(s)).unwrap_or(0);
        let y = b.get(i).map(|s| num(s)).unwrap_or(0);
        if y != x {
            return y > x;
        }
    }
    false
}

pub fn download_zapret(version: &str, progress: impl FnMut(u64, u64)) -> Result<PathBuf, String> {
    let url = RELEASE_FMT.replace("{v}", version);
    let dest = paths::cache_dir().join(format!("zapret-{version}.zip"));
    if fs::metadata(&dest).map(|m| m.len() > 100_000).unwrap_or(false) {
        return Ok(dest);
    }
    download(&url, &dest, progress)?;
    Ok(dest)
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)?.flatten() {
        let p = e.path();
        let t = dst.join(e.file_name());
        if p.is_dir() {
            copy_tree(&p, &t)?;
        } else {
            fs::copy(&p, &t)?;
        }
    }
    Ok(())
}

/// Copies *.bat, lists and utils into <root>\_backup\yyyyMMdd-HHmmss.
fn backup() -> std::io::Result<PathBuf> {
    let dir = paths::root().join("_backup").join(sys::now_stamp());
    fs::create_dir_all(&dir)?;
    for e in fs::read_dir(paths::root())?.flatten() {
        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
        if n.ends_with(".bat") && e.path().is_file() {
            fs::copy(e.path(), dir.join(e.file_name()))?;
        }
    }
    copy_tree(&paths::lists(), &dir.join("lists"))?;
    copy_tree(&paths::utils(), &dir.join("utils"))?;
    Ok(dir)
}

fn has_bat(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().to_ascii_lowercase().ends_with(".bat")))
        .unwrap_or(false)
}

fn copy_over(src: &Path, dst: &Path, rel: &str, copied: &mut u32, skipped: &mut u32, log: &dyn Fn(&str, Kind)) {
    let _ = fs::create_dir_all(dst);
    let Ok(rd) = fs::read_dir(src) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let rel_path = if rel.is_empty() { name.clone() } else { format!("{rel}\\{name}") };
        let target = dst.join(&name);
        if e.path().is_dir() {
            copy_over(&e.path(), &target, &rel_path, copied, skipped, log);
            continue;
        }
        let prot = PROTECTED.iter().any(|p| p.eq_ignore_ascii_case(&rel_path));
        if prot && target.exists() {
            *skipped += 1;
            continue;
        }
        match fs::copy(e.path(), &target) {
            Ok(_) => *copied += 1,
            Err(err) => log(&format!("Не удалось заменить {rel_path}: {err}"), Kind::Warn),
        }
    }
}

/// Extracts the release zip over the installation. winws must be stopped first.
pub fn apply_zapret(zip: &Path, version: &str, log: &dyn Fn(&str, Kind)) -> Result<(), String> {
    let work = paths::cache_dir().join(format!("extract-{version}"));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;

    log("Распаковка архива...", Kind::Info);
    let (code, out) = sys::run(&sys::system32("tar.exe"), &format!("-xf \"{}\" -C \"{}\"", zip.display(), work.display()));
    if code != 0 {
        return Err(format!("не удалось распаковать архив: {}", out.trim()));
    }

    // Release archives may wrap everything in zapret-discord-youtube-<ver>\
    let mut src = work.clone();
    if !has_bat(&src) {
        let subs: Vec<PathBuf> = fs::read_dir(&work)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default();
        if subs.len() == 1 {
            src = subs[0].clone();
        }
    }
    if !has_bat(&src) {
        return Err("В архиве не найдены файлы стратегий — обновление отменено.".into());
    }

    log("Резервная копия текущих файлов...", Kind::Info);
    let b = backup().map_err(|e| format!("резервная копия не создана: {e}"))?;
    log(&format!("Сохранено в {}", b.display()), Kind::Dim);

    let (mut copied, mut skipped) = (0, 0);
    copy_over(&src, paths::root(), "", &mut copied, &mut skipped, log);

    let _ = fs::create_dir_all(paths::root().join(".service"));
    let _ = fs::write(paths::version_txt(), version);
    log(&format!("Обновлено файлов: {copied}, сохранено пользовательских: {skipped}"), Kind::Ok);
    let _ = fs::remove_dir_all(&work);
    Ok(())
}

// ----------------------------------------------------------------- TgWsProxy

const TG_API: &str = "https://api.github.com/repos/Flowseal/tg-ws-proxy/releases/latest";

/// Returns (tag, asset url) of the latest TgWsProxy release.
pub fn tg_query() -> Result<(String, String), String> {
    let txt = get_string(TG_API)?;
    let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().ok_or("Не удалось разобрать ответ GitHub API.")?.to_string();
    let url = v["assets"]
        .as_array()
        .and_then(|a| {
            a.iter()
                .filter_map(|x| x["browser_download_url"].as_str())
                .find(|u| u.ends_with("TgWsProxy_windows.exe"))
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("https://github.com/Flowseal/tg-ws-proxy/releases/download/{tag}/TgWsProxy_windows.exe"));
    Ok((tag, url))
}

pub fn tg_download(url: &str, tag: &str) -> Result<PathBuf, String> {
    let safe: String = tag.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    let dest = paths::cache_dir().join(format!("TgWsProxy_windows-{safe}.exe"));
    if fs::metadata(&dest).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        return Ok(dest);
    }
    download(url, &dest, |_, _| {})?;
    Ok(dest)
}

pub fn tg_apply(downloaded: &Path) -> Result<PathBuf, String> {
    let target = paths::find_tgwsproxy().unwrap_or_else(|| paths::root().join("TgWsProxy_windows.exe"));
    tg::stop();
    std::thread::sleep(Duration::from_millis(600));
    if target.exists() {
        let old = target.with_extension("exe.old");
        let _ = fs::remove_file(&old);
        fs::rename(&target, &old).map_err(|e| format!("файл занят, закройте TgWsProxy: {e}"))?;
        let _ = fs::remove_file(&old);
    }
    fs::copy(downloaded, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

pub fn same_tag(a: &str, b: &str) -> bool {
    let n = |s: &str| s.trim().trim_start_matches(['v', 'V']).to_string();
    !a.is_empty() && !b.is_empty() && n(a) == n(b)
}

// ------------------------------------------------------------------ ZapretGUI

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// GitHub repository with ZapretGUI releases; can be overridden by `UpdateRepo=` in ZapretGUI.ini.
pub const APP_REPO: &str = match option_env!("ZAPRET_GUI_REPO") {
    Some(r) => r,
    None => "20XMAS24/zapret-gui",
};

pub struct AppRelease {
    pub version: String,
    pub setup_name: String,
    pub setup_url: String,
    pub sha_url: Option<String>,
}

/// Latest release of ZapretGUI itself: expects assets `ZapretGUI-Setup-<ver>.exe` and `<that>.sha256`.
pub fn app_query(repo: &str) -> Result<AppRelease, String> {
    let txt = get_string(&format!("https://api.github.com/repos/{repo}/releases/latest"))?;
    let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().ok_or("в репозитории ещё нет релизов")?;
    let version = tag.trim().trim_start_matches(['v', 'V']).to_string();
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let name_url = |pred: &dyn Fn(&str) -> bool| {
        assets.iter().find_map(|a| {
            let n = a["name"].as_str()?;
            if pred(n) {
                Some((n.to_string(), a["browser_download_url"].as_str()?.to_string()))
            } else {
                None
            }
        })
    };
    let (setup_name, setup_url) = name_url(&|n: &str| {
        let l = n.to_ascii_lowercase();
        l.starts_with("zapretgui-setup") && l.ends_with(".exe")
    })
    .ok_or("в релизе нет установщика ZapretGUI-Setup-*.exe")?;
    let sha_name = format!("{setup_name}.sha256");
    let sha_url = name_url(&|n: &str| n.eq_ignore_ascii_case(&sha_name)).map(|(_, u)| u);
    Ok(AppRelease { version, setup_name, setup_url, sha_url })
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    use windows_sys::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let mut out = [0u8; 32];
    let st = unsafe {
        BCryptHash(BCRYPT_SHA256_ALG_HANDLE, std::ptr::null(), 0, data.as_ptr(), data.len() as u32, out.as_mut_ptr(), 32)
    };
    if st != 0 {
        return Err(format!("BCryptHash: 0x{st:08x}"));
    }
    Ok(sys::to_hex(&out))
}

/// Downloads the installer and refuses it unless its SHA-256 matches the published checksum.
pub fn app_download(rel: &AppRelease, progress: impl FnMut(u64, u64)) -> Result<PathBuf, String> {
    let sha_url = rel.sha_url.as_ref().ok_or("в релизе нет файла контрольной суммы .sha256 — обновление небезопасно")?;
    let expected = get_string(sha_url)?
        .split_whitespace()
        .next()
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or("файл контрольной суммы повреждён")?;
    let dest = paths::cache_dir().join(&rel.setup_name);
    if !(dest.is_file() && sha256_file(&dest).map(|h| h == expected).unwrap_or(false)) {
        download(&rel.setup_url, &dest, progress)?;
    }
    let got = sha256_file(&dest)?;
    if got != expected {
        let _ = fs::remove_file(&dest);
        return Err(format!("контрольная сумма не совпала (ожидалась {expected}, получена {got}) — файл удалён"));
    }
    Ok(dest)
}

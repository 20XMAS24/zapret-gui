//! Locations inside the zapret folder.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static ROOT: OnceLock<PathBuf> = OnceLock::new();
static EXE_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_root(root: PathBuf, exe_dir: PathBuf) {
    let _ = ROOT.set(root);
    let _ = EXE_DIR.set(exe_dir);
}

pub fn root() -> &'static Path {
    ROOT.get().map(|p| p.as_path()).unwrap_or(Path::new("."))
}

/// Folder next to ZapretGUI.exe: settings live here.
pub fn exe_dir() -> &'static Path {
    EXE_DIR.get().map(|p| p.as_path()).unwrap_or(Path::new("."))
}

pub fn bin() -> PathBuf {
    root().join("bin")
}
pub fn lists() -> PathBuf {
    root().join("lists")
}
pub fn utils() -> PathBuf {
    root().join("utils")
}
pub fn winws() -> PathBuf {
    bin().join("winws.exe")
}
pub fn singbox() -> PathBuf {
    bin().join("sing-box.exe")
}
pub fn service_bat() -> PathBuf {
    root().join("service.bat")
}
pub fn game_flag() -> PathBuf {
    utils().join("game_filter.enabled")
}
pub fn tests_ps1() -> PathBuf {
    utils().join("test zapret.ps1")
}
pub fn version_txt() -> PathBuf {
    root().join(".service").join("version.txt")
}
pub fn proxy_json() -> PathBuf {
    exe_dir().join("ZapretGUI.proxy.json")
}

/// Scratch space for downloads and the generated sing-box config.
pub fn cache_dir() -> PathBuf {
    let d = std::env::temp_dir().join("ZapretGUI");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn looks_like_zapret(dir: &Path) -> bool {
    dir.join("bin").join("winws.exe").is_file()
}

pub fn find_tgwsproxy() -> Option<PathBuf> {
    let exact = root().join("TgWsProxy_windows.exe");
    if exact.is_file() {
        return Some(exact);
    }
    let rd = std::fs::read_dir(root()).ok()?;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with("tgwsproxy") && name.ends_with(".exe") {
            return Some(e.path());
        }
    }
    None
}

/// Windows paths for winws arguments: always backslashes, no `\\?\` prefix.
pub fn win(p: &Path) -> String {
    p.display().to_string()
}

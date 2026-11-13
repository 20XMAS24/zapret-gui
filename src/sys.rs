//! Thin wrappers over Win32: processes, services, shell, time, DPAPI, interfaces.

use std::ffi::c_void;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, INVALID_HANDLE_VALUE, SYSTEMTIME,
};
use windows_sys::Win32::Globalization::{GetOEMCP, MultiByteToWideChar};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceAliasToLuid, GetBestInterface, GetIfEntry2, GetIpAddrTable, MIB_IF_ROW2, MIB_IPADDRROW_XP,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus, SC_MANAGER_CONNECT,
    SERVICE_QUERY_STATUS, SERVICE_STATUS,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};
use windows_sys::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, IsIconic, MessageBoxW, SetForegroundWindow, ShowWindow, MB_ICONERROR,
    MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, SW_RESTORE, SW_SHOW, SW_SHOWNORMAL,
};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const SVC_STOP_PENDING: u32 = 3;
pub const SVC_RUNNING: u32 = 4;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(once(0)).collect()
}

pub fn wide_path(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(once(0)).collect()
}

/// Console tools (netsh, sc, schtasks, winws) print in the OEM code page.
pub fn oem_decode(b: &[u8]) -> String {
    if b.is_empty() {
        return String::new();
    }
    if let Ok(s) = std::str::from_utf8(b) {
        return s.to_string();
    }
    unsafe {
        let cp = GetOEMCP();
        let n = MultiByteToWideChar(cp, 0, b.as_ptr(), b.len() as i32, null_mut(), 0);
        if n <= 0 {
            return String::from_utf8_lossy(b).into_owned();
        }
        let mut buf = vec![0u16; n as usize];
        MultiByteToWideChar(cp, 0, b.as_ptr(), b.len() as i32, buf.as_mut_ptr(), n);
        String::from_utf16_lossy(&buf)
    }
}

/// A `Command` that never flashes a console window.
pub fn hidden(exe: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(exe);
    c.creation_flags(CREATE_NO_WINDOW).stdin(Stdio::null());
    c
}

/// Runs a console tool with a raw (already quoted) argument string, returns (exit code, output).
pub fn run(exe: &str, raw_args: &str) -> (i32, String) {
    let mut c = hidden(exe);
    c.raw_arg(raw_args);
    match c.output() {
        Ok(o) => {
            let mut s = oem_decode(&o.stdout);
            s.push_str(&oem_decode(&o.stderr));
            (o.status.code().unwrap_or(-1), s)
        }
        Err(e) => (-1, e.to_string()),
    }
}

pub fn system32(exe: &str) -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    format!("{root}\\System32\\{exe}")
}

// ------------------------------------------------------------------ processes

#[derive(Clone, Debug)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
}

pub fn processes() -> Vec<Proc> {
    let mut v = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return v;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                v.push(Proc { pid: e.th32ProcessID, name: String::from_utf16_lossy(&e.szExeFile[..len]) });
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    v
}

pub fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

pub fn kill_pid(pid: u32) {
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return;
        }
        TerminateProcess(h, 1);
        WaitForSingleObject(h, 4000);
        CloseHandle(h);
    }
}

pub fn kill_where(pred: impl Fn(&Proc) -> bool) -> usize {
    let me = std::process::id();
    let mut n = 0;
    for p in processes() {
        if p.pid != me && pred(&p) {
            kill_pid(p.pid);
            n += 1;
        }
    }
    n
}

// ------------------------------------------------------------------- services

/// Current state of a service (SERVICE_RUNNING = 4 ...), None when it does not exist.
pub fn service_state(name: &str) -> Option<u32> {
    unsafe {
        let scm = OpenSCManagerW(null(), null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            return None;
        }
        let w = wide(name);
        let s = OpenServiceW(scm, w.as_ptr(), SERVICE_QUERY_STATUS);
        if s.is_null() {
            CloseServiceHandle(scm);
            return None;
        }
        let mut st: SERVICE_STATUS = std::mem::zeroed();
        let ok = QueryServiceStatus(s, &mut st);
        CloseServiceHandle(s);
        CloseServiceHandle(scm);
        if ok != 0 {
            Some(st.dwCurrentState)
        } else {
            None
        }
    }
}

/// Names and display names of all Win32 services (keys in `sc` output are not localized).
pub fn service_names() -> Vec<String> {
    let (_, out) = run(&system32("sc.exe"), "query type= service state= all");
    let mut v = Vec::new();
    for line in out.lines() {
        let t = line.trim();
        for key in ["SERVICE_NAME:", "DISPLAY_NAME:"] {
            if let Some(rest) = t.strip_prefix(key) {
                v.push(rest.trim().to_string());
            }
        }
    }
    v
}

// ---------------------------------------------------------------------- shell

pub fn is_admin() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

pub fn elevate(args: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let dir = exe.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    let (verb, file, params, d) = (wide("runas"), wide_path(&exe), wide(args), wide_path(&dir));
    unsafe {
        let r = ShellExecuteW(null_mut(), verb.as_ptr(), file.as_ptr(), params.as_ptr(), d.as_ptr(), SW_SHOWNORMAL);
        r as isize > 32
    }
}

pub fn shell_open(target: &str) {
    let (verb, file) = (wide("open"), wide(target));
    unsafe {
        ShellExecuteW(null_mut(), verb.as_ptr(), file.as_ptr(), null(), null(), SW_SHOWNORMAL);
    }
}

pub fn msgbox(text: &str, error: bool) {
    let (t, c) = (wide(text), wide("ZAPRET GUI"));
    let icon = if error { MB_ICONERROR } else { MB_ICONWARNING };
    unsafe {
        MessageBoxW(null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | icon | MB_SETFOREGROUND);
    }
}

/// Returns false when another copy already owns the mutex. The handle is leaked on purpose.
pub fn single_instance(name: &str) -> bool {
    let w = wide(name);
    unsafe {
        let h = CreateMutexW(null(), 1, w.as_ptr());
        !(h.is_null() || GetLastError() == ERROR_ALREADY_EXISTS)
    }
}

pub fn find_window(title: &str) -> isize {
    let w = wide(title);
    unsafe { FindWindowW(null(), w.as_ptr()) as isize }
}

pub fn show_hwnd(hwnd: isize) {
    if hwnd == 0 {
        return;
    }
    let h = hwnd as *mut c_void;
    unsafe {
        ShowWindow(h, SW_SHOW);
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        }
        SetForegroundWindow(h);
    }
}

// ----------------------------------------------------------------------- time

fn local_time() -> SYSTEMTIME {
    unsafe {
        let mut t: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    }
}

pub fn now_hms() -> String {
    let t = local_time();
    format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
}

pub fn now_stamp() -> String {
    let t = local_time();
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

// ------------------------------------------------------------------- network

pub fn iface_luid(alias: &str) -> Option<u64> {
    let w = wide(alias);
    unsafe {
        let mut l: NET_LUID_LH = std::mem::zeroed();
        if ConvertInterfaceAliasToLuid(w.as_ptr(), &mut l) == 0 {
            Some(l.Value)
        } else {
            None
        }
    }
}

/// IPv4 address + netmask of every adapter in the system (other VPN tunnels included).
pub fn local_ipv4_nets() -> Vec<(std::net::Ipv4Addr, std::net::Ipv4Addr)> {
    unsafe {
        let mut size = 0u32;
        GetIpAddrTable(null_mut(), &mut size, 0);
        if size == 0 {
            return Vec::new();
        }
        // u32 buffer keeps the table aligned.
        let mut buf = vec![0u32; (size as usize).div_ceil(4) + 1];
        let table = buf.as_mut_ptr() as *mut windows_sys::Win32::NetworkManagement::IpHelper::MIB_IPADDRTABLE;
        if GetIpAddrTable(table, &mut size, 0) != 0 {
            return Vec::new();
        }
        let n = (*table).dwNumEntries as usize;
        let rows = std::ptr::addr_of!((*table).table) as *const MIB_IPADDRROW_XP;
        (0..n)
            .map(|i| {
                let r = &*rows.add(i);
                (std::net::Ipv4Addr::from(r.dwAddr.to_ne_bytes()), std::net::Ipv4Addr::from(r.dwMask.to_ne_bytes()))
            })
            .collect()
    }
}

pub struct RouteIface {
    pub alias: String,
    pub description: String,
    /// Real network card (Ethernet/Wi-Fi). Virtual adapters — VPN tunnels, TUN/TAP — are `false`.
    pub hardware: bool,
}

/// The adapter Windows would use right now to reach `ip`.
pub fn route_iface(ip: std::net::Ipv4Addr) -> Option<RouteIface> {
    unsafe {
        let mut idx = 0u32;
        if GetBestInterface(u32::from_ne_bytes(ip.octets()), &mut idx) != 0 {
            return None;
        }
        let mut row: MIB_IF_ROW2 = std::mem::zeroed();
        row.InterfaceIndex = idx;
        if GetIfEntry2(&mut row) != 0 {
            return None;
        }
        let s = |w: &[u16]| String::from_utf16_lossy(&w[..w.iter().position(|&c| c == 0).unwrap_or(w.len())]);
        Some(RouteIface {
            alias: s(&row.Alias),
            description: s(&row.Description),
            hardware: row.InterfaceAndOperStatusFlags._bitfield & 1 != 0,
        })
    }
}

// ---------------------------------------------------------------- file info

pub fn file_version(path: &Path) -> Option<String> {
    let w = wide_path(path);
    unsafe {
        let mut h = 0u32;
        let size = GetFileVersionInfoSizeW(w.as_ptr(), &mut h);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(w.as_ptr(), 0, size, buf.as_mut_ptr() as *mut c_void) == 0 {
            return None;
        }
        let root = wide("\\");
        let mut p: *mut c_void = null_mut();
        let mut len = 0u32;
        if VerQueryValueW(buf.as_ptr() as *const c_void, root.as_ptr(), &mut p, &mut len) == 0 || p.is_null() {
            return None;
        }
        let fi = &*(p as *const VS_FIXEDFILEINFO);
        let (ms, ls) = (fi.dwProductVersionMS, fi.dwProductVersionLS);
        if ms == 0 && ls == 0 {
            return None;
        }
        Some(format!("{}.{}.{}", ms >> 16, ms & 0xFFFF, ls >> 16))
    }
}

// --------------------------------------------------------------------- DPAPI

fn dpapi(data: &[u8], protect: bool) -> Option<Vec<u8>> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
        let ok = if protect {
            CryptProtectData(&input, null(), null(), null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&input, null_mut(), null(), null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as *mut c_void);
        Some(v)
    }
}

/// Encrypts a secret for the current Windows user and returns it as hex.
pub fn protect_str(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    dpapi(s.as_bytes(), true).map(|b| to_hex(&b)).unwrap_or_default()
}

pub fn unprotect_str(hex: &str) -> String {
    if hex.is_empty() {
        return String::new();
    }
    from_hex(hex)
        .and_then(|b| dpapi(&b, false))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// Cryptographically secure random bytes (system-preferred BCrypt RNG).
pub fn random_bytes(n: usize) -> Vec<u8> {
    use windows_sys::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let mut v = vec![0u8; n];
    unsafe {
        BCryptGenRandom(null_mut(), v.as_mut_ptr(), n as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG);
    }
    v
}

pub fn windows_build() -> u32 {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    winreg::RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Windows 11: rounded corners and a thin border in the theme color for our frameless window.
/// Silently does nothing on Windows 10.
pub fn style_window(hwnd: isize, border: eframe::egui::Color32) {
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    if hwnd == 0 {
        return;
    }
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWA_BORDER_COLOR: u32 = 34;
    const DWMWCP_ROUND: u32 = 2;
    let colorref: u32 = (border.r() as u32) | ((border.g() as u32) << 8) | ((border.b() as u32) << 16);
    unsafe {
        let h = hwnd as *mut c_void;
        DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_ROUND as *const u32 as *const c_void, 4);
        DwmSetWindowAttribute(h, DWMWA_BORDER_COLOR, &colorref as *const u32 as *const c_void, 4);
    }
}

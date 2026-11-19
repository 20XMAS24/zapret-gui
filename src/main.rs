#![windows_subsystem = "windows"]

mod cfg;
mod ctl;
mod diag;
mod icon;
mod killswitch;
mod logbuf;
mod paths;
mod proxy;
mod sys;
mod tg;
mod theme;
mod ui;
mod updater;
mod warp;
mod zapret;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Developer flag: look at the UI without elevation (actions that need admin will just fail).
    let preview = args.iter().any(|a| a == "--preview");

    // The manifest requests elevation; this is the fallback for odd launch paths.
    if !preview && !sys::is_admin() {
        if !sys::elevate(&args.join(" ")) {
            sys::msgbox(
                "ZAPRET GUI нужны права администратора — без них winws.exe, туннель и kill switch не работают.\n\n\
                 Запустите приложение через «Запуск от имени администратора».",
                false,
            );
        }
        return;
    }

    // Called by the uninstaller: stop everything we started and remove system changes.
    if args.iter().any(|a| a == "--cleanup") {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
        paths::set_root(exe_dir.clone(), exe_dir);
        sys::kill_where(|p| p.name.eq_ignore_ascii_case("ZapretGUI.exe"));
        sys::kill_where(|p| p.name.eq_ignore_ascii_case("winws.exe"));
        tg::stop();
        let sb = paths::singbox().to_string_lossy().to_ascii_lowercase();
        sys::kill_where(|p| {
            p.name.eq_ignore_ascii_case("sing-box.exe")
                && sys::process_path(p.pid).map(|x| x.to_ascii_lowercase() == sb).unwrap_or(false)
        });
        let _ = killswitch::disable();
        if diag::autostart_enabled() {
            let _ = diag::autostart_set(false);
        }
        return;
    }

    if !preview && !sys::single_instance("Global\\ZapretGUI_SingleInstance") {
        sys::show_hwnd(sys::find_window(ui::TITLE));
        return;
    }

    let autostart = args.iter().any(|a| a.eq_ignore_ascii_case("-autostart") || a.eq_ignore_ascii_case("/autostart"));

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let mut cfg = cfg::Cfg::load(exe_dir.join("ZapretGUI.ini"));

    // Prefer the exe folder; otherwise a remembered ZapretFolder; otherwise walk up a few levels
    // (handy when running from ZapretGUI-rs\target\release).
    let root = if paths::looks_like_zapret(&exe_dir) {
        Some(exe_dir.clone())
    } else if let Some(f) = cfg.get("ZapretFolder").map(PathBuf::from).filter(|p| paths::looks_like_zapret(p)) {
        Some(f)
    } else {
        let found = exe_dir.ancestors().skip(1).take(4).find(|d| paths::looks_like_zapret(d)).map(|d| d.to_path_buf());
        if let Some(f) = &found {
            cfg.set("ZapretFolder", f.display().to_string());
            cfg.save();
        }
        found
    };
    let Some(root) = root else {
        sys::msgbox(
            "Не найден bin\\winws.exe.\n\nПоложите ZapretGUI.exe в корень папки zapret (рядом с general.bat) \
             или укажите путь в ZapretGUI.ini: ZapretFolder=C:\\путь\\к\\zapret",
            true,
        );
        return;
    };

    paths::set_root(root, exe_dir);
    let core = ctl::Core::new(cfg);
    ui::run(core, autostart);
}

//! Controller: owns state and child processes, runs every action on a worker thread.
//! The UI only reads snapshots and calls these methods, so the tray works while the window is hidden.

use std::io::{BufRead, BufReader, Read};
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::cfg::Cfg;
use crate::logbuf::{Kind, LogBuf};
use crate::zapret::{self, GameMode};
use crate::{diag, game_diag, killswitch, paths, proxy, sys, tg, updater};

#[derive(Clone, PartialEq, Debug)]
pub enum ProxyState {
    Off,
    Connecting,
    Connected,
    /// Tunnel died while the kill switch keeps everything blocked.
    Blocked(String),
    Failed(String),
}

#[derive(Clone, PartialEq, Debug)]
pub enum Confirm {
    RemoveService,
}

#[derive(Clone)]
pub struct State {
    pub strategies: Vec<String>,
    pub selected: String,
    pub zapret_running: bool,
    pub current_strategy: Option<String>,
    pub bypass_busy: bool,
    pub tg_running: bool,
    pub game: GameMode,
    pub game_debug_busy: bool,
    pub game_debug_result: Option<Result<PathBuf, String>>,
    pub autostart: bool,

    pub zap_local: String,
    pub zap_remote: Option<String>,
    pub tg_local: String,
    pub tg_remote: Option<String>,
    pub pending_zap: Option<(String, PathBuf)>,
    pub pending_tg: Option<(String, PathBuf)>,
    pub app_remote: Option<String>,
    pub pending_app: Option<(String, PathBuf)>,
    /// (version, release notes) to show once after ZapretGUI was updated.
    pub whats_new: Option<(String, String)>,
    pub progress: Option<f32>,
    pub checking: bool,
    pub updating: bool,

    pub proxy: ProxyState,
    pub proxy_since: Option<Instant>,
    pub proxy_name: String,
    pub ext_ip: Option<String>,
    pub ks_active: bool,
    pub proxy_test: Option<(bool, String)>,
    pub testing: bool,

    pub confirm: Option<Confirm>,
}

pub struct Core {
    pub cfg: Mutex<Cfg>,
    pub st: Mutex<State>,
    pub log: Mutex<LogBuf>,
    pub proxy_cfg: Mutex<proxy::Settings>,
    ctx: OnceLock<egui::Context>,
    winws: Mutex<Option<Child>>,
    sb: Mutex<Option<Child>>,
    proxy_gen: AtomicU64,
    proxy_busy: AtomicBool,
    last_ip: Mutex<Option<(String, std::net::SocketAddr)>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Core {
    pub fn new(cfg: Cfg) -> Arc<Self> {
        let selected = cfg.get_or("Strategy", "");
        Arc::new(Core {
            cfg: Mutex::new(cfg),
            st: Mutex::new(State {
                strategies: Vec::new(),
                selected,
                zapret_running: false,
                current_strategy: None,
                bypass_busy: false,
                tg_running: false,
                game: GameMode::read(),
                game_debug_busy: false,
                game_debug_result: None,
                autostart: false,
                zap_local: zapret::local_version(),
                zap_remote: None,
                tg_local: String::new(),
                tg_remote: None,
                pending_zap: None,
                pending_tg: None,
                app_remote: None,
                pending_app: None,
                whats_new: None,
                progress: None,
                checking: false,
                updating: false,
                proxy: ProxyState::Off,
                proxy_since: None,
                proxy_name: String::new(),
                ext_ip: None,
                ks_active: false,
                proxy_test: None,
                testing: false,
                confirm: None,
            }),
            log: Mutex::new(LogBuf::default()),
            proxy_cfg: Mutex::new(proxy::Settings::load()),
            ctx: OnceLock::new(),
            winws: Mutex::new(None),
            sb: Mutex::new(None),
            proxy_gen: AtomicU64::new(0),
            proxy_busy: AtomicBool::new(false),
            last_ip: Mutex::new(None),
        })
    }

    pub fn set_ctx(&self, ctx: egui::Context) {
        let _ = self.ctx.set(ctx);
    }

    pub fn repaint(&self) {
        if let Some(c) = self.ctx.get() {
            c.request_repaint();
        }
    }

    pub fn log(&self, text: &str, kind: Kind) {
        lock(&self.log).push(text, kind);
        self.repaint();
    }

    pub fn snapshot(&self) -> State {
        lock(&self.st).clone()
    }

    fn st(&self) -> MutexGuard<'_, State> {
        lock(&self.st)
    }

    pub fn cfg_bool(&self, key: &str, def: bool) -> bool {
        lock(&self.cfg).get_bool(key, def)
    }

    pub fn set_cfg_bool(&self, key: &str, v: bool) {
        let mut c = lock(&self.cfg);
        c.set_bool(key, v);
        c.save();
    }

    pub fn save_cfg(&self) {
        lock(&self.cfg).save();
    }

    fn spawn(self: &Arc<Self>, f: impl FnOnce(Arc<Core>) + Send + 'static) {
        let me = self.clone();
        thread::spawn(move || f(me));
    }

    fn pipe_to_log(self: &Arc<Self>, r: impl Read + Send + 'static, kind: Kind, prefix: &'static str) {
        self.spawn(move |me| {
            let mut rd = BufReader::new(r);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match rd.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let line = strip_ansi(&sys::oem_decode(&buf));
                        let line = line.trim();
                        if !line.is_empty() {
                            let level = if prefix == "[sing-box] " {
                                if line.starts_with("ERROR") || line.starts_with("FATAL") { Kind::Error }
                                else if line.starts_with("WARN") { Kind::Warn }
                                else { Kind::Dim }
                            } else { kind };
                            me.log(&format!("{prefix}{line}"), level);
                        }
                    }
                }
            }
        });
    }

    // -------------------------------------------------------------- startup

    pub fn init(self: &Arc<Self>, autostart: bool) {
        self.log(&format!("Папка zapret: {}", paths::root().display()), Kind::Dim);
        if !paths::winws().is_file() {
            self.log("bin\\winws.exe не найден — положите ZapretGUI.exe в корень папки zapret.", Kind::Error);
        }
        self.reload_strategies();
        {
            let tgv = tg::local_file_version().unwrap_or_else(|| lock(&self.cfg).get_or("TgTag", "неизвестна"));
            self.st().tg_local = tgv;
        }

        self.spawn(|me| {
            let a = diag::autostart_enabled();
            me.st().autostart = a;
            me.repaint();
        });

        // Leftovers of a crashed session: orphaned sing-box and/or kill switch rules.
        let orphan = kill_singbox_orphans();
        let ks = killswitch::is_active();
        let proxy_on_launch = lock(&self.proxy_cfg).connect_on_launch;
        if orphan > 0 || ks {
            if proxy_on_launch {
                self.log("Найден незавершённый туннель с прошлого запуска — переподключаюсь.", Kind::Warn);
                if ks {
                    self.st().ks_active = true;
                }
            } else {
                self.log("Найден незавершённый туннель с прошлого запуска — снимаю блокировку kill switch.", Kind::Warn);
                if let Err(e) = killswitch::disable() {
                    self.log(&format!("Kill switch: {e}"), Kind::Error);
                    self.st().ks_active = true;
                }
            }
        }

        self.start_monitor();
        self.start_update_timer();
        self.detect_new_version();

        // Right after a self-update: bring back whatever was running before it.
        let resume = {
            let mut c = lock(&self.cfg);
            let r = c.get("ResumeAfterUpdate").map(str::to_string);
            if r.is_some() {
                c.remove("ResumeAfterUpdate");
                c.save();
            }
            r
        };
        let (mut res_bypass, mut res_tg, mut res_proxy) = (false, false, false);
        if let Some(r) = resume {
            self.log(&format!("ZapretGUI обновлён до {}.", updater::APP_VERSION), Kind::Ok);
            res_bypass = r.contains("bypass");
            res_tg = r.contains("tg");
            res_proxy = r.contains("proxy");
        }

        if res_tg && !res_bypass && !tg::is_running() {
            self.start_tg();
        }
        let start_bypass = autostart || res_bypass || self.cfg_bool("StartBypassOnLaunch", false);
        if start_bypass {
            if is_winws_running() {
                self.log("Обход уже работает — повторный запуск не требуется.", Kind::Info);
            } else {
                self.start_bypass();
            }
        }
        if proxy_on_launch || res_proxy {
            self.proxy_connect();
        }
        if res_tg && res_bypass && !self.cfg_bool("StartTgOnLaunch", false) {
            let me = self.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_secs(2));
                if !tg::is_running() {
                    me.start_tg();
                }
            });
        }
        if self.cfg_bool("CheckUpdates", true) {
            self.check_updates(false);
        }
    }

    /// First start of a new version (self-update or manual install): show what changed, once.
    fn detect_new_version(self: &Arc<Self>) {
        let cur = updater::APP_VERSION;
        let last = {
            let mut c = lock(&self.cfg);
            let last = c.get("LastVersion").map(str::to_string);
            if last.as_deref() != Some(cur) {
                c.set("LastVersion", cur);
                c.save();
            }
            last
        };
        let Some(last) = last else { return }; // fresh install: nothing to announce
        if last == cur || !updater::is_newer(&last, cur) {
            return;
        }
        // Notes saved before the update, otherwise ask GitHub.
        let saved = std::fs::read_to_string(updater::whatsnew_file()).ok().and_then(|t| {
            let (v, notes) = t.split_once('\n')?;
            (v.trim() == cur).then(|| notes.trim().to_string())
        });
        let _ = std::fs::remove_file(updater::whatsnew_file());
        let me = self.clone();
        self.spawn(move |_| {
            let notes = saved
                .filter(|n| !n.is_empty())
                .or_else(|| updater::app_notes(&me.update_repo(), cur).ok())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "Список изменений не указан в релизе.".into());
            me.st().whats_new = Some((cur.to_string(), notes));
            me.repaint();
        });
    }

    pub fn dismiss_whats_new(&self) {
        self.st().whats_new = None;
    }

    fn start_monitor(self: &Arc<Self>) {
        self.spawn(|me| loop {
            me.poll();
            thread::sleep(Duration::from_millis(1200));
        });
    }

    fn poll(self: &Arc<Self>) {
        let procs = sys::processes();
        let zap = procs.iter().any(|p| p.name.eq_ignore_ascii_case("winws.exe"));
        let tgr = tg::is_running_in(&procs);

        // Has the tunnel died under us?
        let exited = {
            let connected = self.st().proxy == ProxyState::Connected;
            let mut g = lock(&self.sb);
            match g.as_mut() {
                Some(c) if connected => match c.try_wait() {
                    Ok(Some(s)) => {
                        *g = None;
                        Some(s.code().unwrap_or(-1))
                    }
                    _ => None,
                },
                _ => None,
            }
        };
        if let Some(code) = exited {
            self.on_tunnel_dropped(code);
        }

        let mut s = self.st();
        let mut changed = false;
        if s.zapret_running != zap {
            s.zapret_running = zap;
            if !zap {
                s.current_strategy = None;
            }
            changed = true;
        }
        if s.tg_running != tgr {
            s.tg_running = tgr;
            changed = true;
        }
        drop(s);
        if changed {
            self.repaint();
        }
    }

    // ----------------------------------------------------------- strategies

    pub fn reload_strategies(&self) {
        let mut items = zapret::list_strategies();
        let default = items.first().cloned().unwrap_or_default();
        if zapret::call_of_duty_base(&items).is_some() {
            items.insert(0, zapret::CALL_OF_DUTY.into());
        }
        let mut s = self.st();
        if !items.contains(&s.selected) {
            s.selected = default;
        }
        let n = items.len();
        s.strategies = items;
        drop(s);
        self.log(&format!("Найдено стратегий: {n}"), Kind::Dim);
    }

    pub fn select_strategy(&self, name: &str) {
        self.st().selected = name.to_string();
        let mut c = lock(&self.cfg);
        c.set("Strategy", name);
        c.save();
    }

    pub fn set_game(&self, m: GameMode) {
        if let Err(e) = m.write() {
            self.log(&format!("Игровой фильтр: {e}"), Kind::Error);
            return;
        }
        self.st().game = m;
        let running = self.st().zapret_running;
        self.log(
            &format!("Игровой фильтр: {}.{}", m.label(), if running { " Перезапустите обход, чтобы применить." } else { "" }),
            if running { Kind::Warn } else { Kind::Info },
        );
    }

    pub fn start_bypass(self: &Arc<Self>) {
        self.spawn(|me| me.start_bypass_sync(false));
    }

    /// Called after the user agreed to remove the conflicting zapret service.
    pub fn start_bypass_remove_service(self: &Arc<Self>) {
        self.spawn(|me| me.start_bypass_sync(true));
    }

    pub fn stop_bypass(self: &Arc<Self>) {
        self.spawn(|me| {
            me.change_bypass("остановки", || {
                me.stop_bypass_sync()?;
                me.log("Обход остановлен.", Kind::Info);
                Ok(())
            });
        });
    }

    pub fn restart_bypass(self: &Arc<Self>) {
        self.spawn(|me| {
            me.change_bypass("перезапуска", || {
                me.stop_bypass_sync()?;
                me.start_bypass_inner(false)
            });
        });
    }

    fn start_bypass_sync(self: &Arc<Self>, remove_service: bool) {
        self.change_bypass("запуска", || self.start_bypass_inner(remove_service));
    }

    fn change_bypass(self: &Arc<Self>, operation: &str, action: impl FnOnce() -> Result<(), String>) {
        {
            let mut s = self.st();
            if s.bypass_busy {
                return;
            }
            s.bypass_busy = true;
        }
        self.repaint();
        let r = action();
        self.st().bypass_busy = false;
        if let Err(e) = r {
            if !e.is_empty() {
                self.log(&format!("Ошибка {operation} обхода: {e}"), Kind::Error);
            }
        }
        self.poll();
        self.repaint();
    }

    fn start_bypass_inner(self: &Arc<Self>, remove_service: bool) -> Result<(), String> {
        let (strategy, bat_name) = {
            let s = self.st();
            let strategy = s.selected.clone();
            let bat = if strategy == zapret::CALL_OF_DUTY {
                zapret::call_of_duty_base(&s.strategies)
                    .ok_or("Не найдена базовая стратегия general для Call of Duty.")?.to_string()
            } else {
                strategy.clone()
            };
            (strategy, bat)
        };
        if strategy.is_empty() {
            return Err("стратегия не выбрана.".into());
        }

        if sys::service_state("zapret") == Some(sys::SVC_RUNNING) {
            if !remove_service {
                self.st().confirm = Some(Confirm::RemoveService);
                self.repaint();
                return Err(String::new());
            }
            sys::run(&sys::system32("net.exe"), "stop zapret");
            sys::run(&sys::system32("sc.exe"), "delete zapret");
            self.log("Служба zapret остановлена и удалена.", Kind::Info);
        }

        let call_of_duty = strategy == zapret::CALL_OF_DUTY;
        let bat = paths::root().join(&bat_name);
        if !bat.is_file() {
            return Err(format!("файл стратегии не найден: {bat_name}"));
        }
        if !paths::winws().is_file() {
            return Err("не найден bin\\winws.exe — приложение лежит не в папке zapret.".into());
        }
        let args = zapret::build_args(&bat, call_of_duty)?;
        if call_of_duty {
            self.log(&format!("Call of Duty: доменный список добавлен к {}. Игровой фильтр выключен.", zapret::strip_bat(&bat_name)), Kind::Info);
        }

        self.log(&format!("Запуск: {}", zapret::strip_bat(&strategy)), Kind::Info);
        zapret::ensure_user_lists();
        zapret::enable_tcp_timestamps();
        self.kill_winws();

        let redirect = self.cfg_bool("RedirectWinwsOutput", true);
        let mut cmd = sys::hidden(paths::winws());
        {
            use std::os::windows::process::CommandExt;
            cmd.raw_arg(&args);
        }
        cmd.current_dir(paths::bin());
        if redirect {
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        } else {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
        let mut child = cmd.spawn().map_err(|e| format!("не удалось запустить winws.exe: {e}"))?;
        if let Some(o) = child.stdout.take() {
            self.pipe_to_log(o, Kind::Dim, "");
        }
        if let Some(e) = child.stderr.take() {
            self.pipe_to_log(e, Kind::Warn, "");
        }
        *lock(&self.winws) = Some(child);

        {
            let mut s = self.st();
            s.current_strategy = Some(strategy.clone());
            s.zapret_running = true;
        }
        {
            let mut c = lock(&self.cfg);
            c.set("Strategy", &strategy);
            c.save();
        }
        self.log("Обход запущен.", Kind::Ok);

        if self.cfg_bool("StartTgOnLaunch", false) && !tg::is_running() {
            self.start_tg_sync();
        }
        if call_of_duty || self.cfg_bool("GameDebugOnStart", false) {
            self.run_game_debug();
        }
        Ok(())
    }

    fn kill_winws(&self) {
        if let Some(mut c) = lock(&self.winws).take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        sys::kill_where(|p| p.name.eq_ignore_ascii_case("winws.exe"));
    }

    fn stop_bypass_sync(&self) -> Result<(), String> {
        let state = sys::service_state("zapret");
        if !matches!(state, None | Some(sys::SVC_STOPPED)) {
            if state != Some(sys::SVC_STOP_PENDING) {
                let (code, output) = sys::run(&sys::system32("sc.exe"), "stop zapret");
                if code != 0 && code != 1062 {
                    return Err(format!("служба zapret не остановлена ({code}): {}", output.trim()));
                }
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while !matches!(sys::service_state("zapret"), None | Some(sys::SVC_STOPPED)) {
                if Instant::now() >= deadline {
                    return Err("служба zapret не завершилась за 10 секунд.".into());
                }
                thread::sleep(Duration::from_millis(100));
            }
        }

        self.kill_winws();
        if is_winws_running() {
            return Err("процесс winws.exe ещё работает.".into());
        }
        let mut s = self.st();
        s.zapret_running = false;
        s.current_strategy = None;
        drop(s);
        self.repaint();
        Ok(())
    }

    // ------------------------------------------------------------- telegram

    pub fn start_tg(self: &Arc<Self>) {
        self.spawn(|me| me.start_tg_sync());
    }

    fn start_tg_sync(&self) {
        match tg::start() {
            Ok(_) => self.log("TgWsProxy запущен (значок появится в трее).", Kind::Ok),
            Err(e) => self.log(&format!("TgWsProxy: {e}"), Kind::Error),
        }
        thread::sleep(Duration::from_millis(400));
        self.st().tg_running = tg::is_running();
        self.repaint();
    }

    pub fn stop_tg(self: &Arc<Self>) {
        self.spawn(|me| {
            tg::stop();
            me.st().tg_running = false;
            me.log("TgWsProxy остановлен.", Kind::Info);
        });
    }

    pub fn restart_tg(self: &Arc<Self>) {
        self.spawn(|me| {
            tg::stop();
            thread::sleep(Duration::from_millis(700));
            match tg::start() {
                Ok(_) => me.log("TgWsProxy перезапущен.", Kind::Ok),
                Err(e) => me.log(&format!("TgWsProxy: {e}"), Kind::Error),
            }
        });
    }

    // -------------------------------------------------------------- updates

    pub fn check_updates(self: &Arc<Self>, manual: bool) {
        {
            let mut s = self.st();
            if s.checking {
                return;
            }
            s.checking = true;
        }
        if manual {
            self.log("Проверка обновлений...", Kind::Info);
        }
        self.spawn(move |me| {
            if me.check_app(manual) {
                return; // installer takes over, the app is about to exit
            }
            me.check_zapret(manual);
            me.check_tg(manual);
            me.st().checking = false;
            me.repaint();
            if me.auto_update() {
                if me.st().pending_zap.is_some() {
                    me.log("Автообновление: устанавливаю новую версию zapret.", Kind::Info);
                    me.apply_zapret_sync();
                }
                if me.st().pending_tg.is_some() {
                    me.log("Автообновление: устанавливаю новую версию TgWsProxy.", Kind::Info);
                    me.apply_tg_sync();
                }
            }
        });
    }

    pub fn update_repo(&self) -> String {
        let r = lock(&self.cfg).get_or("UpdateRepo", "");
        if r.trim().is_empty() { updater::APP_REPO.to_string() } else { r.trim().to_string() }
    }

    pub fn auto_update(&self) -> bool {
        self.cfg_bool("AutoUpdate", true)
    }

    /// Re-checks every 6 hours while the app keeps running.
    fn start_update_timer(self: &Arc<Self>) {
        self.spawn(|me| loop {
            thread::sleep(Duration::from_secs(6 * 3600));
            if me.cfg_bool("CheckUpdates", true) && !me.st().updating {
                me.check_updates(false);
            }
        });
    }

    /// Returns true when an automatic self-update has been started.
    fn check_app(self: &Arc<Self>, manual: bool) -> bool {
        let repo = self.update_repo();
        if repo.starts_with("OWNER/") {
            return false; // repository not configured in this build
        }
        let rel = match updater::app_query(&repo) {
            Ok(r) => r,
            Err(e) => {
                if manual {
                    self.log(&format!("Проверка обновлений ZapretGUI не удалась: {e}"), Kind::Warn);
                }
                return false;
            }
        };
        self.st().app_remote = Some(rel.version.clone());
        if !updater::is_newer(updater::APP_VERSION, &rel.version) {
            if manual {
                self.log(&format!("ZapretGUI актуален ({}).", updater::APP_VERSION), Kind::Ok);
            }
            return false;
        }
        self.log(&format!("Доступна новая версия ZapretGUI: {} (у вас {}). Скачиваю...", rel.version, updater::APP_VERSION), Kind::Warn);
        self.st().progress = Some(0.0);
        let r = updater::app_download(&rel, |done, total| {
            if total > 0 {
                let p = done as f32 / total as f32;
                let mut s = self.st();
                if (s.progress.unwrap_or(0.0) - p).abs() > 0.01 {
                    s.progress = Some(p);
                    drop(s);
                    self.repaint();
                }
            }
        });
        self.st().progress = None;
        match r {
            Ok(f) => {
                // Keep the notes so the new version can show "Что нового" on its first start.
                let _ = std::fs::write(updater::whatsnew_file(), format!("{}\n{}", rel.version, rel.notes));
                self.st().pending_app = Some((rel.version.clone(), f));
                if self.auto_update() && !self.st().updating {
                    self.log(&format!("Автообновление: устанавливаю ZapretGUI {}, приложение перезапустится.", rel.version), Kind::Info);
                    self.apply_app_update();
                    return true;
                }
                self.log(&format!("Установщик {} загружен, контрольная сумма совпала. Нажмите «Обновить ZapretGUI» в настройках.", rel.version), Kind::Ok);
            }
            Err(e) => self.log(&format!("Обновление ZapretGUI: {e}"), Kind::Error),
        }
        false
    }

    /// Stops everything (files must not be locked), remembers what was running and hands over
    /// to the silent installer, which restarts ZapretGUI when it is done.
    pub fn apply_app_update(self: &Arc<Self>) {
        let Some((ver, setup)) = self.st().pending_app.clone() else { return };
        self.st().updating = true;
        self.log(&format!("▸ Обновление ZapretGUI до {ver}..."), Kind::Head);
        self.spawn(move |me| {
            let (zap, tgr, proxy) = {
                let s = me.st();
                (s.zapret_running, s.tg_running, matches!(s.proxy, ProxyState::Connected | ProxyState::Connecting | ProxyState::Blocked(_)))
            };
            let mut resume = Vec::new();
            if zap { resume.push("bypass"); }
            if tgr { resume.push("tg"); }
            if proxy { resume.push("proxy"); }
            {
                let mut c = lock(&me.cfg);
                c.set("ResumeAfterUpdate", resume.join(","));
                c.save();
            }
            me.shutdown(true);
            // Give this process a moment to exit before the installer replaces its files.
            // /DIR keeps the update in the folder we run from (also for a copy that was never installed).
            let cmd = format!(
                "/c ping -n 3 127.0.0.1 >nul & \"{}\" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- \"/DIR={}\"",
                setup.display(),
                paths::exe_dir().display()
            );
            let r = {
                use std::os::windows::process::CommandExt;
                sys::hidden(sys::system32("cmd.exe")).raw_arg(&cmd).spawn()
            };
            match r {
                Ok(_) => std::process::exit(0),
                Err(e) => {
                    me.log(&format!("Не удалось запустить установщик: {e}"), Kind::Error);
                    me.st().updating = false;
                }
            }
        });
    }

    fn check_zapret(self: &Arc<Self>, manual: bool) {
        let local = zapret::local_version();
        let remote = match updater::zapret_remote_version() {
            Ok(r) => r,
            Err(e) => {
                self.log(&format!("Проверка обновлений zapret не удалась: {e}"), Kind::Warn);
                return;
            }
        };
        {
            let mut s = self.st();
            s.zap_local = local.clone();
            s.zap_remote = Some(remote.clone());
        }
        if !updater::is_newer(&local, &remote) {
            if manual {
                self.log(&format!("Установлена актуальная версия zapret: {local}"), Kind::Ok);
            }
            return;
        }
        self.log(&format!("Доступна новая версия zapret: {remote} (у вас {local}). Скачиваю..."), Kind::Warn);
        self.st().progress = Some(0.0);
        let r = updater::download_zapret(&remote, |done, total| {
            if total > 0 {
                let p = done as f32 / total as f32;
                let mut s = self.st();
                if (s.progress.unwrap_or(0.0) - p).abs() > 0.01 {
                    s.progress = Some(p);
                    drop(s);
                    self.repaint();
                }
            }
        });
        self.st().progress = None;
        match r {
            Ok(zip) => {
                self.st().pending_zap = Some((remote.clone(), zip));
                self.log(&format!("Архив загружен. Нажмите «Обновить до {remote}» в настройках."), Kind::Ok);
            }
            Err(e) => self.log(&format!("Не удалось скачать обновление zapret: {e}"), Kind::Error),
        }
    }

    fn check_tg(self: &Arc<Self>, manual: bool) {
        let (tag, url) = match updater::tg_query() {
            Ok(v) => v,
            Err(e) => {
                if manual {
                    self.log(&format!("TgWsProxy: {e}"), Kind::Warn);
                }
                return;
            }
        };
        let known = lock(&self.cfg).get_or("TgTag", "");
        let file_v = tg::local_file_version();
        let shown = file_v.clone().unwrap_or_else(|| if known.is_empty() { "неизвестна".into() } else { known.clone() });
        {
            let mut s = self.st();
            s.tg_local = shown.clone();
            s.tg_remote = Some(tag.clone());
        }
        let needs = !updater::same_tag(&shown, &tag) && !updater::same_tag(&known, &tag);
        if needs && paths::find_tgwsproxy().is_some() {
            self.log(&format!("Доступна новая версия TgWsProxy: {tag}"), Kind::Warn);
            match updater::tg_download(&url, &tag) {
                Ok(f) => {
                    self.st().pending_tg = Some((tag.clone(), f));
                    self.log(&format!("TgWsProxy загружен. Нажмите «Обновить до {tag}» в настройках."), Kind::Ok);
                }
                Err(e) => self.log(&format!("Не удалось скачать TgWsProxy: {e}"), Kind::Error),
            }
        } else if manual {
            self.log(&format!("TgWsProxy актуален ({tag})."), Kind::Ok);
        }
    }

    pub fn apply_zapret_update(self: &Arc<Self>) {
        self.spawn(|me| me.apply_zapret_sync());
    }

    fn apply_zapret_sync(self: &Arc<Self>) {
        let Some((ver, zip)) = self.st().pending_zap.clone() else { return };
        self.st().updating = true;
        self.log("▸ Обновление zapret...", Kind::Head);
        let me = self;
        {
            let (was_running, strategy) = {
                let s = me.st();
                (s.zapret_running, s.current_strategy.clone())
            };
            if let Err(e) = me.stop_bypass_sync() {
                me.log(&format!("Обновление прервано: {e}"), Kind::Error);
                me.st().updating = false;
                me.repaint();
                return;
            }
            let log = |t: &str, k: Kind| me.log(t, k);
            match updater::apply_zapret(&zip, &ver, &log) {
                Ok(_) => {
                    me.log(&format!("zapret обновлён до {ver}."), Kind::Ok);
                    {
                        let mut s = me.st();
                        s.pending_zap = None;
                        s.zap_local = zapret::local_version();
                    }
                    me.reload_strategies();
                    if was_running {
                        if let Some(st) = strategy {
                            if me.st().strategies.contains(&st) {
                                me.st().selected = st;
                            }
                        }
                        me.start_bypass_sync(false);
                    }
                }
                Err(e) => me.log(&format!("Обновление не удалось: {e}"), Kind::Error),
            }
            me.st().updating = false;
            me.repaint();
        }
    }

    pub fn apply_tg_update(self: &Arc<Self>) {
        self.spawn(|me| me.apply_tg_sync());
    }

    fn apply_tg_sync(self: &Arc<Self>) {
        let Some((tag, file)) = self.st().pending_tg.clone() else { return };
        self.st().updating = true;
        let me = self;
        let was_running = tg::is_running();
        {
            match updater::tg_apply(&file) {
                Ok(t) => {
                    me.log(&format!("TgWsProxy обновлён: {}", t.file_name().unwrap_or_default().to_string_lossy()), Kind::Ok);
                    {
                        let mut c = lock(&me.cfg);
                        c.set("TgTag", &tag);
                        c.save();
                    }
                    let mut s = me.st();
                    s.pending_tg = None;
                    s.tg_local = tag;
                }
                Err(e) => me.log(&format!("Обновление TgWsProxy не удалось: {e}"), Kind::Error),
            }
            if was_running {
                me.start_tg_sync();
            }
            me.st().updating = false;
            me.repaint();
        }
    }

    // ---------------------------------------------------------------- tools

    pub fn run_diag(self: &Arc<Self>) {
        self.spawn(|me| diag::run(&|t, k| me.log(t, k)));
    }

    pub fn game_debug_settings(&self) -> (bool, String) {
        let cfg = lock(&self.cfg);
        (cfg.get_bool("GameDebugOnStart", false), cfg.get_or("GameDebugTargets", ""))
    }

    pub fn save_game_debug(&self, on_start: bool, input: &str) -> Result<(), String> {
        let targets = game_diag::parse_targets(input)?;
        let text = targets.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
        let mut cfg = lock(&self.cfg);
        cfg.set_bool("GameDebugOnStart", on_start);
        cfg.set("GameDebugTargets", text);
        cfg.save();
        Ok(())
    }

    pub fn run_game_debug(self: &Arc<Self>) {
        let (_, targets) = self.game_debug_settings();
        {
            let mut state = self.st();
            if state.game_debug_busy {
                return;
            }
            state.game_debug_busy = true;
            state.game_debug_result = None;
        }
        self.log("Отладка Call of Duty: проверяю Discord, затем IP игры.", Kind::Info);
        self.spawn(move |me| {
            // Let winws finish initializing when called from strategy startup.
            thread::sleep(Duration::from_millis(500));
            let context = {
                let state = me.st();
                let strategy = state.current_strategy.as_deref().unwrap_or("не определена");
                format!(
                    "Активная стратегия: {strategy}\nwinws.exe: {}\nНастройка игрового фильтра: {}\nТуннель ZapretGUI: {}",
                    if is_winws_running() { "запущен" } else { "остановлен" },
                    if strategy == zapret::CALL_OF_DUTY { GameMode::Off.label() } else { GameMode::read().label() },
                    match state.proxy {
                        ProxyState::Off => "отключён",
                        ProxyState::Connecting => "подключается",
                        ProxyState::Connected => "подключён",
                        ProxyState::Blocked(_) => "трафик заблокирован",
                        ProxyState::Failed(_) => "ошибка подключения",
                    },
                )
            };
            let result = game_diag::run(&context, &targets).map_err(|e| e.to_string());
            match &result {
                Ok(path) => me.log(&format!("Отчёт Call of Duty: {}", path.display()), Kind::Info),
                Err(e) => me.log(&format!("Не удалось сохранить отчёт Call of Duty: {e}"), Kind::Error),
            }
            let mut state = me.st();
            state.game_debug_busy = false;
            state.game_debug_result = Some(result);
            drop(state);
            me.repaint();
        });
    }

    pub fn clear_discord(self: &Arc<Self>) {
        self.spawn(|me| {
            me.log("▸ Очистка кэша Discord...", Kind::Head);
            diag::clear_discord_cache(&|t, k| me.log(t, k));
        });
    }

    pub fn run_tests(&self) {
        let ps1 = paths::tests_ps1();
        if !ps1.is_file() {
            self.log("Не найден utils\\test zapret.ps1", Kind::Error);
            return;
        }
        if let Err(e) = game_diag::add_test_targets(&paths::utils().join("targets.txt")) {
            self.log(&format!("Не удалось добавить Call of Duty в тесты стратегий: {e}"), Kind::Warn);
        }
        let r = std::process::Command::new(sys::system32("WindowsPowerShell\\v1.0\\powershell.exe"))
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&ps1)
            .current_dir(paths::root())
            .spawn();
        match r {
            Ok(_) => self.log("Тесты запущены в отдельном окне PowerShell.", Kind::Info),
            Err(e) => self.log(&format!("Не удалось запустить тесты: {e}"), Kind::Error),
        }
    }

    pub fn set_autostart(self: &Arc<Self>, on: bool) {
        self.spawn(move |me| {
            match diag::autostart_set(on) {
                Ok(_) => me.log(if on { "Автозапуск с Windows включён." } else { "Автозапуск с Windows выключен." }, Kind::Ok),
                Err(e) => me.log(&format!("Не удалось изменить автозапуск: {e}"), Kind::Error),
            }
            me.st().autostart = diag::autostart_enabled();
            me.repaint();
        });
    }

    // ---------------------------------------------------------------- proxy

    pub fn save_proxy(&self, s: proxy::Settings) {
        s.save();
        *lock(&self.proxy_cfg) = s;
    }

    pub fn proxy_settings(&self) -> proxy::Settings {
        lock(&self.proxy_cfg).clone()
    }

    pub fn proxy_test(self: &Arc<Self>, p: proxy::Profile) {
        {
            let mut s = self.st();
            if s.testing {
                return;
            }
            s.testing = true;
            s.proxy_test = None;
        }
        self.repaint();
        self.spawn(move |me| {
            let r = proxy::test(&p);
            let mut s = me.st();
            s.testing = false;
            s.proxy_test = Some(match r {
                Ok(ms) => (true, format!("Работает · {ms} мс")),
                Err(e) => (false, e),
            });
            drop(s);
            me.repaint();
        });
    }

    pub fn proxy_connect(self: &Arc<Self>) {
        let gen = self.proxy_gen.fetch_add(1, Ordering::SeqCst) + 1;
        self.spawn(move |me| {
            me.connect_sync(gen, false);
        });
    }

    pub fn proxy_disconnect(self: &Arc<Self>) {
        self.proxy_gen.fetch_add(1, Ordering::SeqCst);
        self.spawn(|me| me.disconnect_sync(true));
    }

    fn stop_singbox(&self) {
        if let Some(mut c) = lock(&self.sb).take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        kill_singbox_orphans();
    }

    fn disconnect_sync(&self, log: bool) {
        // Wait for an in-flight connect attempt to notice the new generation.
        for _ in 0..60 {
            if !self.proxy_busy.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        self.stop_singbox();
        let ks = killswitch::disable();
        {
            let mut s = self.st();
            s.proxy = ProxyState::Off;
            s.proxy_since = None;
            s.ext_ip = None;
            s.ks_active = ks.is_err();
        }
        if let Err(e) = ks {
            self.log(&format!("Kill switch: {e}"), Kind::Error);
        }
        if log {
            self.log("Прокси-туннель отключён.", Kind::Info);
        }
        self.repaint();
    }

    fn connect_sync(self: &Arc<Self>, gen: u64, reconnect: bool) {
        if self.proxy_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let r = self.connect_inner(gen, reconnect);
        if let Err(e) = r {
            if self.proxy_gen.load(Ordering::SeqCst) != gen {
                self.proxy_busy.store(false, Ordering::SeqCst);
                return; // user pressed "disconnect" meanwhile
            }
            self.stop_singbox();
            let keep_blocked = reconnect && lock(&self.proxy_cfg).kill_switch && killswitch::is_active();
            if keep_blocked {
                self.log(&format!("Переподключение не удалось: {e}"), Kind::Error);
                self.st().proxy = ProxyState::Blocked(e);
                self.schedule_reconnect(gen, 5);
            } else {
                let _ = killswitch::disable();
                self.log(&format!("Прокси: {e}"), Kind::Error);
                let mut s = self.st();
                s.proxy = ProxyState::Failed(e);
                s.ks_active = killswitch::is_active();
                drop(s);
                if reconnect {
                    self.schedule_reconnect(gen, 5);
                }
            }
            self.repaint();
        }
        self.proxy_busy.store(false, Ordering::SeqCst);
    }

    fn connect_inner(self: &Arc<Self>, gen: u64, reconnect: bool) -> Result<(), String> {
        let settings = self.proxy_settings();
        let prof = settings.current().cloned().ok_or("добавьте и выберите прокси-профиль.")?;
        prof.validate()?;
        let sb_exe = paths::singbox();
        if !sb_exe.is_file() {
            return Err("не найден bin\\sing-box.exe.".into());
        }
        {
            let mut s = self.st();
            s.proxy = ProxyState::Connecting;
            s.proxy_name = prof.display_name();
            s.ext_ip = None;
        }
        self.repaint();
        if !reconnect {
            self.log(&format!("▸ Подключение через {} ({})...", prof.display_name(), prof.kind.label()), Kind::Head);
        }

        // Resolve before the kill switch blocks our own DNS; keep the last answer for reconnects.
        let addr = match prof.resolve() {
            Ok(a) => {
                *lock(&self.last_ip) = Some((prof.host.trim().to_string(), a));
                a
            }
            Err(e) => match lock(&self.last_ip).clone() {
                Some((h, a)) if h == prof.host.trim() => std::net::SocketAddr::new(a.ip(), prof.port_num()?),
                _ => return Err(e),
            },
        };

        // Fresh start: kill what is left of an older sing-box and wait for its adapter to vanish.
        self.stop_singbox();
        for _ in 0..30 {
            if sys::iface_luid(proxy::TUN_NAME).is_none() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        // Another full-tunnel VPN (Sota Connect, Hiddify, WireGuard…) overrides our routes with narrower ones
        // and often blocks direct exits: the proxy then works only through it, slowly, and DNS may die.
        if !reconnect {
            if let std::net::IpAddr::V4(v4) = addr.ip() {
                if let Some(i) = sys::route_iface(v4).filter(|i| !i.hardware && !v4.is_loopback()) {
                    return Err(format!(
                        "трафик к прокси перехватывает VPN-адаптер «{}» ({}). \
                         Отключите другой VPN и повторите подключение: его маршруты или блокировка \
                         прямых соединений мешают работе туннеля.", i.alias, i.description));
                }
            }
        }

        let tun_v4 = proxy::pick_tun_v4(&sys::local_ipv4_nets());
        let conf = proxy::singbox_config(&prof, addr, &settings, tun_v4);
        let work = paths::cache_dir().join("sing-box");
        let _ = std::fs::create_dir_all(&work);
        let conf_path = work.join("config.json");
        std::fs::write(&conf_path, serde_json::to_string_pretty(&conf).unwrap_or_default())
            .map_err(|e| format!("не удалось записать конфиг: {e}"))?;

        // Validate with the installed engine before changing firewall rules/routes.
        let check = sys::hidden(&sb_exe).arg("check").arg("-c").arg(&conf_path)
            .output().map_err(|e| format!("не удалось проверить конфиг sing-box: {e}"))?;
        if !check.status.success() {
            return Err(format!("sing-box отклонил конфигурацию: {}",
                String::from_utf8_lossy(&check.stderr).trim()));
        }
        if self.proxy_gen.load(Ordering::SeqCst) != gen {
            return Err("отменено".into());
        }
        if !reconnect && prof.kind == proxy::ProxyKind::Http {
            self.log("HTTP-прокси: TCP через туннель, DNS через DoH; остальной UDP вне исключений блокируется.", Kind::Info);
        }

        if settings.kill_switch {
            killswitch::enable(&sb_exe, settings.allow_lan).map_err(|e| format!("kill switch: {e}"))?;
            killswitch::set_proxy_ip(addr.ip()).map_err(|e| format!("kill switch: {e}"))?;
            self.st().ks_active = true;
            if !reconnect {
                self.log("Kill switch включён: трафик в обход туннеля заблокирован.", Kind::Info);
            }
        } else if killswitch::is_active() {
            let _ = killswitch::disable();
            self.st().ks_active = false;
        }

        let mut child = sys::hidden(&sb_exe)
            .arg("run")
            .arg("-c")
            .arg(&conf_path)
            .arg("-D")
            .arg(&work)
            .current_dir(&work)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("не удалось запустить sing-box: {e}"))?;
        if let Some(o) = child.stdout.take() {
            self.pipe_to_log(o, Kind::Dim, "[sing-box] ");
        }
        if let Some(e) = child.stderr.take() {
            self.pipe_to_log(e, Kind::Warn, "[sing-box] ");
        }
        *lock(&self.sb) = Some(child);

        // Wait for the TUN adapter.
        let t0 = Instant::now();
        let luid = loop {
            if self.proxy_gen.load(Ordering::SeqCst) != gen {
                return Err("отменено".into());
            }
            {
                let mut g = lock(&self.sb);
                match g.as_mut().map(|c| c.try_wait()) {
                    Some(Ok(Some(st))) => {
                        *g = None;
                        return Err(format!("sing-box завершился с кодом {} — подробности в журнале", st.code().unwrap_or(-1)));
                    }
                    None => return Err("sing-box остановлен".into()),
                    _ => {}
                }
            }
            if let Some(l) = sys::iface_luid(proxy::TUN_NAME) {
                break l;
            }
            if t0.elapsed() > Duration::from_secs(20) {
                return Err("TUN-адаптер так и не появился за 20 секунд".into());
            }
            thread::sleep(Duration::from_millis(200));
        };

        if settings.kill_switch {
            killswitch::set_tun(luid).map_err(|e| format!("kill switch: {e}"))?;
        }

        {
            let mut s = self.st();
            s.proxy = ProxyState::Connected;
            s.proxy_since = Some(Instant::now());
        }
        self.log(&format!("TUN-адаптер готов: {}. Проверяю доступ в интернет...", prof.display_name()), Kind::Info);
        self.repaint();

        // External IP through the tunnel.
        self.spawn(move |me| {
            thread::sleep(Duration::from_millis(800));
            for _ in 0..3 {
                if me.proxy_gen.load(Ordering::SeqCst) != gen {
                    return;
                }
                if let Ok(ip) = updater::get_string("https://api.ipify.org") {
                    if me.proxy_gen.load(Ordering::SeqCst) != gen {
                        return;
                    }
                    let ip = ip.trim().to_string();
                    if ip.parse::<IpAddr>().is_ok() {
                        me.log(&format!("Внешний IP через туннель: {ip}"), Kind::Info);
                        me.st().ext_ip = Some(ip);
                        me.repaint();
                        return;
                    }
                }
                thread::sleep(Duration::from_secs(2));
            }
            if me.proxy_gen.load(Ordering::SeqCst) == gen && me.st().proxy == ProxyState::Connected {
                me.log("Адаптер туннеля создан, но доступ в интернет не подтверждён. Проверьте прокси и отключите другой VPN, если он перехватывает маршруты.", Kind::Warn);
            }
        });
        Ok(())
    }

    fn on_tunnel_dropped(self: &Arc<Self>, code: i32) {
        let gen = self.proxy_gen.load(Ordering::SeqCst);
        let settings = self.proxy_settings();
        let ks = settings.kill_switch && killswitch::is_active();
        let msg = format!("sing-box неожиданно завершился (код {code})");
        {
            let mut s = self.st();
            s.proxy_since = None;
            s.ext_ip = None;
            s.proxy = if ks { ProxyState::Blocked(msg.clone()) } else { ProxyState::Failed(msg.clone()) };
        }
        if ks {
            self.log(&format!("{msg}. Kill switch блокирует интернет, пока туннель не восстановится."), Kind::Error);
        } else {
            self.log(&format!("{msg}. Трафик идёт напрямую."), Kind::Error);
        }
        if settings.auto_reconnect {
            self.schedule_reconnect(gen, 3);
        }
        self.repaint();
    }

    fn schedule_reconnect(self: &Arc<Self>, gen: u64, delay: u64) {
        if !lock(&self.proxy_cfg).auto_reconnect {
            return;
        }
        self.spawn(move |me| {
            thread::sleep(Duration::from_secs(delay));
            if me.proxy_gen.load(Ordering::SeqCst) != gen {
                return;
            }
            if !matches!(me.st().proxy, ProxyState::Blocked(_) | ProxyState::Failed(_)) {
                return;
            }
            me.log("Переподключение туннеля...", Kind::Info);
            me.connect_sync(gen, true);
        });
    }

    // ------------------------------------------------------------- shutdown

    /// Tunnel and kill switch never outlive the app; winws / TgWsProxy only when asked.
    pub fn shutdown(&self, stop_everything: bool) {
        self.proxy_gen.fetch_add(1, Ordering::SeqCst);
        let had_tunnel = lock(&self.sb).is_some() || self.st().ks_active;
        if had_tunnel {
            self.stop_singbox();
            let _ = killswitch::disable();
        }
        if stop_everything {
            self.kill_winws();
            tg::stop();
        }
        self.save_cfg();
    }
}

pub fn is_winws_running() -> bool {
    sys::processes().iter().any(|p| p.name.eq_ignore_ascii_case("winws.exe"))
}

/// Kills sing-box processes that run from our bin folder. Returns how many.
fn kill_singbox_orphans() -> usize {
    let ours = paths::singbox().to_string_lossy().to_ascii_lowercase();
    let mut n = 0;
    for p in sys::processes() {
        if p.name.eq_ignore_ascii_case("sing-box.exe") {
            let path = sys::process_path(p.pid).unwrap_or_default().to_ascii_lowercase();
            if path == ours {
                sys::kill_pid(p.pid);
                n += 1;
            }
        }
    }
    n
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

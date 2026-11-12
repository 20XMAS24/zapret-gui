//! Window, pages and tray icon.

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{
    self, vec2, Align, Align2, Color32, FontId, Frame, Layout, Margin, RichText, ScrollArea, Sense,
    Stroke, TextEdit, Ui, ViewportCommand,
};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::ctl::{Confirm, Core, ProxyState, State};
use crate::logbuf::Kind;
use crate::proxy::{self, ProxyKind};
use crate::theme::*;
use crate::zapret::{self, GameMode};
use crate::{paths, sys, tg};

pub const TITLE: &str = "ZAPRET GUI";

thread_local! {
    static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Overview,
    Strategies,
    Proxy,
    Telegram,
    Settings,
    Log,
}

const PAGES: &[(Page, &str, &str)] = &[
    (Page::Overview, "🏠", "Обзор"),
    (Page::Strategies, "⚡", "Стратегии"),
    (Page::Proxy, "🌐", "Прокси"),
    (Page::Telegram, "✈", "Telegram"),
    (Page::Settings, "⚙", "Настройки"),
    (Page::Log, "📄", "Журнал"),
];

#[derive(Clone, PartialEq)]
enum Dialog {
    RemoveService,
    ClearDiscord,
    ApplyZapret(String),
    ApplyApp(String),
    DeleteProfile(usize),
}

pub fn run(core: Arc<Core>, autostart: bool) {
    let icon = egui::IconData { rgba: crate::icon::rgba(64, None), width: 64, height: 64 };
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_inner_size([1000.0, 680.0])
            .with_min_inner_size([860.0, 560.0])
            .with_decorations(false)
            .with_icon(Arc::new(icon)),
        renderer: eframe::Renderer::Glow,
        centered: true,
        ..Default::default()
    };
    let start_hidden = autostart && core.cfg_bool("MinimizeToTray", true);
    let r = eframe::run_native(
        TITLE,
        opts,
        Box::new(move |cc| Ok(Box::new(App::new(cc, core, autostart, start_hidden)))),
    );
    if let Err(e) = r {
        sys::msgbox(&format!("Не удалось открыть окно: {e}"), true);
    }
}

struct App {
    core: Arc<Core>,
    page: Page,
    _hwnd: isize,
    first_frame: bool,
    start_hidden: bool,
    search: String,
    tg_form: Option<tg::Form>,
    tg_msg: Option<(bool, String)>,
    import: String,
    import_msg: Option<(bool, String)>,
    show_pass: bool,
    dialog: Option<Dialog>,
    tray_tip: String,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, core: Arc<Core>, autostart: bool, start_hidden: bool) -> Self {
        apply(&cc.egui_ctx);
        {
            let c = core.cfg.lock().unwrap_or_else(|e| e.into_inner());
            set_theme(&cc.egui_ctx, &c.get_or("Theme", "graphite"), &c.get_or("Accent", "blue"));
        }
        core.set_ctx(cc.egui_ctx.clone());

        let mut hwnd = 0isize;
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = cc.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    hwnd = w.hwnd.get();
                }
            }
        }
        if hwnd == 0 {
            hwnd = sys::find_window(TITLE);
        }
        sys::style_window(hwnd, c_border());

        build_tray(core.clone(), cc.egui_ctx.clone(), hwnd);
        core.init(autostart);

        App {
            core,
            page: std::env::args()
                .find_map(|a| a.strip_prefix("--page=").map(str::to_string))
                .and_then(|p| p.parse::<usize>().ok())
                .and_then(|i| PAGES.get(i).map(|x| x.0))
                .unwrap_or(Page::Overview),
            _hwnd: hwnd,
            first_frame: true,
            start_hidden,
            search: String::new(),
            tg_form: None,
            tg_msg: None,
            import: String::new(),
            import_msg: None,
            show_pass: false,
            dialog: None,
            tray_tip: String::new(),
        }
    }
}

// ----------------------------------------------------------------------- tray

fn build_tray(core: Arc<Core>, ctx: egui::Context, hwnd: isize) {
    let menu = Menu::new();
    let show = MenuItem::new("Показать окно", true, None);
    let start = MenuItem::new("Запустить обход", true, None);
    let stop = MenuItem::new("Остановить обход", true, None);
    let pconn = MenuItem::new("Подключить прокси", true, None);
    let pdisc = MenuItem::new("Отключить прокси", true, None);
    let exit_keep = MenuItem::new("Выход (обход продолжит работать)", true, None);
    let exit_all = MenuItem::new("Выход и остановить всё", true, None);
    let _ = menu.append_items(&[
        &show,
        &PredefinedMenuItem::separator(),
        &start,
        &stop,
        &PredefinedMenuItem::separator(),
        &pconn,
        &pdisc,
        &PredefinedMenuItem::separator(),
        &exit_keep,
        &exit_all,
    ]);

    let icon = tray_icon::Icon::from_rgba(crate::icon::rgba(32, None), 32, 32).ok();
    let mut b = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip(TITLE).with_menu_on_left_click(false);
    if let Some(i) = icon {
        b = b.with_icon(i);
    }
    match b.build() {
        Ok(t) => TRAY.with(|c| *c.borrow_mut() = Some(t)),
        Err(e) => core.log(&format!("Значок в трее не создан: {e}"), Kind::Warn),
    }

    let ids = (
        show.id().clone(),
        start.id().clone(),
        stop.id().clone(),
        pconn.id().clone(),
        pdisc.id().clone(),
        exit_keep.id().clone(),
        exit_all.id().clone(),
    );
    let (c2, ctx2) = (core.clone(), ctx.clone());
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let (show, start, stop, pconn, pdisc, exit_keep, exit_all) = &ids;
        if e.id == *show {
            show_window(hwnd, &ctx2);
        } else if e.id == *start {
            c2.start_bypass();
        } else if e.id == *stop {
            c2.stop_bypass();
        } else if e.id == *pconn {
            c2.proxy_connect();
        } else if e.id == *pdisc {
            c2.proxy_disconnect();
        } else if e.id == *exit_keep || e.id == *exit_all {
            c2.shutdown(e.id == *exit_all);
            TRAY.with(|c| c.borrow_mut().take());
            std::process::exit(0);
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            show_window(hwnd, &ctx);
        }
    }));
}

fn show_window(hwnd: isize, ctx: &egui::Context) {
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
    sys::show_hwnd(hwnd);
    ctx.request_repaint();
}

fn update_tray(st: &State, last: &mut String) {
    let p = match st.proxy {
        ProxyState::Connected => "вкл",
        ProxyState::Connecting => "...",
        ProxyState::Blocked(_) => "блок",
        _ => "выкл",
    };
    let tip = format!(
        "ZAPRET GUI\nОбход: {}\nПрокси: {}\nTelegram: {}",
        if st.zapret_running { "вкл" } else { "выкл" },
        p,
        if st.tg_running { "вкл" } else { "выкл" }
    );
    if *last == tip {
        return;
    }
    *last = tip.clone();
    let dot = if matches!(st.proxy, ProxyState::Blocked(_)) {
        Some([0xff, 0x6b, 0x6b])
    } else if st.proxy == ProxyState::Connected || st.zapret_running {
        Some([0x3e, 0xcf, 0x8e])
    } else {
        None
    };
    TRAY.with(|c| {
        if let Some(t) = c.borrow().as_ref() {
            let _ = t.set_tooltip(Some(tip.as_str()));
            if let Ok(i) = tray_icon::Icon::from_rgba(crate::icon::rgba(32, dot), 32, 32) {
                let _ = t.set_icon(Some(i));
            }
        }
    });
}

// ------------------------------------------------------------------------ app

impl eframe::App for App {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        c_bg().to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.first_frame {
            self.first_frame = false;
            if self.start_hidden {
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            }
            if std::env::args().any(|a| a == "--maximized") {
                ctx.send_viewport_cmd(ViewportCommand::Maximized(true)); // dev check of the frameless maximize
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) && self.core.cfg_bool("MinimizeToTray", true) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }

        let st = self.core.snapshot();
        update_tray(&st, &mut self.tray_tip);
        if let Some(Confirm::RemoveService) = st.confirm {
            self.core.st.lock().unwrap_or_else(|e| e.into_inner()).confirm = None;
            self.dialog = Some(Dialog::RemoveService);
        }
        if st.proxy == ProxyState::Connected && matches!(self.page, Page::Overview | Page::Proxy) {
            ctx.request_repaint_after(Duration::from_secs(1));
        }

        self.sidebar(ctx, &st);
        self.title_bar(ctx);
        egui::CentralPanel::default()
            .frame(Frame::none().fill(c_bg()).inner_margin(Margin { left: 24.0, right: 24.0, top: 2.0, bottom: 16.0 }))
            .show(ctx, |ui| match self.page {
                Page::Overview => self.page_overview(ui, &st),
                Page::Strategies => self.page_strategies(ui, &st),
                Page::Proxy => self.page_proxy(ui, &st),
                Page::Telegram => self.page_telegram(ui, &st),
                Page::Settings => self.page_settings(ui, &st),
                Page::Log => self.page_log(ui),
            });

        self.dialogs(ctx);
        self.whats_new(ctx, &st);
        resize_grips(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.core.shutdown(false);
        TRAY.with(|c| c.borrow_mut().take());
    }
}

fn header(ui: &mut Ui, title: &str, sub: &str) {
    ui.label(RichText::new(title).size(22.0).family(semibold()).color(c_text()));
    ui.label(RichText::new(sub).size(13.0).color(c_muted()));
    ui.add_space(10.0);
}

fn proxy_status(st: &State) -> (Color32, String) {
    match &st.proxy {
        ProxyState::Off => (c_faint(), "отключён".into()),
        ProxyState::Connecting => (c_yellow(), "подключение...".into()),
        ProxyState::Connected => (GREEN, "подключён".into()),
        ProxyState::Blocked(_) => (RED, "туннель упал · интернет заблокирован".into()),
        ProxyState::Failed(_) => (RED, "ошибка".into()),
    }
}

fn elapsed(st: &State) -> String {
    st.proxy_since
        .map(|t| {
            let s = t.elapsed().as_secs();
            format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
        })
        .unwrap_or_default()
}

impl App {
    // -------------------------------------------------------------- sidebar

    fn sidebar(&mut self, ctx: &egui::Context, st: &State) {
        egui::SidePanel::left("nav")
            .exact_width(210.0)
            .resizable(false)
            .frame(Frame::none().fill(c_sidebar()).inner_margin(Margin { left: 12.0, right: 12.0, top: 18.0, bottom: 14.0 }))
            .show(ctx, |ui| {
                let logo = ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    let (rect, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
                    let c = rect.center();
                    ui.painter().circle_filled(c, 17.0, accent());
                    ui.painter().text(c, Align2::CENTER_CENTER, "Z", FontId::new(19.0, semibold()), Color32::WHITE);
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new("ZAPRET GUI").size(15.0).family(semibold()).color(c_text()));
                        ui.label(RichText::new(format!("v{} · zapret {}", crate::updater::APP_VERSION, st.zap_local)).size(11.5).color(c_muted()));
                    });
                });
                let drag = ui.interact(logo.response.rect.expand(8.0), egui::Id::new("logo-drag"), Sense::click_and_drag());
                window_drag(ui.ctx(), &drag);
                ui.add_space(20.0);

                for (page, icon, label) in PAGES {
                    if nav_item(ui, icon, label, self.page == *page).clicked() {
                        self.page = *page;
                        if *page == Page::Telegram {
                            self.tg_form = None;
                        }
                    }
                }

                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    let (pc, _) = proxy_status(st);
                    mini_status(ui, "Telegram", if st.tg_running { GREEN } else { c_faint() });
                    mini_status(ui, "Прокси", pc);
                    mini_status(ui, "Обход", if st.zapret_running { GREEN } else { c_faint() });
                    ui.add_space(4.0);
                    ui.label(RichText::new("СОСТОЯНИЕ").size(11.0).color(c_faint()));
                });
            });
    }

    // ------------------------------------------------------------- overview

    fn page_overview(&mut self, ui: &mut Ui, st: &State) {
        header(ui, "Обзор", "Обход блокировок, системный прокси-туннель и Telegram в одном окне");

        if matches!(st.proxy, ProxyState::Blocked(_)) {
            self.blocked_banner(ui, st);
            ui.add_space(6.0);
        }

        // Fixed-height row so the cards' bottom-aligned buttons don't swallow the whole page.
        let w = ui.available_width();
        ui.allocate_ui_with_layout(vec2(w, 212.0), Layout::top_down(Align::Min), |ui| {
        ui.columns(3, |cols| {
            // zapret
            card(&mut cols[0], |ui| {
                ui.set_min_height(176.0);
                title(ui, "⚡ Обход (winws)");
                ui.add_space(2.0);
                if st.bypass_busy {
                    pill(ui, "запуск...", c_yellow());
                } else if st.zapret_running {
                    pill(ui, "работает", GREEN);
                } else {
                    pill(ui, "остановлен", c_faint());
                }
                let detail = match (&st.current_strategy, st.zapret_running) {
                    (Some(s), true) => zapret::strip_bat(s).to_string(),
                    (None, true) => "запущен вне приложения".into(),
                    _ => format!("выбрано: {}", zapret::strip_bat(&st.selected)),
                };
                ui.add(egui::Label::new(RichText::new(detail).size(13.0).color(c_muted())).truncate());
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    if st.zapret_running {
                        if big_button(ui, "Остановить", Bk::Danger, true).clicked() {
                            self.core.stop_bypass();
                        }
                    } else if big_button(ui, "Запустить", Bk::Primary, !st.bypass_busy).clicked() {
                        self.core.start_bypass();
                    }
                });
            });

            // proxy
            card(&mut cols[1], |ui| {
                ui.set_min_height(176.0);
                title(ui, "🌐 Прокси-туннель");
                ui.add_space(2.0);
                let (c, t) = proxy_status(st);
                pill(ui, &t, c);
                let ps = self.core.proxy_settings();
                let detail = match &st.proxy {
                    ProxyState::Connected => format!(
                        "{} · {}{}",
                        st.proxy_name,
                        elapsed(st),
                        st.ext_ip.as_ref().map(|i| format!(" · IP {i}")).unwrap_or_default()
                    ),
                    ProxyState::Failed(e) | ProxyState::Blocked(e) => e.clone(),
                    _ => ps.current().map(|p| p.display_name()).unwrap_or_else(|| "профиль не настроен".into()),
                };
                ui.add(egui::Label::new(RichText::new(detail).size(13.0).color(c_muted())).truncate());
                if ps.kill_switch {
                    ui.label(RichText::new("🔒 kill switch включён").size(12.0).color(c_faint()));
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    match st.proxy {
                        ProxyState::Off | ProxyState::Failed(_) => {
                            if big_button(ui, "Подключить", Bk::Primary, ps.current().is_some()).clicked() {
                                self.core.proxy_connect();
                            }
                        }
                        _ => {
                            if big_button(ui, "Отключить", Bk::Danger, true).clicked() {
                                self.core.proxy_disconnect();
                            }
                        }
                    }
                });
            });

            // telegram
            card(&mut cols[2], |ui| {
                ui.set_min_height(176.0);
                title(ui, "✈ Telegram · TgWsProxy");
                ui.add_space(2.0);
                if st.tg_running {
                    pill(ui, "работает", GREEN);
                } else {
                    pill(ui, "остановлен", c_faint());
                }
                let found = paths::find_tgwsproxy().is_some();
                ui.label(
                    RichText::new(if found { format!("версия {}", st.tg_local) } else { "TgWsProxy_windows.exe не найден".into() })
                        .size(13.0)
                        .color(c_muted()),
                );
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    if st.tg_running {
                        if big_button(ui, "Остановить", Bk::Danger, true).clicked() {
                            self.core.stop_tg();
                        }
                    } else if big_button(ui, "Запустить", Bk::Primary, found).clicked() {
                        self.core.start_tg();
                    }
                });
            });
        });
        });

        ui.add_space(6.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                title(ui, "Последние события");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("Весь журнал →").size(12.5).color(accent())).frame(false)).clicked() {
                        self.page = Page::Log;
                    }
                });
            });
            let log = self.core.log.lock().unwrap_or_else(|e| e.into_inner());
            let n = log.lines.len();
            let h = ui.available_height().max(80.0);
            ScrollArea::vertical().id_salt("ov-log").max_height(h).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                for l in log.lines.iter().skip(n.saturating_sub(60)) {
                    log_line(ui, l);
                }
            });
        });
    }

    fn blocked_banner(&mut self, ui: &mut Ui, st: &State) {
        Frame::none()
            .fill(tint(RED, 28))
            .stroke(Stroke::new(1.0_f32, tint(RED, 90)))
            .rounding(12.0)
            .inner_margin(Margin::same(14.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Kill switch: интернет заблокирован").family(semibold()).color(RED));
                        let why = if let ProxyState::Blocked(e) = &st.proxy { e.as_str() } else { "" };
                        ui.label(RichText::new(format!("{why}. Туннель переподключается автоматически, если включено.")).size(12.5).color(c_text()));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if button(ui, "Снять блокировку", Bk::Danger, true).clicked() {
                            self.core.proxy_disconnect();
                        }
                        if button(ui, "Переподключить", Bk::Ghost, true).clicked() {
                            self.core.proxy_connect();
                        }
                    });
                });
            });
    }

    // ----------------------------------------------------------- strategies

    fn page_strategies(&mut self, ui: &mut Ui, st: &State) {
        header(ui, "Стратегии", "Набор правил winws.exe из .bat-файлов zapret. Двойной клик — запустить.");
        let right_w = 300.0;
        ui.horizontal_top(|ui| {
            let left_w = ui.available_width() - right_w - 12.0;
            ui.allocate_ui_with_layout(vec2(left_w, ui.available_height()), Layout::top_down(Align::Min), |ui| {
                card(ui, |ui| {
                    ui.add(TextEdit::singleline(&mut self.search).hint_text("🔍  Поиск стратегии").desired_width(ui.available_width() - 2.0));
                    ui.add_space(4.0);
                    let q = self.search.to_lowercase();
                    let h = ui.available_height();
                    ScrollArea::vertical().id_salt("strat").max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for name in &st.strategies {
                            let label = zapret::strip_bat(name);
                            if !q.is_empty() && !label.to_lowercase().contains(&q) {
                                continue;
                            }
                            let sel = st.selected == *name;
                            let running = st.zapret_running && st.current_strategy.as_deref() == Some(name.as_str());
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                            let bg = if sel {
                                tint(accent(), 46)
                            } else if resp.hovered() {
                                c_btn()
                            } else {
                                Color32::TRANSPARENT
                            };
                            ui.painter().rect_filled(rect, 8.0, bg);
                            if sel {
                                let bar = egui::Rect::from_min_size(rect.min + vec2(0.0, 8.0), vec2(3.0, rect.height() - 16.0));
                                ui.painter().rect_filled(bar, 2.0, accent());
                            }
                            ui.painter().text(
                                rect.left_center() + vec2(14.0, 0.0),
                                Align2::LEFT_CENTER,
                                label,
                                FontId::proportional(14.0),
                                if sel { c_text() } else { c_muted() },
                            );
                            if running {
                                ui.painter().text(
                                    rect.right_center() - vec2(12.0, 0.0),
                                    Align2::RIGHT_CENTER,
                                    "● работает",
                                    FontId::proportional(12.5),
                                    GREEN,
                                );
                            }
                            if resp.clicked() {
                                self.core.select_strategy(name);
                            }
                            if resp.double_clicked() {
                                self.core.select_strategy(name);
                                if st.zapret_running {
                                    self.core.restart_bypass();
                                } else {
                                    self.core.start_bypass();
                                }
                            }
                        }
                        if st.strategies.is_empty() {
                            muted(ui, "В папке нет .bat-стратегий.");
                        }
                    });
                });
            });

            ui.allocate_ui_with_layout(vec2(right_w, ui.available_height()), Layout::top_down(Align::Min), |ui| {
                card(ui, |ui| {
                    title(ui, "Управление");
                    if st.bypass_busy {
                        pill(ui, "запуск...", c_yellow());
                    } else if st.zapret_running {
                        pill(ui, "работает", GREEN);
                    } else {
                        pill(ui, "остановлен", c_faint());
                    }
                    field_label(ui, "Выбрана");
                    ui.label(RichText::new(zapret::strip_bat(&st.selected)).family(semibold()).color(c_text()));
                    if let (true, Some(cur)) = (st.zapret_running, &st.current_strategy) {
                        if cur != &st.selected {
                            ui.label(RichText::new(format!("сейчас работает: {}", zapret::strip_bat(cur))).size(12.5).color(c_yellow()));
                        }
                    }
                    ui.add_space(4.0);
                    if st.zapret_running {
                        if big_button(ui, "Перезапустить с выбранной", Bk::Primary, !st.bypass_busy).clicked() {
                            self.core.restart_bypass();
                        }
                        if big_button(ui, "Остановить", Bk::Danger, true).clicked() {
                            self.core.stop_bypass();
                        }
                    } else if big_button(ui, "Запустить", Bk::Primary, !st.bypass_busy && !st.selected.is_empty()).clicked() {
                        self.core.start_bypass();
                    }
                });
                ui.add_space(4.0);
                card(ui, |ui| {
                    title(ui, "Игровой фильтр");
                    muted(ui, "Расширяет обход на порты 1024–65535 (игры, голос). Применяется при перезапуске.");
                    let mut g = st.game;
                    egui::ComboBox::from_id_salt("game")
                        .selected_text(g.label())
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            for m in GameMode::ALL {
                                ui.selectable_value(&mut g, m, m.label());
                            }
                        });
                    if g != st.game {
                        self.core.set_game(g);
                    }
                });
                ui.add_space(4.0);
                card(ui, |ui| {
                    title(ui, "Подбор");
                    muted(ui, "Штатный скрипт проверит все стратегии на ваших сайтах.");
                    if big_button(ui, "Тесты стратегий", Bk::Ghost, true).clicked() {
                        self.core.run_tests();
                    }
                    if big_button(ui, "Обновить список", Bk::Ghost, true).clicked() {
                        self.core.reload_strategies();
                    }
                });
            });
        });
    }

    // ---------------------------------------------------------------- proxy

    fn page_proxy(&mut self, ui: &mut Ui, st: &State) {
        header(ui, "Прокси-туннель", "Весь трафик системы через SOCKS5/HTTP-прокси (TUN-адаптер sing-box) с kill switch");

        if matches!(st.proxy, ProxyState::Blocked(_)) {
            self.blocked_banner(ui, st);
            ui.add_space(6.0);
        }

        let orig = self.core.proxy_settings();
        let mut ps = orig.clone();
        let busy = matches!(st.proxy, ProxyState::Connecting);
        let live = !matches!(st.proxy, ProxyState::Off | ProxyState::Failed(_));

        // Connection card
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let (c, t) = proxy_status(st);
                    ui.horizontal(|ui| {
                        title(ui, "Соединение");
                        pill(ui, &t, c);
                        if st.ks_active {
                            pill(ui, "kill switch активен", accent());
                        }
                    });
                    let line = match &st.proxy {
                        ProxyState::Connected => format!(
                            "{} · {} · внешний IP: {}",
                            st.proxy_name,
                            elapsed(st),
                            st.ext_ip.clone().unwrap_or_else(|| "проверяется...".into())
                        ),
                        ProxyState::Connecting => format!("{} · поднимаю TUN-адаптер...", st.proxy_name),
                        ProxyState::Failed(e) | ProxyState::Blocked(e) => e.clone(),
                        ProxyState::Off => match ps.current() {
                            Some(p) => format!("будет использован: {}", p.display_name()),
                            None => "добавьте прокси ниже".into(),
                        },
                    };
                    ui.add(egui::Label::new(RichText::new(line).size(13.0).color(c_muted())).wrap());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if live {
                        if ui.add(egui::Button::new(RichText::new("  Отключить  ").color(RED).size(15.0))
                            .fill(tint(RED, 34)).stroke(Stroke::new(1.0_f32, tint(RED, 70))).rounding(10.0).min_size(vec2(150.0, 42.0))).clicked()
                        {
                            self.core.proxy_disconnect();
                        }
                    } else if ui
                        .add_enabled(
                            ps.current().is_some() && !busy,
                            egui::Button::new(RichText::new("  Подключить  ").color(Color32::WHITE).size(15.0))
                                .fill(accent()).rounding(10.0).min_size(vec2(150.0, 42.0)),
                        )
                        .clicked()
                    {
                        self.core.proxy_connect();
                    }
                });
            });
            if !paths::singbox().is_file() {
                ui.label(RichText::new("bin\\sing-box.exe не найден — положите его рядом с winws.exe.").size(12.5).color(RED));
            }
        });
        ui.add_space(4.0);

        ScrollArea::vertical().id_salt("proxy-page").auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                // Profiles list
                ui.allocate_ui_with_layout(vec2(240.0, 0.0), Layout::top_down(Align::Min), |ui| {
                    card(ui, |ui| {
                        title(ui, "Профили");
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let mut select = None;
                        for (i, p) in ps.profiles.iter().enumerate() {
                            let sel = i == ps.selected;
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
                            let bg = if sel { tint(accent(), 46) } else if resp.hovered() { c_btn() } else { Color32::TRANSPARENT };
                            ui.painter().rect_filled(rect, 8.0, bg);
                            ui.painter().text(rect.left_top() + vec2(12.0, 7.0), Align2::LEFT_TOP, p.display_name(), FontId::proportional(14.0), c_text());
                            ui.painter().text(
                                rect.left_top() + vec2(12.0, 25.0),
                                Align2::LEFT_TOP,
                                format!("{} · {}:{}", p.kind.label(), p.host.trim(), p.port.trim()),
                                FontId::proportional(11.5),
                                c_muted(),
                            );
                            if resp.clicked() {
                                select = Some(i);
                            }
                        }
                        if let Some(i) = select {
                            ps.selected = i;
                            self.core.st.lock().unwrap_or_else(|e| e.into_inner()).proxy_test = None;
                        }
                        if ps.profiles.is_empty() {
                            muted(ui, "Пока пусто.");
                        }
                        ui.add_space(6.0);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        if big_button(ui, "+  Добавить", Bk::Ghost, true).clicked() {
                            ps.profiles.push(proxy::Profile { port: "1080".into(), ..Default::default() });
                            ps.selected = ps.profiles.len() - 1;
                        }
                        if !ps.profiles.is_empty() && big_button(ui, "Удалить", Bk::Danger, !live).clicked() {
                            self.dialog = Some(Dialog::DeleteProfile(ps.selected));
                        }
                    });
                });

                // Editor + options
                ui.vertical(|ui| {
                    if let Some(p) = ps.profiles.get_mut(ps.selected) {
                        card(ui, |ui| {
                            title(ui, "Параметры прокси");
                            if live {
                                ui.label(RichText::new("Изменения применятся после переподключения.").size(12.0).color(c_yellow()));
                            }
                            {
                                form_row(ui, 90.0, "Название", |ui| {
                                    ui.add(TextEdit::singleline(&mut p.name).hint_text("например, Работа").desired_width(ui.available_width() - 2.0));
                                });
                                form_row(ui, 90.0, "Тип", |ui| {
                                    ui.horizontal(|ui| {
                                        for k in [ProxyKind::Socks5, ProxyKind::Http] {
                                            if ui.selectable_label(p.kind == k, RichText::new(k.label()).size(13.5)).clicked() {
                                                p.kind = k;
                                            }
                                        }
                                    });
                                });
                                form_row(ui, 90.0, "Сервер", |ui| {
                                    ui.horizontal(|ui| {
                                        let w = ui.available_width() - 110.0;
                                        ui.add(TextEdit::singleline(&mut p.host).hint_text("адрес или IP").desired_width(w));
                                        ui.label(RichText::new(":").color(c_muted()));
                                        ui.add(TextEdit::singleline(&mut p.port).hint_text("порт").desired_width(ui.available_width()));
                                    });
                                });
                                form_row(ui, 90.0, "Логин", |ui| {
                                    ui.add(TextEdit::singleline(&mut p.username).hint_text("необязательно").desired_width(ui.available_width() - 2.0));
                                });
                                form_row(ui, 90.0, "Пароль", |ui| {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if ui.add_sized([32.0, 28.0], egui::Button::new(if self.show_pass { "🙈" } else { "👁" })).clicked() {
                                            self.show_pass = !self.show_pass;
                                        }
                                        ui.add(TextEdit::singleline(&mut p.password).password(!self.show_pass).hint_text("необязательно").desired_width(ui.available_width()));
                                    });
                                });
                            }
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if button(ui, if st.testing { "Проверка..." } else { "Проверить прокси" }, Bk::Ghost, !st.testing).clicked() {
                                    self.core.proxy_test(p.clone());
                                }
                                if let Some((ok, msg)) = &st.proxy_test {
                                    ui.label(RichText::new(msg).size(13.0).color(if *ok { GREEN } else { RED }));
                                }
                            });
                            ui.label(RichText::new("Пароль хранится зашифрованным (DPAPI, только для вашей учётной записи Windows).").size(11.5).color(c_faint()));
                        });
                        ui.add_space(4.0);
                    }

                    card(ui, |ui| {
                        title(ui, "Импорт строкой");
                        muted(ui, "socks5://логин:пароль@хост:порт, http://хост:порт или хост:порт:логин:пароль");
                        ui.horizontal(|ui| {
                            let w = ui.available_width() - 128.0;
                            ui.add(TextEdit::singleline(&mut self.import).hint_text("вставьте строку прокси").desired_width(w));
                            if button(ui, "Добавить", Bk::Ghost, !self.import.trim().is_empty()).clicked() {
                                match proxy::parse_line(&self.import) {
                                    Some(p) => {
                                        let name = p.display_name();
                                        ps.profiles.push(p);
                                        ps.selected = ps.profiles.len() - 1;
                                        self.import.clear();
                                        self.import_msg = Some((true, format!("Добавлен {name}")));
                                    }
                                    None => self.import_msg = Some((false, "Не удалось разобрать строку".into())),
                                }
                            }
                        });
                        if let Some((ok, m)) = &self.import_msg {
                            ui.label(RichText::new(m).size(12.5).color(if *ok { GREEN } else { RED }));
                        }
                    });
                    ui.add_space(4.0);

                    card(ui, |ui| {
                        title(ui, "Защита и маршрутизация");
                        toggle_row(ui, &mut ps.kill_switch, "Kill switch",
                            "Если туннель упадёт, интернет блокируется, а не идёт напрямую (Windows Filtering Platform).");
                        toggle_row(ui, &mut ps.auto_reconnect, "Переподключаться автоматически", "Поднимать туннель заново, если sing-box завершился.");
                        toggle_row(ui, &mut ps.allow_lan, "Локальная сеть напрямую", "Принтеры, роутер, NAS, общие папки — в обход прокси.");
                        toggle_row(ui, &mut ps.block_quic, "Блокировать QUIC (UDP 443)",
                            "Браузеры сразу перейдут на TCP. Нужно для HTTP-прокси — он не передаёт UDP.");
                        toggle_row(ui, &mut ps.connect_on_launch, "Подключать при запуске приложения", "");
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            field_label(ui, "DNS через прокси (DoH)");
                            let cur = proxy::DNS_CHOICES.iter().find(|d| d.0 == ps.dns).map(|d| d.1).unwrap_or(ps.dns.as_str()).to_string();
                            egui::ComboBox::from_id_salt("dns").selected_text(cur).width(200.0).show_ui(ui, |ui| {
                                for (v, l) in proxy::DNS_CHOICES {
                                    ui.selectable_value(&mut ps.dns, v.to_string(), *l);
                                }
                            });
                        });
                    });
                    ui.add_space(4.0);

                    card(ui, |ui| {
                        title(ui, "Исключения (напрямую, мимо прокси)");
                        muted(ui, "Через запятую или с новой строки. Прямой трафик по-прежнему проходит через обход zapret, если он запущен.");
                        ui.columns(2, |c| {
                            field_label(&mut c[0], "Программы (имя .exe)");
                            c[0].add(TextEdit::multiline(&mut ps.bypass_processes).hint_text("Discord.exe\nsteam.exe").desired_rows(4).desired_width(f32::INFINITY));
                            field_label(&mut c[1], "Домены (вместе с поддоменами)");
                            c[1].add(TextEdit::multiline(&mut ps.bypass_domains).hint_text("gosuslugi.ru\nsber.ru").desired_rows(4).desired_width(f32::INFINITY));
                        });
                    });
                });
            });
        });

        if ps != orig {
            if ps.selected >= ps.profiles.len() {
                ps.selected = ps.profiles.len().saturating_sub(1);
            }
            self.core.save_proxy(ps);
        }
    }

    // ------------------------------------------------------------- telegram

    fn page_telegram(&mut self, ui: &mut Ui, st: &State) {
        header(ui, "Telegram", "TgWsProxy — локальный MTProto-прокси для Telegram через WebSocket");
        let found = paths::find_tgwsproxy().is_some();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        title(ui, "TgWsProxy");
                        if st.tg_running {
                            pill(ui, "работает", GREEN);
                        } else {
                            pill(ui, "остановлен", c_faint());
                        }
                    });
                    muted(ui, if found { "Настройки ниже правят %APPDATA%\\TgWsProxy\\config.json" } else { "TgWsProxy_windows.exe не найден в папке zapret" });
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if st.tg_running {
                        if button(ui, "Остановить", Bk::Danger, true).clicked() {
                            self.core.stop_tg();
                        }
                    } else if button(ui, "Запустить", Bk::Primary, found).clicked() {
                        self.core.start_tg();
                    }
                    if button(ui, "Открыть в Telegram", Bk::Ghost, true).clicked() {
                        sys::shell_open(&tg::telegram_link());
                    }
                    if button(ui, "Папка конфига", Bk::Ghost, true).clicked() {
                        let _ = std::fs::create_dir_all(tg::dir());
                        sys::shell_open(&tg::dir().display().to_string());
                    }
                });
            });
            let mut on = self.core.cfg_bool("StartTgOnLaunch", false);
            if toggle_row(ui, &mut on, "Запускать вместе с обходом", "TgWsProxy стартует, когда запускается стратегия.") {
                self.core.set_cfg_bool("StartTgOnLaunch", on);
            }
        });
        ui.add_space(4.0);

        if self.tg_form.is_none() {
            self.tg_form = Some(tg::Form::load());
        }
        let mut save = false;
        let mut reload = false;
        ScrollArea::vertical().id_salt("tg").auto_shrink([false, false]).show(ui, |ui| {
            let Some(f) = self.tg_form.as_mut() else { return };
            ui.columns(2, |c| {
                card(&mut c[0], |ui| {
                    title(ui, "Подключение");
                    {
                        form_row(ui, 120.0, "Адрес", |ui| {
                            ui.add(TextEdit::singleline(&mut f.host).desired_width(ui.available_width() - 2.0));
                        });
                        form_row(ui, 120.0, "Порт", |ui| {
                            ui.add(TextEdit::singleline(&mut f.port).desired_width(ui.available_width() - 2.0));
                        });
                        form_row(ui, 120.0, "Секрет", |ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.add_sized([32.0, 28.0], egui::Button::new("🎲")).on_hover_text("Сгенерировать").clicked() {
                                    f.secret = tg::random_secret();
                                }
                                ui.add(TextEdit::singleline(&mut f.secret).desired_width(ui.available_width()).font(egui::TextStyle::Monospace));
                            });
                        });
                        form_row(ui, 120.0, "IP дата-центров", |ui| {
                            ui.add(TextEdit::singleline(&mut f.dc).desired_width(ui.available_width() - 2.0));
                        });
                    }
                    muted(ui, "через запятую, «номер:адрес», например 2:149.154.167.220");
                });
                card(&mut c[1], |ui| {
                    title(ui, "Cloudflare");
                    toggle_row(ui, &mut f.cfproxy, "Использовать Cloudflare-прокси", "");
                    {
                        form_row(ui, 140.0, "Домен пользователя", |ui| {
                            ui.add(TextEdit::singleline(&mut f.cf_user).desired_width(ui.available_width() - 2.0));
                        });
                        form_row(ui, 140.0, "Домен worker'а", |ui| {
                            ui.add(TextEdit::singleline(&mut f.cf_worker).desired_width(ui.available_width() - 2.0));
                        });
                    }
                });
            });
            ui.add_space(4.0);
            card(ui, |ui| {
                title(ui, "Дополнительно");
                ui.columns(2, |c| {
                    { let ui = &mut c[0];
                        form_row(ui, 150.0, "Размер буфера, КБ", |ui| {
                            ui.add(TextEdit::singleline(&mut f.buf_kb).desired_width(ui.available_width() - 2.0));
                        });
                        form_row(ui, 150.0, "Пул соединений", |ui| {
                            ui.add(TextEdit::singleline(&mut f.pool).desired_width(ui.available_width() - 2.0));
                        });
                        form_row(ui, 150.0, "Макс. размер лога, МБ", |ui| {
                            ui.add(TextEdit::singleline(&mut f.log_mb).desired_width(ui.available_width() - 2.0));
                        });
                        form_row(ui, 150.0, "Тема окон", |ui| {
                            egui::ComboBox::from_id_salt("tgtheme").selected_text(f.appearance.clone()).show_ui(ui, |ui| {
                                for a in ["auto", "light", "dark"] {
                                    ui.selectable_value(&mut f.appearance, a.to_string(), a);
                                }
                            });
                        });
                    }
                    let ui = &mut c[1];
                    toggle_row(ui, &mut f.verbose, "Подробный лог", "verbose");
                    toggle_row(ui, &mut f.test_dc, "Тестовые дата-центры", "force_test_dc");
                    if tg::tray_supported() {
                        toggle_row(ui, &mut f.hide_tray, "Прятать значок в скрытые", "Под стрелочку «Показать скрытые значки»; после перезапуска TgWsProxy.");
                    } else {
                        muted(ui, "Windows 10: перетащите значок TgWsProxy под стрелочку в трее мышкой — система запомнит.");
                    }
                });
                muted(ui, "Автозапуск и проверку обновлений TgWsProxy выполняет ZapretGUI — в его конфиге они выключаются.");
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if button(ui, "Сохранить", Bk::Primary, true).clicked() {
                    save = true;
                }
                if button(ui, "Отменить изменения", Bk::Ghost, true).clicked() {
                    reload = true;
                }
                if let Some((ok, m)) = &self.tg_msg {
                    ui.label(RichText::new(m).size(13.0).color(if *ok { GREEN } else { RED }));
                }
            });
        });

        if reload {
            self.tg_form = None;
            self.tg_msg = None;
        }
        if save {
            if let Some(f) = self.tg_form.clone() {
                match f.save() {
                    Ok(_) => {
                        self.core.log("Настройки TgWsProxy сохранены.", Kind::Ok);
                        self.tg_msg = Some((true, "Сохранено".into()));
                        if tg::tray_supported() && f.hide_tray != tg::tray_is_hidden("TgWsProxy") {
                            match tg::tray_set_promoted("TgWsProxy", !f.hide_tray) {
                                Ok(_) => self.core.log(
                                    if f.hide_tray { "Значок TgWsProxy переведён в скрытые значки." } else { "Значок TgWsProxy снова показывается в трее." },
                                    Kind::Ok,
                                ),
                                Err(e) => self.core.log(&e, Kind::Warn),
                            }
                        }
                        // A running TgWsProxy keeps the old secret/port until restarted, and the
                        // "Открыть в Telegram" link already uses the new ones: restart right away.
                        if st.tg_running {
                            self.core.restart_tg();
                            self.tg_msg = Some((true, "Сохранено, TgWsProxy перезапущен. Нажмите «Открыть в Telegram», чтобы обновить прокси в Telegram.".into()));
                        }
                    }
                    Err(e) => self.tg_msg = Some((false, e)),
                }
            }
        }
    }

    // ------------------------------------------------------------- settings

    fn page_settings(&mut self, ui: &mut Ui, st: &State) {
        header(ui, "Настройки", "Оформление, запуск, обновления и инструменты");
        ScrollArea::vertical().id_salt("settings").auto_shrink([false, false]).show(ui, |ui| {
            self.appearance_card(ui);
            ui.add_space(4.0);
            ui.columns(2, |c| {
                card(&mut c[0], |ui| {
                    title(ui, "Запуск");
                    let mut a = st.autostart;
                    if toggle_row(ui, &mut a, "Запускать вместе с Windows", "Задача планировщика с правами администратора, без запроса UAC.") {
                        self.core.set_autostart(a);
                    }
                    for (key, def, label, hint) in [
                        ("StartBypassOnLaunch", false, "Включать обход при запуске", "Сразу запускать выбранную стратегию."),
                        ("MinimizeToTray", true, "Сворачивать в трей", "Крестик прячет окно, приложение остаётся в трее."),
                        ("CheckUpdates", true, "Проверять обновления при запуске", "zapret и TgWsProxy с GitHub."),
                        ("RedirectWinwsOutput", true, "Вывод winws.exe в журнал", "Сообщения winws появятся во вкладке «Журнал»."),
                    ] {
                        let mut v = self.core.cfg_bool(key, def);
                        if toggle_row(ui, &mut v, label, hint) {
                            self.core.set_cfg_bool(key, v);
                        }
                    }
                });

                card(&mut c[1], |ui| {
                    title(ui, "Обновления");
                    let mut auto = self.core.auto_update();
                    if toggle_row(ui, &mut auto, "Обновлять автоматически", "ZapretGUI, zapret и TgWsProxy ставятся сами; проверка при запуске и каждые 6 часов.") {
                        self.core.set_cfg_bool("AutoUpdate", auto);
                    }
                    let ver_line = |ui: &mut Ui, name: &str, local: &str, remote: &Option<String>| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).color(c_text()));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if let Some(r) = remote {
                                    ui.label(RichText::new(format!("на GitHub {r}")).size(12.5).color(c_muted()));
                                }
                                ui.label(RichText::new(local).family(semibold()).color(c_text()));
                            });
                        });
                    };
                    ver_line(ui, "ZapretGUI", crate::updater::APP_VERSION, &st.app_remote);
                    ver_line(ui, "zapret", &st.zap_local, &st.zap_remote);
                    ver_line(ui, "TgWsProxy", &st.tg_local, &st.tg_remote);
                    if let Some(p) = st.progress {
                        ui.add(egui::ProgressBar::new(p).desired_height(6.0).fill(accent()));
                    }
                    ui.add_space(4.0);
                    if big_button(ui, if st.checking { "Проверка..." } else { "Проверить обновления" }, Bk::Ghost, !st.checking).clicked() {
                        self.core.check_updates(true);
                    }
                    if let Some((v, _)) = &st.pending_app {
                        if big_button(ui, &format!("Обновить ZapretGUI до {v}"), Bk::Success, !st.updating).clicked() {
                            self.dialog = Some(Dialog::ApplyApp(v.clone()));
                        }
                    }
                    if let Some((v, _)) = &st.pending_zap {
                        if big_button(ui, &format!("Обновить zapret до {v}"), Bk::Success, !st.updating).clicked() {
                            self.dialog = Some(Dialog::ApplyZapret(v.clone()));
                        }
                    }
                    if let Some((v, _)) = &st.pending_tg {
                        if big_button(ui, &format!("Обновить TgWsProxy до {v}"), Bk::Success, !st.updating).clicked() {
                            self.core.apply_tg_update();
                        }
                    }
                    muted(ui, "Ваши списки (*-user.txt, ipset-all.txt) и настройки при обновлении не перезаписываются, копия старых файлов — в _backup.");
                });
            });
            ui.add_space(4.0);
            card(ui, |ui| {
                title(ui, "Инструменты");
                ui.horizontal_wrapped(|ui| {
                    if button(ui, "Диагностика", Bk::Ghost, true).clicked() {
                        self.core.run_diag();
                        self.page = Page::Log;
                    }
                    if button(ui, "Тесты стратегий", Bk::Ghost, true).clicked() {
                        self.core.run_tests();
                    }
                    if button(ui, "Очистить кэш Discord", Bk::Ghost, true).clicked() {
                        self.dialog = Some(Dialog::ClearDiscord);
                    }
                    if button(ui, "Открыть папку zapret", Bk::Ghost, true).clicked() {
                        sys::shell_open(&paths::root().display().to_string());
                    }
                    if button(ui, "Списки доменов", Bk::Ghost, true).clicked() {
                        sys::shell_open(&paths::lists().display().to_string());
                    }
                });
            });
        });
    }

    // ------------------------------------------------------------------ log

    fn page_log(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Журнал").size(22.0).family(semibold()).color(c_text()));
                ui.label(RichText::new("События приложения, вывод winws.exe и sing-box").size(13.0).color(c_muted()));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, "Очистить", Bk::Ghost, true).clicked() {
                    self.core.log.lock().unwrap_or_else(|e| e.into_inner()).clear();
                }
                if button(ui, "Копировать", Bk::Ghost, true).clicked() {
                    let t = self.core.log.lock().unwrap_or_else(|e| e.into_inner()).as_text();
                    ui.ctx().copy_text(t);
                }
            });
        });
        ui.add_space(10.0);
        Frame::none()
            .fill(c_input())
            .stroke(Stroke::new(1.0_f32, c_border()))
            .rounding(12.0)
            .inner_margin(Margin::same(12.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let h = ui.available_height();
                let log = self.core.log.lock().unwrap_or_else(|e| e.into_inner());
                ScrollArea::vertical()
                    .id_salt("log")
                    .max_height(h)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for l in &log.lines {
                            log_line(ui, l);
                        }
                    });
            });
    }

    // ------------------------------------------------------------ title bar

    /// Own title bar in the content color: the window has no OS frame, so nothing gray on top.
    fn title_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("titlebar")
            .exact_height(36.0)
            .show_separator_line(false)
            .frame(Frame::none().fill(c_bg()))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                let bw = 46.0;
                let close = egui::Rect::from_min_size(egui::pos2(rect.right() - bw, rect.top()), vec2(bw, rect.height()));
                let max = close.translate(vec2(-bw, 0.0));
                let min = max.translate(vec2(-bw, 0.0));
                let drag_rect = egui::Rect::from_min_max(rect.min, egui::pos2(min.left(), rect.bottom()));
                let drag = ui.interact(drag_rect, egui::Id::new("titlebar-drag"), Sense::click_and_drag());
                window_drag(ctx, &drag);

                if win_button(ui, min, WinBtn::Minimize).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                }
                if win_button(ui, max, if maximized { WinBtn::Restore } else { WinBtn::Maximize }).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                }
                if win_button(ui, close, WinBtn::Close).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            });
    }

    // ----------------------------------------------------------- appearance

    fn appearance_card(&mut self, ui: &mut Ui) {
        let (cur_theme, cur_accent) = {
            let c = self.core.cfg.lock().unwrap_or_else(|e| e.into_inner());
            (c.get_or("Theme", "graphite"), c.get_or("Accent", "blue"))
        };
        let (mut theme, mut acc) = (cur_theme.clone(), cur_accent.clone());
        card(ui, |ui| {
            title(ui, "Оформление");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for (id, name, p) in THEMES {
                    let sel = theme_index(&theme) == theme_index(id);
                    let (rect, resp) = ui.allocate_exact_size(vec2(112.0, 92.0), Sense::click());
                    let prev = egui::Rect::from_min_size(rect.min, vec2(112.0, 66.0));
                    let pt = ui.painter();
                    pt.rect_filled(prev, 9.0, p.bg);
                    pt.rect_filled(
                        egui::Rect::from_min_size(prev.min, vec2(30.0, prev.height())),
                        egui::Rounding { nw: 9.0, sw: 9.0, ne: 0.0, se: 0.0 },
                        p.sidebar,
                    );
                    pt.rect_filled(egui::Rect::from_min_size(prev.min + vec2(6.0, 10.0), vec2(18.0, 5.0)), 2.0, accent());
                    pt.rect_filled(egui::Rect::from_min_size(prev.min + vec2(6.0, 20.0), vec2(18.0, 4.0)), 2.0, p.faint);
                    pt.rect_filled(egui::Rect::from_min_size(prev.min + vec2(40.0, 12.0), vec2(62.0, 42.0)), 6.0, p.card);
                    pt.rect_filled(egui::Rect::from_min_size(prev.min + vec2(48.0, 20.0), vec2(36.0, 5.0)), 2.0, p.text);
                    pt.rect_filled(egui::Rect::from_min_size(prev.min + vec2(48.0, 31.0), vec2(26.0, 4.0)), 2.0, p.muted);
                    pt.rect_filled(egui::Rect::from_min_size(prev.min + vec2(48.0, 41.0), vec2(30.0, 7.0)), 3.0, accent());
                    let stroke = if sel {
                        Stroke::new(2.0_f32, accent())
                    } else if resp.hovered() {
                        Stroke::new(1.0_f32, c_muted())
                    } else {
                        Stroke::new(1.0_f32, c_border())
                    };
                    pt.rect_stroke(prev, 9.0, stroke);
                    pt.text(
                        egui::pos2(rect.center().x, rect.bottom() - 10.0),
                        Align2::CENTER_CENTER,
                        *name,
                        FontId::proportional(13.0),
                        if sel { c_text() } else { c_muted() },
                    );
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        theme = id.to_string();
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                field_label(ui, "Акцент");
                ui.add_space(8.0);
                for (id, name, col) in ACCENTS {
                    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                    let sel = accent_index(&acc) == accent_index(id);
                    if sel {
                        ui.painter().circle_stroke(rect.center(), 12.0, Stroke::new(2.0_f32, *col));
                    }
                    ui.painter().circle_filled(rect.center(), if sel { 8.0 } else { 10.0 }, *col);
                    if resp.on_hover_text(*name).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        acc = id.to_string();
                    }
                }
            });
        });
        if theme != cur_theme || acc != cur_accent {
            {
                let mut c = self.core.cfg.lock().unwrap_or_else(|e| e.into_inner());
                c.set("Theme", theme.clone());
                c.set("Accent", acc.clone());
                c.save();
            }
            set_theme(ui.ctx(), &theme, &acc);
            sys::style_window(self._hwnd, c_border());
        }
    }

    // ------------------------------------------------------------ what's new

    fn whats_new(&mut self, ctx: &egui::Context, st: &State) {
        let Some((ver, notes)) = &st.whats_new else { return };
        if self.dialog.is_some() {
            return;
        }
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("wn-dim")).order(egui::Order::Middle).fixed_pos(screen.min).show(ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(150));
        });
        let mut close = false;
        egui::Window::new("whatsnew")
            .id(egui::Id::new("whatsnew"))
            .order(egui::Order::Foreground)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .min_width(480.0).max_width(480.0)
            .frame(Frame::window(&ctx.style()).inner_margin(Margin::same(22.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 18.0, tint(accent(), 40));
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, "✨", FontId::proportional(18.0), accent());
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new("ZapretGUI обновлён").size(18.0).family(semibold()).color(c_text()));
                        ui.label(RichText::new(format!("Версия {ver} — что нового")).size(13.0).color(c_muted()));
                    });
                });
                ui.add_space(10.0);
                ScrollArea::vertical().max_height(340.0).auto_shrink([false, true]).show(ui, |ui| {
                    render_notes(ui, notes);
                });
                ui.add_space(12.0);
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 34.0), Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Отлично", Bk::Primary, true).clicked() {
                        close = true;
                    }
                    if ui.add(egui::Button::new(RichText::new("Все релизы").size(13.0).color(accent())).frame(false)).clicked() {
                        sys::shell_open(&format!("https://github.com/{}/releases", self.core.update_repo()));
                    }
                });
            });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.core.dismiss_whats_new();
        }
    }

    // -------------------------------------------------------------- dialogs

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(d) = self.dialog.clone() else { return };
        let (title_s, body, yes, danger) = match &d {
            Dialog::RemoveService => (
                "Служба zapret",
                "Служба «zapret» уже запущена и перехватывает трафик.\nОстановить и удалить её, чтобы запустить стратегию из этого приложения?".to_string(),
                "Удалить службу и запустить",
                true,
            ),
            Dialog::ClearDiscord => ("Кэш Discord", "Discord будет закрыт, его кэш удалён. Продолжить?".to_string(), "Очистить", true),
            Dialog::ApplyZapret(v) => (
                "Обновление zapret",
                format!("Обновить zapret до {v}?\n\nОбход будет остановлен, файлы заменены, ваши списки сохранены.\nРезервная копия ляжет в папку _backup."),
                "Обновить",
                false,
            ),
            Dialog::ApplyApp(v) => (
                "Обновление ZapretGUI",
                format!("Установить ZapretGUI {v}?

Обход, прокси и TgWsProxy будут остановлены, приложение закроется, установится новая версия и запустится снова — всё, что работало, включится обратно."),
                "Обновить",
                false,
            ),
            Dialog::DeleteProfile(i) => {
                let name = self.core.proxy_settings().profiles.get(*i).map(|p| p.display_name()).unwrap_or_default();
                ("Удалить профиль", format!("Удалить прокси-профиль «{name}»?"), "Удалить", true)
            }
        };

        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("dim"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (r, _) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(150));
            });

        let mut result: Option<bool> = None;
        egui::Window::new(title_s)
            .id(egui::Id::new("dialog"))
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .min_width(420.0).max_width(420.0)
            .frame(Frame::window(&ctx.style()).inner_margin(Margin::same(20.0)))
            .show(ctx, |ui| {
                ui.label(RichText::new(title_s).size(17.0).family(semibold()).color(c_text()));
                ui.add_space(4.0);
                ui.label(RichText::new(body).color(c_text()));
                ui.add_space(14.0);
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 34.0), Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, yes, if danger { Bk::Danger } else { Bk::Primary }, true).clicked() {
                        result = Some(true);
                    }
                    if button(ui, "Отмена", Bk::Ghost, true).clicked() {
                        result = Some(false);
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            result = Some(false);
        }

        if let Some(ok) = result {
            self.dialog = None;
            if ok {
                match d {
                    Dialog::RemoveService => self.core.start_bypass_remove_service(),
                    Dialog::ClearDiscord => {
                        self.core.clear_discord();
                        self.page = Page::Log;
                    }
                    Dialog::ApplyZapret(_) => self.core.apply_zapret_update(),
                    Dialog::ApplyApp(_) => self.core.apply_app_update(),
                    Dialog::DeleteProfile(i) => {
                        let mut ps = self.core.proxy_settings();
                        if i < ps.profiles.len() {
                            ps.profiles.remove(i);
                            ps.selected = ps.selected.min(ps.profiles.len().saturating_sub(1));
                            self.core.save_proxy(ps);
                        }
                    }
                }
            }
        }
    }
}

// --------------------------------------------------------------------- bits

fn nav_item(ui: &mut Ui, icon: &str, label: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    let bg = if selected {
        tint(accent(), 40)
    } else if resp.hovered() {
        c_btn()
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 9.0, bg);
    if selected {
        let bar = egui::Rect::from_min_size(rect.min + vec2(0.0, 9.0), vec2(3.0, rect.height() - 18.0));
        ui.painter().rect_filled(bar, 2.0, accent());
    }
    let fg = if selected { c_text() } else { c_muted() };
    ui.painter().text(rect.left_center() + vec2(16.0, 0.0), Align2::LEFT_CENTER, icon, FontId::proportional(15.0), if selected { accent() } else { fg });
    ui.painter().text(rect.left_center() + vec2(42.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(14.5), fg);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn mini_status(ui: &mut Ui, label: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        dot(ui, color);
        ui.label(RichText::new(label).size(13.0).color(if color == c_faint() { c_muted() } else { c_text() }));
    });
}

fn log_line(ui: &mut Ui, l: &crate::logbuf::Line) {
    let c = match l.kind {
        Kind::Ok => GREEN,
        Kind::Warn => c_yellow(),
        Kind::Error => RED,
        Kind::Dim => c_faint(),
        Kind::Head => accent(),
        Kind::Info => c_text(),
    };
    let mut job = egui::text::LayoutJob::default();
    let font = |s: f32| FontId::monospace(s);
    job.append(&l.time, 0.0, egui::TextFormat { font_id: font(12.0), color: c_faint(), ..Default::default() });
    job.append(&l.text, 12.0, egui::TextFormat { font_id: font(12.5), color: c, ..Default::default() });
    job.wrap.max_width = ui.available_width();
    ui.add(egui::Label::new(job));
}

fn form_row(ui: &mut Ui, label_w: f32, label: &str, add: impl FnOnce(&mut Ui)) {

    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(label_w, 28.0), Sense::hover());
        ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(12.5), c_muted());
        add(ui);
    });
}

// ------------------------------------------------------------- window chrome

fn window_drag(ctx: &egui::Context, r: &egui::Response) {
    if r.double_clicked() {
        let m = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!m));
    } else if r.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum WinBtn {
    Minimize,
    Maximize,
    Restore,
    Close,
}

fn win_button(ui: &mut Ui, rect: egui::Rect, kind: WinBtn) -> egui::Response {
    let resp = ui.interact(rect, egui::Id::new(("winbtn", kind as u8)), Sense::click());
    let hot = resp.hovered();
    let close = kind == WinBtn::Close;
    if hot {
        let fill = if close { Color32::from_rgb(0xc4, 0x2b, 0x1c) } else { c_btn() };
        ui.painter().rect_filled(rect, 0.0, fill);
    }
    let col = if hot && close {
        Color32::WHITE
    } else if hot {
        c_text()
    } else {
        c_muted()
    };
    let st = Stroke::new(1.0_f32, col);
    let c = rect.center();
    let p = ui.painter();
    match kind {
        WinBtn::Minimize => {
            p.line_segment([c + vec2(-5.0, 0.0), c + vec2(5.0, 0.0)], st);
        }
        WinBtn::Maximize => {
            p.rect_stroke(egui::Rect::from_center_size(c, vec2(10.0, 10.0)), 1.0, st);
        }
        WinBtn::Restore => {
            p.rect_stroke(egui::Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(8.0, 8.0)), 1.0, st);
            p.line_segment([c + vec2(-2.5, -4.5), c + vec2(4.5, -4.5)], st);
            p.line_segment([c + vec2(4.5, -4.5), c + vec2(4.5, 2.5)], st);
        }
        WinBtn::Close => {
            p.line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], st);
            p.line_segment([c + vec2(-5.0, 5.0), c + vec2(5.0, -5.0)], st);
        }
    }
    resp
}

/// Invisible strips along the window edges that start an OS resize (the window has no frame).
fn resize_grips(ctx: &egui::Context) {
    use egui::{CursorIcon as C, ResizeDirection as D};
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let r = ctx.screen_rect();
    let (t, k) = (5.0, 12.0);
    let p = egui::pos2;
    let grips: [(D, egui::Rect, C); 8] = [
        (D::North, egui::Rect::from_min_max(p(r.left() + k, r.top()), p(r.right() - k, r.top() + t)), C::ResizeVertical),
        (D::South, egui::Rect::from_min_max(p(r.left() + k, r.bottom() - t), p(r.right() - k, r.bottom())), C::ResizeVertical),
        (D::West, egui::Rect::from_min_max(p(r.left(), r.top() + k), p(r.left() + t, r.bottom() - k)), C::ResizeHorizontal),
        (D::East, egui::Rect::from_min_max(p(r.right() - t, r.top() + k), p(r.right(), r.bottom() - k)), C::ResizeHorizontal),
        (D::NorthWest, egui::Rect::from_min_size(r.min, vec2(k, k)), C::ResizeNwSe),
        (D::SouthEast, egui::Rect::from_min_max(r.max - vec2(k, k), r.max), C::ResizeNwSe),
        (D::NorthEast, egui::Rect::from_min_max(p(r.right() - k, r.top()), p(r.right(), r.top() + k)), C::ResizeNeSw),
        (D::SouthWest, egui::Rect::from_min_max(p(r.left(), r.bottom() - k), p(r.left() + k, r.bottom())), C::ResizeNeSw),
    ];
    for (i, (dir, rect, cursor)) in grips.into_iter().enumerate() {
        egui::Area::new(egui::Id::new(("grip", i)))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                let (_, resp) = ui.allocate_exact_size(rect.size(), Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ctx.set_cursor_icon(cursor);
                }
                if resp.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
                }
            });
    }
}

/// Minimal Markdown for release notes: headings, bullets; bold/code markers are stripped.
fn render_notes(ui: &mut Ui, md: &str) {
    ui.spacing_mut().item_spacing.y = 4.0;
    for raw in md.lines() {
        let line = raw.trim_end().replace("**", "").replace('`', "");
        let t = line.trim_start();
        if t.is_empty() {
            ui.add_space(4.0);
        } else if t.len() >= 3 && t.chars().all(|c| c == '-') {
            ui.separator();
        } else if let Some(h) = t.strip_prefix("### ").or_else(|| t.strip_prefix("## ")).or_else(|| t.strip_prefix("# ")) {
            ui.add_space(4.0);
            ui.label(RichText::new(h).size(14.5).family(semibold()).color(c_text()));
        } else if let Some(b) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new("•").color(accent()));
                ui.add(egui::Label::new(RichText::new(b).color(c_text())).wrap());
            });
        } else {
            ui.add(egui::Label::new(RichText::new(t).color(c_text())).wrap());
        }
    }
}

//! Colors, fonts and small reusable widgets.

use std::cell::Cell;

use eframe::egui::{
    self, vec2, Align, Color32, FontData, FontDefinitions, FontFamily, FontId, Frame, Layout, Margin,
    Response, RichText, Rounding, Sense, Stroke, TextStyle, Ui,
};

/// Colors of one theme. Status colors (green/red) stay the same in every theme.
#[derive(Clone, Copy, PartialEq)]
pub struct Palette {
    pub bg: Color32,
    pub sidebar: Color32,
    pub card: Color32,
    pub input: Color32,
    pub btn: Color32,
    pub btn_hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub yellow: Color32,
    pub light: bool,
}

const fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

#[allow(clippy::too_many_arguments)]
const fn pal(bg: u32, sidebar: u32, card: u32, input: u32, btn: u32, btn_hover: u32, border: u32, text: u32, muted: u32, faint: u32, light: bool) -> Palette {
    Palette {
        bg: rgb(bg), sidebar: rgb(sidebar), card: rgb(card), input: rgb(input), btn: rgb(btn), btn_hover: rgb(btn_hover),
        border: rgb(border), text: rgb(text), muted: rgb(muted), faint: rgb(faint),
        yellow: if light { rgb(0xb7_7a_00) } else { rgb(0xf5_c4_51) }, light,
    }
}

/// (id stored in ZapretGUI.ini, name shown in settings, palette)
pub const THEMES: &[(&str, &str, Palette)] = &[
    ("graphite", "Графит", pal(0x1e1f22, 0x18191c, 0x26272b, 0x151619, 0x2e3035, 0x383a40, 0x303236, 0xe9eaec, 0x9a9ca3, 0x686b73, false)),
    ("warm", "Тёплая", pal(0x22211f, 0x1b1a18, 0x2b2a27, 0x181716, 0x34332f, 0x3e3d39, 0x36352f, 0xecebe6, 0xa19f97, 0x6f6d66, false)),
    ("midnight", "Полночь", pal(0x0f1117, 0x0f1117, 0x191d27, 0x11141b, 0x232835, 0x2b3140, 0x272c3a, 0xe6e9f2, 0x8a92a8, 0x5d657a, false)),
    ("oled", "OLED", pal(0x000000, 0x000000, 0x0f0f10, 0x070708, 0x1a1a1c, 0x252527, 0x1e1e20, 0xececec, 0x8f8f94, 0x5c5c61, false)),
    ("light", "Светлая", pal(0xf6f7f9, 0xf6f7f9, 0xffffff, 0xf0f2f5, 0xeceef2, 0xe1e4ea, 0xdde1e7, 0x1d2129, 0x5b6270, 0x8a909c, true)),
];

/// (id, name, color)
pub const ACCENTS: &[(&str, &str, Color32)] = &[
    ("blue", "Синий", rgb(0x6d8dff)),
    ("blurple", "Discord", rgb(0x5865f2)),
    ("violet", "Фиолетовый", rgb(0x9b7bff)),
    ("teal", "Бирюзовый", rgb(0x22b5c4)),
    ("emerald", "Изумрудный", rgb(0x26b47f)),
    ("orange", "Оранжевый", rgb(0xd97757)),
    ("pink", "Розовый", rgb(0xe36aa8)),
];

pub const GREEN: Color32 = Color32::from_rgb(0x3e, 0xcf, 0x8e);
pub const RED: Color32 = Color32::from_rgb(0xff, 0x6b, 0x6b);

thread_local! {
    // The UI runs on one thread; widgets read the active palette from here.
    static PAL: Cell<(Palette, Color32)> = const { Cell::new((THEMES[0].2, ACCENTS[0].2)) };
}

fn p() -> Palette {
    PAL.with(|c| c.get().0)
}
pub fn accent() -> Color32 {
    PAL.with(|c| c.get().1)
}
pub fn c_bg() -> Color32 { p().bg }
pub fn c_sidebar() -> Color32 { p().sidebar }
pub fn c_card_bg() -> Color32 { p().card }
pub fn c_input() -> Color32 { p().input }
pub fn c_btn() -> Color32 { p().btn }
pub fn c_btn_hover() -> Color32 { p().btn_hover }
pub fn c_border() -> Color32 { p().border }
pub fn c_text() -> Color32 { p().text }
pub fn c_muted() -> Color32 { p().muted }
pub fn c_faint() -> Color32 { p().faint }
pub fn c_yellow() -> Color32 { p().yellow }
pub fn is_light() -> bool { p().light }

pub fn theme_index(id: &str) -> usize {
    THEMES.iter().position(|t| t.0 == id).unwrap_or(0)
}
pub fn accent_index(id: &str) -> usize {
    ACCENTS.iter().position(|a| a.0 == id).unwrap_or(0)
}

/// Switches theme + accent and rebuilds the egui style.
pub fn set_theme(ctx: &egui::Context, theme: &str, accent_id: &str) {
    PAL.with(|c| c.set((THEMES[theme_index(theme)].2, ACCENTS[accent_index(accent_id)].2)));
    apply_style(ctx);
}

pub fn tint(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

fn load_font(fonts: &mut FontDefinitions, name: &str, file: &str) -> bool {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    match std::fs::read(format!("{dir}\\Fonts\\{file}")) {
        Ok(b) => {
            fonts.font_data.insert(name.into(), FontData::from_owned(b));
            true
        }
        Err(_) => false,
    }
}

pub fn apply(ctx: &egui::Context) {
    // Fonts: Segoe UI (+ Semibold) and Consolas when present, egui's bundled fonts as fallback
    // (they carry the emoji/icon glyphs used in the sidebar).
    let mut fonts = FontDefinitions::default();
    let base_prop = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut prop = Vec::new();
    if load_font(&mut fonts, "segoe", "segoeui.ttf") {
        prop.push("segoe".to_string());
    }
    prop.extend(base_prop.iter().cloned());
    let mut semi = Vec::new();
    if load_font(&mut fonts, "segoe-sb", "seguisb.ttf") {
        semi.push("segoe-sb".to_string());
    }
    semi.extend(prop.iter().cloned());
    if load_font(&mut fonts, "consolas", "consola.ttf") {
        if let Some(m) = fonts.families.get_mut(&FontFamily::Monospace) {
            m.insert(0, "consolas".into());
        }
    }
    fonts.families.insert(FontFamily::Proportional, prop);
    fonts.families.insert(semibold(), semi);
    ctx.set_fonts(fonts);
    apply_style(ctx);
}

pub fn apply_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (TextStyle::Heading, FontId::new(22.0, semibold())),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
    ]
    .into();
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(14.0, 6.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.combo_height = 260.0;
    style.spacing.scroll.bar_width = 6.0;
    style.spacing.scroll.floating = true;

    let v = &mut style.visuals;
    *v = if is_light() { egui::Visuals::light() } else { egui::Visuals::dark() };
    v.panel_fill = c_bg();
    v.window_fill = c_card_bg();
    v.window_stroke = Stroke::new(1.0_f32, c_border());
    v.window_rounding = Rounding::same(14.0);
    v.menu_rounding = Rounding::same(10.0);
    v.extreme_bg_color = c_input();
    v.faint_bg_color = c_card_bg();
    v.code_bg_color = c_input();
    v.hyperlink_color = accent();
    v.selection.bg_fill = tint(accent(), 90);
    v.selection.stroke = Stroke::new(1.0_f32, accent());
    v.override_text_color = None;

    let r = Rounding::same(8.0);
    v.widgets.noninteractive.bg_fill = c_card_bg();
    v.widgets.noninteractive.weak_bg_fill = c_card_bg();
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, c_border());
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, c_text());
    v.widgets.noninteractive.rounding = r;

    v.widgets.inactive.bg_fill = c_btn();
    v.widgets.inactive.weak_bg_fill = c_btn();
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, c_border());
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, c_text());
    v.widgets.inactive.rounding = r;

    v.widgets.hovered.bg_fill = c_btn_hover();
    v.widgets.hovered.weak_bg_fill = c_btn_hover();
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, tint(accent(), 120));
    v.widgets.hovered.fg_stroke = Stroke::new(1.5_f32, c_text());
    v.widgets.hovered.rounding = r;
    v.widgets.hovered.expansion = 0.0;

    v.widgets.active.bg_fill = c_btn_hover();
    v.widgets.active.weak_bg_fill = c_btn_hover();
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, accent());
    v.widgets.active.fg_stroke = Stroke::new(1.5_f32, c_text());
    v.widgets.active.rounding = r;
    v.widgets.active.expansion = 0.0;

    v.widgets.open = v.widgets.active;
    ctx.set_style(style);
}

// ------------------------------------------------------------------- widgets

pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    // Width is fixed up front (outer width minus margins and stroke) so nested layouts can't
    // push the card wider than its column — that feeds back through ScrollArea otherwise.
    let w = (ui.available_width() - 34.0).max(10.0);
    Frame::none()
        .fill(c_card_bg())
        .stroke(Stroke::new(1.0_f32, c_border()))
        .rounding(12.0)
        .inner_margin(Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(w);
            ui.set_max_width(w);
            add(ui)
        })
        .inner
}

pub fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(15.5).family(semibold()).color(c_text()));
}

pub fn muted(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.5).color(c_muted()));
}

pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.5, color);
}

pub fn pill(ui: &mut Ui, text: &str, color: Color32) {
    Frame::none()
        .fill(tint(color, 30))
        .rounding(20.0)
        .inner_margin(Margin::symmetric(10.0, 3.0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.horizontal(|ui| {
                dot(ui, color);
                ui.label(RichText::new(text).size(12.5).color(color));
            });
        });
}

#[derive(Clone, Copy, PartialEq)]
pub enum Bk {
    Primary,
    Danger,
    Ghost,
    Success,
}

fn styled(text: &str, kind: Bk, min_w: f32, h: f32) -> egui::Button<'static> {
    let (fill, fg, stroke) = match kind {
        Bk::Primary => (accent(), Color32::WHITE, Stroke::NONE),
        Bk::Danger => (tint(RED, 34), RED, Stroke::new(1.0_f32, tint(RED, 70))),
        Bk::Ghost => (c_btn(), c_text(), Stroke::new(1.0_f32, c_border())),
        Bk::Success => (tint(GREEN, 34), GREEN, Stroke::new(1.0_f32, tint(GREEN, 70))),
    };
    egui::Button::new(RichText::new(text.to_string()).color(fg).size(14.0))
        .fill(fill)
        .stroke(stroke)
        .rounding(8.0)
        .min_size(vec2(min_w, h))
}

pub fn button(ui: &mut Ui, text: &str, kind: Bk, enabled: bool) -> Response {
    ui.add_enabled(enabled, styled(text, kind, 0.0, 34.0))
}

pub fn big_button(ui: &mut Ui, text: &str, kind: Bk, enabled: bool) -> Response {
    let w = ui.available_width();
    ui.add_enabled(enabled, styled(text, kind, w, 40.0))
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let bg = lerp_color(c_btn_hover(), accent(), t);
    ui.painter().rect_filled(rect, 11.0, bg);
    let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), 8.0, Color32::WHITE);
    resp
}

/// Title + hint on the left, a switch on the right. Returns true when toggled.
pub fn toggle_row(ui: &mut Ui, value: &mut bool, label: &str, hint: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if toggle(ui, value).changed() {
                changed = true;
            }
            ui.add_space(8.0);
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(label).color(c_text()));
                if !hint.is_empty() {
                    ui.add(egui::Label::new(RichText::new(hint).size(12.0).color(c_muted())).wrap());
                }
            });
        });
    });
    changed
}

pub fn field_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.5).color(c_muted()));
}

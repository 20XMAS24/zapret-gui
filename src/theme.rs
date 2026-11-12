//! Colors, fonts and small reusable widgets.

use eframe::egui::{
    self, vec2, Align, Color32, FontData, FontDefinitions, FontFamily, FontId, Frame, Layout, Margin,
    Response, RichText, Rounding, Sense, Stroke, TextStyle, Ui,
};

pub const BG: Color32 = Color32::from_rgb(0x0f, 0x11, 0x17);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x13, 0x16, 0x1e);
pub const CARD: Color32 = Color32::from_rgb(0x19, 0x1d, 0x27);
pub const INPUT: Color32 = Color32::from_rgb(0x11, 0x14, 0x1b);
pub const BTN: Color32 = Color32::from_rgb(0x23, 0x28, 0x35);
pub const BTN_HOVER: Color32 = Color32::from_rgb(0x2b, 0x31, 0x40);
pub const BORDER: Color32 = Color32::from_rgb(0x27, 0x2c, 0x3a);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe9, 0xf2);
pub const MUTED: Color32 = Color32::from_rgb(0x8a, 0x92, 0xa8);
pub const FAINT: Color32 = Color32::from_rgb(0x5d, 0x65, 0x7a);
pub const ACCENT: Color32 = Color32::from_rgb(0x6d, 0x8d, 0xff);
pub const GREEN: Color32 = Color32::from_rgb(0x3e, 0xcf, 0x8e);
pub const RED: Color32 = Color32::from_rgb(0xff, 0x6b, 0x6b);
pub const YELLOW: Color32 = Color32::from_rgb(0xf5, 0xc4, 0x51);

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
    *v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_rounding = Rounding::same(14.0);
    v.menu_rounding = Rounding::same(10.0);
    v.extreme_bg_color = INPUT;
    v.faint_bg_color = CARD;
    v.code_bg_color = INPUT;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = tint(ACCENT, 90);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.override_text_color = None;

    let r = Rounding::same(8.0);
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.weak_bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.noninteractive.rounding = r;

    v.widgets.inactive.bg_fill = BTN;
    v.widgets.inactive.weak_bg_fill = BTN;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.rounding = r;

    v.widgets.hovered.bg_fill = BTN_HOVER;
    v.widgets.hovered.weak_bg_fill = BTN_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, tint(ACCENT, 120));
    v.widgets.hovered.fg_stroke = Stroke::new(1.5_f32, TEXT);
    v.widgets.hovered.rounding = r;
    v.widgets.hovered.expansion = 0.0;

    v.widgets.active.bg_fill = BTN_HOVER;
    v.widgets.active.weak_bg_fill = BTN_HOVER;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.5_f32, TEXT);
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
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
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
    ui.label(RichText::new(text).size(15.5).family(semibold()).color(TEXT));
}

pub fn muted(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.5).color(MUTED));
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
        Bk::Primary => (ACCENT, Color32::WHITE, Stroke::NONE),
        Bk::Danger => (tint(RED, 34), RED, Stroke::new(1.0_f32, tint(RED, 70))),
        Bk::Ghost => (BTN, TEXT, Stroke::new(1.0_f32, BORDER)),
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
    let bg = lerp_color(Color32::from_rgb(0x34, 0x3a, 0x4a), ACCENT, t);
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
                ui.label(RichText::new(label).color(TEXT));
                if !hint.is_empty() {
                    ui.add(egui::Label::new(RichText::new(hint).size(12.0).color(MUTED)).wrap());
                }
            });
        });
    });
    changed
}

pub fn field_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.5).color(MUTED));
}

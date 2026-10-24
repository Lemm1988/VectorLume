//! Erscheinungsbild von VectorLume: Farben, Schriften, Abstände (dunkles und helles Design).

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle, Theme, Visuals,
};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub panel_alt: Color32,
    pub widget: Color32,
    pub widget_hover: Color32,
    pub widget_active: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    pub accent: Color32,
    pub success: Color32,
    pub danger: Color32,
    pub warning: Color32,
    pub canvas: Color32,
    pub bed: Color32,
    pub grid_minor: Color32,
    pub grid_major: Color32,
    pub ruler: Color32,
    pub selection: Color32,
    pub travel: Color32,
}

pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(13, 15, 20),
    panel: Color32::from_rgb(22, 25, 32),
    panel_alt: Color32::from_rgb(28, 32, 41),
    widget: Color32::from_rgb(36, 41, 53),
    widget_hover: Color32::from_rgb(46, 53, 68),
    widget_active: Color32::from_rgb(58, 66, 86),
    border: Color32::from_rgb(44, 50, 64),
    text: Color32::from_rgb(230, 233, 240),
    text_weak: Color32::from_rgb(148, 157, 176),
    accent: Color32::from_rgb(92, 140, 255),
    success: Color32::from_rgb(52, 199, 120),
    danger: Color32::from_rgb(239, 83, 80),
    warning: Color32::from_rgb(245, 166, 35),
    canvas: Color32::from_rgb(15, 17, 22),
    bed: Color32::from_rgb(24, 28, 36),
    grid_minor: Color32::from_rgb(33, 38, 49),
    grid_major: Color32::from_rgb(46, 53, 68),
    ruler: Color32::from_rgb(19, 22, 28),
    selection: Color32::from_rgb(92, 140, 255),
    travel: Color32::from_rgba_premultiplied(120, 60, 60, 140),
};

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(246, 247, 250),
    panel: Color32::from_rgb(252, 252, 254),
    panel_alt: Color32::from_rgb(243, 245, 249),
    widget: Color32::from_rgb(233, 236, 242),
    widget_hover: Color32::from_rgb(222, 227, 236),
    widget_active: Color32::from_rgb(206, 214, 228),
    border: Color32::from_rgb(218, 223, 232),
    text: Color32::from_rgb(28, 32, 42),
    text_weak: Color32::from_rgb(98, 107, 125),
    accent: Color32::from_rgb(45, 98, 235),
    success: Color32::from_rgb(22, 163, 74),
    danger: Color32::from_rgb(220, 38, 38),
    warning: Color32::from_rgb(217, 119, 6),
    canvas: Color32::from_rgb(232, 235, 241),
    bed: Color32::from_rgb(255, 255, 255),
    grid_minor: Color32::from_rgb(236, 239, 244),
    grid_major: Color32::from_rgb(215, 221, 231),
    ruler: Color32::from_rgb(244, 246, 250),
    selection: Color32::from_rgb(45, 98, 235),
    travel: Color32::from_rgba_premultiplied(200, 80, 80, 140),
};

pub fn palette(dark: bool) -> &'static Palette {
    if dark { &DARK } else { &LIGHT }
}

pub fn of(ui: &egui::Ui) -> &'static Palette {
    palette(ui.visuals().dark_mode)
}

fn visuals(p: &Palette, dark: bool) -> Visuals {
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    let r6 = CornerRadius::same(6);
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.panel_alt;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.selection.bg_fill = p.accent.gamma_multiply(if dark { 0.55 } else { 0.35 });
    v.selection.stroke = Stroke::new(1.0, p.text);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text_weak);
    w.noninteractive.corner_radius = r6;

    w.inactive.bg_fill = p.widget;
    w.inactive.weak_bg_fill = p.widget;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r6;

    w.hovered.bg_fill = p.widget_hover;
    w.hovered.weak_bg_fill = p.widget_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent.gamma_multiply(0.6));
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = r6;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.widget_active;
    w.active.weak_bg_fill = p.widget_active;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(2.0, p.text);
    w.active.corner_radius = r6;
    w.active.expansion = 0.0;

    w.open = w.active;
    v
}

/// Lädt Schriften (Segoe UI, falls vorhanden, plus Phosphor-Icons) und setzt das Design.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, path) in [("segoe", r"C:\Windows\Fonts\segoeui.ttf"), ("segoe-sb", r"C:\Windows\Fonts\seguisb.ttf")] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
        }
    }
    if fonts.font_data.contains_key("segoe") {
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    let strong = if fonts.font_data.contains_key("segoe-sb") { "segoe-sb" } else { "segoe" };
    if fonts.font_data.contains_key(strong) {
        let mut fam = vec![strong.to_string()];
        fam.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
        fonts.families.insert(FontFamily::Name("strong".into()), fam);
    } else {
        let prop = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
        fonts.families.insert(FontFamily::Name("strong".into()), prop);
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // Icons auch in der fetten Überschriftenschrift verfügbar machen.
    if let Some(strong) = fonts.families.get_mut(&FontFamily::Name("strong".into())) {
        strong.push("phosphor".into());
    }
    ctx.set_fonts(fonts);

    ctx.set_visuals_of(Theme::Dark, visuals(&DARK, true));
    ctx.set_visuals_of(Theme::Light, visuals(&LIGHT, false));
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(9.0, 4.0);
        style.spacing.interact_size.y = 24.0;
        style.spacing.window_margin = Margin::same(14);
        style.spacing.menu_margin = Margin::same(6);
        style.spacing.combo_width = 140.0;
        style.text_styles = [
            (TextStyle::Heading, FontId::new(17.0, FontFamily::Name("strong".into()))),
            (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
        ]
        .into();
    });
}

pub fn strong_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("strong".into()))
}

pub fn layer_color(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

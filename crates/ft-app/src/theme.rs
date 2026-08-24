//! macOS-inspired visual theme for File Transfer.

use eframe::egui;

pub mod colors {
    use egui::Color32;

    pub const WINDOW_BG: Color32 = Color32::from_rgb(236, 236, 241);
    pub const SIDEBAR_BG: Color32 = Color32::from_rgb(246, 246, 248);
    pub const CARD_BG: Color32 = Color32::from_rgb(255, 255, 255);
    pub const FOOTER_BG: Color32 = Color32::from_rgb(251, 251, 253);
    pub const SEPARATOR: Color32 = Color32::from_rgb(210, 210, 215);
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(28, 28, 30);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(99, 99, 102);
    pub const ACCENT: Color32 = Color32::from_rgb(0, 122, 255);
    pub const SUCCESS: Color32 = Color32::from_rgb(52, 199, 89);
    pub const ERROR: Color32 = Color32::from_rgb(255, 59, 48);
    pub const SIDEBAR_SELECTED: Color32 = Color32::from_rgb(0, 122, 255);
    pub const SIDEBAR_HOVER: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 12);
    pub const PROGRESS_TRACK: Color32 = Color32::from_rgb(229, 229, 234);
    pub const PROGRESS_FILL: Color32 = ACCENT;
}

pub fn setup(cc: &eframe::CreationContext<'_>) {
    let ctx = &cc.egui_ctx;
    let mut visuals = egui::Visuals::light();

    visuals.window_fill = colors::WINDOW_BG;
    visuals.panel_fill = colors::WINDOW_BG;
    visuals.extreme_bg_color = colors::WINDOW_BG;
    visuals.faint_bg_color = colors::CARD_BG;
    visuals.window_stroke = egui::Stroke::new(1.0_f32, colors::SEPARATOR);
    visuals.override_text_color = Some(colors::TEXT_PRIMARY);

    visuals.widgets.noninteractive.bg_fill = colors::CARD_BG;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, colors::TEXT_SECONDARY);
    visuals.widgets.inactive.bg_fill = colors::CARD_BG;
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(242, 242, 247);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, colors::TEXT_PRIMARY);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(242, 242, 247);
    visuals.widgets.active.bg_fill = colors::ACCENT;
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    visuals.widgets.open.bg_fill = colors::CARD_BG;

    visuals.selection.bg_fill = colors::ACCENT.linear_multiply(0.25);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, colors::ACCENT);
    visuals.hyperlink_color = colors::ACCENT;

    visuals.window_corner_radius = egui::CornerRadius::same(12);
    visuals.menu_corner_radius = egui::CornerRadius::same(8);
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(8);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(8);
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(8);
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(8);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style.spacing.indent = 18.0;
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(22.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(13.5, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(13.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::new(11.5, egui::FontFamily::Proportional),
    );
    ctx.set_style(style);
}

use egui::Color32;

pub fn sidebar_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(colors::SIDEBAR_BG)
        .inner_margin(egui::Margin::symmetric(12, 16))
        .stroke(egui::Stroke::NONE)
}

pub fn content_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(colors::WINDOW_BG)
        .inner_margin(egui::Margin::symmetric(20, 16))
        .stroke(egui::Stroke::NONE)
}

pub fn footer_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(colors::FOOTER_BG)
        .inner_margin(egui::Margin::symmetric(20, 10))
        .stroke(egui::Stroke::new(1.0_f32, colors::SEPARATOR))
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(colors::CARD_BG)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(16))
        .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(220, 220, 225)))
        .shadow(egui::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_rgba_premultiplied(0, 0, 0, 18),
        })
}

pub fn section_heading(ui: &mut egui::Ui, title: &str, subtitle: Option<&str>) {
    ui.vertical(|ui| {
        ui.label(
            egui::RichText::new(title)
                .size(20.0)
                .strong()
                .color(colors::TEXT_PRIMARY),
        );
        if let Some(sub) = subtitle {
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(sub)
                    .size(13.0)
                    .color(colors::TEXT_SECONDARY),
            );
        }
    });
}

pub fn card_section<R>(
    ui: &mut egui::Ui,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    card_frame().show(ui, |ui| {
        ui.label(
            egui::RichText::new(title)
                .size(14.0)
                .strong()
                .color(colors::TEXT_PRIMARY),
        );
        ui.add_space(10.0);
        add_contents(ui)
    }).inner
}

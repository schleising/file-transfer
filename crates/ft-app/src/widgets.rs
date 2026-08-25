//! Reusable UI widgets.

use crate::icons::Icon;
use crate::theme::colors;
use crate::util::{format_bytes, format_eta, format_rate};
use egui::{Color32, RichText, Ui, Vec2};
use ft_exec::Progress;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavTab {
    Source,
    Files,
    Destination,
    History,
}

impl NavTab {
    pub fn prev(self) -> Option<Self> {
        match self {
            NavTab::Source => None,
            NavTab::Files => Some(NavTab::Source),
            NavTab::Destination => Some(NavTab::Files),
            NavTab::History => None,
        }
    }

    pub fn next(self) -> Option<Self> {
        match self {
            NavTab::Source => Some(NavTab::Files),
            NavTab::Files => Some(NavTab::Destination),
            NavTab::Destination => None,
            NavTab::History => None,
        }
    }

    pub fn is_wizard(self) -> bool {
        matches!(self, NavTab::Source | NavTab::Files | NavTab::Destination)
    }
}

pub enum WizardNavAction {
    None,
    Back,
    Next,
}

pub fn app_sidebar(ui: &mut Ui, selected: &mut NavTab) {
    ui.vertical(|ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            Icon::Transfer.ui(ui, colors::ACCENT);
            ui.add_space(6.0);
            ui.label(
                RichText::new("File Transfer")
                    .size(17.0)
                    .strong()
                    .color(colors::TEXT_PRIMARY),
            );
        });
        ui.add_space(20.0);

        nav_item(ui, selected, NavTab::Source, Icon::Computer, "Source");
        nav_item(ui, selected, NavTab::Files, Icon::Document, "Files");
        nav_item(ui, selected, NavTab::Destination, Icon::Folder, "Destination");

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        nav_item(ui, selected, NavTab::History, Icon::History, "History");

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Direct rsync over SSH")
                    .size(11.0)
                    .color(colors::TEXT_SECONDARY),
            );
        });
    });
}

fn nav_item(ui: &mut Ui, selected: &mut NavTab, tab: NavTab, icon: Icon, label: &str) {
    let is_sel = *selected == tab;
    let fill = if is_sel {
        colors::SIDEBAR_SELECTED.linear_multiply(0.12)
    } else {
        Color32::TRANSPARENT
    };
    let text_color = if is_sel {
        colors::ACCENT
    } else {
        colors::TEXT_PRIMARY
    };
    let icon_color = if is_sel {
        colors::ACCENT
    } else {
        colors::TEXT_SECONDARY
    };

    let desired = Vec2::new(ui.available_width(), 36.0);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        if response.hovered() && !is_sel {
            ui.painter().rect_filled(rect, 8.0, colors::SIDEBAR_HOVER);
        }
        ui.painter().rect_filled(rect, 8.0, fill);

        let icon_rect = egui::Rect::from_min_size(
            rect.min + Vec2::new(12.0, (rect.height() - icon.size()) * 0.5),
            Vec2::splat(icon.size()),
        );
        icon.paint(ui, icon_rect, icon_color);

        ui.painter().text(
            rect.min + Vec2::new(40.0, rect.height() * 0.5),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::new(13.5, egui::FontFamily::Proportional),
            text_color,
        );
    }
    if response.clicked() {
        *selected = tab;
    }
    ui.add_space(4.0);
}

pub fn progress_footer(
    ui: &mut Ui,
    progress: &Progress,
    transferring: bool,
    status_line: &str,
    can_transfer: bool,
    on_cancel: impl FnOnce(),
) -> bool {
    let mut transfer = false;
    constrain_content(ui);
    let content_w = ui.available_width();

    ui.vertical(|ui| {
        ui.set_width(content_w);

        let status = if !status_line.is_empty() {
            status_line.to_string()
        } else if transferring {
            "Transferring…".to_string()
        } else {
            "Ready".to_string()
        };

        // Status on the left, Transfer/Cancel on the right — same width as the bar below.
        ui.horizontal(|ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if !transferring
                    && (status_line.contains("complete") || status_line.contains("Complete"))
                {
                    Icon::Checkmark.ui(ui, colors::SUCCESS);
                } else if !transferring
                    && (status_line.contains("failed") || status_line.contains("Failed"))
                {
                    Icon::Xmark.ui(ui, colors::ERROR);
                }
                ui.label(
                    RichText::new(status)
                        .size(13.0)
                        .color(colors::TEXT_PRIMARY),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if transferring {
                    if secondary_button(ui, "Cancel").clicked() {
                        on_cancel();
                    }
                    ui.add_space(8.0);
                }
                ui.add_enabled_ui(can_transfer && !transferring, |ui| {
                    if primary_button(ui, "Transfer").clicked() {
                        transfer = true;
                    }
                });
            });
        });

        ui.add_space(6.0);
        progress_detail(ui, progress, transferring, content_w);
        ui.add_space(4.0);
        progress_bar_track(ui, progress, transferring, content_w);
    });
    transfer
}

fn progress_detail(ui: &mut Ui, progress: &Progress, transferring: bool, width: f32) {
    let rate_eta = progress_rate_eta(progress, transferring);
    let frac = progress_fraction(progress, transferring);
    let detail = progress_detail_text(progress, transferring, frac, &rate_eta);

    let font = egui::FontId::new(11.5, egui::FontFamily::Proportional);
    let galley = ui.painter().layout(
        detail,
        font,
        colors::TEXT_SECONDARY,
        width,
    );
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(width, galley.size().y),
        egui::Sense::hover(),
    );
    if ui.is_rect_visible(rect) {
        ui.painter().galley(rect.min, galley, colors::TEXT_SECONDARY);
    }
}

fn progress_bar_track(ui: &mut Ui, progress: &Progress, transferring: bool, width: f32) {
    const BAR_H: f32 = 8.0;
    let frac = progress_fraction(progress, transferring);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, BAR_H), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.painter()
        .rect_filled(rect, BAR_H * 0.5, colors::PROGRESS_TRACK);
    let fill_w = if frac < 0.0 {
        let pulse = (ui.input(|i| i.time) * 2.0).sin() * 0.5 + 0.5;
        rect.width() * (0.25 + pulse as f32 * 0.35)
    } else {
        rect.width() * frac
    };
    if fill_w > 0.5 {
        let fill = egui::Rect::from_min_size(rect.min, Vec2::new(fill_w, BAR_H));
        ui.painter()
            .rect_filled(fill, BAR_H * 0.5, colors::PROGRESS_FILL);
    }
}

fn progress_rate_eta(progress: &Progress, transferring: bool) -> String {
    let mut parts = Vec::new();
    if let Some(rate) = progress.bytes_per_sec.filter(|r| *r > 0.0) {
        parts.push(format_rate(rate));
    }
    if let Some(eta) = progress.eta_secs {
        if eta != 0 || transferring {
            if transferring {
                parts.push(format!("ETA {}", format_eta(eta)));
            }
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" · {}", parts.join(" · "))
    }
}

fn progress_fraction(progress: &Progress, transferring: bool) -> f32 {
    if let Some(pct) = progress.percent {
        (pct / 100.0).clamp(0.0, 1.0)
    } else {
        match (progress.bytes_done, progress.bytes_total) {
            (done, Some(total)) if total > 0 => (done as f32 / total as f32).clamp(0.0, 1.0),
            _ if transferring => -1.0,
            (done, _) if done > 0 => 1.0,
            _ => 0.0,
        }
    }
}

fn progress_detail_text(
    progress: &Progress,
    _transferring: bool,
    frac: f32,
    rate_eta: &str,
) -> String {
    if frac < 0.0 {
        format!("{}{}", format_bytes(progress.bytes_done), rate_eta)
    } else {
        let pct_label = (frac * 100.0).round();
        match progress.bytes_total {
            Some(t) => format!(
                "{} / {} ({pct_label:.0}%){rate_eta}",
                format_bytes(progress.bytes_done),
                format_bytes(t),
            ),
            None => format!(
                "{} ({pct_label:.0}%){rate_eta}",
                format_bytes(progress.bytes_done)
            ),
        }
    }
}

pub fn constrain_content(ui: &mut Ui) {
    let w = ui.available_width();
    if w.is_finite() && w > 0.0 {
        ui.set_max_width(w);
    }
}

pub fn wrapped_label(ui: &mut Ui, text: impl Into<RichText>) {
    ui.add(egui::Label::new(text.into()).wrap());
}

pub fn page_body<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    constrain_content(ui);
    egui::ScrollArea::vertical()
        .id_salt("page_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            constrain_content(ui);
            add_contents(ui)
        })
        .inner
}

/// Main content area for wizard steps — no scroll wrapper, respects panel margins.
pub fn wizard_nav_bar(ui: &mut Ui, tab: NavTab, can_advance: bool) -> WizardNavAction {
    constrain_content(ui);
    let mut action = WizardNavAction::None;
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(12.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.set_width(ui.available_width());
        if tab.next().is_some() {
            ui.add_enabled_ui(can_advance, |ui| {
                if primary_button(ui, "Next").clicked() {
                    action = WizardNavAction::Next;
                }
            });
        }
        if tab.prev().is_some() {
            ui.add_space(8.0);
            if secondary_button(ui, "Back").clicked() {
                action = WizardNavAction::Back;
            }
        }
    });
    action
}

pub fn path_field(ui: &mut Ui, text: &mut String, hint: &str) -> egui::Response {
    constrain_content(ui);
    let w = ui.available_width().max(96.0);
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(w)
            .hint_text(hint),
    )
}

pub fn combo_width(ui: &Ui) -> f32 {
    ui.available_width().max(120.0)
}

pub fn primary_button(ui: &mut Ui, label: &str) -> egui::Response {
    let text = RichText::new(label).color(Color32::WHITE).size(13.5);
    ui.add(
        egui::Button::new(text)
            .fill(colors::ACCENT)
            .stroke(egui::Stroke::NONE)
            .corner_radius(egui::CornerRadius::same(8))
            .min_size(Vec2::new(120.0, 32.0)),
    )
}

pub fn field_label(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(12.0)
            .color(colors::TEXT_SECONDARY),
    );
}

pub fn secondary_button(ui: &mut Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(label)
                .color(colors::TEXT_PRIMARY)
                .size(13.0),
        )
        .fill(Color32::from_rgb(242, 242, 247))
        .stroke(egui::Stroke::new(1.0_f32, colors::SEPARATOR))
        .corner_radius(egui::CornerRadius::same(8))
        .min_size(Vec2::new(88.0, 30.0)),
    )
}

pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        icon.ui(ui, colors::TEXT_SECONDARY);
        secondary_button(ui, label)
    })
    .inner
}

pub fn status_message(ui: &mut Ui, result: &Result<String, String>) {
    constrain_content(ui);
    match result {
        Ok(m) => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                Icon::Checkmark.ui(ui, colors::SUCCESS);
                ui.label(RichText::new(m).color(colors::SUCCESS));
            });
        }
        Err(e) => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                Icon::Xmark.ui(ui, colors::ERROR);
                ui.add(egui::Label::new(RichText::new(e).color(colors::ERROR)).wrap());
            });
        }
    }
}

pub fn status_badge(ui: &mut Ui, status: &str) {
    let (bg, fg) = match status {
        "OK" | "Ok" => (colors::SUCCESS.linear_multiply(0.15), colors::SUCCESS),
        "Failed" => (colors::ERROR.linear_multiply(0.15), colors::ERROR),
        "Cancelled" => (
            Color32::from_rgb(255, 149, 0).linear_multiply(0.15),
            Color32::from_rgb(255, 149, 0),
        ),
        "Running" => (colors::ACCENT.linear_multiply(0.15), colors::ACCENT),
        _ => (colors::PROGRESS_TRACK, colors::TEXT_SECONDARY),
    };
    let galley = ui.painter().layout_no_wrap(
        status.to_string(),
        egui::FontId::new(11.0, egui::FontFamily::Proportional),
        fg,
    );
    let pad = Vec2::new(8.0, 3.0);
    let size = galley.size() + pad * 2.0;
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(rect, 6.0, bg);
    ui.painter().galley(rect.min + pad, galley, fg);
}

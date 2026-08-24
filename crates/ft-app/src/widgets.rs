//! Reusable UI widgets.

use crate::icons::Icon;
use crate::theme::colors;
use crate::util::{format_bytes, format_eta, format_rate};
use egui::{Color32, RichText, Ui, Vec2};
use ft_exec::Progress;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavTab {
    Transfer,
    Computers,
    Locations,
    History,
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

        nav_item(ui, selected, NavTab::Transfer, Icon::Transfer, "Transfer");
        nav_item(ui, selected, NavTab::Computers, Icon::Computer, "Computers");
        nav_item(ui, selected, NavTab::Locations, Icon::Folder, "Locations");
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
    on_cancel: impl FnOnce(),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_min_width(ui.available_width() - 100.0);

            let status = if !status_line.is_empty() {
                status_line.to_string()
            } else if transferring {
                "Transferring…".to_string()
            } else {
                "Ready".to_string()
            };

            ui.horizontal(|ui| {
                if transferring {
                    ui.spinner();
                    ui.add_space(4.0);
                } else if status_line.contains("complete") || status_line.contains("Complete") {
                    Icon::Checkmark.ui(ui, colors::SUCCESS);
                    ui.add_space(2.0);
                } else if status_line.contains("failed") || status_line.contains("Failed") {
                    Icon::Xmark.ui(ui, colors::ERROR);
                    ui.add_space(2.0);
                }
                ui.label(
                    RichText::new(&status)
                        .size(13.0)
                        .color(colors::TEXT_PRIMARY),
                );
            });

            ui.add_space(6.0);
            progress_bar(ui, progress, transferring);
        });

        if transferring {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if secondary_button(ui, "Cancel").clicked() {
                    on_cancel();
                }
            });
        }
    });
}

fn progress_bar(ui: &mut Ui, progress: &Progress, transferring: bool) {
    let rate_eta = {
        let mut parts = Vec::new();
        if let Some(rate) = progress.bytes_per_sec.filter(|r| *r > 0.0) {
            parts.push(format_rate(rate));
        }
        if let Some(eta) = progress.eta_secs {
            if eta == 0 && !transferring {
                // done
            } else if transferring {
                parts.push(format!("ETA {}", format_eta(eta)));
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!(" · {}", parts.join(" · "))
        }
    };

    let frac = if let Some(pct) = progress.percent {
        (pct / 100.0).clamp(0.0, 1.0)
    } else {
        match (progress.bytes_done, progress.bytes_total) {
            (done, Some(total)) if total > 0 => (done as f32 / total as f32).clamp(0.0, 1.0),
            _ if transferring => -1.0,
            (done, _) if done > 0 => 1.0,
            _ => 0.0,
        }
    };

    let bar_height = 8.0;
    let (rect, _response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), bar_height + 18.0),
        egui::Sense::hover(),
    );

    let track = egui::Rect::from_min_size(
        rect.min + Vec2::new(0.0, 16.0),
        Vec2::new(rect.width(), bar_height),
    );
    ui.painter()
        .rect_filled(track, bar_height * 0.5, colors::PROGRESS_TRACK);

    let fill_w = if frac < 0.0 {
        let pulse = (ui.input(|i| i.time) * 2.0).sin() * 0.5 + 0.5;
        track.width() * (0.25 + pulse as f32 * 0.35)
    } else {
        track.width() * frac
    };
    if fill_w > 0.5 {
        let fill = egui::Rect::from_min_size(track.min, Vec2::new(fill_w, bar_height));
        ui.painter()
            .rect_filled(fill, bar_height * 0.5, colors::PROGRESS_FILL);
    }

    let detail = if frac < 0.0 {
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
    };

    ui.painter().text(
        rect.min,
        egui::Align2::LEFT_TOP,
        detail,
        egui::FontId::new(11.5, egui::FontFamily::Proportional),
        colors::TEXT_SECONDARY,
    );
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
        icon.ui(ui, colors::TEXT_SECONDARY);
        ui.add_space(4.0);
        secondary_button(ui, label)
    })
    .inner
}

pub fn field_label(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(12.0)
            .color(colors::TEXT_SECONDARY),
    );
}

pub fn status_message(ui: &mut Ui, result: &Result<String, String>) {
    match result {
        Ok(m) => {
            ui.horizontal(|ui| {
                Icon::Checkmark.ui(ui, colors::SUCCESS);
                ui.label(RichText::new(m).color(colors::SUCCESS));
            });
        }
        Err(e) => {
            ui.horizontal(|ui| {
                Icon::Xmark.ui(ui, colors::ERROR);
                ui.label(RichText::new(e).color(colors::ERROR));
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

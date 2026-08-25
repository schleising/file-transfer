//! Simple vector icons (SF Symbol–inspired) drawn with egui.

use egui::{Color32, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Transfer,
    Computer,
    Folder,
    FolderOpen,
    Document,
    Checkmark,
    Xmark,
    Network,
    Home,
    Refresh,
}

impl Icon {
    pub fn size(self) -> f32 {
        18.0
    }

    pub fn paint(self, ui: &mut Ui, rect: Rect, color: Color32) {
        let painter = ui.painter();
        let c = rect.center();
        let s = rect.width().min(rect.height()) * 0.5;
        let stroke = Stroke::new(1.6_f32, color);

        match self {
            Icon::Transfer => {
                let y = c.y;
                painter.line_segment(
                    [Pos2::new(c.x - s * 0.55, y - s * 0.2), Pos2::new(c.x + s * 0.1, y - s * 0.2)],
                    stroke,
                );
                painter.line_segment(
                    [Pos2::new(c.x + s * 0.1, y - s * 0.2), Pos2::new(c.x - s * 0.05, y - s * 0.45)],
                    stroke,
                );
                painter.line_segment(
                    [Pos2::new(c.x + s * 0.1, y - s * 0.2), Pos2::new(c.x - s * 0.05, y + s * 0.05)],
                    stroke,
                );
                painter.line_segment(
                    [Pos2::new(c.x - s * 0.1, y + s * 0.2), Pos2::new(c.x + s * 0.55, y + s * 0.2)],
                    stroke,
                );
                painter.line_segment(
                    [Pos2::new(c.x - s * 0.1, y + s * 0.2), Pos2::new(c.x + s * 0.05, y - s * 0.05)],
                    stroke,
                );
                painter.line_segment(
                    [Pos2::new(c.x - s * 0.1, y + s * 0.2), Pos2::new(c.x + s * 0.05, y + s * 0.45)],
                    stroke,
                );
            }
            Icon::Computer => {
                let r = egui::Rect::from_center_size(
                    Pos2::new(c.x, c.y - s * 0.12),
                    Vec2::new(s * 1.1, s * 0.72),
                );
                painter.rect_stroke(r, 3.0, stroke, egui::StrokeKind::Outside);
                painter.line_segment(
                    [
                        Pos2::new(c.x - s * 0.35, c.y + s * 0.48),
                        Pos2::new(c.x + s * 0.35, c.y + s * 0.48),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        Pos2::new(c.x, c.y + s * 0.48),
                        Pos2::new(c.x, c.y + s * 0.62),
                    ],
                    stroke,
                );
            }
            Icon::Folder | Icon::FolderOpen => {
                let tab = egui::Rect::from_min_size(
                    Pos2::new(c.x - s * 0.5, c.y - s * 0.35),
                    Vec2::new(s * 0.45, s * 0.18),
                );
                let body = egui::Rect::from_min_size(
                    Pos2::new(c.x - s * 0.55, c.y - s * 0.2),
                    Vec2::new(s * 1.1, s * 0.65),
                );
                painter.rect_stroke(tab, 2.0, stroke, egui::StrokeKind::Outside);
                painter.rect_stroke(body, 4.0, stroke, egui::StrokeKind::Outside);
                if matches!(self, Icon::FolderOpen) {
                    painter.line_segment(
                        [
                            Pos2::new(c.x - s * 0.3, c.y + s * 0.05),
                            Pos2::new(c.x, c.y - s * 0.05),
                        ],
                        stroke,
                    );
                }
            }
            Icon::Document => {
                let r = egui::Rect::from_center_size(c, Vec2::new(s * 0.65, s * 0.85));
                painter.rect_stroke(r, 3.0, stroke, egui::StrokeKind::Outside);
                for i in 0..3 {
                    let y = c.y - s * 0.15 + i as f32 * s * 0.22;
                    painter.line_segment(
                        [
                            Pos2::new(c.x - s * 0.2, y),
                            Pos2::new(c.x + s * 0.2, y),
                        ],
                        Stroke::new(1.2_f32, color),
                    );
                }
            }
            Icon::Checkmark => {
                painter.line_segment(
                    [
                        Pos2::new(c.x - s * 0.35, c.y),
                        Pos2::new(c.x - s * 0.05, c.y + s * 0.28),
                    ],
                    Stroke::new(2.0_f32, color),
                );
                painter.line_segment(
                    [
                        Pos2::new(c.x - s * 0.05, c.y + s * 0.28),
                        Pos2::new(c.x + s * 0.4, c.y - s * 0.3),
                    ],
                    Stroke::new(2.0_f32, color),
                );
            }
            Icon::Xmark => {
                painter.line_segment(
                    [
                        Pos2::new(c.x - s * 0.3, c.y - s * 0.3),
                        Pos2::new(c.x + s * 0.3, c.y + s * 0.3),
                    ],
                    Stroke::new(2.0_f32, color),
                );
                painter.line_segment(
                    [
                        Pos2::new(c.x + s * 0.3, c.y - s * 0.3),
                        Pos2::new(c.x - s * 0.3, c.y + s * 0.3),
                    ],
                    Stroke::new(2.0_f32, color),
                );
            }
            Icon::Network => {
                painter.circle_stroke(c, s * 0.12, stroke);
                for i in 0..3 {
                    let angle = std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::TAU / 3.0;
                    let p = Pos2::new(c.x + angle.cos() * s * 0.42, c.y + angle.sin() * s * 0.42);
                    painter.circle_stroke(p, s * 0.1, stroke);
                    painter.line_segment([c, p], Stroke::new(1.2_f32, color));
                }
            }
            Icon::Home => {
                painter.line_segment(
                    [
                        Pos2::new(c.x, c.y - s * 0.45),
                        Pos2::new(c.x - s * 0.45, c.y),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        Pos2::new(c.x, c.y - s * 0.45),
                        Pos2::new(c.x + s * 0.45, c.y),
                    ],
                    stroke,
                );
                let base = egui::Rect::from_min_size(
                    Pos2::new(c.x - s * 0.32, c.y),
                    Vec2::new(s * 0.64, s * 0.42),
                );
                painter.rect_stroke(base, 2.0, stroke, egui::StrokeKind::Outside);
            }
            Icon::Refresh => {
                painter.circle_stroke(c, s * 0.38, stroke);
                painter.line_segment(
                    [
                        Pos2::new(c.x + s * 0.2, c.y - s * 0.35),
                        Pos2::new(c.x + s * 0.38, c.y - s * 0.15),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        Pos2::new(c.x + s * 0.38, c.y - s * 0.15),
                        Pos2::new(c.x + s * 0.15, c.y - s * 0.15),
                    ],
                    stroke,
                );
            }
        }
    }

    pub fn paint_sized(self, ui: &mut Ui, center: Pos2, size: f32, color: Color32) {
        let rect = Rect::from_center_size(center, Vec2::splat(size));
        self.paint(ui, rect, color);
    }

    pub fn ui(self, ui: &mut Ui, color: Color32) -> Response {
        let size = self.size();
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(size + 4.0), Sense::hover());
        if ui.is_rect_visible(rect) {
            self.paint(ui, rect.shrink(2.0), color);
        }
        response
    }
}

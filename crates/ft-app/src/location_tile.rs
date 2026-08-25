//! Saved-location tile: select, delete, and drag-to-reorder within a host group.

use crate::icons::Icon;
use crate::theme::colors;
use egui::{Color32, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use uuid::Uuid;

const TILE_HEIGHT: f32 = 108.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocationDragPayload {
    pub location_id: Uuid,
    pub computer_id: Uuid,
}

#[derive(Default)]
pub struct LocationTileResponse {
    pub selected: bool,
    pub delete: bool,
    pub dropped: Option<LocationDragPayload>,
}

/// Invisible append target — only allocated while a drag is active.
pub fn location_tile_drop_tail(
    ui: &mut Ui,
    computer_id: Uuid,
) -> Option<LocationDragPayload> {
    if !egui::DragAndDrop::has_payload_of_type::<LocationDragPayload>(ui.ctx()) {
        return None;
    }

    let (_, resp) = ui.allocate_exact_size(Vec2::new(12.0, TILE_HEIGHT), Sense::hover());
    resp.dnd_release_payload::<LocationDragPayload>()
        .map(|p| *p)
        .filter(|p| p.computer_id == computer_id)
}

pub fn location_tile(
    ui: &mut Ui,
    id: Id,
    folder_name: &str,
    host_color: Color32,
    selected: bool,
    width: f32,
    payload: LocationDragPayload,
) -> LocationTileResponse {
    let mut out = LocationTileResponse::default();
    let size = Vec2::new(width, TILE_HEIGHT);

    let is_being_dragged = egui::DragAndDrop::payload::<LocationDragPayload>(ui.ctx())
        .is_some_and(|p| p.location_id == payload.location_id);

    let (rect, tile_resp) = ui.allocate_exact_size(size, Sense::click_and_drag());

    if ui.is_rect_visible(rect) {
        let mut fill = if selected {
            host_color.linear_multiply(0.12)
        } else if tile_resp.hovered() {
            colors::SIDEBAR_HOVER
        } else {
            colors::CARD_BG
        };
        if is_being_dragged {
            fill = fill.linear_multiply(0.45);
        }

        let stroke = if selected {
            Stroke::new(2.0_f32, host_color)
        } else {
            Stroke::new(1.0_f32, colors::SEPARATOR)
        };

        if tile_resp.dnd_hover_payload::<LocationDragPayload>().is_some() {
            ui.painter().rect_stroke(
                rect.expand(2.0),
                12.0,
                Stroke::new(2.0_f32, host_color),
                StrokeKind::Outside,
            );
        }

        ui.painter().rect_filled(rect, 12.0, fill);
        ui.painter().rect_stroke(rect, 12.0, stroke, StrokeKind::Inside);

        let icon_y = rect.top() + 36.0;
        Icon::Folder.paint_sized(
            ui,
            Pos2::new(rect.center().x, icon_y),
            36.0,
            host_color,
        );

        let text_y = icon_y + 28.0;
        let text_width = width - 16.0;
        let galley = ui.painter().layout(
            folder_name.to_owned(),
            FontId::new(12.5, egui::FontFamily::Proportional),
            colors::TEXT_PRIMARY,
            text_width,
        );
        ui.painter().galley(
            Pos2::new(rect.center().x - galley.size().x * 0.5, text_y),
            galley,
            colors::TEXT_PRIMARY,
        );
    }

    let delete_size = 22.0;
    let delete_rect = Rect::from_min_size(
        Pos2::new(rect.right() - delete_size - 4.0, rect.top() + 4.0),
        Vec2::splat(delete_size),
    );
    let delete_resp = ui.put(
        delete_rect,
        egui::Button::new(
            egui::RichText::new("×")
                .size(15.0)
                .color(colors::TEXT_SECONDARY),
        )
        .frame(false)
        .min_size(Vec2::splat(delete_size)),
    );

    if delete_resp.clicked() {
        out.delete = true;
    } else if tile_resp.clicked() {
        out.selected = true;
    }

    if tile_resp.drag_started() {
        tile_resp.dnd_set_drag_payload(payload);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if tile_resp.hovered() && !delete_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    if let Some(drag) = tile_resp.dnd_release_payload::<LocationDragPayload>() {
        let drag = *drag;
        if drag.computer_id == payload.computer_id && drag.location_id != payload.location_id {
            out.dropped = Some(drag);
        }
    }

    let _ = id;
    out
}

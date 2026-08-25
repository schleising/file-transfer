mod app;
mod icons;
mod location_tile;
mod theme;
mod util;
mod widgets;

use anyhow::Result;

fn main() -> Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 740.0])
            .with_min_inner_size([900.0, 560.0])
            .with_title("File Transfer"),
        ..Default::default()
    };
    eframe::run_native(
        "File Transfer",
        native_options,
        Box::new(|cc| Ok(Box::new(app::FileTransferApp::new(cc)?))),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))
}

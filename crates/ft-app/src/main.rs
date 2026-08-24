mod app;

use anyhow::Result;

fn main() -> Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
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

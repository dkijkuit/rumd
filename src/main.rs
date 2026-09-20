mod app;
mod document;
mod search;
mod source;
mod viewer;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let path = std::env::args().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "rumd",
        options,
        Box::new(move |_cc| Ok(Box::new(app::App::new(path)))),
    )
}

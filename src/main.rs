mod document;
mod source;
mod viewer;
use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "rumd",
        options,
        Box::new(|_cc| {
            Ok(Box::new(PlaceholderApp))
        }),
    )
}

struct PlaceholderApp;

impl eframe::App for PlaceholderApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.label("rumd");
    }
}

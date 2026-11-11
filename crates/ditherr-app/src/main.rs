use std::path::PathBuf;

use ditherr_app::app::App;

fn main() -> eframe::Result {
    // `ditherr-app [image]` opens the image straight away.
    let open = std::env::args_os().nth(1).map(PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ditherr")
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([800.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Ditherr",
        options,
        Box::new(move |cc| Ok(Box::new(Shell(App::new(&cc.egui_ctx, open.as_deref()))))),
    )
}

struct Shell(App);

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.show(ui);
    }
}

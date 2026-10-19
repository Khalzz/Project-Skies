use crate::tooling::base_frame::BaseFrame;

#[derive(Debug)]
pub struct ToolingManager {
    pub should_play: bool,
    pub scene: String,
}



pub fn tooling_handling() -> ToolingManager {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_maximized(true),
        ..Default::default()
    };

    let (tx, rx) = std::sync::mpsc::channel::<ToolingManager>();


    match eframe::run_native(
        "Tooling Manager",
        options,
        Box::new(|cc| {
            Ok(Box::new(BaseFrame::new(tx)))
        }),
    ) {
        Ok(_) => {},
        Err(e) => println!("Error running eframe: {:?}", e),
    }

    let result = rx.recv().unwrap();

    result
}

use crate::tooling::tooling_manager::ToolingManager;
use std::sync::mpsc::Sender;
use egui_plot::{Line, MarkerShape, Plot, PlotPoints, Points};
use egui::{Color32, RichText};

pub struct BaseFrame {
    pub data_sender: Sender<ToolingManager>,
    pub selected_scene: String,
}

impl BaseFrame {
    pub fn new(data_sender: Sender<ToolingManager>) -> Self {
        Self {
            data_sender: data_sender,
            selected_scene: "".to_string(),
        }
    }
}

impl eframe::App for BaseFrame {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Style the horizontal layout
            ui.horizontal(|ui| {
                // You can customize the background, borders, etc.
                ui.style_mut().visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(40, 200, 20);
                ui.style_mut().visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 80, 80));
                
                // Add some padding around the content
                ui.add_space(10.0);

                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.label("Scene Viewer");
                });
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let button = ui.button("Play");
                    if button.clicked() {
                        send_data(&self.data_sender, ToolingManager { scene: "".to_string(), should_play: true });
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                
                ui.add_space(10.0);
            });

            ui.vertical(|ui| {
                /* 
                egui::ComboBox::from_label("Select one!")
                    .selected_text(format!("{}", if self.selected_scene == "" { "Select a scene".to_string() } else { self.selected_scene.clone() }))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.selected_scene, "First".to_string(), "First");
                        ui.selectable_value(&mut self.selected_scene, "Second".to_string(), "Second");
                        ui.selectable_value(&mut self.selected_scene, "Third".to_string(), "Third");
                    }
                );
                */

                ui.horizontal(|ui| {
                    ui.allocate_ui(egui::Vec2::new(200.0, ui.available_height()), |ui| {
                        ui.vertical(|ui| {
                            ui.heading("GameObjects");
                            // make this buttons to use all the width they can
                            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                                ui.button(RichText::new("F16"));
                                ui.button(RichText::new("F14"));
                                ui.button(RichText::new("Player"));
                            });
                        });
                    });

                    let point = Points::new("F16", vec![[0.0, 0.0]]).shape(MarkerShape::Up).radius(10.0);

                        // Show the plot with axes and grid (default)
                    Plot::new("cartesian_plane")
                        .view_aspect(1.0)
                        .height(600.0)
                        .width(ui.available_width() - 20.0)
                        .show_axes(false)
                        .show(ui, |plot_ui| {
                        plot_ui.points(point);
                    });
                });
            });
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        send_data(&self.data_sender, ToolingManager { scene: "".to_string(), should_play: false });
    }

    
}

fn send_data(data_sender: &Sender<ToolingManager>, data: ToolingManager) {
    match data_sender.send(data) {
        Ok(_) => {},
        Err(e) => println!("Error sending data: {:?}", e),
    }
}
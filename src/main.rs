use app::App;

mod app;
mod transform;
mod resources;

mod game_nodes {
    pub mod game_object_2d;
    pub mod game_object;
    pub mod timing;
    pub mod scene;
}

mod ui {
    pub mod vertical_container;
    pub mod ui_transform;
    pub mod ui_node;
    pub mod button;
    pub mod label;
}

mod audio {
    pub mod subtitles;
    pub mod audio;
}

mod input {
}

mod gameplay {
    pub mod event_handling;
    pub mod plane_selection;
    pub mod controller;
    pub mod main_menu;
    pub mod airfoil;
    pub mod wheel;
    pub mod wing;
    pub mod play;
}

mod primitive {
    pub mod manual_vertex;
}

mod rendering {
    pub mod instance_management;
    pub mod physics_rendering;
    pub mod rendering_utils;
    pub mod depth_renderer;
    pub mod render_line;
    pub mod textures;
    pub mod vertex;
    pub mod camera;
    pub mod model;
    pub mod light;
    pub mod ui;
}

mod utils {
    pub mod lerps;
}

// this tokio trait means that main WILL AND CAN be asyncronous (without tokio this is not achievable)
#[tokio::main]
async fn main() -> Result<(), String> {
    match App::new("Pankarta Software", None, None).await {
        Ok(app) => app.update(),
        Err(err) => eprintln!("Something went wrong in the definition of the app: {}", err), 
    }
    Ok(())
}
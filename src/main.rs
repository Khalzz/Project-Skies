use app::App;

mod app;
mod transform;
mod resources;

mod game_nodes {
    pub mod game_object_2d;
    pub mod game_object;
    pub mod scene;
    pub mod timing;
}

mod ui {
    pub mod vertical_container;
    pub mod ui_transform;
    pub mod ui_node;
    pub mod button;
    pub mod label;
}

mod input {
}

mod gameplay {
    pub mod controller;
    pub mod play;
    pub mod main_menu;
    pub mod plane_selection;
    pub mod airfoil;
    pub mod wing;
    pub mod wheel;
}

mod primitive {
    pub mod manual_vertex;
}

mod rendering {
    pub mod render_line;
    pub mod textures;
    pub mod camera;
    pub mod model;
    pub mod vertex;
    pub mod depth_renderer;
    pub mod ui;
    pub mod instance_management;
    pub mod physics_rendering;
    pub mod rendering_utils;
    pub mod light;
}

mod utils {
    pub mod lerps;
}

// this tokio trait means that main WILL AND CAN be asyncronous (without tokio this is not achievable)
#[tokio::main]
async fn main() -> Result<(), String> {
    let app = App::new("Pankarta Software", None, None);
    app.await.update();
    Ok(())
}
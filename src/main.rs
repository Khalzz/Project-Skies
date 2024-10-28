use app::App;

mod app;
mod game_object;
mod transform;
mod resources;

mod ui {
    pub mod button;
}

mod input {
    pub mod button_module;
}

mod gameplay {
    pub mod controller;
    pub mod play;
    pub mod main_menu;
}

mod primitive {
    pub mod manual_vertex;
    pub mod rectangle;
    pub mod text;
}

mod rendering {
    pub mod textures;
    pub mod camera;
    pub mod model;
    pub mod vertex;
    pub mod depth_renderer;
    pub mod ui;
    pub mod instance_management;
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
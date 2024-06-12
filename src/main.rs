use app::App;

mod app;
mod game_object;
mod resources;

mod ui {
    pub mod text;
}

mod input {
    pub mod button_module;
}

mod gameplay {
    pub mod play;
}

mod primitive {
    pub mod manual_vertex;
    pub mod rectangle;
    pub mod text;
    pub mod button;
}

mod rendering {
    pub mod textures;
    pub mod camera;
    pub mod model;
    pub mod vertex;
    pub mod depth_renderer;
}


// this tokio trait means that main WILL AND CAN be asyncronous (without tokio this is not achievable)
#[tokio::main]
async fn main() -> Result<(), String> {
    let app = App::new("Pankarta Software", Some(1280), Some(720));
    app.await.update();
    Ok(())
}
//! Pankarta engine. The whole module tree is declared here - each
//! `pub mod x { pub mod y; }` maps to the file `src/x/y.rs`, so folders don't
//! need their own `x.rs` / `mod.rs` file just to list their children.

pub mod app;
pub mod resources;
pub mod transform;
pub mod splash_screen;

pub mod window {
    pub mod window;
}

pub mod audio {
    pub mod audio;
    pub mod subtitles;
}

pub mod input {
    pub mod action;
    pub mod input;
    pub mod mouse;
    pub mod pressable;
    pub mod utils;
}

pub mod physics {
    pub mod physics;
    pub mod physics_behavior;
    pub mod physics_handler;
    pub mod physics_resources;
    pub mod water_contact;
}

pub mod game_nodes {
    pub mod game_object;
    pub mod game_object_2d;
    pub mod scene;
    pub mod timing;
}

pub mod ui {
    pub mod button;
    pub mod color;
    pub mod layer;
    pub mod ui_node;
    pub mod ui_structure;
    pub mod ui_transform;
    pub mod components {
        pub mod container;
        pub mod image;
        pub mod label;
    }
}

pub mod scene_manager {
    pub mod behavior;
    pub mod loading_screen;
    pub mod node;
    pub mod physics_bridge;
    pub mod properties;
    pub mod render_bridge;
    pub mod scene;
    pub mod scene_nodes;
}

pub mod tooling {
    pub mod base_frame;
    pub mod debug_console;
    pub mod tooling_manager;
}

pub mod primitive {
    pub mod manual_vertex;
}

pub mod rendering {
    pub mod egui_overlay;
    pub mod frame;
    pub mod instance_management;
    pub mod render_sets;
    pub mod renderer;
    pub mod sdf_text;
    pub mod shader_library;
    pub mod thick_lines;
    pub mod tiled_meshes;
    pub mod vertex;
    pub mod camera {
        pub mod camera;
        pub mod handler;
        pub mod projection;
        pub mod uniform;
    }
    pub mod enviroment {
        pub mod environment;
        pub mod light;
        pub mod skybox_renderer;
    }
    pub mod models {
        pub mod model;
        pub mod textures;
    }
    pub mod render_pipeline {
        pub mod blur_renderer;
        pub mod depth_renderer;
        pub mod render_pass;
    }
    pub mod ui {
        pub mod physics_rendering;
        pub mod render_line;
        pub mod rendering_utils;
        pub mod ui;
    }
}

/// GPU particles and trails, emitted from nodes - see `emitter::Emitter`
/// (what goes on a node), `effect` (what an effect looks like and does) and
/// `renderer::ParticleRenderer` (the engine side that simulates and draws
/// them).
pub mod particles {
    pub mod effect;
    pub mod emitter;
    pub mod renderer;

    pub use effect::*;
    pub use emitter::{Emitter, EmitterKind, ParticleEmitters};
}

pub mod utils {
    pub mod lerps;
}

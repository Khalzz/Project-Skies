//! Pankarta engine. The whole module tree is declared here - each
//! `pub mod x { pub mod y; }` maps to the file `src/x/y.rs`, so folders don't
//! need their own `x.rs` / `mod.rs` file just to list their children.

pub mod app;
pub mod assets;
pub mod resources;
pub mod settings;
pub mod transform;

// Libraries whose types are part of the engine's own API - a game uses them
// through these, so its versions always match the engine's (an `egui::Context`
// from a different egui version wouldn't be the same type).
pub use egui;
pub use nalgebra;
pub use sdl2;

/// The things most game code uses, in one import:
///
///     use pankarta_software::prelude::*;
pub mod prelude {
    pub use nalgebra::{UnitQuaternion, Vector2, Vector3};

    pub use crate::app::App;
    pub use crate::input::input;
    pub use crate::input::source::{Axis, Button, InputSource, Key, MouseButton};
    pub use crate::scene_manager::behavior::Behavior;
    pub use crate::scene_manager::node::{Node, NodeLookup, NodeLookupMut};
    pub use crate::scene_manager::node::properties::{
        degrees_from_rotation, rotation_from_degrees, CameraLens, LookAt, Material, Model, Offset2D, Shape2D, ShapeKind, Transform2D, Transform3D,
    };
    pub use crate::scene_manager::scene::{FrameContext, Scene, SceneBehaviour};
    pub use crate::scene_manager::scene_access::SceneAccess;
    pub use crate::ui::color::UiColor;
}

pub mod window {
    pub mod window;
}

pub mod audio {
    pub mod audio;
    pub mod subtitles;
}

pub mod input {
    pub mod action;
    pub mod clipboard;
    pub mod input;
    pub mod mouse;
    pub mod pressable;
    pub mod source;
    pub mod utils;
}

pub mod physics {
    pub mod physics;
    pub mod physics_behavior;
    pub mod physics_handler;
    pub mod physics_resources;
}

pub mod game_nodes {
    pub mod game_object;
    pub mod game_object_2d;
    pub mod scene;
    pub mod timing;
}

pub mod ui {
    /// How text sits in its box (left, center, right...) - see `TextStyle`.
    pub use glyphon::cosmic_text::Align;
    /// How thick text is (`Weight::BOLD`...) - see `TextStyle`.
    pub use glyphon::Weight;
    pub use text_style::TextStyle;
    /// The mouse cursor over a node - see `UiNode::set_cursor`.
    pub use crate::window::window::CursorIcon;
    /// How the UI follows the window's size - `app.ui.scaling`.
    pub use crate::rendering::ui::ui::UiScaling;

    pub mod color;
    pub mod text_style;
    pub mod layer;
    pub mod ui_node;
    pub mod ui_structure;
    pub mod ui_transform;
    pub mod components {
        pub mod container;
        pub mod image;
        pub mod label;
    }
    /// Ready-made controls built from `UiNode`s - each with a `...Style` whose
    /// `Default` is the engine's base look.
    pub mod widgets {
        pub mod button;
        pub mod card;
        pub mod dropdown;
        pub mod modal;
        pub mod scrollable;
        pub mod text;
        pub mod text_input;
        pub mod toast;
        pub mod toggle;

        pub use button::{button, ButtonStyle};
        pub use card::{card, CardStyle};
        pub use dropdown::{dropdown, DropdownStyle};
        pub use modal::{close_modal, is_modal_open, modal, open_modal, ModalStyle};
        pub use scrollable::{scrollable, ScrollableStyle};
        pub use text::text;
        pub use text_input::{text_input, TextInputStyle};
        pub use toast::{show_toast, ToastKind, ToastStyle};
        pub use toggle::{toggle, ToggleStyle};
        pub use crate::ui::components::container::Scrollbar;
    }
}

pub mod scene_manager {
    pub mod behavior;
    pub mod loading_screen;
    pub mod node {
        pub mod node;
        pub mod properties;

        pub use node::{Node, NodeLookup, NodeLookupMut};
    }
    pub mod physics_bridge;
    pub mod render_bridge;
    pub mod scene;
    pub mod scene_access;
    pub mod scene_nodes;
    pub mod scene_switch;
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
    pub mod graphics_backend;
    pub mod instance_management;
    pub mod render_sets;
    pub mod renderer;
    pub mod sdf_text;
    pub mod shader_library;
    pub mod shapes_2d;
    pub mod thick_lines;
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

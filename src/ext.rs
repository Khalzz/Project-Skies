//! Things the game used to get from its own copy of the engine that
//! pankarta-software doesn't provide - kept here, built on the engine's API.

/// Aircraft data-file types (`data.ron`): cameras, head limits, colliders.
pub mod game_object;
/// Mesh data read straight from glTF files, and the ocean's ring meshes.
pub mod mesh_data;
/// The old physics-thread `PhysicsBehavior` API, run in-process from a
/// `Behavior::fixed_update` (see `PhysicsHost`).
pub mod physics_behavior;
/// The splash scene shown at start.
pub mod splash_screen;
/// Subtitles for voice lines.
pub mod subtitles;
/// Floating and sinking on the sea.
pub mod water_contact;
/// Several named particle emitters on one node, as child nodes.
pub mod emitters;
/// Input helpers the engine's `Input` doesn't have.
pub mod input;
/// The game's screen UI, as one engine UiLayer node per top-level key.
pub mod ui_layers;
/// Where a node is drawn - what the game read off render instances.
pub mod drawn;

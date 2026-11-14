// This code defines whats "arround the missiles" every frame, in the airspace
// this data its what we use to guide the missiles in some sort of way.

use std::sync::Arc;

use nalgebra::Vector3;

use pankarta_software::app::App;
use pankarta_software::scene_manager::node::Node;
use pankarta_software::scene_manager::scene::Scene;

use super::flight::TargetTrack;
use crate::scenes::play::world_collision::WorldCollision;

/// The node the airspace is on.
pub const AIRSPACE: &str = "airspace";

/// What's round every missile this frame.
#[derive(Default)]
pub struct Airspace {
    /// Every aircraft still flying (and those falling, still hot) and every
    /// flare burning - where, how it's moving, how hot, what the fuze hits.
    pub aircraft: Vec<TargetTrack>,
    /// The land and runway.
    pub ground: Arc<WorldCollision>,
    /// Which way the sun is (a unit direction) - None: no sun.
    pub sun: Option<Vector3<f32>>,
}

impl Airspace {
    /// Its node, over `ground` - with nothing in the air yet.
    pub fn node(ground: Arc<WorldCollision>) -> Node {
        Node::new(AIRSPACE).with_property(Airspace {
            ground,
            ..Default::default()
        })
    }

    /// This frame's: `aircraft` in the air, the sun `sun` - before the
    /// missiles fly (their behaviors run after the scene's update).
    pub fn publish(scene: &mut Scene, aircraft: Vec<TargetTrack>, sun: Option<Vector3<f32>>) {
        let Some(airspace) = scene
            .content
            .nodes
            .get_mut(AIRSPACE)
            .and_then(|node| node.get_property_mut::<Airspace>())
        else {
            return;
        };
        airspace.aircraft = aircraft;
        airspace.sun = sun;
    }

    /// Puts its node in the scene, over `ground`.
    pub fn spawn(scene: &mut Scene, app: &mut App, ground: Arc<WorldCollision>) {
        if let Err(error) = scene.spawn_node(app, Self::node(ground)) {
            eprintln!("Couldn't put the airspace up: {error}");
        }
    }
}

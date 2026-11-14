/*
    A ground unit its a unit thats capable of interacting with ground or air units mainly defined in the type
    it has

    Ground units can be separated in multiple types and subtypes, the main ones are:

    Infantry: On foot soldiers that battle in the ground
    Vehicle: Mostly for war or logistics, this category includes tanks, trucks, and others
    Air Defense: Anti-aircraft waponry but also service ones, stuff like radar vehicles, manpads, sams and others
    Structure: Stationary targets like buildings, bunkers and other ground structures
*/

use nalgebra::Vector3;
use pankarta_software::prelude::*;

pub mod air_defense;
mod utils;

use air_defense::AirDefenseType;

/// What a ground unit is - set once, when it's placed (see `ground_unit`).
#[derive(Clone, Debug)]
pub enum UnitType {
    Infantry,                   // On foot soldiers that battle in the ground
    Vehicle,                    // Tanks, trucks, and other vehicles that battle in the ground
    AirDefense(AirDefenseType), // Anti-aircraft like sams, manpads, and others
    Structure, // Stationary targets like buildings, bunkers and other ground structures
}

pub fn ground_unit(name: &str, position: Vector3<f32>, kind: UnitType) -> Node {
    let node = Node::new(name)
        .with_property(Transform3D {
            position,
            ..Default::default()
        })
        .with_behavior(GroundUnit::new(kind.clone()));
    match kind {
        UnitType::AirDefense(air_defense) => air_defense::air_defense(node, air_defense),
        // Nothing of their own yet - just the unit.
        UnitType::Infantry | UnitType::Vehicle | UnitType::Structure => node,
    }
}

/// What every ground unit has, whatever it is.
pub struct GroundUnit {
    debug: bool,
    unit_type: UnitType,
}

impl GroundUnit {
    pub fn new(unit_type: UnitType) -> Self {
        GroundUnit {
            debug: true,
            unit_type,
        }
    }

    /// What it is.
    pub fn unit_type(&self) -> &UnitType {
        &self.unit_type
    }
}

impl Behavior for GroundUnit {
    fn update(&mut self, _node: &mut Node, _scene: &mut SceneAccess, _app: &mut App) {}
}

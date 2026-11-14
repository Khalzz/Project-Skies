/*
    Defines the type of air defense unit

    The values of detection range define only the range where either the sensor or the eye of the person can
    detect the target, the locking and launching of the missile its based on the missile itself

    TODO: Add more units in particular CWIS, C-RAMS and SPAAG, and others
        - CWIS: Close-in weapon sistem, its a system designated to lock on a self contained radar and shoot bullets to targets
        - C-RAMS: Counter rocket, artillery and mortar its a cwis adapted for defense against rockets, artillery and mortars
        - Spaag: Self-propelled anti-aircraft gun, its basically a antiaircraft tank, with radar assisted guns it also shoots bullets, masive explosive ones
*/

use pankarta_software::prelude::*;

pub mod radar;
pub mod sam;

use super::utils;

#[derive(Clone, Debug)]
pub enum AirDefenseType {
    Manpads {
        detection_range: f32, // this rangue defines the "finding objective" range, the launch range is defined by the missile
    },
    Sam(sam::Sam),
    Radar(radar::Radar),
}

/// Adds what the air defense `kind` is to the unit `node` - its look, body,
/// sensors and behaviors, from its own file.
pub fn air_defense(node: Node, kind: AirDefenseType) -> Node {
    match kind {
        AirDefenseType::Sam(sam) => sam.prefab(node),
        AirDefenseType::Radar(radar) => radar.prefab(node),
        // No look or behavior of their own yet - just what they see.
        AirDefenseType::Manpads { detection_range } => {
            node.with_child(utils::sensor(detection_range))
        }
    }
}

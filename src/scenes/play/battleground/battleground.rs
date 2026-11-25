/*
    Battleground its a system of "tagging instances" and defining behaviour of units in the battle

    mostyle defining:

    - Player ID: The id of the player thats "controllable", this is optional so there could be NO PLAYER

    for allowing a specific behaviour for all the units appearing in the battleground itself.
*/

use pankarta_software::prelude::*;

use crate::prefabs::{
    aircraft::aircraft::Aircraft,
    ground_unit::{self, GroundUnit},
};

pub struct Player;

pub enum BattleUnit {
    Ground { unit: GroundUnit },
    Aircraft { unit: Aircraft },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coalition {
    Blue,
    Red,
    White,
}

pub struct Battleground {
    player_id: Option<String>,
    units: Vec<String>,
}

impl Battleground {
    pub fn new() -> Battleground {
        Battleground {
            player_id: Some("Example for now".to_owned()),
            units: vec![],
        }
    }

    pub fn spawn(
        &mut self,
        scene: &mut Scene,
        app: &mut App,
        unit: BattleUnit,
        transform: Transform3D,
        coalition: Coalition,
    ) -> Result<String, String> {
        let id = format!("{coalition:?}-{}", self.units.len()).to_lowercase();

        let node = match unit {
            BattleUnit::Aircraft { unit } => unit.prefab(id.clone(), transform),
            BattleUnit::Ground { unit } => {
                ground_unit::ground_unit(&id, transform.position, unit.unit_type().clone())
                    .with_property(transform)
            }
        };

        scene.spawn_node(app, node.with_property(coalition))?;
        self.units.push(id.clone());
        Ok(id)
    }

    pub fn set_player(&mut self, scene: &mut Scene, id: &str) {
        if let Some(old) = &self.player_id {
            if let Some(node) = scene.get_node_mut(old) {
                node.remove_property::<Player>();
            }
        }
        if let Some(node) = scene.get_node_mut(id) {
            node.set_property(Player);
        }
        self.player_id = Some(id.to_string());
    }
}

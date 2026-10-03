//! A plane's pylons - `data.ron`'s `pylons: [ ... ]`: where on the airframe
//! weapons can hang, and what each one takes. Physical, part of the plane
//! like its gear; what's actually hung on them for a mission is the
//! aircraft's loadout, set in the flight management (play::map's `loadout`).

use nalgebra::Vector3;
use serde::Deserialize;

use crate::game::scenes::play::weapon::weapon_spec::WeaponCategory;

/// One pylon.
#[derive(Clone, Debug, Deserialize)]
pub struct PylonSpec {
    /// Its name - what a loadout puts a weapon on ("Left wingtip"). Unique
    /// on the plane.
    pub name: String,
    /// Where the weapon hangs, meters in the jet's frame (+X left wing, +Y
    /// up, +Z nose) - the weapon's center.
    pub position: Vector3<f32>,
    /// What kinds of weapon it takes - optional: every kind when left out.
    #[serde(default)]
    pub accepts: Vec<WeaponCategory>,
}

impl PylonSpec {
    /// Whether it takes a weapon of `category`.
    pub fn accepts(&self, category: WeaponCategory) -> bool {
        self.accepts.is_empty() || self.accepts.contains(&category)
    }
}

/// Checks a plane's pylons - names not empty and unique.
pub fn check(pylons: &[PylonSpec]) -> Result<(), String> {
    let mut names = std::collections::HashSet::new();
    for pylon in pylons {
        if pylon.name.trim().is_empty() {
            return Err("a pylon has an empty name".to_owned());
        }
        if !names.insert(pylon.name.as_str()) {
            return Err(format!("two pylons are named '{}'", pylon.name));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::scenes::play::plane::aircraft_spec::AircraftSpec;

    #[test]
    fn the_f16_has_its_nine_stations() {
        let f16 = AircraftSpec::load("f16").expect("the F-16 should load");
        assert_eq!(f16.pylons.len(), 9);
        let wingtip = f16.pylons.iter().find(|pylon| pylon.name == "1 Left wingtip").expect("station 1");
        assert!(wingtip.position.x > 4.0, "station 1 is on the left (+X) wingtip");
        assert!(wingtip.accepts(WeaponCategory::Missile));
        assert!(!wingtip.accepts(WeaponCategory::Bomb), "wingtip rails take missiles only");
        let centerline = f16.pylons.iter().find(|pylon| pylon.name == "5 Centerline").expect("station 5");
        assert!(centerline.accepts(WeaponCategory::Bomb), "no accepts: every kind");
    }

    #[test]
    fn two_pylons_cant_share_a_name() {
        let pylon = |name: &str| PylonSpec { name: name.to_owned(), position: Vector3::zeros(), accepts: Vec::new() };
        assert!(check(&[pylon("a"), pylon("b")]).is_ok());
        assert!(check(&[pylon("a"), pylon("a")]).is_err());
        assert!(check(&[pylon(" ")]).is_err());
    }
}

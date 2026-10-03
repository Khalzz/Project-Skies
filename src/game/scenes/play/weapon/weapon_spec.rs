//! A weapon's definition - `assets/weapons/<category>/<name>/data.ron`, laid
//! out like a plane's (see plane::aircraft_spec): its `model`, its `physics`
//! body (rigidbody and colliders - the same `Physics` the planes use), then
//! its own systems - `motor`, `aero`, `guidance`, `fuze`, `warhead`. What its
//! flight will be simulated from: a point mass under thrust, drag and
//! gravity, turned by a guidance law within the G it can pull.
//!
//! `cargo test --bin pankarta-software weapon_spec` checks every weapon's
//! data.ron.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::engine::game_nodes::game_object::Physics;

/// Where every weapon's folder lives: `<category>/<name>` under it
/// (`assets/weapons/missiles/aim-9x/`).
const WEAPONS_DIR: &str = "assets/weapons";

/// The speed of sound Mach is worked out with - the same fixed sea-level
/// value the aircraft instruments use.
pub const SPEED_OF_SOUND: f32 = 340.29;

/// What kind of weapon it is - from its category folder. Also what a
/// plane's pylon `accepts` (see plane::pylon_spec).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub enum WeaponCategory {
    Missile,
    Bomb,
    Gun,
}

impl WeaponCategory {
    fn from_folder(folder: &str) -> Option<Self> {
        match folder {
            "missiles" => Some(Self::Missile),
            "bombs" => Some(Self::Bomb),
            "guns" => Some(Self::Gun),
            _ => None,
        }
    }
}

/// How it finds its target.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub enum Seeker {
    /// Unguided - flies where it's launched.
    None,
    /// Homes on heat (AIM-9).
    InfraRed,
    /// Its own radar (AIM-120).
    ActiveRadar,
    /// The launching aircraft's radar lights the target (AIM-7).
    SemiActiveRadar,
}

/// `model: ( ... )` - the same fields as a plane's.
#[derive(Clone, Debug, Deserialize)]
pub struct WeaponModel {
    /// Relative to the weapon's folder in data.ron - resolved to its full
    /// path when read (`assets/weapons/missiles/aim-9x/model/aim-9x.glb`).
    pub model: String,
    /// Optional - 1.0 when left out.
    #[serde(default = "one")]
    pub scale: f32,
    /// Optional - meshes drawn two-sided (no backface culling).
    #[serde(default)]
    pub double_sided: Vec<String>,
    /// Optional - how the model is turned to face the game's way (nose
    /// +Z, up +Y), degrees about X, Y and Z. Blender's glTF export turns
    /// its +Y into -Z, so a model built pointing +Y in Blender comes out
    /// backwards: `(0.0, 180.0, 0.0)` turns it round.
    #[serde(default)]
    pub rotation: (f32, f32, f32),
}

impl WeaponModel {
    /// `rotation` as a rotation.
    pub fn rotation(&self) -> nalgebra::UnitQuaternion<f32> {
        let (x, y, z) = self.rotation;
        nalgebra::UnitQuaternion::from_euler_angles(x.to_radians(), y.to_radians(), z.to_radians())
    }
}

/// One stretch of the motor's burn: `thrust` newtons for `duration` seconds.
#[derive(Clone, Debug, Deserialize)]
pub struct MotorPhase {
    pub duration: f32,
    pub thrust: f32,
}

/// `motor: ( ... )` - its rocket motor, the weapon's "engine". Optional: a
/// bomb has none.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Motor {
    /// One after another from launch - a boost, then a sustain...
    #[serde(default)]
    pub phases: Vec<MotorPhase>,
    /// What it burns (kg) - lost evenly over the burn.
    #[serde(default)]
    pub propellant_mass: f32,
}

/// One point of the drag table.
#[derive(Clone, Debug, Deserialize)]
pub struct DragPoint {
    pub mach: f32,
    /// Drag coefficient on the body tube's cross-section.
    pub cd: f32,
}

/// `aero: ( ... )` - how it flies.
#[derive(Clone, Debug, Deserialize)]
pub struct Aero {
    /// The body tube's diameter (m) - the cross-section drag acts on.
    pub diameter: f32,
    /// Drag coefficient by Mach, in rising Mach order - straight lines
    /// between points, the end points held past either end.
    pub drag: Vec<DragPoint>,
    /// The most G it can pull, once it's fast enough (`max_g_speed`).
    pub max_g: f32,
    /// From this speed (m/s) up it can pull `max_g` - slower, its fins have
    /// less air to work with: the G falls with the square of the speed.
    pub max_g_speed: f32,
}

/// `guidance: ( ... )` - how it homes.
#[derive(Clone, Debug, Deserialize)]
pub struct Guidance {
    pub seeker: Seeker,
    /// The furthest its seeker can lock a target (m).
    #[serde(default)]
    pub lock_range: f32,
    /// How far off its nose the seeker can look (deg) - past that it loses
    /// the target.
    #[serde(default = "ninety")]
    pub gimbal_limit_deg: f32,
    /// Proportional navigation's gain: how hard it turns per unit of the
    /// line of sight's turn rate - 3 to 5 for real missiles.
    #[serde(default = "four")]
    pub navigation_constant: f32,
    /// Seconds after launch before it starts guiding (clearing the
    /// aircraft).
    #[serde(default)]
    pub guidance_delay: f32,
    /// How well its seeker tells a flare from a jet (0..1) - the chance it
    /// ignores each flare that would otherwise pull it off: 0 falls for every
    /// one, 1 for none (see play::weapon::missile). Optional: 0.
    #[serde(default)]
    pub flare_resistance: f32,
}

/// `fuze: ( ... )` - when it goes off.
#[derive(Clone, Debug, Deserialize)]
pub struct Fuze {
    /// It goes off passing this close to the target (m).
    pub proximity_radius: f32,
    /// Seconds after launch before it can go off at all.
    #[serde(default)]
    pub arm_delay: f32,
    /// It self-destructs after this long (s).
    pub max_flight_time: f32,
}

/// `warhead: ( ... )`.
#[derive(Clone, Debug, Deserialize)]
pub struct Warhead {
    pub mass: f32,
    /// Within this far of the blast (m), an aircraft is destroyed.
    pub lethal_radius: f32,
}

/// `data.ron` as written - plane-like: model, physics, then its systems.
#[derive(Deserialize)]
struct WeaponData {
    model: WeaponModel,
    physics: Physics,
    #[serde(default)]
    motor: Motor,
    aero: Aero,
    guidance: Guidance,
    fuze: Fuze,
    warhead: Warhead,
}

fn one() -> f32 { 1.0 }
fn ninety() -> f32 { 90.0 }
fn four() -> f32 { 4.0 }

#[derive(Clone, Debug)]
pub struct WeaponSpec {
    /// Its folder under assets/weapons/ - what it's loaded by
    /// (`"missiles/aim-9x"`).
    pub id: String,
    pub category: WeaponCategory,
    /// The model, its file resolved to its full path.
    pub model: WeaponModel,
    /// The physics body - `physics.rigidbody.mass` is its launch mass.
    pub physics: Physics,
    pub motor: Motor,
    pub aero: Aero,
    pub guidance: Guidance,
    pub fuze: Fuze,
    pub warhead: Warhead,
}

impl WeaponSpec {
    /// The folder of weapon `id` (`"missiles/aim-9x"`).
    pub fn folder(id: &str) -> PathBuf {
        Path::new(WEAPONS_DIR).join(id)
    }

    /// Every weapon folder with a data.ron, by id, sorted - whether it loads
    /// or not.
    pub fn ids() -> Vec<String> {
        let mut ids = Vec::new();
        let Ok(categories) = std::fs::read_dir(WEAPONS_DIR) else { return ids };
        for category in categories.filter_map(Result::ok).filter(|entry| entry.path().is_dir()) {
            let Ok(weapons) = std::fs::read_dir(category.path()) else { continue };
            for weapon in weapons.filter_map(Result::ok) {
                if weapon.path().join("data.ron").is_file() {
                    ids.push(format!("{}/{}", category.file_name().to_string_lossy(), weapon.file_name().to_string_lossy()));
                }
            }
        }
        ids.sort();
        ids
    }

    /// Every weapon that loads, by id - ones that don't are reported and
    /// left out.
    pub fn available() -> Vec<String> {
        Self::ids().into_iter()
            .filter(|id| match Self::load(id) {
                Ok(_) => true,
                Err(error) => {
                    eprintln!("{error} - not available");
                    false
                }
            })
            .collect()
    }

    /// Loads weapon `id` (`"missiles/aim-9x"`) - its data checked (see
    /// `read`), its model file there.
    pub fn load(id: &str) -> Result<WeaponSpec, String> {
        let spec = Self::read(id)?;
        if !Path::new(&spec.model.model).is_file() {
            return Err(format!("weapon '{id}': model '{}' doesn't exist (export it from Blender as .glb)", spec.model.model));
        }
        Ok(spec)
    }

    /// `load` without the model file having to exist yet - the data alone.
    pub fn read(id: &str) -> Result<WeaponSpec, String> {
        let folder = Self::folder(id);
        let category_folder = id.split('/').next().unwrap_or_default();
        let category = WeaponCategory::from_folder(category_folder)
            .ok_or_else(|| format!("weapon '{id}': '{category_folder}' isn't a weapon category (missiles, bombs or guns)"))?;
        let data_path = folder.join("data.ron");
        let text = std::fs::read_to_string(&data_path)
            .map_err(|error| format!("weapon '{id}': couldn't read {}: {error}", data_path.display()))?;
        // Optional fields are written plainly, like in the planes' data.ron.
        let ron_options = ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
        let data: WeaponData = ron_options.from_str(&text)
            .map_err(|error| format!("weapon '{id}': {} isn't valid: {error}", data_path.display()))?;

        let mut model = data.model;
        model.model = folder.join(&model.model).to_string_lossy().into_owned();
        let spec = WeaponSpec {
            id: id.to_owned(),
            category,
            model,
            physics: data.physics,
            motor: data.motor,
            aero: data.aero,
            guidance: data.guidance,
            fuze: data.fuze,
            warhead: data.warhead,
        };
        spec.check().map_err(|error| format!("weapon '{id}': {error}"))?;
        Ok(spec)
    }

    /// Whether its numbers make sense - masses and sizes positive, the
    /// propellant lighter than the whole, the drag table in Mach order...
    fn check(&self) -> Result<(), String> {
        let positive = |value: f32, what: &str| if value > 0.0 { Ok(()) } else { Err(format!("{what} has to be more than 0 (it's {value})")) };
        let mass = self.physics.rigidbody.mass;
        positive(mass, "physics.rigidbody.mass")?;
        positive(self.aero.diameter, "aero.diameter")?;
        if self.motor.propellant_mass < 0.0 || self.motor.propellant_mass >= mass {
            return Err(format!("motor.propellant_mass ({}) has to be at least 0 and less than the mass ({mass})", self.motor.propellant_mass));
        }
        for (index, phase) in self.motor.phases.iter().enumerate() {
            positive(phase.duration, &format!("motor.phases[{index}].duration"))?;
            if phase.thrust < 0.0 {
                return Err(format!("motor.phases[{index}].thrust can't be negative"));
            }
        }
        if !self.motor.phases.is_empty() && self.motor.propellant_mass <= 0.0 {
            return Err("a motor with phases needs motor.propellant_mass".to_owned());
        }
        if self.aero.drag.is_empty() {
            return Err("aero.drag needs at least one point".to_owned());
        }
        if self.aero.drag.windows(2).any(|pair| pair[1].mach <= pair[0].mach) {
            return Err("aero.drag's points have to be in rising Mach order".to_owned());
        }
        positive(self.aero.max_g, "aero.max_g")?;
        positive(self.aero.max_g_speed, "aero.max_g_speed")?;
        if self.guidance.seeker != Seeker::None {
            positive(self.guidance.lock_range, "guidance.lock_range")?;
            positive(self.guidance.navigation_constant, "guidance.navigation_constant")?;
            if !(0.0..=180.0).contains(&self.guidance.gimbal_limit_deg) {
                return Err("guidance.gimbal_limit_deg has to be 0 to 180".to_owned());
            }
        }
        positive(self.fuze.proximity_radius, "fuze.proximity_radius")?;
        positive(self.fuze.max_flight_time, "fuze.max_flight_time")?;
        positive(self.warhead.lethal_radius, "warhead.lethal_radius")?;
        Ok(())
    }

    /// How long the motor burns, all phases (s).
    pub fn burn_time(&self) -> f32 {
        self.motor.phases.iter().map(|phase| phase.duration).sum()
    }

    /// The motor's thrust `time` seconds after launch (N) - 0 once it's
    /// burnt out.
    pub fn thrust_at(&self, time: f32) -> f32 {
        let mut start = 0.0;
        for phase in &self.motor.phases {
            if time >= start && time < start + phase.duration {
                return phase.thrust;
            }
            start += phase.duration;
        }
        0.0
    }

    /// Its mass `time` seconds after launch (kg) - the propellant burnt off
    /// evenly over the burn.
    pub fn mass_at(&self, time: f32) -> f32 {
        let burn = self.burn_time();
        let burnt = if burn > 0.0 { (time / burn).clamp(0.0, 1.0) } else { 0.0 };
        self.physics.rigidbody.mass - self.motor.propellant_mass * burnt
    }

    /// The body tube's cross-section - what drag acts on (m²).
    pub fn reference_area(&self) -> f32 {
        std::f32::consts::PI * (self.aero.diameter * 0.5).powi(2)
    }

    /// Its drag coefficient at `mach` - see `Aero::drag`.
    pub fn drag_coefficient(&self, mach: f32) -> f32 {
        let table = &self.aero.drag;
        let Some(first) = table.first() else { return 0.0 };
        if mach <= first.mach {
            return first.cd;
        }
        for pair in table.windows(2) {
            if mach <= pair[1].mach {
                let t = (mach - pair[0].mach) / (pair[1].mach - pair[0].mach);
                return pair[0].cd + (pair[1].cd - pair[0].cd) * t;
            }
        }
        table.last().map(|point| point.cd).unwrap_or(0.0)
    }

    /// The most G it can pull at `speed` (m/s) - see `Aero::max_g_speed`.
    pub fn max_g_at(&self, speed: f32) -> f32 {
        let fraction = (speed / self.aero.max_g_speed).clamp(0.0, 1.0);
        self.aero.max_g * fraction * fraction
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_weapon_folders_data_is_valid() {
        // The data only - a model still to be exported doesn't fail this.
        let errors: Vec<String> = WeaponSpec::ids().iter().filter_map(|id| WeaponSpec::read(id).err()).collect();
        assert!(errors.is_empty(), "{}", errors.join("\n"));
    }

    #[test]
    fn the_aim_9x_reads_like_the_real_one() {
        let aim9 = WeaponSpec::read("missiles/aim-9x").expect("assets/weapons/missiles/aim-9x should read");
        assert_eq!(aim9.category, WeaponCategory::Missile);
        assert_eq!(aim9.guidance.seeker, Seeker::InfraRed);
        assert_eq!(aim9.physics.colliders.len(), 1);
        assert!(aim9.model.model.ends_with("aim-9.glb"));
        // Burns ~5 s, ~27 kg of propellant gone after.
        assert!((aim9.burn_time() - 5.2).abs() < 0.5);
        assert!(aim9.thrust_at(1.0) > 0.0);
        assert_eq!(aim9.thrust_at(aim9.burn_time() + 0.1), 0.0);
        assert!((aim9.mass_at(0.0) - aim9.physics.rigidbody.mass).abs() < 1e-3);
        assert!(aim9.mass_at(100.0) < aim9.physics.rigidbody.mass - 20.0);
        // Enough motor to pass Mach 2 - its impulse over its average mass,
        // no drag: a rough upper bound on what it can reach.
        let impulse: f32 = aim9.motor.phases.iter().map(|phase| phase.thrust * phase.duration).sum();
        let average_mass = (aim9.mass_at(0.0) + aim9.mass_at(aim9.burn_time())) * 0.5;
        assert!(impulse / average_mass > 2.0 * SPEED_OF_SOUND, "delta-v {:.0} m/s", impulse / average_mass);
    }

    #[test]
    fn drag_and_g_follow_their_tables() {
        let aim9 = WeaponSpec::read("missiles/aim-9x").unwrap();
        let first = &aim9.aero.drag[0];
        assert_eq!(aim9.drag_coefficient(first.mach - 1.0), first.cd, "held below the table");
        assert_eq!(aim9.drag_coefficient(10.0), aim9.aero.drag.last().unwrap().cd, "held above it");
        assert_eq!(aim9.max_g_at(aim9.aero.max_g_speed * 2.0), aim9.aero.max_g);
        assert!((aim9.max_g_at(aim9.aero.max_g_speed * 0.5) - aim9.aero.max_g * 0.25).abs() < 1e-3, "a quarter at half the speed");
    }

    #[test]
    fn the_aim_9x_loads_with_its_model() {
        let aim9 = WeaponSpec::load("missiles/aim-9x").expect("the AIM-9X and its model should load");
        assert!(Path::new(&aim9.model.model).is_file());
        assert!(WeaponSpec::available().contains(&"missiles/aim-9x".to_owned()));
    }

    #[test]
    fn a_bad_category_folder_is_an_error() {
        assert!(WeaponSpec::read("rockets/nothing").is_err());
    }
}

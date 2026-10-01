use std::path::Path;

use serde::Deserialize;

use crate::engine::game_nodes::game_object::{ColliderType, Physics};

use super::aero_spec::AeroSpec;
use super::engine::EngineSpec;
use super::gear_spec::GearSpec;
use super::effects::EffectSpec;

/// Where every plane's folder lives - one folder per plane, named after it
/// (`assets/planes/f16/`), each with its own `data.ron` (see
/// `AircraftSpec::load`).
const PLANES_DIR: &str = "assets/planes";

pub struct Model {
    /// The model file, resolved to its full path inside the plane's own
    /// folder (`assets/planes/f16/model/f16.glb`).
    pub model: String,
    pub scale: f32,
    /// Meshes drawn two-sided (no backface culling) - e.g. the F-16's
    /// afterburner flame, seen from inside.
    pub double_sided: Vec<String>,
}

pub struct AircraftSpec {
    /// The plane's folder name - what it was loaded by (`"f16"`).
    pub name: String,
    pub model: Model,
    /// The physics body: rigidbody (is_static, mass, center of mass,
    /// initial velocity) and colliders - `data.ron`'s `physics: ( ... )`.
    pub physics: Physics,
    /// The flight physics' wings/surfaces and the pilot's seat -
    /// `data.ron`'s `aero: ( ... )`. Airfoil table paths are resolved inside
    /// the plane's folder (`aero/f16-wing-body.ron` ->
    /// `assets/planes/f16/aero/f16-wing-body.ron`).
    pub aero: AeroSpec,
    /// Thrust, afterburner and spool times - `data.ron`'s `engine: ( ... )`.
    pub engine: EngineSpec,
    /// The wheels and their steering - `data.ron`'s `gear: ( ... )`.
    pub gear: GearSpec,
    /// Every particle emitter on the plane, and where - `data.ron`'s
    /// `effects: [ ... ]` (optional). Each wheel in `gear` also gets tyre
    /// smoke on its own.
    pub effects: Vec<EffectSpec>,
}

/// A plane's `data.ron`, as written in the file. Paths in it are relative to
/// the plane's own folder.
#[derive(Deserialize)]
struct PlaneData {
    model: ModelData,
    physics: Physics,
    aero: AeroSpec,
    engine: EngineSpec,
    gear: GearSpec,
    #[serde(default)]
    effects: Vec<EffectSpec>,
}

/// `data.ron`'s `model: ( ... )` - mirrors `Model`.
#[derive(Deserialize)]
struct ModelData {
    model: String,
    /// Optional - 1.0 when left out.
    #[serde(default = "default_scale")]
    scale: f32,
    /// Optional - mesh names drawn two-sided.
    #[serde(default)]
    double_sided: Vec<String>,
}

fn default_scale() -> f32 {
    1.0
}

impl AircraftSpec {
    /// The plane `name`'s data.ron - `assets/planes/<name>/data.ron`.
    pub fn data_path(name: &str) -> std::path::PathBuf {
        Path::new(PLANES_DIR).join(name).join("data.ron")
    }

    /// Every plane folder under assets/planes/ that loads, sorted - what a
    /// map's aircraft can be (see play::flight_editor). Ones that don't load
    /// are reported and left out.
    pub fn available() -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(PLANES_DIR) else { return Vec::new() };
        let mut planes: Vec<String> = entries.filter_map(Result::ok)
            .filter(|entry| entry.path().join("data.ron").is_file())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| match Self::load(name) {
                Ok(_) => true,
                Err(error) => {
                    eprintln!("{error} - not offered in the flight editor");
                    false
                }
            })
            .collect();
        planes.sort();
        planes
    }

    /// Loads the plane in `assets/planes/<name>/`, from its `data.ron`. Its
    /// model file is looked up inside that same folder: `model: (model:
    /// "model/f16.glb", ...)` in `assets/planes/f16/data.ron` is
    /// `assets/planes/f16/model/f16.glb`.
    pub fn load(name: &str) -> Result<AircraftSpec, String> {
        let folder = Path::new(PLANES_DIR).join(name);
        let data_path = Self::data_path(name);

        let text = std::fs::read_to_string(&data_path)
            .map_err(|error| format!("plane '{name}': couldn't read {}: {error}", data_path.display()))?;
        // Optional fields (a wing's `shape`, `aerodynamic_center`...) are
        // written plainly, without RON's `Some( ... )` around them.
        let ron_options = ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
        let data: PlaneData = ron_options.from_str(&text)
            .map_err(|error| format!("plane '{name}': {} isn't valid: {error}", data_path.display()))?;

        let model_path = folder.join(&data.model.model);
        if !model_path.is_file() {
            return Err(format!("plane '{name}': model '{}' (from {}) doesn't exist", model_path.display(), data_path.display()));
        }

        // Airfoil tables live in the plane's folder too.
        let mut aero = data.aero;
        for wing in &mut aero.wings {
            // A wing given by its shape gets its area, chord and lift point
            // worked out from it.
            wing.resolve_shape().map_err(|error| format!("plane '{name}': {error}"))?;
            let table = folder.join(&wing.airfoil_path);
            if !table.is_file() {
                return Err(format!("plane '{name}': wing '{}' airfoil table '{}' doesn't exist", wing.label, table.display()));
            }
            wing.airfoil_path = table.to_string_lossy().into_owned();
        }

        // Wing colliders follow their wing's shape.
        let mut physics = data.physics;
        for collider in &mut physics.colliders {
            let ColliderType::Wing { label, thickness } = collider else { continue };
            let wing = aero.wings.iter().find(|wing| wing.label == *label)
                .ok_or_else(|| format!("plane '{name}': collider Wing(label: \"{label}\") - no wing has that label"))?;
            let shape = wing.shape.as_ref()
                .ok_or_else(|| format!("plane '{name}': collider Wing(label: \"{label}\") - that wing has no `shape` to follow"))?;
            // The planform's corners, pushed half the thickness either way
            // across the wing (perpendicular to its span and to the chord).
            let across = shape.span_axis(&wing.normal).cross(&nalgebra::Vector3::z()).try_normalize(1e-6).unwrap_or_else(nalgebra::Vector3::y);
            let half = across * (*thickness * 0.5);
            let points = shape.corners(&wing.normal).iter().flat_map(|corner| [corner + half, corner - half]).collect();
            *collider = ColliderType::ConvexHull { points };
        }

        Ok(AircraftSpec {
            name: name.to_owned(),
            model: Model {
                model: model_path.to_string_lossy().into_owned(),
                scale: data.model.scale,
                double_sided: data.model.double_sided,
            },
            physics,
            aero,
            engine: data.engine,
            gear: data.gear,
            effects: data.effects,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_the_f16_from_its_folder() {
        let spec = AircraftSpec::load("f16").expect("assets/planes/f16 should load");
        assert_eq!(spec.name, "f16");
        let expected = Path::new(PLANES_DIR).join("f16").join("model/f16.glb");
        assert_eq!(Path::new(&spec.model.model), expected);
        assert!(Path::new(&spec.model.model).is_file());
        assert_eq!(spec.physics.rigidbody.mass, 11_900.0);
        assert_eq!(spec.physics.colliders.len(), 3);
        assert_eq!(spec.aero.wings.len(), 5);
        // The engine section loads, and gives the same thrust curve the old
        // constants did (idle / military at the gate / full afterburner).
        assert_eq!(spec.engine.target_thrust(0.0, 0.0), 4_000.0);
        assert_eq!(spec.engine.target_thrust(0.85, 0.0), 76_000.0);
        assert_eq!(spec.engine.target_thrust(1.0, 0.0), 129_000.0);
        assert_eq!(spec.engine.afterburner_activation(0.85), 0.0);
        // Shaped wings get their area, chord and lift point from their shape
        // (checked against the shape itself, so tuning data.ron doesn't
        // break this), all-moving ones move their whole area, and a
        // negative span mirrors a wing.
        for wing in &spec.aero.wings {
            let Some(shape) = &wing.shape else { continue };
            assert!((wing.chord - shape.mean_aerodynamic_chord()).abs() < 1e-4, "{}: chord {}", wing.label, wing.chord);
            let (span_fraction, chord_fraction) = wing.aerodynamic_center.unwrap_or((shape.mac_span_fraction(), 0.25));
            assert!((wing.pressure_center - shape.point(&wing.normal, span_fraction, chord_fraction)).norm() < 1e-4, "{}: center", wing.label);
            if wing.control_surface_area > 0.0 {
                assert_eq!(wing.control_surface_area, wing.wing_area, "{}: all-moving area", wing.label);
            }
        }
        let left = spec.aero.wings.iter().find(|wing| wing.label == "Left wing").unwrap();
        let right = spec.aero.wings.iter().find(|wing| wing.label == "Right wing").unwrap();
        assert!((left.pressure_center.x + right.pressure_center.x).abs() < 1e-4, "wings mirror: {:?} / {:?}", left.pressure_center, right.pressure_center);
    }

    #[test]
    fn every_plane_folder_loads() {
        // Each folder in assets/planes/ with a data.ron is a plane - all of
        // them should load, so a broken file shows up here, not mid-level.
        let mut errors = Vec::new();
        for entry in std::fs::read_dir(PLANES_DIR).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !entry.path().join("data.ron").is_file() {
                continue;
            }
            if let Err(error) = AircraftSpec::load(&name) {
                errors.push(error);
            }
        }
        assert!(errors.is_empty(), "{}", errors.join("
"));
    }

    #[test]
    fn a_missing_plane_is_an_error() {
        assert!(AircraftSpec::load("no_such_plane").is_err());
    }
}

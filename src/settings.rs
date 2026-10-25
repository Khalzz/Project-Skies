//! Project settings and the scene settings that override them.
//!
//! The project (`app.settings`) holds the defaults every scene uses. A scene
//! changes one only for itself by setting its own (`scene.settings`), and
//! everything it leaves as `None` stays the project's:
//!
//!     app.settings.physics.gravity = Vector3::new(0.0, -9.81, 0.0); // every scene
//!     scene.settings.gravity = Some(Vector3::new(0.0, -1.62, 0.0));  // only this one
//!
//! Both can change while a scene runs - App::run applies whatever the active
//! scene ends up with every frame.

use nalgebra::Vector3;

use crate::rendering::enviroment::environment::Environment;

/// The sun - where its light comes from and its color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lighting {
    /// Toward the sun - the light travels the other way. Any length.
    pub sun_direction: Vector3<f32>,
    /// Linear RGB, 0..1 (more for a brighter sun).
    pub sun_color: [f32; 3],
}

impl Default for Lighting {
    fn default() -> Self {
        Self { sun_direction: Vector3::new(0.3, 1.0, 0.2), sun_color: [1.0, 1.0, 1.0] }
    }
}

/// The world's air and water, as particles see them - drifting with the wind,
/// stopping at the sea.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleSettings {
    /// Where the wind blows toward, in m/s.
    pub wind: Vector3<f32>,
    /// The height (world Y) of a sea particles splash on. None: no sea.
    pub sea_level: Option<f32>,
}

impl Default for ParticleSettings {
    fn default() -> Self {
        Self { wind: Vector3::zeros(), sea_level: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicsSettings {
    /// The default gravity - a scene can override it (`SceneSettings::gravity`).
    pub gravity: Vector3<f32>,
    /// How long one physics step is, in seconds - the same for every scene.
    /// A change applies from the next scene opened.
    pub fixed_timestep: f32,
}

impl Default for PhysicsSettings {
    fn default() -> Self {
        Self { gravity: Vector3::new(0.0, -9.81, 0.0), fixed_timestep: 1.0 / 120.0 }
    }
}

/// The project's defaults - `app.settings`.
#[derive(Debug, Clone, Default)]
pub struct ProjectSettings {
    pub physics: PhysicsSettings,
    /// What's drawn behind everything.
    pub environment: Environment,
    pub lighting: Lighting,
    pub particles: ParticleSettings,
}

/// What one scene sets differently from the project - `scene.settings`.
/// `None` (the default for each) is "the project's".
#[derive(Debug, Clone, Default)]
pub struct SceneSettings {
    pub gravity: Option<Vector3<f32>>,
    pub environment: Option<Environment>,
    pub lighting: Option<Lighting>,
    pub particles: Option<ParticleSettings>,
}

impl SceneSettings {
    /// The gravity this scene runs with - its own, or the project's.
    pub fn gravity(&self, project: &ProjectSettings) -> Vector3<f32> {
        self.gravity.unwrap_or(project.physics.gravity)
    }

    pub fn lighting(&self, project: &ProjectSettings) -> Lighting {
        self.lighting.unwrap_or(project.lighting)
    }

    pub fn particles(&self, project: &ProjectSettings) -> ParticleSettings {
        self.particles.unwrap_or(project.particles)
    }
}

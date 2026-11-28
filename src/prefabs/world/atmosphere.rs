/*
    The air at any height, from the International Standard Atmosphere: set the conditions at sea level and
    everything else follows. Lives as a property on the scene's "world" node.
*/

use nalgebra::Vector3;
use pankarta_software::prelude::{Node, SceneAccess, WorldOrigin};

// The node that carries the scene's Atmosphere
pub const WORLD_NODE: &str = "world";

const GAS_CONSTANT: f32 = 287.05; // J/(kg·K), dry air
const HEAT_RATIO: f32 = 1.4;
const GRAVITY: f32 = 9.80665;
const LAPSE_RATE: f32 = 0.0065; // K per meter, up to the tropopause
const KNOTS_PER_MS: f32 = 1.943_844;
const TROPOPAUSE: f32 = 11_000.0; // meters; above it the temperature stops falling

#[derive(Debug, Clone, Copy)]
pub struct Air {
    pub density: f32,        // kg/m³
    pub temperature: f32,    // K
    pub pressure: f32,       // Pa
    pub speed_of_sound: f32, // m/s
}

// Weather convention: `from` is the heading it blows from (0 north = -Z, 90 east = +X)
#[derive(Debug, Clone, Copy)]
pub struct Wind {
    pub from: f32,  // degrees
    pub speed: f32, // m/s
}

#[derive(Debug, Clone, Copy)]
pub struct Atmosphere {
    pub sea_level: f32,             // height of the sea, meters (in the world; `of` hands it in the scene's local numbers)
    pub sea_level_temperature: f32, // K
    pub sea_level_pressure: f32,    // Pa
    pub wind: Wind,
}

impl Atmosphere {
    pub const STANDARD: Atmosphere = Atmosphere {
        sea_level: 0.0,
        sea_level_temperature: 288.15,
        sea_level_pressure: 101_325.0,
        wind: Wind {
            from: 0.0,
            speed: 0.0,
        },
    };

    // The scene's, from its world node (the standard day without one), with the sea where the floating origin has
    // moved it, so heights compare with the scene's positions
    pub fn of(scene: &SceneAccess, origin: &WorldOrigin) -> Atmosphere {
        Self::from_world(scene.get_node(WORLD_NODE), origin)
    }

    pub fn from_world(world: Option<&Node>, origin: &WorldOrigin) -> Atmosphere {
        let mut atmosphere = world
            .and_then(|world| world.get_property::<Atmosphere>())
            .copied()
            .unwrap_or(Atmosphere::STANDARD);
        atmosphere.sea_level -= origin.origin().y as f32;
        atmosphere
    }

    pub fn with_sea_level(mut self, sea_level: f32) -> Atmosphere {
        self.sea_level = sea_level;
        self
    }

    // Given like a weather report: the heading it blows from, and knots
    pub fn with_wind(mut self, from: f32, knots: f32) -> Atmosphere {
        self.wind = Wind {
            from,
            speed: knots / KNOTS_PER_MS,
        };
        self
    }

    // The air's velocity at a world position; the same everywhere for now
    pub fn wind_at(&self, _position: Vector3<f32>) -> Vector3<f32> {
        let from = self.wind.from.to_radians();
        -Vector3::new(from.sin(), 0.0, -from.cos()) * self.wind.speed
    }

    // The air at a world height
    pub fn air_at(&self, height: f32) -> Air {
        let altitude = height - self.sea_level;
        let exponent = GRAVITY / (LAPSE_RATE * GAS_CONSTANT);

        let tropopause_temperature = self.sea_level_temperature - LAPSE_RATE * TROPOPAUSE;
        let (temperature, pressure) = if altitude <= TROPOPAUSE {
            let temperature = self.sea_level_temperature - LAPSE_RATE * altitude;
            let pressure = self.sea_level_pressure
                * (temperature / self.sea_level_temperature).powf(exponent);
            (temperature, pressure)
        } else {
            let tropopause_pressure = self.sea_level_pressure
                * (tropopause_temperature / self.sea_level_temperature).powf(exponent);
            let pressure = tropopause_pressure
                * (-GRAVITY * (altitude - TROPOPAUSE) / (GAS_CONSTANT * tropopause_temperature))
                    .exp();
            (tropopause_temperature, pressure)
        };

        Air {
            density: pressure / (GAS_CONSTANT * temperature),
            temperature,
            pressure,
            speed_of_sound: (HEAT_RATIO * GAS_CONSTANT * temperature).sqrt(),
        }
    }
}

impl Default for Atmosphere {
    fn default() -> Self {
        Atmosphere::STANDARD
    }
}

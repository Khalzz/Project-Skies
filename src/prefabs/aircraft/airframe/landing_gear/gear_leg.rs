use nalgebra::{UnitQuaternion, Vector3};
use pankarta_software::prelude::Transform3D;

use super::wheel::Wheel;

const GAS_STIFFENING: f32 = 2.0; // how much stiffer the spring is at full stroke (3x)
const BOTTOM_OUT: f32 = 20.0; // the stop past full stroke, times the spring rate
const REBOUND: f32 = 2.5; // extending damping, times the compressing one

#[derive(Debug, Clone)]
pub struct LegSpec {
    pub mount: Vector3<f32>,
    pub length: f32,
    pub spring_rate: f32,
    pub damping: f32,
    pub stroke: f32, // how far the strut can compress before it bottoms out, meters
    pub retracted: Transform3D,
    pub wheel: Wheel,
}

impl LegSpec {
    pub fn new(mesh: impl Into<String>, mount: Vector3<f32>, length: f32) -> Self {
        Self {
            mount,
            length,
            spring_rate: 0.0,
            damping: 0.0,
            stroke: length * 0.2,
            retracted: Transform3D {
                position: mount,
                ..Default::default()
            },
            wheel: Wheel::new(mesh),
        }
    }

    pub fn spring(mut self, spring_rate: f32, damping: f32) -> Self {
        self.spring_rate = spring_rate;
        self.damping = damping;
        self
    }

    pub fn stroke(mut self, stroke: f32) -> Self {
        self.stroke = stroke;
        self
    }

    pub fn retracted(mut self, retracted: Transform3D) -> Self {
        self.retracted = retracted;
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.wheel.radius = radius;
        self
    }

    pub fn steerable(mut self) -> Self {
        self.wheel.steerable = true;
        self
    }

    pub fn braked(mut self) -> Self {
        self.wheel.braked = true;
        self
    }
}

#[derive(Debug, Clone)]
pub struct Contact {
    pub ground: Option<String>,
    pub point: Vector3<f32>,
    pub compression: f32,
    pub load: f32,
}

#[derive(Debug, Clone)]
pub struct GearLeg {
    pub spec: LegSpec,
    pub contact: Option<Contact>,
    pub anchor: Option<Vector3<f32>>, // where the stopped tyre sits stuck to the ground
    pub broke_free: bool,             // pushed off its anchor, rolling until it picks up speed
    pub side_stretch: f32,            // how far the tyre's rubber is bent sideways, meters
}

impl GearLeg {
    pub fn new(spec: LegSpec) -> Self {
        Self {
            spec,
            contact: None,
            anchor: None,
            broke_free: false,
            side_stretch: 0.0,
        }
    }

    /*
        An oleo strut: its gas spring stiffens the further it's pushed in, it bottoms out hard past its stroke, and
        its oil resists extending more than compressing so a landing doesn't spring the jet back into the air
    */
    pub fn suspension_force(&self, compression: f32, closing_speed: f32) -> f32 {
        let spec = &self.spec;
        let travel = compression / spec.stroke;
        let spring = spec.spring_rate * compression * (1.0 + GAS_STIFFENING * travel * travel);
        let bottomed = spec.spring_rate * BOTTOM_OUT * (compression - spec.stroke).max(0.0);
        let damping = if closing_speed > 0.0 {
            spec.damping
        } else {
            spec.damping * REBOUND
        };
        (spring + bottomed + damping * closing_speed).max(0.0)
    }

    // The axle: the strut ends where the tyre touches the ground, the axle sits one radius above it
    pub fn extended_position(&self) -> Vector3<f32> {
        let compression = self
            .contact
            .as_ref()
            .map_or(0.0, |contact| contact.compression.min(self.spec.stroke));
        self.spec.mount - Vector3::y() * (self.spec.length - compression - self.spec.wheel.radius)
    }

    pub fn pose(&self, deployed: f32, steer_angle: f32) -> Transform3D {
        let deployed = deployed.clamp(0.0, 1.0);
        let angle = if self.spec.wheel.steerable {
            steer_angle
        } else {
            0.0
        };
        let down = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), -angle);
        Transform3D {
            position: self
                .spec
                .retracted
                .position
                .lerp(&self.extended_position(), deployed),
            rotation: self.spec.retracted.rotation.slerp(&down, deployed),
            scale: self.spec.retracted.scale,
        }
    }
}

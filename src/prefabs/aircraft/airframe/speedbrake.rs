use nalgebra::{UnitQuaternion, UnitVector3, Vector3};
use pankarta_software::{
    rendering::gizmos::{GizmoColor, GizmoDrawer, Pose},
    scene_manager::node::properties::Model,
};

use crate::prefabs::aircraft::airframe::wings::surface::AeroForce;

/*
    The speedbrakes (airbrakes) are panels that open into the air only to make drag: they don't lift,
    so they're not a Surface, just flat plates turning about a hinge. The cockpit asks for a share of
    their travel (0 closed .. 1 fully open); here's how far that is, how fast they get there and what
    drag they make.

    The F-16's are clamshells beside the nozzle: an upper petal opening up and a lower one opening
    down on each side, 60 degrees each.
*/

const PLATE_DRAG: f32 = 1.2; // a flat plate square to the air

#[derive(Clone)]
pub struct Panel {
    pub hinge: Vector3<f32>,       // the hinge line's middle, in the aircraft's space
    pub axis: UnitVector3<f32>,    // it opens turning about this, by the right hand rule
    pub chord: Vector3<f32>,       // closed, from the hinge to the trailing edge
    pub width: f32,                // along the hinge, meters
    pub mesh: Option<String>,      // the model's mesh, turned about the hinge; its origin on the hinge
}

impl Panel {
    pub fn area(&self) -> f32 {
        self.width * self.chord.norm()
    }

    fn turn(&self, deflection: f32) -> UnitQuaternion<f32> {
        UnitQuaternion::from_axis_angle(&self.axis, deflection.to_radians())
    }

    // Leading left, leading right, trailing right, trailing left, open `deflection` degrees
    fn corners(&self, deflection: f32) -> [Vector3<f32>; 4] {
        let half_span = self.axis.into_inner() * (self.width / 2.0);
        let chord = self.turn(deflection) * self.chord;
        [
            self.hinge - half_span,
            self.hinge + half_span,
            self.hinge + half_span + chord,
            self.hinge - half_span + chord,
        ]
    }

    fn middle(&self, deflection: f32) -> Vector3<f32> {
        self.hinge + self.turn(deflection) * self.chord * 0.5
    }
}

pub struct Speedbrake {
    pub panels: Vec<Panel>,
    pub max_deflection: f32, // degrees, fully open
    pub rate: f32,           // degrees per second
    deflection: f32,         // where they are now, degrees
    drag: f32,               // what they made last step, newtons
}

impl Speedbrake {
    pub fn new(panels: Vec<Panel>, max_deflection: f32, rate: f32) -> Speedbrake {
        Speedbrake {
            panels,
            max_deflection,
            rate,
            deflection: 0.0,
            drag: 0.0,
        }
    }

    // An aircraft without any
    pub fn none() -> Speedbrake {
        Speedbrake::new(vec![], 0.0, 0.0)
    }

    pub fn deflection(&self) -> f32 {
        self.deflection
    }

    // Moves toward the requested share of its travel
    pub fn command(&mut self, request: f32, dt: f32) {
        let target = request.clamp(0.0, 1.0) * self.max_deflection;
        let step = self.rate * dt;
        self.deflection += (target - self.deflection).clamp(-step, step);
    }

    // Each open panel's drag: a flat plate showing the air its area times the sine of its opening
    pub fn aero_forces(
        &mut self,
        airflow: impl Fn(Vector3<f32>) -> Vector3<f32>,
        density: f32,
    ) -> Vec<AeroForce> {
        self.drag = 0.0;
        if self.deflection <= 0.0 {
            return vec![];
        }
        let showing = self.deflection.to_radians().sin();
        let forces: Vec<AeroForce> = self
            .panels
            .iter()
            .filter_map(|panel| {
                let point = panel.middle(self.deflection);
                let air = airflow(point);
                let speed = air.norm();
                let along = air.try_normalize(1e-3)?;
                let pressure = 0.5 * density * speed * speed;
                Some(AeroForce {
                    point,
                    lift: Vector3::zeros(),
                    drag: along * pressure * PLATE_DRAG * panel.area() * showing,
                    airflow: air,
                    alpha: self.deflection,
                    section: true,
                })
            })
            .collect();
        self.drag = forces.iter().map(|force| force.drag.norm()).sum();
        forces
    }

    pub fn place(&self, model: &mut Model) {
        for panel in &self.panels {
            let Some(name) = &panel.mesh else { continue };
            if let Some(mesh) = model.mesh_mut(name) {
                let base = mesh.original_transform().rotation;
                mesh.transform.rotation = panel.turn(self.deflection) * base;
            }
        }
    }

    // Each panel's outline: cyan closed, orange open, its opening and drag on the first one
    pub fn debug(&self, g: &mut GizmoDrawer, pose: Pose) {
        let open = self.deflection > 0.05;
        let color = if open { GizmoColor::ORANGE } else { GizmoColor::CYAN };
        for (index, panel) in self.panels.iter().enumerate() {
            let outline = panel.corners(self.deflection).map(|corner| pose.point(corner));
            let line = g.outline(&outline);
            line.color(color);
            if index == 0 {
                line.label(format!(
                    "speedbrake {:.0}° {:.1} kN",
                    self.deflection,
                    self.drag / 1000.0
                ));
            }
        }
    }
}

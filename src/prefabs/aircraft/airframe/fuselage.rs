/*
    Fuselage its the physical definition of the body of the plane: the shape it bumps into things with (its
    colliders) and the shape the air pushes on (its aero), both from the same rings that describe it.
*/

use nalgebra::Vector3;
use pankarta_software::physics::Colliders;

use crate::prefabs::aircraft::airframe::wings::surface::AeroForce;

// A slender body moving a little sideways tries to turn further sideways (Munk): the share of the ideal moment
// a real fuselage keeps, less than its ~0.9 alone as the wings and tail break up the flow along it
const MUNK_FACTOR: f32 = 0.7;
// Air crossing a round body side-on, like a cylinder in a crosswind
const CROSSFLOW_DRAG: f32 = 1.2;

// One ring of the body: (meters forward from its section's tail end, radius, offset, stretch)
pub type Ring = (f32, f32, (f32, f32), (f32, f32));

pub struct FuselageAero {
    pub volume: f32,                 // m³
    pub length: f32,                 // m, nose to tail
    pub center: Vector3<f32>,        // its volume's middle, in the aircraft's space
    pub side_area: f32,              // m², seen from the side
    pub side_center: Vector3<f32>,
    pub top_area: f32,               // m², seen from above
    pub top_center: Vector3<f32>,
    pub drag_area: f32,              // m², drag coefficient × area of everything the wings don't count
}

impl FuselageAero {
    // From the same sections of rings the colliders are made of: each a tail end and rings running forward (-Z)
    pub fn from_rings(sections: &[(Vector3<f32>, &[Ring])], drag_area: f32) -> FuselageAero {
        let (mut volume, mut side_area, mut top_area) = (0.0, 0.0, 0.0);
        let (mut center, mut side_center, mut top_center) = (Vector3::zeros(), Vector3::zeros(), Vector3::zeros());
        let (mut nose, mut tail) = (f32::MAX, f32::MIN);

        for &(tail_end, rings) in sections {
            for pair in rings.windows(2) {
                let (aft, fore) = (pair[0], pair[1]);
                let step = fore.0 - aft.0;
                let width = aft.1 * (aft.3).0 + fore.1 * (fore.3).0; // the two rings' widths, averaged
                let height = aft.1 * (aft.3).1 + fore.1 * (fore.3).1;
                let middle = tail_end
                    + Vector3::new(0.0, -((aft.2).1 + (fore.2).1) / 2.0, -(aft.0 + fore.0) / 2.0);

                let slice = std::f32::consts::PI * width / 2.0 * height / 2.0 * step;
                volume += slice;
                center += middle * slice;
                side_area += height * step;
                side_center += middle * height * step;
                top_area += width * step;
                top_center += middle * width * step;
                nose = nose.min(tail_end.z - fore.0);
                tail = tail.max(tail_end.z - aft.0);
            }
        }

        if volume <= 0.0 {
            return FuselageAero::none();
        }

        FuselageAero {
            volume,
            length: tail - nose,
            center: center / volume,
            side_area,
            side_center: side_center / side_area,
            top_area,
            top_center: top_center / top_area,
            drag_area,
        }
    }

    // No body for the air to push on
    pub fn none() -> FuselageAero {
        FuselageAero {
            volume: 0.0,
            length: 0.0,
            center: Vector3::zeros(),
            side_area: 0.0,
            side_center: Vector3::zeros(),
            top_area: 0.0,
            top_center: Vector3::zeros(),
            drag_area: 0.0,
        }
    }

    // The forces the air puts on the body, in the aircraft's space (see Surface::aero_forces)
    pub fn aero_forces(&self, airflow: impl Fn(Vector3<f32>) -> Vector3<f32>, density: f32) -> Vec<AeroForce> {
        let air = airflow(self.center);
        let speed = air.norm();
        if speed < 1e-3 || self.volume <= 0.0 {
            return vec![];
        }
        let nose = -Vector3::z();
        let across = air - nose * air.dot(&nose); // the air crossing the body, not running along it
        let pressure = 0.5 * density * speed * speed;
        let mut forces = vec![];

        // Along the body: its drag
        forces.push(AeroForce {
            point: self.center,
            lift: Vector3::zeros(),
            drag: air / speed * pressure * self.drag_area,
            airflow: air,
            alpha: 0.0,
            section: false,
        });

        // Munk: a pure turning moment toward broadside, as two equal, opposite forces at the ends
        let velocity = -air;
        let moment = velocity.dot(&nose) * velocity.cross(&nose) * density * self.volume * MUNK_FACTOR;
        let push = moment.cross(&nose) / self.length;
        for (end, force) in [(1.0, push), (-1.0, -push)] {
            forces.push(AeroForce {
                point: self.center + nose * end * self.length / 2.0,
                lift: force,
                drag: Vector3::zeros(),
                airflow: air,
                alpha: 0.0,
                section: true,
            });
        }

        // Crossflow: the air crossing it pushes on its side and its top like on a cylinder
        let crossing = 0.5 * density * across.norm() * CROSSFLOW_DRAG;
        for (point, area, direction) in [
            (self.side_center, self.side_area, Vector3::x()),
            (self.top_center, self.top_area, Vector3::y()),
        ] {
            forces.push(AeroForce {
                point,
                lift: Vector3::zeros(),
                drag: direction * across.dot(&direction) * crossing * area,
                airflow: air,
                alpha: 0.0,
                section: true,
            });
        }

        forces
    }
}

pub struct Fuselage {
    pub colliders: Colliders,
    pub aero: FuselageAero,
}

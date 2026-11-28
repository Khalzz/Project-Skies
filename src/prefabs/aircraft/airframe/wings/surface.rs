/*
    pub struct Wing {
        // General identification
        pub label: String,

        // prerequisites for calculations
        pub air_foil: AirFoil,

        // Physics and colliders: Planes contain multiple elements in each surface divided by functionality
            // Aerodynaic surfaces: Wings, Stabilizers (horizontal and vertical)


            // Control surfaces: Elevators, Ailerons, Flaps, Rudders
            max deflection // this used to be made by if its roll axis so i think we should check the real values
            pub incidence_angle: f32, // Check why we have it
            pub chord: f32, // Check what it does
            pub surfaces: Vec<ControlSurfaceSpec>, // is it the "control surface space??"
            pub control_surface_area: f32, // this can be calculated from the control surface spec???

        // Input: elements related to controling the forces applied to the wing
        pub flap_input: f32,

        // Wing Integrity: health of the wing
        pub hp: f32,
        pub max_hp: f32,
    }
*/

/*
pub struct Geometry {
    pub pressure_center: nalgebra::Vector3<f32>,
    pub wing_area: f32, // is it needed to be public???
    pub normal: nalgebra::Vector3<f32>,
    pub shape: Option<WingShape>,
    area: f32,
}
*/

/*
    An Airframe Surface its mostly a body that contains habilities of generating either lift or a deviation of it
    these are mainly thought to follow the structure of:

    A surface its a body that contains different settings defining his "form" but also contains multiple shapes
    a shape its mostly defined ideally as a definitor of what the wing is, for example:

    A normal wing its nothing but a main shape, thats "static" while it contains 3 shapes inside that aren't,
    these are "Flaps, slats and ailerons" but for example the f16 instead has "slats and flaperons"

    The definition of theese "sub surfaces" should ideally be defined in a "comfortable way" and since they
    variate on each plane, their configuration should allow us to define them in multiple ways

    Main wing {
        Shape: (defined like we do now)
        Sections: [
            {
                Type: Flaperon,
                Shape: defined like we do now
            },
            {
                Type: slat,
                Shape: defined like we do now
            }
        ]
    }
*/

use std::f32::consts::PI;

use nalgebra::{Point3, UnitVector3, Vector3};
use pankarta_software::prelude::ColliderBuilder;
use pankarta_software::{
    app::App, rendering::gizmos::Icon, scene_manager::node::properties::Model,
};

use crate::prefabs::aircraft::{
    airframe::wings::{
        airfoil::Airfoil,
        body::{Edge, Motion, Sections, SurfaceBody},
    },
    flight_controls::flight_controls::Commands,
};

// How far a real wing's lift distribution is from the ideal elliptical one
const SPAN_EFFICIENCY: f32 = 0.8;
// The strake vortices hold up to the first angle, then break down by the second (degrees)
const VORTEX_BURST: (f32, f32) = (30.0, 50.0);

pub struct Integrity {
    pub hp: f32,
    max_hp: f32,
}

impl Integrity {
    pub fn new(max_hp: f32) -> Integrity {
        Integrity { hp: max_hp, max_hp }
    }
}

pub struct Surface {
    pub label: String,
    pub airfoil: Airfoil,
    pub integrity: Integrity,
    pub body: SurfaceBody,
    pub root: Vector3<f32>, // root leading edge, in the aircraft's own space
    pub span_axis: UnitVector3<f32>, // which way the tip points
    alpha_scale: f32,       // how much of its angle the section feels, the rest lost to its tip vortices
    section_slope: f32,     // the section's lift per radian
    zero_lift: f32,         // the section's, degrees
    stall: (f32, f32),      // the section's, degrees (negative, positive)
    induced_drag: f32,      // drag per Cl², from the tip vortices
    vortex_lift: f32,       // strake vortex strength (Polhamus Kv), 0 without strakes
    thickness: f32,         // the section's thickest over its chord (the NACA name's last two digits: 64A204 is 4%)
}

// An f16 contains flaperons (flaps that work as alerons) and slats
impl Surface {
    pub fn new(
        label: &str,
        health: f32,
        body: SurfaceBody,
        root: Vector3<f32>,
        span_axis: UnitVector3<f32>,
        airfoil: Airfoil,
    ) -> Surface {
        let planform = body.planform;
        let aspect_ratio = planform.aspect_ratio();
        let sweep = planform.half_chord_sweep().tan();
        let finite_slope = 2.0 * PI * aspect_ratio
            / (2.0 + (4.0 + aspect_ratio * aspect_ratio * (1.0 + sweep * sweep)).sqrt());

        Surface {
            label: label.to_string(),
            integrity: Integrity::new(health),
            alpha_scale: finite_slope / airfoil.lift_slope(),
            section_slope: airfoil.lift_slope(),
            zero_lift: airfoil.zero_lift_alpha(),
            stall: airfoil.stall_alphas(),
            induced_drag: 1.0 / (PI * SPAN_EFFICIENCY * aspect_ratio),
            vortex_lift: 0.0,
            thickness: 0.06,
            body,
            root,
            span_axis,
            airfoil,
        }
    }

    // Strakes ahead of the wing roll the air into vortices over it that keep lifting at high angles
    pub fn with_vortex_lift(mut self, strength: f32) -> Surface {
        self.vortex_lift = strength;
        self
    }

    pub fn with_thickness(mut self, thickness: f32) -> Surface {
        self.thickness = thickness;
        self
    }

    /*
        Its solid shape, from the same planform the air sees: each corner half the local thickness above and
        below it along its lift side (the thickness follows the chord, so the root is deep and the tip thin), the
        hull around those 8 points. A swept, tapered panel is convex, so the hull is the panel. No mass of its
        own (density 0): the jet's mass and inertia are given whole, so it only adds what can be hit. Fixed at
        no deflection, a moving surface (a stabilator) keeps its neutral shape.
    */
    pub fn collider(&self) -> Option<ColliderBuilder> {
        let planform = &self.body.planform;
        let up = self.up();
        let points: Vec<Point3<f32>> = planform
            .corners()
            .iter()
            .flat_map(|&(along, back)| {
                let center = self.on_aircraft((along, back));
                let half = up * (self.thickness * planform.chord_at(along) / 2.0);
                [Point3::from(center + half), Point3::from(center - half)]
            })
            .collect();
        ColliderBuilder::convex_hull(&points).map(|collider| collider.friction(0.8).density(0.0))
    }

    // Cl and Cd at `alpha` (deg): the section's, corrected for this surface's shape
    pub fn coefficients(&self, alpha: f32) -> (f32, f32) {
        let (mut cl, mut cd) = self.wing_coefficients(alpha);

        if self.vortex_lift > 0.0 && alpha > 0.0 {
            let angle = alpha.to_radians();
            let burst = 1.0 - ((alpha - VORTEX_BURST.0) / (VORTEX_BURST.1 - VORTEX_BURST.0)).clamp(0.0, 1.0);
            let extra = self.vortex_lift * angle.sin().powi(2) * angle.cos() * burst;
            cl += extra;
            cd += extra * angle.tan();
        }
        (cl, cd)
    }

    // The wing's own share, without the strakes' vortices
    fn wing_coefficients(&self, alpha: f32) -> (f32, f32) {
        let (cl, cd) = self.airfoil.sample(self.section_alpha(alpha));
        (cl, cd + cl * cl * self.induced_drag)
    }

    /*
        The same with the leading edge drooped `droop` degrees by a flap `chord_ratio` deep. Thin airfoil
        theory (Glauert), for a nose section hinged at cos θ = 1 − 2E:
        - the loading at the nose, what makes a thin wing like the F-16's stall there, drops as if the
          angle was δ·θ/π lower: the stall comes that much later, and the drag past it with it;
        - the extra camber costs a little lift at any one angle: ΔCl = −2δ(θ − sin θ).
        Below the stall both together give back the clean lift minus that small loss.
        The theory has the air following the bend all the way; past 10 deg it starts separating there, so
        the droop counts for less (the same loss the trailing edge sections take).
    */
    fn drooped_coefficients(&self, alpha: f32, droop: f32, chord_ratio: f32) -> (f32, f32) {
        let theta = (1.0 - 2.0 * chord_ratio).clamp(-1.0, 1.0).acos();
        let droop = (droop * separation_falloff(droop)).to_radians();
        let relief = droop * theta / PI;
        let camber = -2.0 * droop * (theta - theta.sin());

        let (cl, cd) = self.airfoil.sample(self.section_alpha(alpha) - relief.to_degrees());
        let cl = cl + self.section_slope * relief + camber;
        (cl, cd + cl * cl * self.induced_drag)
    }

    /*
        The angle the section itself feels: the tip vortices take part of the surface's angle away, so a
        short surface reaches its section's stall later. Past it, the rest of the way to 90 deg is
        stretched so the stalled, flat plate end of the table still lines up.
    */
    fn section_alpha(&self, alpha: f32) -> f32 {
        let (negative, positive) = self.stall;
        let surface_stall = |stall: f32| self.zero_lift + (stall - self.zero_lift) / self.alpha_scale;
        let (negative_3d, positive_3d) = (surface_stall(negative), surface_stall(positive));

        if alpha > positive_3d {
            positive + (alpha - positive_3d) * (90.0 - positive) / (90.0 - positive_3d)
        } else if alpha < negative_3d {
            negative + (alpha - negative_3d) * (-90.0 - negative) / (-90.0 - negative_3d)
        } else {
            self.zero_lift + (alpha - self.zero_lift) * self.alpha_scale
        }
    }

    // Its lift side: +Y for a wing or stabilator on either side, sideways for a fin
    fn up(&self) -> Vector3<f32> {
        let up = self.span_axis.cross(&-Vector3::z()).normalize();
        if up.y < 0.0 {
            -up
        } else {
            up
        }
    }

    /*
        The forces it makes, each with the point it acts at, both in the aircraft's space.
        `airflow` gives the air's velocity past a point (wind minus the aircraft's own motion there),
        so a turning aircraft's surfaces each feel their own share of the turn.
    */
    pub fn aero_forces(
        &self,
        airflow: impl Fn(Vector3<f32>) -> Vector3<f32>,
        density: f32,
    ) -> Vec<AeroForce> {
        let up = self.up();
        let planform = &self.body.planform;
        let mut forces = vec![];

        let center = self.on_aircraft(planform.aerodynamic_center());
        let own_deflection = match &self.body.motion {
            Motion::Pivoting { hinge } => hinge.deflection,
            Motion::Fixed { .. } => 0.0,
        };
        if let Some(flow) = Flow::new(airflow(center), up, density) {
            let alpha = flow.alpha + own_deflection;
            let (cl, cd) = self.coefficients(alpha);
            forces.push(flow.force(cl, cd, planform.area(), center, alpha, false));
        }

        // A deflected section changes its strip of the surface as if that strip met the air at another angle
        if let Motion::Fixed { sections } = &self.body.motion {
            for section in sections {
                let Motion::Pivoting { hinge } = &section.motion else {
                    continue;
                };
                if hinge.deflection.abs() < 1e-3 {
                    continue;
                }
                if section.edge == Edge::Leading {
                    // It changes how the whole strip behind it meets the air
                    let point = self.on_aircraft(section.strip_center(planform));
                    let Some(flow) = Flow::new(airflow(point), up, density) else {
                        continue;
                    };
                    let (base_cl, base_cd) = self.wing_coefficients(flow.alpha);
                    let (cl, cd) = self.drooped_coefficients(
                        flow.alpha,
                        hinge.deflection,
                        section.chord_ratio(planform),
                    );
                    forces.push(flow.force(
                        cl - base_cl,
                        cd - base_cd,
                        section.strip_area(planform),
                        point,
                        flow.alpha,
                        true,
                    ));
                    continue;
                }

                let point = self.on_aircraft(section.center());
                let Some(flow) = Flow::new(airflow(point), up, density) else {
                    continue;
                };
                let shift =
                    flap_effectiveness(section.chord_ratio(planform), hinge.deflection) * hinge.deflection;
                let (base_cl, base_cd) = self.coefficients(flow.alpha);
                let (cl, cd) = self.coefficients(flow.alpha + shift);
                forces.push(flow.force(
                    cl - base_cl,
                    cd - base_cd,
                    section.area(),
                    point,
                    flow.alpha + shift,
                    true,
                ));
            }
        }

        forces
    }

    pub fn on_aircraft(&self, (along_span, back): (f32, f32)) -> Vector3<f32> {
        self.root + self.span_axis.into_inner() * along_span + Vector3::z() * back
    }

    pub fn command(&mut self, commands: &Commands, dt: f32) {
        // right stick: left wing's flaperon goes down, right one goes up
        let side = -self.span_axis.x.signum();
        match &mut self.body.motion {
            Motion::Fixed { sections } => {
                for section in sections {
                    if let Some(hinge) = section.motion.hinge_mut() {
                        hinge.command(commands, side, dt);
                    }
                }
            }
            Motion::Pivoting { hinge } => hinge.command(commands, side, dt),
        }
    }

    pub fn place(&self, model: &mut Model) {
        match &self.body.motion {
            Motion::Fixed { sections } => {
                for section in sections {
                    if let Motion::Pivoting { hinge } = &section.motion {
                        hinge.place(model, self.hinge_axis(section));
                    }
                }
            }
            Motion::Pivoting { hinge } => hinge.place(model, self.pivot_axis()),
        }
    }

    // A section pivots about its leading edge (root to tip), in the aircraft's space; a leading edge
    // section the other way, so + droops its nose (its mesh's origin goes on its trailing edge)
    fn hinge_axis(&self, section: &Sections) -> UnitVector3<f32> {
        let [root, tip, ..] = section.corners();
        let axis = Self::trailing_edge_down(self.on_aircraft(tip) - self.on_aircraft(root));
        match section.edge {
            Edge::Trailing => axis,
            Edge::Leading => -axis,
        }
    }

    // A whole moving surface (stabilator, rudder) pivots about its span
    fn pivot_axis(&self) -> UnitVector3<f32> {
        Self::trailing_edge_down(self.span_axis.into_inner())
    }

    // Points the axis so a positive deflection moves the trailing edge down on either side:
    // turning about the axis carries the trailing edge (+z) along axis × z, which has to point down
    fn trailing_edge_down(axis: Vector3<f32>) -> UnitVector3<f32> {
        let axis = if axis.cross(&Vector3::z()).y > 0.0 {
            -axis
        } else {
            axis
        };
        UnitVector3::new_normalize(axis)
    }
}

// What a surface (or one of its sections) makes at one point, all in the aircraft's space
#[derive(Debug, Clone, Copy)]
pub struct AeroForce {
    pub point: Vector3<f32>,
    pub lift: Vector3<f32>,
    pub drag: Vector3<f32>,
    pub airflow: Vector3<f32>, // the air past it, in its own plane
    pub alpha: f32,
    pub section: bool, // a section's extra share rather than a whole surface
}

// The air past one point of a surface, split along the surface's chord (+Z, back) and its up
struct Flow {
    alpha: f32,          // degrees, + with the air coming from below its up
    lift: Vector3<f32>,  // square to the air, on its up side
    drag: Vector3<f32>,  // along the air
    pressure: f32,       // ½ρv² of the air across it; air along its span makes no lift
    speed: f32,
}

impl Flow {
    fn new(air: Vector3<f32>, up: Vector3<f32>, density: f32) -> Option<Flow> {
        let along = air.z;
        let across = air.dot(&up);
        let in_plane = Vector3::z() * along + up * across;
        let drag = in_plane.try_normalize(1e-3)?;
        Some(Flow {
            alpha: across.atan2(along).to_degrees(),
            lift: (up * along - Vector3::z() * across).normalize(),
            drag,
            pressure: 0.5 * density * in_plane.norm_squared(),
            speed: in_plane.norm(),
        })
    }

    fn force(&self, cl: f32, cd: f32, area: f32, point: Vector3<f32>, alpha: f32, section: bool) -> AeroForce {
        AeroForce {
            point,
            lift: self.lift * cl * self.pressure * area,
            drag: self.drag * cd * self.pressure * area,
            airflow: self.drag * self.speed,
            alpha,
            section,
        }
    }
}

// How much of a hinged section's deflection the strip feels as angle of attack (Glauert's thin airfoil
// theory for a section `chord_ratio` deep), losing some past 10 deg as the flow separates over the hinge
fn flap_effectiveness(chord_ratio: f32, deflection: f32) -> f32 {
    let theta = (2.0 * chord_ratio - 1.0).acos();
    let tau = 1.0 - (theta - theta.sin()) / PI;
    tau * separation_falloff(deflection)
}

// How much of a hinged section's deflection still works once the flow separates over its hinge: all of
// it up to 10 deg, falling to 60% by 25 deg
fn separation_falloff(deflection: f32) -> f32 {
    1.0 - 0.4 * ((deflection.abs() - 10.0) / 15.0).clamp(0.0, 1.0)
}
